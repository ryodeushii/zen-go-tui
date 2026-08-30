use anyhow::Result;
use clap::Parser;

use zen_go_slint::cli::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();
    zen_go_slint::runtime::run(cli)
}
