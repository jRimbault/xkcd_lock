//! xkcd domain types plus the downloader that fetches comics and reuses cache state.

use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use super::cache::Store;

const LATEST_TTL: Duration = Duration::from_secs(24 * 3600);

/// xkcd metadata we keep around so later locks can reuse a comic offline.
#[derive(Debug, Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Comic {
    img: String,
    title: String,
    alt: String,
    num: u32,
}

impl Comic {
    /// Returns the xkcd number.
    pub fn number(&self) -> u32 {
        self.num
    }

    /// Returns the comic title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the alt text used in rendered lockscreen backgrounds.
    pub fn alt(&self) -> &str {
        &self.alt
    }

    /// Returns the sanitized filename used for cached PNG assets.
    pub fn filename(&self) -> String {
        format!(
            "{:0>4} - {}.png",
            self.num,
            filename_title_fragment(&self.title)
        )
    }

    /// Builds a partial comic so an image-only cache hit can still be reused offline.
    pub fn from_cache(num: u32, title: String) -> Self {
        Self {
            title,
            num,
            ..Self::default()
        }
    }

    fn has_metadata(&self) -> bool {
        !self.img.is_empty()
    }
}

/// Looks up xkcd comics while preferring cached state so repeat locks stay fast and offline-friendly.
#[derive(Debug, Clone)]
pub struct Downloader {
    agent: ureq::Agent,
    cache: Store,
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
                log::debug!(number; "Comic metadata cache hit");
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
                    log::warn!(
                        number,
                        error:% = error;
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
            log::debug!(
                number = comic.number(),
                path:% = path.display();
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

        log::info!(
            number = comic.number(),
            path:% = path.display();
            "Downloading comic image"
        );
        let mut reader = self.agent.get(&comic.img).call()?.into_body();
        self.cache.store_image(comic, &mut reader.as_reader())
    }

    /// Reuses a recent latest-comic marker so random selection does not hit xkcd on every run.
    fn latest_number(&self) -> anyhow::Result<u32> {
        if let Some(number) = self.cache.cached_latest_number(LATEST_TTL)? {
            log::debug!(number; "Latest comic marker cache hit");
            return Ok(number);
        }

        log::debug!("Refreshing latest comic marker");
        match self.latest() {
            Ok(latest) => {
                self.cache.store_latest_number(latest.number())?;
                Ok(latest.number())
            }
            Err(error) => {
                log::warn!(
                    error:% = error;
                    "Latest comic marker refresh failed; using cached value"
                );
                self.cache.read_latest_number()
            }
        }
    }

    /// Fetches the current latest comic when the cached upper bound is too old to trust.
    fn latest(&self) -> anyhow::Result<Comic> {
        log::debug!("Fetching latest comic metadata from xkcd");
        Ok(self
            .agent
            .get("https://xkcd.com/info.0.json")
            .call()?
            .into_body()
            .read_json()?)
    }

    /// Fetches fresh metadata for a specific comic number from xkcd.
    fn fetch(&self, number: u32) -> anyhow::Result<Comic> {
        log::debug!(number; "Fetching comic metadata from xkcd");
        Ok(self
            .agent
            .get(&format!("https://xkcd.com/{number}/info.0.json"))
            .call()?
            .into_body()
            .read_json()?)
    }
}

/// Keeps cache filenames predictable enough to round-trip back into partial cached comics.
fn filename_title_fragment(title: &str) -> String {
    title
        .chars()
        .filter(|&character| character.is_alphanumeric() || character == ' ')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_title_fragment_is_idempotent() {
        let comic = Comic {
            title: "%$*alphanumericstuff-()-".to_owned(),
            ..Comic::default()
        };
        assert_eq!(comic.filename(), "0000 - alphanumericstuff.png");

        let comic = Comic {
            title: comic.filename()[7..comic.filename().len() - 4].to_owned(),
            ..Comic::default()
        };
        assert_eq!(comic.filename(), "0000 - alphanumericstuff.png");
    }
}
