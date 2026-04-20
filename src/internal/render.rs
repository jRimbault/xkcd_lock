//! Rendering support that turns a downloaded comic image plus metadata into a lockscreen background.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use super::{cache::Store, comic::Comic};

/// Produces rendered backgrounds and caches them on disk.
#[derive(Debug, Clone)]
pub struct BackgroundRenderer {
    cache: Store,
}

impl Default for BackgroundRenderer {
    fn default() -> Self {
        Self::new(Store::default())
    }
}

impl BackgroundRenderer {
    /// Creates a renderer backed by `cache`.
    pub fn new(cache: Store) -> Self {
        Self { cache }
    }

    /// Renders a comic title and alt text onto the downloaded image when no cached render exists.
    pub fn render(&self, comic: &Comic, image: &Path) -> anyhow::Result<PathBuf> {
        let output = self.cache.rendered_path(comic);
        if output.exists() {
            log::info!("using cache of background comic #{}", comic.number());
            return Ok(output);
        }

        log::info!(
            "writting background version of comic #{} to cache",
            comic.number()
        );
        self.cache.ensure_rendered_dir()?;
        let alt = textwrap::wrap(comic.alt(), 70).join("\n");
        let staged = self.cache.staged_path(&output)?;
        let result = (|| -> anyhow::Result<()> {
            let status = Command::new("convert")
                .args(["-size", "1920x1080", "xc:white"])
                .arg(image)
                .args([
                    "-gravity",
                    "center",
                    "-gravity",
                    "center",
                    "-composite",
                    "-gravity",
                    "north",
                    "-pointsize",
                    "36",
                    "-annotate",
                    "+0+100",
                ])
                .arg(comic.title())
                .args([
                    "-gravity",
                    "south",
                    "-pointsize",
                    "20",
                    "-annotate",
                    "+0+100",
                ])
                .arg(alt)
                .arg(&staged)
                .status()?;
            anyhow::ensure!(status.success(), "convert exited with {status}");
            self.cache.commit_staged_path(&staged, &output)?;
            Ok(())
        })();
        if result.is_err() {
            self.cache.remove_staged_path(&staged);
        }
        result?;
        Ok(output)
    }
}
