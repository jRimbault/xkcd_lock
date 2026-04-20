//! Filesystem-backed cache for comic metadata, source images, and rendered backgrounds.

use std::{
    fs,
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use super::comic::Comic;

/// Keeps downloaded comics, metadata, and rendered backgrounds together so later runs can reuse them.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

/// Summary of whether the on-disk cache still looks reusable.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CacheHealth {
    root: PathBuf,
    latest_marker: LatestMarkerHealth,
    images: CacheSectionHealth,
    metadata: CacheSectionHealth,
    rendered: CacheSectionHealth,
    staged_files: Vec<PathBuf>,
}

impl CacheHealth {
    /// Returns the cache root that was inspected.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the status of the cached latest-comic marker.
    pub fn latest_marker(&self) -> &LatestMarkerHealth {
        &self.latest_marker
    }

    /// Returns the status of cached raw comic images.
    pub fn images(&self) -> &CacheSectionHealth {
        &self.images
    }

    /// Returns the status of cached comic metadata files.
    pub fn metadata(&self) -> &CacheSectionHealth {
        &self.metadata
    }

    /// Returns the status of rendered backgrounds.
    pub fn rendered(&self) -> &CacheSectionHealth {
        &self.rendered
    }

    /// Returns leftover staged files from interrupted atomic writes.
    pub fn staged_files(&self) -> &[PathBuf] {
        &self.staged_files
    }

    /// Returns whether the cache can be trusted without cleanup or repair.
    pub fn is_healthy(&self) -> bool {
        !matches!(self.latest_marker, LatestMarkerHealth::Invalid(_))
            && self.images.invalid_entries().is_empty()
            && self.metadata.invalid_entries().is_empty()
            && self.rendered.invalid_entries().is_empty()
            && self.staged_files.is_empty()
    }
}

/// Health status for one cache section.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct CacheSectionHealth {
    valid_entries: usize,
    invalid_entries: Vec<PathBuf>,
}

impl CacheSectionHealth {
    /// Returns how many entries in this section looked valid.
    pub fn valid_entries(&self) -> usize {
        self.valid_entries
    }

    /// Returns relative paths for entries that looked malformed.
    pub fn invalid_entries(&self) -> &[PathBuf] {
        &self.invalid_entries
    }
}

/// Health status for the cached latest-comic marker.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum LatestMarkerHealth {
    /// No latest-comic marker has been cached yet.
    Missing,
    /// The marker exists and can be read.
    Valid(u32),
    /// The marker exists but is malformed.
    Invalid(String),
}

impl Default for Store {
    fn default() -> Self {
        Self::new(default_root())
    }
}

impl Store {
    /// Creates a cache rooted at `root` so tests and callers can isolate on-disk state.
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Returns the marker that bounds random comic selection without a fresh network lookup.
    fn latest_number_path(&self) -> PathBuf {
        self.root.join("latest").join("keep")
    }

    fn rendered_dir(&self) -> PathBuf {
        self.root.join("with_text")
    }

    /// Returns where the raw downloaded comic image should live.
    pub fn image_path(&self, comic: &Comic) -> PathBuf {
        self.root.join(comic.filename())
    }

    /// Returns where the text-rendered background should live.
    pub fn rendered_path(&self, comic: &Comic) -> PathBuf {
        self.rendered_dir().join(comic.filename())
    }

