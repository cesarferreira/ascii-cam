use anyhow::Result;
use ascii_cam::app::{Cli, run};
use clap::Parser;

fn main() -> Result<()> {
    run(Cli::parse())
}
