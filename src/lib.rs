//! Core library for turning xkcd comics into reusable lockscreen backgrounds.

mod internal {
    mod cache;
    mod comic;
    mod locker;
    mod render;

    pub use cache::{CacheHealth, CacheSectionHealth, LatestMarkerHealth, Store};
    pub use comic::{Comic, Downloader};
    pub use locker::{lock, resolve, Kind, LockOptions};
    pub use render::BackgroundRenderer;
}

pub use internal::{
    lock, resolve, BackgroundRenderer, CacheHealth, CacheSectionHealth, Comic, Downloader, Kind,
    LatestMarkerHealth, LockOptions, Store,
};
