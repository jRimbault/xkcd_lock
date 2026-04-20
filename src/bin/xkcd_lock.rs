use std::path::PathBuf;

use clap::Parser;

/// Lock the screen with a cached or freshly downloaded xkcd comic.
///
/// By default, `xkcd_lock` picks a random comic, downloads it if needed,
/// renders the title and alt text onto the image, and passes the result to
/// the selected lockscreen backend.
#[derive(Debug, Parser)]
struct App {
    /// Select the lockscreen backend explicitly.
    ///
    /// If omitted, `xkcd_lock` chooses a backend from `XDG_SESSION_TYPE`.
    #[command(subcommand)]
    locker: Option<Locker>,
    /// Use this local image instead of selecting an xkcd comic.
    ///
    /// This skips comic lookup, download, and text rendering.
    #[arg(short, long, conflicts_with = "number")]
    image: Option<PathBuf>,
    /// Lock with a specific xkcd comic number.
    ///
    /// This may use the network unless the comic is already cached locally.
    #[arg(short, long, conflicts_with = "image")]
    number: Option<u32>,
    /// Ask `swaylock` to detach after locking.
    ///
    /// This flag is currently ignored by the `i3lock` backend.
    #[arg(short = 'f', long)]
    daemonize: bool,
}

#[derive(Debug, Parser)]
enum Locker {
    /// Use `swaylock` as the lockscreen backend.
    Sway,
    /// Use `i3lock` as the lockscreen backend.
    I3,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let app = App::parse();
    let (sender, receiver) = std::sync::mpsc::channel();
    ctrlc::set_handler(move || sender.send(()).unwrap())?;
    log::debug!("{:#?}", app);
    let cache = xkcd_lock::Store::default();
    let downloader = xkcd_lock::Downloader::new(cache.clone());
    let renderer = xkcd_lock::BackgroundRenderer::new(cache);
    let file = {
        if let Some(image) = &app.image {
            image.to_owned()
        } else if let Some(n) = app.number {
            let comic = downloader.by_number(n)?;
            log::debug!("{:#?}", comic);
            let file = downloader.download(&comic)?;
            log::debug!("{:?}", file);
            renderer.render(&comic, &file)?
        } else {
            let comic = downloader.random()?;
            log::debug!("{:#?}", comic);
            let file = downloader.download(&comic)?;
            log::debug!("{:?}", file);
            renderer.render(&comic, &file)?
        }
    };
    log::debug!("{:?}", file);
    log::info!("locking screen");
    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    let kind = xkcd_lock::resolve(app.locker.map(Into::into), session_type.as_deref())?;
    xkcd_lock::lock(
        kind,
        &file,
        xkcd_lock::LockOptions::new(app.daemonize, Some(receiver)),
    )
}

impl From<Locker> for xkcd_lock::Kind {
    fn from(value: Locker) -> Self {
        match value {
            Locker::Sway => Self::Sway,
            Locker::I3 => Self::I3,
        }
    }
}
