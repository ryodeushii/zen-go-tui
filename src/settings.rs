use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::app::{AppSettings, PeakHoldDuration, RefreshRate};

const APP_CONFIG_DIR: &str = "zen-go-tui";
const SETTINGS_FILE_NAME: &str = "settings.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableSettings {
    refresh_rate_fps: u8,
    peak_threshold_raw: u8,
    peak_enabled: bool,
    peak_hold_duration_secs: u64,
    auto_save: bool,
}

impl From<AppSettings> for SerializableSettings {
    fn from(s: AppSettings) -> Self {
        let refresh_rate_fps = s.refresh_rate.fps();
        let peak_hold_duration_secs = s.peak_hold_duration.duration().as_secs();
        Self {
            refresh_rate_fps,
            peak_threshold_raw: s.peak_threshold_raw,
            peak_enabled: s.peak_enabled,
            peak_hold_duration_secs,
            auto_save: s.auto_save,
        }
    }
}

impl From<SerializableSettings> for AppSettings {
    fn from(s: SerializableSettings) -> Self {
        let refresh_rate = match s.refresh_rate_fps {
            15 => RefreshRate::Fps15,
            60 => RefreshRate::Fps60,
            _ => RefreshRate::Fps30,
        };
        let peak_hold_duration = match s.peak_hold_duration_secs {
            1 => PeakHoldDuration::Sec1,
            5 => PeakHoldDuration::Sec5,
            10 => PeakHoldDuration::Sec10,
            _ => PeakHoldDuration::Sec3,
        };
        Self {
            refresh_rate,
            peak_threshold_raw: s.peak_threshold_raw,
            peak_enabled: s.peak_enabled,
            peak_hold_duration,
            auto_save: s.auto_save,
        }
    }
}

fn settings_dir(xdg_config_home: Option<&Path>, home_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = xdg_config_home.filter(|path| !path.as_os_str().is_empty()) {
        return Ok(path.join(APP_CONFIG_DIR));
    }
    let Some(home_dir) = home_dir.filter(|path| !path.as_os_str().is_empty()) else {
        anyhow::bail!("XDG_CONFIG_HOME and HOME are both unset")
    };
    Ok(home_dir.join(".config").join(APP_CONFIG_DIR))
}

fn settings_path() -> Result<PathBuf> {
    let dir = settings_dir(
        env::var_os("XDG_CONFIG_HOME").as_deref().map(Path::new),
        env::var_os("HOME").as_deref().map(Path::new),
    )?;
    Ok(dir.join(SETTINGS_FILE_NAME))
}

pub fn load_settings() -> Result<AppSettings> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let content = fs::read_to_string(&path).context("Failed to read settings file")?;
    let serializable: SerializableSettings =
        toml::from_str(&content).context("Failed to parse settings file")?;
    Ok(serializable.into())
}

pub fn save_settings(settings: &AppSettings) -> Result<()> {
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create settings directory")?;
    }
    let serializable = SerializableSettings::from(*settings);
    let content = toml::to_string_pretty(&serializable).context("Failed to serialize settings")?;
    fs::write(&path, content).context("Failed to write settings file")?;
    Ok(())
}