    /// Ensures the image cache exists before we try to populate it.
    pub fn ensure_images_dir(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// Ensures the rendered-background cache exists before `convert` writes into it.
    pub fn ensure_rendered_dir(&self) -> anyhow::Result<()> {
        fs::create_dir_all(self.rendered_dir())?;
        Ok(())
    }

    /// Reuses a recent latest-comic marker so random selection can stay offline for a while.
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

    /// Reads the latest-comic marker and rejects truncated state instead of silently misreading it.
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

    /// Stores the latest-comic marker atomically so interrupted writes do not poison later runs.
    pub fn store_latest_number(&self, number: u32) -> anyhow::Result<()> {
        let path = self.latest_number_path();
        self.write_bytes_atomically(&path, &number.to_le_bytes())?;
        Ok(())
    }

    /// Stores full comic metadata atomically so text rendering can be recreated offline later.
    pub fn store_comic(&self, comic: &Comic) -> anyhow::Result<()> {
        let path = self.metadata_path(comic.number());
        self.write_bytes_atomically(&path, &serde_json::to_vec(comic)?)?;
        Ok(())
    }

    /// Inspects the cache for malformed entries and abandoned staged files.
    pub fn health(&self) -> anyhow::Result<CacheHealth> {
        Ok(CacheHealth {
            root: self.root.clone(),
            latest_marker: self.latest_marker_health(),
            images: self.image_health()?,
            metadata: self.metadata_health()?,
            rendered: self.rendered_health()?,
            staged_files: self.staged_files()?,
        })
    }

    /// Publishes a downloaded image only after the full file is present on disk.
    pub fn store_image<R: Read>(&self, comic: &Comic, reader: &mut R) -> anyhow::Result<PathBuf> {
        let path = self.image_path(comic);
        self.write_reader_atomically(&path, reader)?;
        Ok(path)
    }

    /// Reserves a staging path in the final directory so success can be published with a rename.
    pub fn staged_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let (staged_path, staged_file) = self.create_staged_file(path)?;
        drop(staged_file);
        Ok(staged_path)
    }

    /// Publishes completed staged work into the cache in one step.
    pub fn commit_staged_path(&self, staged_path: &Path, path: &Path) -> anyhow::Result<()> {
        fs::rename(staged_path, path)?;
        Ok(())
    }

    /// Cleans up abandoned staged work so failed attempts do not accumulate in the cache.
    pub fn remove_staged_path(&self, staged_path: &Path) {
        let _ = fs::remove_file(staged_path);
    }

    /// Finds the best cached representation of a comic, even when only the image filename survives.
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

    fn latest_marker_health(&self) -> LatestMarkerHealth {
        let path = self.latest_number_path();
        if !path.exists() {
            return LatestMarkerHealth::Missing;
        }
        match self.read_latest_number() {
            Ok(number) => LatestMarkerHealth::Valid(number),
            Err(error) => LatestMarkerHealth::Invalid(error.to_string()),
        }
    }

    fn image_health(&self) -> anyhow::Result<CacheSectionHealth> {
        self.section_health(&self.root, |path| cached_comic(path).is_some())
    }

    fn metadata_health(&self) -> anyhow::Result<CacheSectionHealth> {
        self.section_health(&self.root.join("metadata"), |path| {
            let extension = path.extension().and_then(|extension| extension.to_str());
            let stem = path.file_stem().and_then(|stem| stem.to_str());
            if extension != Some("json") || stem.and_then(|stem| stem.parse::<u32>().ok()).is_none()
            {
                return false;
            }
            match fs::read(path) {
                Ok(bytes) => serde_json::from_slice::<Comic>(&bytes).is_ok(),
                Err(_) => false,
            }
        })
    }

    fn rendered_health(&self) -> anyhow::Result<CacheSectionHealth> {
        self.section_health(&self.rendered_dir(), |path| cached_comic(path).is_some())
    }

    fn section_health<F>(&self, dir: &Path, is_valid: F) -> anyhow::Result<CacheSectionHealth>
    where
        F: Fn(&Path) -> bool,
    {
        let entries = match dir.read_dir() {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(CacheSectionHealth::default());
            }
            Err(error) => return Err(error.into()),
        };

