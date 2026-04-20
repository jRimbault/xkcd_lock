//! Core library for downloading, caching, rendering, and locking with xkcd art.

mod internal {
    mod cache;
    mod comic;
    mod locker;
    mod render;

    pub use cache::Store;
    pub use comic::{Comic, Downloader};
    pub use locker::{lock, resolve, Kind, LockOptions};
    pub use render::BackgroundRenderer;
}

pub use internal::{
    lock, resolve, BackgroundRenderer, Comic, Downloader, Kind, LockOptions, Store,
};
