use std::{path::PathBuf, time::Duration};

use super::Comic;

use crate::Store;

const LATEST_TTL: Duration = Duration::from_secs(24 * 3600);

/// Looks up xkcd comics while preferring cached state so repeat locks stay fast and offline-friendly.
#[derive(Debug, Clone)]
pub struct Downloader {
    pub(crate) agent: ureq::Agent,
    pub(crate) cache: Store,
}

impl Default for Downloader {
    fn default() -> Self {
        Self::new(Store::default())
    }
}

impl Downloader {
    /// Creates a downloader that shares cached metadata and images with other components.
    pub fn new(cache: Store) -> Self {
        Self {
            agent: ureq::Agent::new_with_defaults(),
            cache,
        }
    }

    /// Picks a random comic using the latest known upper bound, refreshing it only when needed.
    pub fn random(&self) -> anyhow::Result<Comic> {
        let latest = self.latest_number()?;
        let number = rand::random_range(1..=latest);
        self.by_number(number)
    }

    /// Returns metadata for a specific comic while falling back to stale cache when the network is unavailable.
    pub fn by_number(&self, number: u32) -> anyhow::Result<Comic> {
        let cached = self.cache.find_cached_comic(number)?;
        if let Some(comic) = &cached {
            if comic.has_metadata() {
                tracing::debug!(number, "Comic metadata cache hit");
                return Ok(comic.clone());
            }
        }

        match self.fetch(number) {
            Ok(comic) => {
                self.cache.store_comic(&comic)?;
                Ok(comic)
            }
            Err(error) => match cached {
                Some(comic) => {
                    tracing::warn!(
                        number,
                        error = %error,
                        "Comic metadata refresh failed; using cached image-only entry"
                    );
                    Ok(comic)
                }
                None => Err(error),
            },
        }
    }

    /// Ensures the comic image is cached locally before rendering or locking with it.
    pub fn download(&self, comic: &Comic) -> anyhow::Result<PathBuf> {
        let path = self.cache.image_path(comic);
        if path.exists() {
            tracing::debug!(
                number = comic.number(),
                path = %path.display(),
                "Comic image cache hit"
            );
            return Ok(path);
        }

        if comic.img.is_empty() {
            anyhow::bail!(
                "comic #{} is missing a downloadable image URL",
                comic.number()
            );
        }

        tracing::info!(
            number = comic.number(),
            path = %path.display(),
            "Downloading comic image"
        );
        let mut reader = self.agent.get(&comic.img).call()?.into_body();
        self.cache.store_image(comic, &mut reader.as_reader())
    }

    /// Reuses a recent latest-comic marker so random selection does not hit xkcd on every run.
    pub(crate) fn latest_number(&self) -> anyhow::Result<u32> {
        if let Some(number) = self.cache.cached_latest_number(LATEST_TTL)? {
            tracing::debug!(number, "Latest comic marker cache hit");
            return Ok(number);
        }

        tracing::debug!("Refreshing latest comic marker");
        match self.latest() {
            Ok(latest) => {
                self.cache.store_latest_number(latest.number())?;
                Ok(latest.number())
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "Latest comic marker refresh failed; using cached value"
                );
                self.cache.read_latest_number()
            }
        }
    }

    /// Fetches the current latest comic when the cached upper bound is too old to trust.
    pub(crate) fn latest(&self) -> anyhow::Result<Comic> {
        tracing::debug!("Fetching latest comic metadata from xkcd");
        Ok(self
            .agent
            .get("https://xkcd.com/info.0.json")
            .call()?
            .into_body()
            .read_json()?)
    }

    /// Fetches fresh metadata for a specific comic number from xkcd.
    pub(crate) fn fetch(&self, number: u32) -> anyhow::Result<Comic> {
        tracing::debug!(number, "Fetching comic metadata from xkcd");
        Ok(self
            .agent
            .get(&format!("https://xkcd.com/{number}/info.0.json"))
            .call()?
            .into_body()
            .read_json()?)
    }
}