        let mut health = CacheSectionHealth::default();
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path();
            if is_staged_file(&path) {
                continue;
            }
            if is_valid(&path) {
                health.valid_entries += 1;
            } else {
                health.invalid_entries.push(self.relative_path(&path));
            }
        }
        health.invalid_entries.sort();
        Ok(health)
    }

    fn staged_files(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut staged_files = Vec::new();
        self.collect_staged_files(&self.root, &mut staged_files)?;
        staged_files.sort();
        Ok(staged_files)
    }

    fn collect_staged_files(
        &self,
        dir: &Path,
        staged_files: &mut Vec<PathBuf>,
    ) -> anyhow::Result<()> {
        let entries = match dir.read_dir() {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                self.collect_staged_files(&path, staged_files)?;
            } else if is_staged_file(&path) {
                staged_files.push(self.relative_path(&path));
            }
        }
        Ok(())
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

    fn relative_path(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root).unwrap_or(path).to_path_buf()
    }
}

/// Chooses a picture-oriented cache root because the stored artifacts are user-visible images.
fn default_root() -> PathBuf {
    dirs::picture_dir()
        .expect("you should have a Pictures directory")
        .join("xkcd")
}

/// Reconstructs enough comic identity from a cached filename to keep offline fallback working.
fn cached_comic(path: &Path) -> Option<Comic> {
    let filename = path.file_name()?.to_str()?;
    let filename = filename.strip_suffix(".png")?;
    let (number, title) = filename.split_once(" - ")?;
    Some(Comic::from_cache(number.parse().ok()?, title.to_owned()))
}

fn is_staged_file(path: &Path) -> bool {
    let Some(filename) = path.file_name().and_then(|filename| filename.to_str()) else {
        return false;
    };
    filename.starts_with('.') && filename.ends_with(".tmp")
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
    fn health_accepts_valid_cache() {
        let root = test_dir("health-ok");
        let store = Store::new(root.clone());
        let comic: Comic = serde_json::from_str(
            "{\"img\":\"https://imgs.xkcd.com/comics/test.png\",\"title\":\"Some Title\",\"alt\":\"Alt text\",\"num\":42}",
        )
        .unwrap();
        store.store_latest_number(42).unwrap();
        store.store_comic(&comic).unwrap();
        fs::write(store.image_path(&comic), []).unwrap();
        store.ensure_rendered_dir().unwrap();
        fs::write(store.rendered_path(&comic), []).unwrap();

        let health = store.health().unwrap();

        assert!(health.is_healthy());
        assert_eq!(health.images().valid_entries(), 1);
        assert_eq!(health.metadata().valid_entries(), 1);
        assert_eq!(health.rendered().valid_entries(), 1);
        assert!(health.staged_files().is_empty());
        assert_eq!(health.latest_marker(), &LatestMarkerHealth::Valid(42));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn health_reports_invalid_entries() {
        let root = test_dir("health-invalid");
        let store = Store::new(root.clone());
        fs::create_dir_all(root.join("latest")).unwrap();
        fs::write(root.join("latest").join("keep"), [1, 2, 3]).unwrap();
        fs::create_dir_all(&store.root).unwrap();
        fs::write(store.root.join("not-a-comic.txt"), []).unwrap();
        fs::create_dir_all(root.join("metadata")).unwrap();
        fs::write(root.join("metadata").join("oops.json"), "{").unwrap();
        fs::create_dir_all(root.join("with_text")).unwrap();
        fs::write(root.join("with_text").join("broken.png"), []).unwrap();
        let staged = store
            .staged_path(&store.root.join("0001 - Example.png"))
            .unwrap();

        let health = store.health().unwrap();

        assert!(!health.is_healthy());
        assert!(matches!(
            health.latest_marker(),
            LatestMarkerHealth::Invalid(error) if error.contains("should contain 4 bytes")
        ));
        assert_eq!(
            health.images().invalid_entries(),
            &[PathBuf::from("not-a-comic.txt")]
        );
        assert_eq!(
            health.metadata().invalid_entries(),
            &[PathBuf::from("metadata").join("oops.json")]
        );
        assert_eq!(
            health.rendered().invalid_entries(),
            &[PathBuf::from("with_text").join("broken.png")]
        );
        assert_eq!(
            health.staged_files(),
            &[staged.strip_prefix(&root).unwrap().to_path_buf()]
        );
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
