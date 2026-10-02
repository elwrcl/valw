mod capture;
mod config;
mod detach;
mod dnd;
mod doctor;
mod editor;
mod error;
mod frame;
mod host;
mod ipc;
mod lock;
mod log;
mod niri;
mod output;
mod region;
mod render;
mod selection;
mod shadow;
mod sound;
mod stack;
mod thumbnail;
mod toolbar;
mod wayland;
mod window;
mod zoom;

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
    /// Click a window to capture it (Cmd+Shift+4, then Space).
    Window {
        #[command(flatten)]
        common: Common,
    },
    /// Freeze the screen and zoom in: wheel, drag, f, c, 0, Esc.
    Zoom {
        #[command(flatten)]
        common: Common,
    },
    /// Mark up an image (opened by clicking a preview).
    Edit {
        /// The image to edit.
        file: PathBuf,
    },
    /// Pick a mode from a floating bar (Cmd+Shift+5).
    Toolbar,
    /// Report what the compositor and system support.
    Doctor,
    /// Internal: the process that shows preview thumbnails.
    #[command(name = "__preview-host", hide = true)]
    PreviewHost,
    /// Internal: play the whole combo, for tuning the sounds by ear.
    #[command(name = "__combo-demo", hide = true)]
    ComboDemo,
    /// Internal: serve a PNG from stdin as the clipboard (the editor's copy).
    #[command(name = "__clipboard", hide = true)]
    Clipboard,
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
    /// Don't show the preview thumbnail.
    #[arg(long)]
    no_preview: bool,
    /// The toolbar's choices: unlike the flags they can also turn off what
    /// the config turns on. Never set from the command line.
    #[arg(skip)]
    toolbar_cursor: Option<bool>,
    #[arg(skip)]
    toolbar_preview: Option<bool>,
    #[arg(skip)]
    toolbar_sound: Option<bool>,
}

/// Whether the cursor goes into the shot.
fn wants_cursor(common: &Common, config: &Config) -> bool {
    common
        .toolbar_cursor
        .unwrap_or(common.cursor || config.capture.show_cursor)
}

/// Whether the shutter sound plays.
fn wants_sound(common: &Common, config: &Config) -> bool {
    common.toolbar_sound.unwrap_or(config.sound.enabled)
}

/// Whether the preview thumbnail is shown.
fn wants_preview(common: &Common, config: &Config) -> bool {
    common
        .toolbar_preview
        .unwrap_or(config.preview.enabled && !common.no_preview)
}

fn parse_delay(s: &str) -> Result<Duration, String> {
    let secs: f64 = s.parse().map_err(|_| format!("not a number: {s}"))?;
    Duration::try_from_secs_f64(secs)
        .map_err(|_| format!("must be a non-negative number of seconds: {s}"))
}

/// (image, output name for the file name) pairs, which one goes to the
/// clipboard, and the output the preview appears on.
type Shots = (Vec<(RgbaImage, Option<String>)>, usize, String);

enum Mode {
    Screen { all: bool },
    Region,
    Window,
    Zoom,
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
        Command::PreviewHost => host::run(),
        Command::Screen { all, common } => capture(Mode::Screen { all }, common),
        Command::Region { common } => capture(Mode::Region, common),
        Command::Window { common } => capture(Mode::Window, common),
        Command::Zoom { common } => capture(Mode::Zoom, common),
        Command::Edit { file } => editor::run(&file),
        Command::ComboDemo => sound::demo(&config::load(&config::default_path())?.sound),
        Command::Toolbar => {
            let config = config::load(&config::default_path())?;
            let (mode, common) = from_toolbar(toolbar::run(&config)?);
            capture(mode, common)
        }
        Command::Clipboard => {
            let mut png = Vec::new();
            std::io::Read::read_to_end(&mut std::io::stdin(), &mut png)
                .context("could not read the image")?;
            output::serve_clipboard(png)
        }
    }
}

