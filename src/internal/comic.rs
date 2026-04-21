//! xkcd domain types plus the downloader that fetches comics and reuses cache state.
mod downloader;

pub use downloader::Downloader;
use serde::{Deserialize, Serialize};

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
