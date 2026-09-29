// Modules land before the CLI uses them; Task 11 removes this.
#![allow(dead_code)]

mod config;
mod detach;
mod error;
mod frame;
mod lock;
mod log;
mod output;
mod render;
mod selection;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "macOS-style screenshots for niri")]
struct Cli {}

fn main() {
    Cli::parse();
}
