//! Filesystem-backed cache for comic metadata, source images, and rendered backgrounds.

use std::{
    fs,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use super::comic::Comic;

/// Stores xkcd assets under a single root directory.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self::new(default_root())
    }
}

impl Store {
    /// Creates a cache rooted at `root`.
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Returns the file that caches the latest known xkcd number.
    fn latest_number_path(&self) -> PathBuf {
        self.root.join("latest").join("keep")
    }

    fn rendered_dir(&self) -> PathBuf {
        self.root.join("with_text")
    }

    /// Returns the cached source-image path for `comic`.
    pub fn image_path(&self, comic: &Comic) -> PathBuf {
        self.root.join(comic.filename())
    }

    /// Returns the cached rendered-background path for `comic`.
    pub fn rendered_path(&self, comic: &Comic) -> PathBuf {
        self.rendered_dir().join(comic.filename())
    }

    /// Ensures the cache root for downloaded comic images exists.
    pub fn ensure_images_dir(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// Ensures the cache root for rendered lockscreen backgrounds exists.
    pub fn ensure_rendered_dir(&self) -> anyhow::Result<()> {
        fs::create_dir_all(self.rendered_dir())?;
        Ok(())
    }

    /// Returns the cached latest comic number when the marker is younger than `max_age`.
    pub fn cached_latest_number(&self, max_age: Duration) -> anyhow::Result<Option<u32>> {
        let path = self.latest_number_path();
        let last_modified = match path.metadata().and_then(|metadata| metadata.modified()) {
            Ok(last_modified) => last_modified,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let since_last_modified = SystemTime::now().duration_since(last_modified)?;
        if since_last_modified > max_age {
            return Ok(None);
        }
        self.read_latest_number().map(Some)
    }

    /// Reads the latest comic number from disk.
    pub fn read_latest_number(&self) -> anyhow::Result<u32> {
        let path = self.latest_number_path();
        let bytes = fs::read(&path)?;
        let len = bytes.len();
        let bytes: [u8; 4] = bytes.try_into().map_err(|_| {
            anyhow::anyhow!(
                "latest cache at {} should contain 4 bytes, found {len}",
                path.display()
            )
        })?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// Stores the latest known comic number on disk.
    pub fn store_latest_number(&self, number: u32) -> anyhow::Result<()> {
        let path = self.latest_number_path();
        self.write_bytes_atomically(&path, &number.to_le_bytes())?;
        Ok(())
    }

    /// Stores full comic metadata so cached renders can be recreated offline.
    pub fn store_comic(&self, comic: &Comic) -> anyhow::Result<()> {
        let path = self.metadata_path(comic.number());
        self.write_bytes_atomically(&path, &serde_json::to_vec(comic)?)?;
        Ok(())
    }

    /// Stores a downloaded comic image without exposing partial files at the final cache path.
    pub fn store_image<R: Read>(&self, comic: &Comic, reader: &mut R) -> anyhow::Result<PathBuf> {
        let path = self.image_path(comic);
        self.write_reader_atomically(&path, reader)?;
        Ok(path)
    }

    /// Reserves a temporary path in the final directory so successful work can be renamed atomically.
    pub fn staged_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let (staged_path, staged_file) = self.create_staged_file(path)?;
        drop(staged_file);
        Ok(staged_path)
    }

    /// Publishes a staged file into its final cache location.
    pub fn commit_staged_path(&self, staged_path: &Path, path: &Path) -> anyhow::Result<()> {
        fs::rename(staged_path, path)?;
        Ok(())
    }

    /// Best-effort cleanup for an abandoned staged file.
    pub fn remove_staged_path(&self, staged_path: &Path) {
        let _ = fs::remove_file(staged_path);
    }

    /// Looks up a cached comic by metadata first, then by filename-derived fallback data.
    pub fn find_cached_comic(&self, number: u32) -> anyhow::Result<Option<Comic>> {
        if let Some(comic) = self.read_comic(number)? {
            return Ok(Some(comic));
        }

        let entries = match self.root.read_dir() {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                continue;
            }
            let Some(comic) = cached_comic(entry.path().as_path()) else {
                continue;
            };
            if comic.number() == number {
                return Ok(Some(comic));
            }
        }
        Ok(None)
    }

    fn metadata_path(&self, number: u32) -> PathBuf {
        self.root
            .join("metadata")
            .join(format!("{number:0>4}.json"))
    }

    fn read_comic(&self, number: u32) -> anyhow::Result<Option<Comic>> {
        let path = self.metadata_path(number);
        match fs::read(path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn write_bytes_atomically(&self, path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
        let mut reader = std::io::Cursor::new(bytes);
        self.write_reader_atomically(path, &mut reader)
    }

    fn write_reader_atomically<R: Read>(&self, path: &Path, reader: &mut R) -> anyhow::Result<()> {
        let (staged_path, mut staged_file) = self.create_staged_file(path)?;
        let result = (|| -> anyhow::Result<()> {
            std::io::copy(reader, &mut staged_file)?;
            staged_file.flush()?;
            staged_file.sync_all()?;
            drop(staged_file);
            fs::rename(&staged_path, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&staged_path);
        }
        result
    }

    fn create_staged_file(&self, path: &Path) -> anyhow::Result<(PathBuf, fs::File)> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("cache");
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        for attempt in 0..1024 {
            let staged_path = path.with_file_name(format!(".{file_name}.{unique}.{attempt}.tmp"));
            match fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&staged_path)
            {
                Ok(staged_file) => return Ok((staged_path, staged_file)),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow::anyhow!(
            "could not create a unique staged file for {}",
            path.display()
        ))
    }
}

/// Returns the default cache root under the user's pictures directory.
fn default_root() -> PathBuf {
    dirs::picture_dir()
        .expect("you should have a Pictures directory")
        .join("xkcd")
}

/// Builds a partial comic from an on-disk image filename.
fn cached_comic(path: &Path) -> Option<Comic> {
    let filename = path.file_name()?.to_str()?;
    let filename = filename.strip_suffix(".png")?;
    let (number, title) = filename.split_once(" - ")?;
    Some(Comic::from_cache(number.parse().ok()?, title.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Read, time::SystemTime};

    use super::*;

    fn test_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("xkcd-lock-{label}-{unique}"))
    }

    #[test]
    fn latest_number_roundtrip() {
        let root = test_dir("latest");
        let store = Store::new(root.clone());
        store.store_latest_number(42).unwrap();
        assert_eq!(store.read_latest_number().unwrap(), 42);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn metadata_roundtrip() {
        let root = test_dir("metadata");
        let store = Store::new(root.clone());
        let comic: Comic = serde_json::from_str(
            "{\"img\":\"https://imgs.xkcd.com/comics/test.png\",\"title\":\"Some Title\",\"alt\":\"Alt text\",\"num\":42}",
        )
        .unwrap();
        store.store_comic(&comic).unwrap();
        assert_eq!(store.find_cached_comic(42).unwrap(), Some(comic));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cached_comic_from_filename() {
        let root = test_dir("comic");
        let store = Store::new(root.clone());
        fs::create_dir_all(&store.root).unwrap();
        fs::write(store.root.join("0042 - Some Title.png"), []).unwrap();
        let comic = store.find_cached_comic(42).unwrap().unwrap();
        assert_eq!(comic.number(), 42);
        assert_eq!(comic.title(), "Some Title");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn truncated_latest_number_is_rejected() {
        let root = test_dir("truncated-latest");
        let store = Store::new(root.clone());
        fs::create_dir_all(root.join("latest")).unwrap();
        fs::write(root.join("latest").join("keep"), [1, 2, 3]).unwrap();
        let error = store.read_latest_number().unwrap_err();
        assert!(error.to_string().contains("should contain 4 bytes"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_atomic_write_leaves_existing_file_untouched() {
        let root = test_dir("failed-atomic-write");
        let store = Store::new(root.clone());
        let path = store.root.join("cache.bin");
        fs::create_dir_all(&store.root).unwrap();
        fs::write(&path, "existing").unwrap();

        let mut reader = ErrorAfterChunk::new(b"replacement");
        let error = store
            .write_reader_atomically(&path, &mut reader)
            .unwrap_err();
        assert_eq!(error.to_string(), "boom");
        assert_eq!(fs::read_to_string(&path).unwrap(), "existing");
        assert_eq!(fs::read_dir(&store.root).unwrap().count(), 1);
        let _ = fs::remove_dir_all(root);
    }

    struct ErrorAfterChunk<'a> {
        chunk: &'a [u8],
        finished_chunk: bool,
    }

    impl<'a> ErrorAfterChunk<'a> {
        fn new(chunk: &'a [u8]) -> Self {
            Self {
                chunk,
                finished_chunk: false,
            }
        }
    }

    impl Read for ErrorAfterChunk<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.finished_chunk {
                return Err(std::io::Error::other("boom"));
            }
            let len = self.chunk.len().min(buf.len());
            buf[..len].copy_from_slice(&self.chunk[..len]);
            self.finished_chunk = true;
            Ok(len)
        }
    }
}
