//! CLI parsing, constants, and transport setup.

use std::thread;

use anyhow::Result;
use clap::{Parser, Subcommand};

use zen_go_tui::transport::{is_device_error, HidTransport, MockTransport, Transport};

use crate::timing;

/// CLI entry point arguments.
#[derive(Parser, Debug)]
#[command(author, version, about = "Zen Go Synergy Core terminal control panel")]
pub(crate) struct Cli {
    #[arg(long)]
    pub(crate) mock: bool,

    #[arg(long)]
    pub(crate) headless: bool,

    #[command(subcommand)]
    pub(crate) command: Option<CliCommand>,
}

/// Top-level CLI subcommands.
#[derive(Subcommand, Debug)]
pub(crate) enum CliCommand {
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
}

/// Profile management subcommands.
#[derive(Subcommand, Debug)]
pub(crate) enum ProfileCommand {
    Save { name: String },
    Load { name: String },
}

/// USB vendor ID for Antelope Audio Zen Go.
pub(crate) const ZEN_GO_VID: u16 = 0x23e5;

/// USB product ID for Antelope Audio Zen Go Synergy Core.
pub(crate) const ZEN_GO_PID: u16 = 0xa015;

/// Open a transport (real HID or mock) based on CLI flags.
pub(crate) fn open_transport(mock: bool) -> Result<Box<dyn Transport>> {
    let transport: Box<dyn Transport> = if mock {
        Box::new(MockTransport::default())
    } else {
        wait_for_transport(
            || Ok(Box::new(HidTransport::open(ZEN_GO_VID, ZEN_GO_PID)?) as Box<dyn Transport>),
            |attempt, error| {
                if attempt == 1 {
                    eprintln!("Waiting for Zen Go device...\n{error:#}");
                }
                thread::sleep(timing::device_retry_interval(attempt));
                Ok(())
            },
        )?
    };

    Ok(transport)
}

/// Repeatedly attempt to open the transport, retrying on device-unavailable errors.
pub(crate) fn wait_for_transport<T, F, R>(mut open: F, mut on_retry: R) -> Result<T>
where
    F: FnMut() -> Result<T>,
    R: FnMut(usize, &anyhow::Error) -> Result<()>,
{
    let mut retries = 0;

    loop {
        match open() {
            Ok(transport) => return Ok(transport),
            Err(error) if is_device_error(&error) => {
                retries += 1;
                on_retry(retries, &error)?;
            }
            Err(error) => return Err(error),
        }
    }
}
