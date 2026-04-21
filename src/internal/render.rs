//! Rendering support that turns a downloaded comic image plus metadata into a lockscreen background.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use super::{cache::Store, comic::Comic};

/// Produces cached lockscreen-ready backgrounds so repeated locks avoid rerunning ImageMagick.
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
    /// Creates a renderer that publishes finished backgrounds into the shared cache.
    pub fn new(cache: Store) -> Self {
        Self { cache }
    }

    /// Adds the comic title and alt text once so later locks can reuse the finished background.
    pub fn render(&self, comic: &Comic, image: &Path) -> anyhow::Result<PathBuf> {
        let output = self.cache.rendered_path(comic);
        if output.exists() {
            tracing::debug!(
                number = comic.number(),
                path = %output.display(),
                "Background render cache hit"
            );
            return Ok(output);
        }

        tracing::info!(
            number = comic.number(),
            path = %output.display(),
            "Rendering background"
        );
        self.cache.ensure_rendered_dir()?;
        let alt = textwrap::wrap(comic.alt(), 70).join("\n");
        let staged = self.cache.staged_path(&output)?;
        let result = self.convert(comic, image, &output, alt, &staged);
        if let Err(e) = result {
            tracing::error!(error = %e, "Rendering error");
            self.cache.remove_staged_path(&staged);
            return Err(e);
        }
        Ok(output)
    }

    fn convert(
        &self,
        comic: &Comic,
        image: &Path,
        output: &Path,
        alt: String,
        staged: &Path,
    ) -> Result<(), anyhow::Error> {
        let staged_output = format!("png:{}", staged.display());
        let command_output = Command::new("convert")
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
            .arg(&staged_output)
            .output()?;
        if !command_output.status.success() {
            tracing::error!(
                status = %command_output.status,
                stdout = command_text(&command_output.stdout),
                stderr = command_text(&command_output.stderr),
                "convert failed"
            );
            anyhow::bail!("convert exited with {}", command_output.status);
        }
        self.cache.commit_staged_path(staged, output)?;
        Ok(())
    }
}

fn command_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}