fn capture(mode: Mode, common: Common) -> Result<()> {
    let config = config::load(&config::default_path())?;
    let lock = Lock::acquire(&lock::default_path())?;
    if let Some(delay) = common.delay {
        std::thread::sleep(delay);
    }
    let cursor = wants_cursor(&common, &config);
    let preview = wants_preview(&common, &config);
    let sound = wants_sound(&common, &config);
    let mut wl = Wayland::connect()?;
    if let Ok(version) = niri::version() {
        tracing::info!("niri {version}");
    }
    let outputs = wl.outputs();
    anyhow::ensure!(!outputs.is_empty(), "the compositor reported no outputs");
    // Thumbnails from earlier captures must not end up in this one.
    let mut hide = ipc::HideGuard::new(&ipc::socket_path());

    // Window shots get their (slow) shadow after the shutter sound.
    let mut windowed = false;
    let (mut shots, clip, source): Shots = match mode {
        Mode::Screen { all } => {
            let focused = focused_output(&outputs);
            let source = outputs[focused].geom.name.clone();
            if all {
                let frames = wl.capture(&outputs, cursor)?;
                let shots = frames
                    .into_iter()
                    .map(|f| (f.to_rgba(f.full()), Some(f.output.name)))
                    .collect();
                (shots, focused, source)
            } else {
                let frame = wl.capture(&outputs[focused..=focused], cursor)?.remove(0);
                (vec![(frame.to_rgba(frame.full()), None)], 0, source)
            }
        }
        Mode::Region => {
            let frames = wl.capture(&outputs, cursor)?;
            match region::select(&mut wl, &outputs, &frames)? {
                region::Choice::Region(i, r) => {
                    let source = frames[i].output.name.clone();
                    (vec![(frames[i].to_rgba(r), None)], 0, source)
                }
                region::Choice::Window => {
                    drop(frames);
                    windowed = true;
                    window_shot(&outputs, cursor)?
                }
            }
        }
        Mode::Window => {
            windowed = true;
            window_shot(&outputs, cursor)?
        }
        Mode::Zoom => {
            let focused = focused_output(&outputs);
            let frame = wl.capture(&outputs[focused..=focused], cursor)?.remove(0);
            match zoom::run(&mut wl, &outputs[focused], &frame, &config.zoom)? {
                zoom::Outcome::Capture(r) => {
                    let source = frame.output.name.clone();
                    (vec![(frame.to_rgba(r), None)], 0, source)
                }
                zoom::Outcome::Leave => {
                    tracing::info!("left zoom without capturing");
                    return Ok(());
                }
            }
        }
    };
    sound::play(&config.sound, sound);
    if windowed && config.capture.window_shadow {
        for (image, _) in &mut shots {
            *image = shadow::add(image);
        }
    }
    drop(wl);

    let target = Target::from_args(common.output, common.clipboard_only);
    let (pngs, saved) = save(&shots, clip, &target, &config)?;
    if preview
        && let Some(file) = saved
        && let Err(e) = hide.add(&ipc::socket_path(), &file, &source, ipc::start_host)
    {
        tracing::warn!("no preview: {e:#}");
    }
    // Show the thumbnails again before forking the clipboard server; the
    // child would inherit this connection and keep them hidden.
    drop(hide);

    if target == Target::ClipboardOnly || config.save.copy_to_clipboard {
        let png = pngs
            .into_iter()
            .nth(clip)
            .context("no image for the clipboard")?;
        output::copy_to_clipboard(png, lock)?;
    }
    Ok(())
}

/// A window shot, shaped like the other modes' results. The preview goes to
/// the window's output, or niri's focused one if that is unknown.
fn window_shot(outputs: &[wayland::Output], cursor: bool) -> Result<Shots> {
    let (image, output) = window::capture(cursor)?;
    let source = output.unwrap_or_else(|| outputs[focused_output(outputs)].geom.name.clone());
    Ok((vec![(image, None)], 0, source))
}

/// What the toolbar picked, as the command line would have said it.
fn from_toolbar(pick: toolbar::Picked) -> (Mode, Common) {
    let mode = match pick.mode {
        toolbar::state::Mode::Screen => Mode::Screen { all: false },
        toolbar::state::Mode::Window => Mode::Window,
        toolbar::state::Mode::Region => Mode::Region,
        toolbar::state::Mode::Zoom => Mode::Zoom,
    };
    let common = Common {
        clipboard_only: false,
        output: None,
        delay: None,
        cursor: false,
        no_preview: false,
        toolbar_cursor: Some(pick.cursor),
        toolbar_preview: Some(pick.preview),
        toolbar_sound: Some(pick.sound),
    };
    (mode, common)
}

