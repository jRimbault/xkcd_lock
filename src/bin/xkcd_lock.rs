use std::{path::PathBuf, process::Command, sync::mpsc::Receiver, thread};

use clap::Parser;
use tap::Pipe;

#[derive(Debug, Parser)]
struct App {
    #[command(subcommand)]
    locker: Option<Locker>,
    /// Override everything and use this image instead
    ///
    /// Allows some fully offline use-cases
    #[arg(short, long, conflicts_with = "number")]
    image: Option<PathBuf>,
    /// Override everything and get this xkcd specifically instead
    ///
    /// Requires network if not already in cache
    #[arg(short, long, conflicts_with = "image")]
    number: Option<u32>,
    #[arg(short = 'f', long)]
    daemonize: bool,
}

#[derive(Debug, Parser)]
enum Locker {
    /// Use swaylock
    Sway,
    /// Use i3lock
    I3,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let app = App::parse();
    let (sender, receiver) = std::sync::mpsc::channel();
    ctrlc::set_handler(move || sender.send(()).unwrap())?;
    log::debug!("{:#?}", app);
    let file = {
        if let Some(image) = &app.image {
            image.to_owned()
        } else if let Some(n) = app.number {
            let comic = utils::comic::Xkcd::number(n)?;
            log::debug!("{:#?}", comic);
            let file = comic.download()?;
            log::debug!("{:?}", file);
            comic.write_to_file_as_bg(&file)?
        } else {
            let comic = utils::comic::Xkcd::random()?;
            log::debug!("{:#?}", comic);
            let file = comic.download()?;
            log::debug!("{:?}", file);
            comic.write_to_file_as_bg(&file)?
        }
    };
    log::debug!("{:?}", file);
    let file = file.to_string_lossy();
    let displays = utils::displays()?;
    let mut displays = displays.into_iter();
    let displays: Vec<_> = displays
        .next()
        .into_iter()
        .map(|d| ["-i".to_owned(), format!("{}:{}", d, file)])
        .chain(displays.map(|d| ["-i".to_owned(), format!("{}:{}", d, file)]))
        .flatten()
        .collect();
    log::debug!("{:#?}", displays);
    log::info!("locking screen");
    match (&app.locker, std::env::var("XDG_SESSION_TYPE").as_deref()) {
        (Some(Locker::Sway), _) => swaylock(&displays, receiver, &app),
        (Some(Locker::I3), _) => i3lock(&displays),
        (None, Ok("wayland")) => swaylock(&displays, receiver, &app),
        (None, Ok("x11")) => i3lock(&displays),
        (None, Ok(session)) => Err(anyhow::anyhow!("unknown session type {session:?}")),
        (None, Err(_)) => Err(anyhow::anyhow!(
            "no XDG_SESSION_TYPE set and no locker was specified by the user"
        )),
    }
}

fn swaylock(displays: &[String], kill: Receiver<()>, app: &App) -> anyhow::Result<()> {
    let mut lockscreen = Command::new("swaylock")
        .pipe(|mut a| {
            if app.daemonize {
                a.arg("--daemonize");
            }
            a
        })
        .args([
            "--ignore-empty-password",
            "--show-failed-attempts",
            "-s",
            "center",
        ])
        .args(displays)
        .spawn()?;
    let id = lockscreen.id();
    let _ = thread::spawn(move || {
        if kill.recv().is_ok() {
            Command::new("kill")
                .args(["-s", "TERM"])
                .arg(id.to_string())
                .spawn()
                .unwrap()
                .wait()
                .unwrap();
        }
    });
    lockscreen.wait()?;
    Ok(())
}

fn i3lock(displays: &[String]) -> anyhow::Result<()> {
    Command::new("i3lock")
        .args([
            "--textcolor=00000000",
            "--insidecolor=00000000",
            "--ringcolor=fafafaff",
            "--linecolor=00000000",
            "--keyhlcolor=fabb5cff",
            "--ringvercolor=fadd5cff",
            "--separatorcolor=00000000",
            "--insidevercolor=00000000",
            "--ringwrongcolor=f13459ff",
            "--insidewrongcolor=00000000",
        ])
        .args(displays)
        .spawn()?
        .wait()?;
    Ok(())
}
