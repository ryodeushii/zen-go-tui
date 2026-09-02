//! CLI parsing and catalog-driven runtime startup.

use std::path::{Path, PathBuf};

use antelope_protocol::load_profile_pack_file;
use anyhow::Result;
use clap::{Parser, Subcommand};
use zen_go_tui::device::{DeviceSelection, ProfileCatalog, RuntimeDeviceState};
use zen_go_tui::transport::is_device_error;

/// CLI entry point arguments.
#[derive(Parser, Debug)]
#[command(author, version, about = "Antelope Audio terminal control panel")]
pub(crate) struct Cli {
    #[arg(long)]
    pub(crate) mock: bool,

    #[arg(long)]
    pub(crate) headless: bool,

    /// Exact HID path, unique serial, or hexadecimal VID:PID. Use path: or serial: to disambiguate.
    #[arg(long, value_name = "PATH|VID:PID|SERIAL|path:PATH|serial:SERIAL")]
    pub(crate) device: Option<String>,

    /// Validated normalized runtime profile pack.
    #[arg(long, value_name = "PATH")]
    pub(crate) profile_pack: Option<PathBuf>,

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

pub(crate) fn load_catalog(profile_pack: Option<&Path>) -> Result<ProfileCatalog> {
    let mut catalog = ProfileCatalog::builtin();
    if let Some(path) = profile_pack {
        let pack = load_profile_pack_file(path)?;
        catalog.add_external(pack)?;
    }
    Ok(catalog)
}

/// Validate external profiles before performing any HID discovery.
pub(crate) fn open_runtime(cli: &Cli) -> Result<RuntimeDeviceState> {
    let catalog = load_catalog(cli.profile_pack.as_deref())?;
    if cli.mock {
        return RuntimeDeviceState::mock(catalog);
    }
    let selection = cli
        .device
        .as_deref()
        .map(DeviceSelection::parse)
        .transpose()?;
    RuntimeDeviceState::new(catalog, selection)
}

/// Repeatedly attempt an operation, retrying only unavailable/disconnected errors.
/// Kept for deterministic timing tests; normal startup uses read-only picker discovery.
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