/// Index of niri's focused output, or 0 if niri can't tell us.
pub(crate) fn focused_output(outputs: &[wayland::Output]) -> usize {
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

/// Encodes and writes the shots. Returns the PNGs and, when the clipboard
/// shot went to a regular file, that file's absolute path for the preview.
fn save(
    shots: &[(RgbaImage, Option<String>)],
    clip: usize,
    target: &Target,
    config: &Config,
) -> Result<(Vec<Vec<u8>>, Option<PathBuf>)> {
    let now = Local::now();
    let pngs = shots
        .iter()
        .map(|(img, _)| output::encode_png(img))
        .collect::<Result<Vec<_>>>()?;

    let saved = match target {
        Target::Default => {
            let dir = config.save_dir();
            let mut paths = Vec::new();
            for (png, (_, name)) in pngs.iter().zip(shots) {
                let file = output::file_name(&config.save.filename, &now, name.as_deref())?;
                let path = output::unique_path(&dir, &file);
                output::write_atomic(&path, png)?;
                println!("{}", path.display());
                paths.push(path);
            }
            paths.into_iter().nth(clip)
        }
        Target::Path(path) => {
            output::write_atomic(path, &pngs[0])?;
            println!("{}", path.display());
            // No preview for devices and pipes (`-o /dev/null`).
            std::fs::metadata(path)
                .is_ok_and(|m| m.is_file())
                .then(|| path.clone())
        }
        Target::Stdout => {
            output::write_stdout(&pngs[0])?;
            None
        }
        Target::ClipboardOnly => None,
    };
    // The host may run in another directory than this capture.
    let saved = saved.map(|p| std::fs::canonicalize(&p).unwrap_or(p));
    Ok((pngs, saved))
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
        assert!(parses(&["region", "--no-preview"]));
        assert!(parses(&["screen", "--all", "--no-preview"]));
        assert!(parses(&["window", "--cursor", "--delay", "2"]));
        assert!(!parses(&["window", "--clipboard-only", "-o", "a.png"]));
        assert!(parses(&["zoom", "--cursor", "--no-preview"]));
        assert!(!parses(&["zoom", "--clipboard-only", "-o", "a.png"]));
        assert!(parses(&["edit", "a.png"]));
        assert!(!parses(&["edit"]));
        assert!(parses(&["__clipboard"]));
        assert!(parses(&["toolbar"]));
        assert!(parses(&["__combo-demo"]));
    }

    #[test]
    fn a_toolbar_pick_becomes_a_capture() {
        use toolbar::state::Mode as T;
        let pick = |mode, cursor, preview| toolbar::Picked {
            mode,
            cursor,
            preview,
            sound: true,
        };
        let (mode, common) = from_toolbar(pick(T::Screen, true, false));
        assert!(matches!(mode, Mode::Screen { all: false }));
        assert_eq!(
            (common.toolbar_cursor, common.toolbar_preview),
            (Some(true), Some(false))
        );
        assert!(matches!(
            from_toolbar(pick(T::Window, false, true)).0,
            Mode::Window
        ));
        assert!(matches!(
            from_toolbar(pick(T::Region, false, true)).0,
            Mode::Region
        ));
        let (mode, common) = from_toolbar(pick(T::Zoom, false, true));
        assert!(matches!(mode, Mode::Zoom));
        assert_eq!(
            (common.toolbar_cursor, common.toolbar_preview),
            (Some(false), Some(true))
        );
        assert!(common.delay.is_none() && common.output.is_none());
    }

    #[test]
    fn toolbar_options_win_over_the_config_both_ways() {
        use toolbar::state::Mode as T;
        let mut on = Config::default();
        on.capture.show_cursor = true;
        on.preview.enabled = true;
        let mut off = Config::default();
        off.capture.show_cursor = false;
        off.preview.enabled = false;
        let pick = |cursor, preview| {
            from_toolbar(toolbar::Picked {
                mode: T::Screen,
                cursor,
                preview,
                sound: preview,
            })
            .1
        };
        on.sound.enabled = true;
        off.sound.enabled = false;
        assert!(!wants_sound(&pick(false, false), &on));
        assert!(wants_sound(&pick(true, true), &off));
        assert!(
            !wants_cursor(&pick(false, false), &on),
            "unticked beats the config"
        );
        assert!(!wants_preview(&pick(false, false), &on));
        assert!(
            wants_cursor(&pick(true, true), &off),
            "ticked beats the config"
        );
        assert!(wants_preview(&pick(true, true), &off));
    }

    #[test]
    fn the_command_line_still_follows_the_config() {
        let cli = |args: &[&str]| match Cli::try_parse_from(
            std::iter::once("valw").chain(args.iter().copied()),
        )
        .unwrap()
        .command
        {
            Command::Screen { common, .. } => common,
            _ => unreachable!(),
        };
        let mut on = Config::default();
        on.capture.show_cursor = true;
        assert!(wants_cursor(&cli(&["screen"]), &on));
        assert!(wants_cursor(
            &cli(&["screen", "--cursor"]),
            &Config::default()
        ));
        assert!(!wants_cursor(&cli(&["screen"]), &Config::default()));
        assert!(wants_preview(&cli(&["screen"]), &Config::default()));
        assert!(!wants_preview(
            &cli(&["screen", "--no-preview"]),
            &Config::default()
        ));
    }

    #[test]
    fn preview_host_is_hidden_from_help() {
        assert!(parses(&["__preview-host"]));
        let help = Cli::command().render_help().to_string();
        assert!(!help.contains("preview-host"), "{help}");
        assert!(!help.contains("__clipboard"), "{help}");
        assert!(!help.contains("combo-demo"), "{help}");
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
