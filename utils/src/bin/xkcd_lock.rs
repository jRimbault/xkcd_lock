use std::process::Command;

use clap::Parser;

#[derive(Debug, Parser)]
struct App {
    /// Background image to display on all monitors
    #[arg(long, env)]
    bg_lock_image: String,
    #[command(subcommand)]
    locker: Option<Locker>,
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
    log::debug!("{:#?}", app);
    let comic = utils::comic::Xkcd::random()?;
    log::debug!("{:#?}", comic);
    let file = comic.download()?;
    log::debug!("{:?}", file);
    let file = comic.write_to_file_as_bg(&file)?;
    log::debug!("{:?}", file);
    let file = file.to_string_lossy();
    let displays = utils::displays()?;
    let mut displays = displays.into_iter();
    let displays: Vec<_> = displays
        .next()
        .into_iter()
        .map(|d| ["-i".to_owned(), format!("{}:{}", d, file)])
        .chain(displays.map(|d| ["-i".to_owned(), format!("{}:{}", d, app.bg_lock_image)]))
        .flatten()
        .collect();
    log::debug!("{:#?}", displays);
    log::info!("locking screen");
    match (app.locker, std::env::var("XDG_SESSION_TYPE").as_deref()) {
        (Some(Locker::Sway), _) => swaylock(&displays),
        (Some(Locker::I3), _) => i3lock(&displays),
        (None, Ok("wayland")) => swaylock(&displays),
        (None, Ok("x11")) => i3lock(&displays),
        (None, Ok(session)) => Err(anyhow::anyhow!("unknown session type {session:?}")),
        (None, Err(_)) => Err(anyhow::anyhow!(
            "no XDG_SESSION_TYPE set and no locker was specified by the user"
        )),
    }
}

fn swaylock(displays: &[String]) -> anyhow::Result<()> {
    Command::new("swaylock")
        .args([
            "--ignore-empty-password",
            "--show-failed-attempts",
            "--daemonize",
            "-s",
            "center",
        ])
        .args(displays)
        .spawn()?
        .wait()?;
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
