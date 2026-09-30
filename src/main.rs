// Phase 2 modules land before the CLI uses them; Task 6 removes this.
#![allow(dead_code)]

mod capture;
mod config;
mod detach;
mod doctor;
mod error;
mod frame;
mod ipc;
mod lock;
mod log;
mod niri;
mod output;
mod region;
mod render;
mod selection;
mod stack;
mod thumbnail;
mod wayland;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Local;
use clap::{Args, Parser, Subcommand};
use image::RgbaImage;

use crate::capture::Capturer;
use crate::config::Config;
use crate::error::Cancelled;
use crate::lock::Lock;
use crate::output::Target;
use crate::wayland::Wayland;

#[derive(Parser)]
#[command(version, about = "macOS-style screenshots for niri")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Capture the focused output (Cmd+Shift+3).
    Screen {
        /// Capture every output, one file each.
        #[arg(long, conflicts_with_all = ["output", "clipboard_only"])]
        all: bool,
        #[command(flatten)]
        common: Common,
    },
    /// Drag to select a region (Cmd+Shift+4).
    Region {
        #[command(flatten)]
        common: Common,
    },
    /// Report what the compositor and system support.
    Doctor,
}

#[derive(Args)]
struct Common {
    /// Copy to the clipboard without saving a file.
    #[arg(long, conflicts_with = "output")]
    clipboard_only: bool,
    /// Write to this path instead of the configured directory; "-" for stdout.
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,
    /// Wait this many seconds before capturing.
    #[arg(long, value_name = "SECS", value_parser = parse_delay)]
    delay: Option<Duration>,
    /// Include the mouse cursor.
    #[arg(long)]
    cursor: bool,
}

fn parse_delay(s: &str) -> Result<Duration, String> {
    let secs: f64 = s.parse().map_err(|_| format!("not a number: {s}"))?;
    Duration::try_from_secs_f64(secs)
        .map_err(|_| format!("must be a non-negative number of seconds: {s}"))
}

enum Mode {
    Screen { all: bool },
    Region,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let log = log::init(&log::default_dir(), Local::now());
    log::install_panic_hook(log.clone());
    tracing::info!(
        "valw {} {:?}",
        env!("CARGO_PKG_VERSION"),
        std::env::args().collect::<Vec<_>>()
    );

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.downcast_ref::<Cancelled>().is_some() => {
            tracing::info!("cancelled by the user");
            ExitCode::from(3)
        }
        Err(e) => {
            tracing::error!("{e:#}");
            eprint!("{}", error::render(&e, log.as_deref()));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Doctor => doctor::run(),
        Command::Screen { all, common } => capture(Mode::Screen { all }, common),
        Command::Region { common } => capture(Mode::Region, common),
    }
}

fn capture(mode: Mode, common: Common) -> Result<()> {
    let config = config::load(&config::default_path())?;
    let lock = Lock::acquire(&lock::default_path())?;
    if let Some(delay) = common.delay {
        std::thread::sleep(delay);
    }
    let cursor = common.cursor || config.capture.show_cursor;
    let mut wl = Wayland::connect()?;
    if let Ok(version) = niri::version() {
        tracing::info!("niri {version}");
    }
    let outputs = wl.outputs();
    anyhow::ensure!(!outputs.is_empty(), "the compositor reported no outputs");

    // (image, output name for the file name) and which one goes to the clipboard.
    let (shots, clip): (Vec<(RgbaImage, Option<String>)>, usize) = match mode {
        Mode::Screen { all } => {
            let focused = focused_output(&outputs);
            if all {
                let frames = wl.capture(&outputs, cursor)?;
                let shots = frames
                    .into_iter()
                    .map(|f| (f.to_rgba(f.full()), Some(f.output.name)))
                    .collect();
                (shots, focused)
            } else {
                let frame = wl.capture(&outputs[focused..=focused], cursor)?.remove(0);
                (vec![(frame.to_rgba(frame.full()), None)], 0)
            }
        }
        Mode::Region => {
            let frames = wl.capture(&outputs, cursor)?;
            let (i, r) = region::select(&mut wl, &outputs, &frames)?;
            (vec![(frames[i].to_rgba(r), None)], 0)
        }
    };
    drop(wl);

    let target = Target::from_args(common.output, common.clipboard_only);
    deliver(shots, clip, &target, &config, lock)
}

/// Index of niri's focused output, or 0 if niri can't tell us.
fn focused_output(outputs: &[wayland::Output]) -> usize {
    let name = match niri::focused_output() {
        Ok(Some(name)) => name,
        Ok(None) => return 0,
        Err(e) => {
            tracing::warn!("using the first output: {e:#}");
            return 0;
        }
    };
    outputs
        .iter()
        .position(|o| o.geom.name == name)
        .unwrap_or_else(|| {
            tracing::warn!("niri's focused output {name} is not among the Wayland outputs");
            0
        })
}

fn deliver(
    shots: Vec<(RgbaImage, Option<String>)>,
    clip: usize,
    target: &Target,
    config: &Config,
    lock: Lock,
) -> Result<()> {
    let now = Local::now();
    let pngs = shots
        .iter()
        .map(|(img, _)| output::encode_png(img))
        .collect::<Result<Vec<_>>>()?;

    match target {
        Target::Default => {
            let dir = config.save_dir();
            for (png, (_, name)) in pngs.iter().zip(&shots) {
                let file = output::file_name(&config.save.filename, &now, name.as_deref())?;
                let path = output::unique_path(&dir, &file);
                output::write_atomic(&path, png)?;
                println!("{}", path.display());
            }
        }
        Target::Path(path) => {
            output::write_atomic(path, &pngs[0])?;
            println!("{}", path.display());
        }
        Target::Stdout => output::write_stdout(&pngs[0])?,
        Target::ClipboardOnly => {}
    }

    if *target == Target::ClipboardOnly || config.save.copy_to_clipboard {
        let png = pngs
            .into_iter()
            .nth(clip)
            .context("no image for the clipboard")?;
        output::copy_to_clipboard(png, lock)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parses(args: &[&str]) -> bool {
        Cli::try_parse_from(std::iter::once("valw").chain(args.iter().copied())).is_ok()
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn flag_conflicts() {
        assert!(parses(&["screen", "--all"]));
        assert!(parses(&["screen", "-o", "-"]));
        assert!(parses(&["region", "--clipboard-only", "--cursor"]));
        assert!(!parses(&["screen", "--all", "-o", "-"]));
        assert!(!parses(&["screen", "--all", "--clipboard-only"]));
        assert!(!parses(&["region", "--clipboard-only", "-o", "a.png"]));
    }

    #[test]
    fn delay_values() {
        assert_eq!(parse_delay("1.5"), Ok(Duration::from_millis(1500)));
        assert_eq!(parse_delay("0"), Ok(Duration::ZERO));
        assert!(parse_delay("-1").is_err());
        assert!(parse_delay("soon").is_err());
        assert!(parse_delay("NaN").is_err());
    }
}
