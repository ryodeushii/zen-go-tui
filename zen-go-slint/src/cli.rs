use clap::Parser;

/// Native Slint GUI for Antelope Zen Go control.
#[derive(Debug, Clone, Parser)]
#[command(author, version, about = "Zen Go Synergy Core native control panel")]
pub struct Cli {
    /// Use an in-memory mocked transport instead of HID hardware.
    #[arg(long)]
    pub mock: bool,

    /// Start the GUI without sending startup queries.
    #[arg(long)]
    pub no_bootstrap: bool,
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn parses_mock_and_no_bootstrap_flags() {
        let cli = Cli::parse_from(["zen-go-slint", "--mock", "--no-bootstrap"]);

        assert!(cli.mock);
        assert!(cli.no_bootstrap);
    }
}
