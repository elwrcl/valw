// Modules land before the CLI uses them; Task 11 removes this.
#![allow(dead_code)]

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "macOS-style screenshots for niri")]
struct Cli {}

fn main() {
    Cli::parse();
}
