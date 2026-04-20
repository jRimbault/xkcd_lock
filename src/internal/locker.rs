//! Locker selection plus concrete strategies for swaylock and i3lock.

use std::{
    cmp::Reverse,
    io::{BufRead, BufReader},
    path::Path,
    process::Command,
    sync::mpsc::Receiver,
    thread,
};

use serde::Deserialize;

/// Supported lockscreen backends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// Lock with `swaylock`.
    Sway,
    /// Lock with `i3lock`.
    I3,
}

/// Runtime options passed into a locker backend.
#[derive(Debug)]
pub struct LockOptions {
    daemonize: bool,
    kill: Option<Receiver<()>>,
}

impl LockOptions {
    /// Creates locker runtime options.
    pub fn new(daemonize: bool, kill: Option<Receiver<()>>) -> Self {
        Self { daemonize, kill }
    }
}

trait Strategy {
    /// Executes the backend-specific locker command for `image`.
    fn lock(&self, image: &Path, options: LockOptions) -> anyhow::Result<()>;
}

/// Resolves the effective locker kind from CLI choice and session type.
pub fn resolve(kind: Option<Kind>, session_type: Option<&str>) -> anyhow::Result<Kind> {
    match (kind, session_type) {
        (Some(kind), _) => Ok(kind),
        (None, Some("wayland")) => Ok(Kind::Sway),
        (None, Some("x11")) => Ok(Kind::I3),
        (None, Some(session)) => Err(anyhow::anyhow!("unknown session type {session:?}")),
        (None, None) => Err(anyhow::anyhow!(
            "no XDG_SESSION_TYPE set and no locker was specified by the user"
        )),
    }
}

/// Locks the screen with the concrete backend for `kind`.
pub fn lock(kind: Kind, image: &Path, options: LockOptions) -> anyhow::Result<()> {
    strategy(kind).lock(image, options)
}

/// Returns the concrete locker backend for `kind`.
fn strategy(kind: Kind) -> Box<dyn Strategy> {
    match kind {
        Kind::Sway => Box::new(Sway),
        Kind::I3 => Box::new(I3),
    }
}

/// `swaylock` backend implementation.
struct Sway;

impl Strategy for Sway {
    fn lock(&self, image: &Path, options: LockOptions) -> anyhow::Result<()> {
        let display_args = display_args(image)?;
        let mut command = Command::new("swaylock");
        if options.daemonize {
            command.arg("--daemonize");
        }
        let mut lockscreen = command
            .args([
                "--ignore-empty-password",
                "--show-failed-attempts",
                "-s",
                "center",
            ])
            .args(display_args)
            .spawn()?;
        if let Some(kill) = options.kill {
            let id = lockscreen.id();
            let _ = thread::spawn(move || {
                if kill.recv().is_ok() {
                    let _ = Command::new("kill")
                        .args(["-s", "TERM"])
                        .arg(id.to_string())
                        .status();
                }
            });
        }
        let status = lockscreen.wait()?;
        anyhow::ensure!(status.success(), "swaylock exited with {status}");
        Ok(())
    }
}

/// `i3lock` backend implementation.
struct I3;

impl Strategy for I3 {
    fn lock(&self, image: &Path, _options: LockOptions) -> anyhow::Result<()> {
        let status = Command::new("i3lock")
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
            .args(display_args(image)?)
            .status()?;
        anyhow::ensure!(status.success(), "i3lock exited with {status}");
        Ok(())
    }
}

/// Formats per-output `-i` arguments for the chosen background image.
fn display_args(image: &Path) -> anyhow::Result<Vec<String>> {
    let image = image.to_string_lossy();
    let mut displays = outputs()?.into_iter();
    let display_args = displays
        .next()
        .into_iter()
        .map(|display| ["-i".to_owned(), format!("{display}:{image}")])
        .chain(
            displays.map(|display| ["-i".to_owned(), format!("{display}:{image}")]),
        )
        .flatten()
        .collect();
    Ok(display_args)
}

/// Detects outputs, preferring `swaymsg` and falling back to `xrandr`.
fn outputs() -> anyhow::Result<Vec<String>> {
    match sway_outputs() {
        Ok(outputs) => Ok(outputs),
        Err(error) => {
            log::debug!("falling back to xrandr display discovery: {error:?}");
            xrandr_outputs()
        }
    }
}

/// Reads ordered outputs from `swaymsg -t get_outputs`.
fn sway_outputs() -> anyhow::Result<Vec<String>> {
    #[derive(Debug, Deserialize)]
    struct Output {
        name: String,
        rect: Dimensions,
    }

    #[derive(Debug, Deserialize)]
    struct Dimensions {
        width: u32,
    }

    let output = Command::new("swaymsg").args(["-t", "get_outputs"]).output()?;
    anyhow::ensure!(
        output.status.success(),
        "swaymsg exited with {}",
        output.status
    );
    let mut outputs: Vec<Output> = serde_json::from_slice(&output.stdout)?;
    outputs.sort_by_key(|output| Reverse(output.rect.width));
    Ok(outputs.into_iter().map(|output| output.name).collect())
}

/// Reads connected outputs from `xrandr`.
fn xrandr_outputs() -> anyhow::Result<Vec<String>> {
    let output = Command::new("xrandr").output()?;
    anyhow::ensure!(output.status.success(), "xrandr exited with {}", output.status);
    let stdout = BufReader::new(output.stdout.as_slice());
    let outputs = stdout
        .lines()
        .filter_map(|line| {
            let line = line.ok()?;
            if line.contains(" connected ") {
                line.split(' ').next().map(str::to_owned)
            } else {
                None
            }
        })
        .collect();
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_choice_wins() {
        assert_eq!(resolve(Some(Kind::I3), Some("wayland")).unwrap(), Kind::I3);
    }

    #[test]
    fn wayland_defaults_to_sway() {
        assert_eq!(resolve(None, Some("wayland")).unwrap(), Kind::Sway);
    }

    #[test]
    fn x11_defaults_to_i3() {
        assert_eq!(resolve(None, Some("x11")).unwrap(), Kind::I3);
    }
}
