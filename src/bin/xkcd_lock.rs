use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Lock the screen with a cached or freshly downloaded xkcd comic.
///
/// By default, `xkcd_lock` picks a random comic, downloads it if needed,
/// renders the title and alt text onto the image, and passes the result to
/// the selected lockscreen backend.
#[derive(Debug, Parser)]
struct App {
    #[command(flatten)]
    verbosity: clap_verbosity_flag::Verbosity,
    /// Select a lockscreen backend explicitly or inspect cache state.
    #[command(subcommand)]
    command: Option<Command>,
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

#[derive(Debug, Subcommand)]
enum Command {
    /// Use `swaylock` as the lockscreen backend.
    Sway,
    /// Use `i3lock` as the lockscreen backend.
    I3,
    /// Inspect the on-disk comic cache.
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
}

#[derive(Debug, Subcommand)]
enum CacheCommand {
    /// Check whether cached files still look reusable.
    Health,
}

fn main() -> anyhow::Result<()> {
    let app = App::parse();
    init_logger(&app.verbosity);
    let (sender, receiver) = std::sync::mpsc::channel();
    ctrlc::set_handler(move || sender.send(()).unwrap())?;
    let requested_command = app.command.as_ref().map(command_name);
    log::debug!(
        command:? = requested_command,
        image:? = app.image.as_ref(),
        number:? = app.number,
        daemonize = app.daemonize,
        verbosity:% = app.verbosity.log_level_filter();
        "Parsed CLI options"
    );
    if let Some(Command::Cache {
        command: CacheCommand::Health,
    }) = &app.command
    {
        ensure_cache_health_args(&app)?;
        return run_cache_health();
    }

    run_lock(app, receiver)
}

fn run_lock(app: App, receiver: std::sync::mpsc::Receiver<()>) -> anyhow::Result<()> {
    let cache = xkcd_lock::Store::default();
    let downloader = xkcd_lock::Downloader::new(cache.clone());
    let renderer = xkcd_lock::BackgroundRenderer::new(cache);
    let requested_backend = app.command.as_ref().and_then(lock_command_name);
    let file = {
        if let Some(image) = &app.image {
            log::info!(path:% = image.display(); "Using image override");
            image.to_owned()
        } else if let Some(n) = app.number {
            let comic = downloader.by_number(n)?;
            log::info!(
                number = comic.number(),
                title = comic.title();
                "Selected requested comic"
            );
            let file = downloader.download(&comic)?;
            renderer.render(&comic, &file)?
        } else {
            let comic = downloader.random()?;
            log::info!(
                number = comic.number(),
                title = comic.title();
                "Selected random comic"
            );
            let file = downloader.download(&comic)?;
            renderer.render(&comic, &file)?
        }
    };
    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    let kind = xkcd_lock::resolve(command_kind(app.command), session_type.as_deref())?;
    log::debug!(
        requested:? = requested_backend,
        session_type:? = session_type,
        backend = kind_name(kind);
        "Resolved lock backend"
    );
    log::info!(
        backend = kind_name(kind),
        image:% = file.display();
        "Starting lockscreen"
    );
    xkcd_lock::lock(
        kind,
        &file,
        xkcd_lock::LockOptions::new(app.daemonize, Some(receiver)),
    )
}

fn run_cache_health() -> anyhow::Result<()> {
    let health = xkcd_lock::Store::default().health()?;
    print_cache_health(&health);
    if health.is_healthy() {
        Ok(())
    } else {
        anyhow::bail!("cache health check failed")
    }
}

fn ensure_cache_health_args(app: &App) -> anyhow::Result<()> {
    if let Some(image) = &app.image {
        anyhow::bail!(
            "--image cannot be used with `cache health` (received {})",
            image.display()
        );
    }
    if let Some(number) = app.number {
        anyhow::bail!("--number cannot be used with `cache health` (received {number})");
    }
    if app.daemonize {
        anyhow::bail!("--daemonize cannot be used with `cache health`");
    }
    Ok(())
}

fn init_logger(verbosity: &clap_verbosity_flag::Verbosity) {
    let default_filter = verbosity
        .log_level_filter()
        .to_string()
        .to_ascii_lowercase();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter))
        .init();
}

fn print_cache_health(health: &xkcd_lock::CacheHealth) {
    println!("cache root: {}", health.root().display());
    println!("healthy: {}", yes_or_no(health.is_healthy()));
    match health.latest_marker() {
        xkcd_lock::LatestMarkerHealth::Missing => println!("latest marker: missing"),
        xkcd_lock::LatestMarkerHealth::Valid(number) => println!("latest marker: ok ({number})"),
        xkcd_lock::LatestMarkerHealth::Invalid(error) => {
            println!("latest marker: invalid ({error})")
        }
    }
    print_section("images", health.images());
    print_section("metadata", health.metadata());
    print_section("rendered", health.rendered());
    println!("staged files: {}", health.staged_files().len());
    for staged in health.staged_files() {
        println!("warning: staged file left behind at {}", staged.display());
    }
}

fn print_section(label: &str, section: &xkcd_lock::CacheSectionHealth) {
    println!(
        "{}: {} valid, {} invalid",
        label,
        section.valid_entries(),
        section.invalid_entries().len()
    );
    for invalid in section.invalid_entries() {
        println!("warning: invalid {} entry at {}", label, invalid.display());
    }
}

fn yes_or_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn command_kind(command: Option<Command>) -> Option<xkcd_lock::Kind> {
    match command {
        Some(Command::Sway) => Some(xkcd_lock::Kind::Sway),
        Some(Command::I3) => Some(xkcd_lock::Kind::I3),
        Some(Command::Cache { .. }) | None => None,
    }
}

fn kind_name(kind: xkcd_lock::Kind) -> &'static str {
    match kind {
        xkcd_lock::Kind::Sway => "sway",
        xkcd_lock::Kind::I3 => "i3",
    }
}

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Sway => "sway",
        Command::I3 => "i3",
        Command::Cache {
            command: CacheCommand::Health,
        } => "cache health",
    }
}

fn lock_command_name(command: &Command) -> Option<&'static str> {
    match command {
        Command::Sway => Some("sway"),
        Command::I3 => Some("i3"),
        Command::Cache { .. } => None,
    }
}
