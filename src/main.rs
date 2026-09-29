// Modules land before the CLI uses them; Task 11 removes this.
#![allow(dead_code)]

mod config;
mod detach;
mod doctor;
mod error;
mod frame;
mod lock;
mod log;
mod niri;
mod output;
mod render;
mod selection;
mod wayland;

use std::process::ExitCode;

use anyhow::Result;
use chrono::Local;
use clap::{Parser, Subcommand};

use crate::error::Cancelled;

#[derive(Parser)]
#[command(version, about = "macOS-style screenshots for niri")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report what the compositor and system support.
    Doctor,
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
    }
}
