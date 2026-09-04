use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::app::AppState;
use antelope_protocol::{
    MixerAssignment, MixerChannelState, MixerSurface, OutputMode, OutputState, PanState,
    PreampInputState, PreampMode,
};

const APP_CONFIG_DIR: &str = "zen-go-tui";
const PROFILE_DIR_NAME: &str = "profiles";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceProfile {
    pub outputs: OutputProfiles,
    pub preamps: PreampProfiles,
    pub assignments: Vec<MixerAssignmentEntry>,
    pub mixers: MixerProfiles,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputProfiles {
    pub monitor: OutputProfile,
    pub hp1: OutputProfile,
    pub hp2: OutputProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputProfile {
    pub volume_step: u8,
    pub mode: OutputModeProfile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputModeProfile {
    Normal,
    Mute,
    Dim,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreampProfiles {
    pub input1: PreampInputProfile,
    pub input2: PreampInputProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreampInputProfile {
    pub gain_raw: u8,
    pub mode: PreampModeProfile,
    pub phantom_on: bool,
    pub phase_inverted: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreampModeProfile {
    Mic,
    Line,
    HiZ,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MixerAssignmentEntry {
    pub channel: u8,
    pub source: MixerAssignmentProfile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "index", rename_all = "snake_case")]
pub enum MixerAssignmentProfile {
    Preamp(u8),
    ComputerPlay(u8),
    SpdifIn(u8),
    Mute,
    Oscillator(u8),
    EmuMic(u8),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MixerProfiles {
    pub mix1: Vec<MixerStripProfile>,
    pub mix2: Vec<MixerStripProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MixerStripProfile {
    pub channel: u8,
    pub level_raw: u8,
    pub pan_raw: u8,
    pub muted: bool,
    pub soloed: bool,
    pub linked: bool,
}

impl DeviceProfile {
    pub fn capture(state: &AppState) -> Result<Self> {
        Ok(Self {
            outputs: OutputProfiles {
                monitor: OutputProfile::from_device(state.output.states[0])?,
                hp1: OutputProfile::from_device(state.output.states[1])?,
                hp2: OutputProfile::from_device(state.output.states[2])?,
            },
            preamps: PreampProfiles {
                input1: PreampInputProfile::from_device(state.preamp.state.input1)?,
                input2: PreampInputProfile::from_device(state.preamp.state.input2)?,
            },
            assignments: capture_assignments(state)?,
            mixers: MixerProfiles {
                mix1: capture_surface(&state.mixer.channels[MixerSurface::Mix1.index()])?,
                mix2: capture_surface(&state.mixer.channels[MixerSurface::Mix2.index()])?,
            },
        })
    }

    pub fn validate(&self) -> Result<()> {
        validate_channel_sequence(
            self.assignments.iter().map(|entry| entry.channel).collect(),
            "assignments",
        )?;
        validate_strip_profiles(&self.mixers.mix1, "mix1")?;
        validate_strip_profiles(&self.mixers.mix2, "mix2")?;
        validate_output_profile(&self.outputs.monitor, "monitor")?;
        validate_output_profile(&self.outputs.hp1, "hp1")?;
        validate_output_profile(&self.outputs.hp2, "hp2")?;
        validate_preamp_profile(&self.preamps.input1, "input1")?;
        validate_preamp_profile(&self.preamps.input2, "input2")?;
        Ok(())
    }

    pub fn assignment_table(&self) -> Result<[MixerAssignment; 16]> {
        self.validate()?;
        let mut assignments = [MixerAssignment::Mute; 16];
        for entry in &self.assignments {
            assignments[entry.channel as usize - 1] = entry.source.into_device();
        }
        Ok(assignments)
    }

    pub fn apply_to_state(&self, state: &mut AppState) {
        state.output.states[0].volume = self.outputs.monitor.volume_step;
        state.output.states[0].mode = self.outputs.monitor.mode.into_device();
        state.output.states[1].volume = self.outputs.hp1.volume_step;
        state.output.states[1].mode = self.outputs.hp1.mode.into_device();
        state.output.states[2].volume = self.outputs.hp2.volume_step;
        state.output.states[2].mode = self.outputs.hp2.mode.into_device();

        apply_preamp_to_state(&mut state.preamp.state.input1, &self.preamps.input1);
        apply_preamp_to_state(&mut state.preamp.state.input2, &self.preamps.input2);
        state.preamp.state.cluster = [
            state.preamp.state.input1.gain_raw,
            state.preamp.state.input2.gain_raw,
            state.preamp.state.input1.mode_raw,
            state.preamp.state.input2.mode_raw,
        ];

        for entry in &self.assignments {
            let assignment = Some(entry.source.into_device());
            for surface in &mut state.mixer.channels {
                surface[entry.channel as usize - 1].assignment = assignment;
            }
        }

        for (mixer, strips) in [
            (MixerSurface::Mix1, &self.mixers.mix1),
            (MixerSurface::Mix2, &self.mixers.mix2),
        ] {
            for strip in strips {
                if let Some(channel) =
                    state.mixer.channels[mixer.index()].get_mut(strip.channel as usize - 1)
                {
                    channel.level = Some(strip.level_raw);
                    channel.pan = PanState::from_raw(strip.pan_raw);
                    channel.muted = Some(strip.muted);
                    channel.soloed = Some(strip.soloed);
                    channel.linked = Some(strip.linked);
                }
            }
        }
    }

    pub fn write_named(&self, name: &str) -> Result<PathBuf> {
        self.validate()?;
        let path = profile_path(name)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create profile directory {}", parent.display())
            })?;
        }
        let rendered = toml::to_string_pretty(self).context("failed to serialize profile")?;
        fs::write(&path, rendered)
            .with_context(|| format!("failed to write profile {}", path.display()))?;
        Ok(path)
    }

    pub fn read_named(name: &str) -> Result<Self> {
        let path = profile_path(name)?;
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read profile {}", path.display()))?;
        let profile: Self = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        profile.validate()?;
        Ok(profile)
    }
}

impl OutputProfile {
    fn from_device(output: OutputState) -> Result<Self> {
        Ok(Self {
            volume_step: output.volume,
            mode: OutputModeProfile::try_from(output.mode)?,
        })
    }
}

impl TryFrom<OutputMode> for OutputModeProfile {
    type Error = anyhow::Error;

    fn try_from(value: OutputMode) -> Result<Self> {
        Ok(match value {
            OutputMode::Normal => Self::Normal,
            OutputMode::Mute => Self::Mute,
            OutputMode::Dim => Self::Dim,
            OutputMode::Unknown(raw) => bail!("cannot save unknown output mode 0x{raw:02x}"),
        })
    }
}

impl OutputModeProfile {
    pub fn into_device(self) -> OutputMode {
        match self {
            Self::Normal => OutputMode::Normal,
            Self::Mute => OutputMode::Mute,
            Self::Dim => OutputMode::Dim,
        }
    }
}

impl PreampInputProfile {
    fn from_device(input: PreampInputState) -> Result<Self> {
        Ok(Self {
            gain_raw: input.gain_raw,
            mode: PreampModeProfile::try_from(input.mode)?,
            phantom_on: input.phantom_on,
            phase_inverted: input.mode_raw & 0x40 != 0,
        })
    }
}

impl TryFrom<PreampMode> for PreampModeProfile {
    type Error = anyhow::Error;

    fn try_from(value: PreampMode) -> Result<Self> {
        Ok(match value {
            PreampMode::Mic => Self::Mic,
            PreampMode::Line => Self::Line,
            PreampMode::HiZ => Self::HiZ,
            PreampMode::Unknown(raw) => bail!("cannot save unknown preamp mode 0x{raw:02x}"),
        })
    }
}

impl PreampModeProfile {
    pub fn into_device(self) -> PreampMode {
        match self {
            Self::Mic => PreampMode::Mic,
            Self::Line => PreampMode::Line,
            Self::HiZ => PreampMode::HiZ,
        }
    }
}

impl From<MixerAssignment> for MixerAssignmentProfile {
    fn from(value: MixerAssignment) -> Self {
        match value {
            MixerAssignment::Preamp(index) => Self::Preamp(index),
            MixerAssignment::ComputerPlay(index) => Self::ComputerPlay(index),
            MixerAssignment::SpdifIn(index) => Self::SpdifIn(index),
            MixerAssignment::Mute => Self::Mute,
            MixerAssignment::Oscillator(index) => Self::Oscillator(index),
            MixerAssignment::EmuMic(index) => Self::EmuMic(index),
        }
    }
}

impl MixerAssignmentProfile {
    pub fn into_device(self) -> MixerAssignment {
        match self {
            Self::Preamp(index) => MixerAssignment::Preamp(index),
            Self::ComputerPlay(index) => MixerAssignment::ComputerPlay(index),
            Self::SpdifIn(index) => MixerAssignment::SpdifIn(index),
            Self::Mute => MixerAssignment::Mute,
            Self::Oscillator(index) => MixerAssignment::Oscillator(index),
            Self::EmuMic(index) => MixerAssignment::EmuMic(index),
        }
    }
}

pub fn profile_path(name: &str) -> Result<PathBuf> {
    let xdg_config_home = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home_dir = env::var_os("HOME").map(PathBuf::from);
    let dir = profile_dir_from(xdg_config_home.as_deref(), home_dir.as_deref())?;
    Ok(dir.join(format!("{}.toml", validated_profile_stem(name)?)))
}

pub fn list_profile_names() -> Result<Vec<String>> {
    let xdg_config_home = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home_dir = env::var_os("HOME").map(PathBuf::from);
    let dir = profile_dir_from(xdg_config_home.as_deref(), home_dir.as_deref())?;
    list_profile_names_in_dir(&dir)
}

pub fn rename_profile(old_name: &str, new_name: &str) -> Result<PathBuf> {
    let source = profile_path(old_name)?;
    let destination = profile_path(new_name)?;
    rename_profile_at_paths(&source, &destination)
}

pub fn delete_profile(name: &str) -> Result<()> {
    let path = profile_path(name)?;
    delete_profile_at_path(&path)
}

pub(crate) fn profile_dir_from(
    xdg_config_home: Option<&Path>,
    home_dir: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(path) = xdg_config_home.filter(|path| !path.as_os_str().is_empty()) {
        return Ok(path.join(APP_CONFIG_DIR).join(PROFILE_DIR_NAME));
    }
    let Some(home_dir) = home_dir.filter(|path| !path.as_os_str().is_empty()) else {
        bail!("XDG_CONFIG_HOME and HOME are both unset")
    };
    Ok(home_dir
        .join(".config")
        .join(APP_CONFIG_DIR)
        .join(PROFILE_DIR_NAME))
}

pub fn preamp_mode_raw(mode: PreampModeProfile, phantom_on: bool, phase_inverted: bool) -> u8 {
    mode.into_device().code()
        | if phantom_on && matches!(mode, PreampModeProfile::Mic) {
            0x10
        } else {
            0x00
        }
        | if phase_inverted { 0x40 } else { 0x00 }
}

/// Returns whether a character is allowed in a saved profile name.
pub fn is_profile_name_character(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' ')
}

fn validated_profile_stem(name: &str) -> Result<&str> {
    let name = name.trim();
    let stem = name.strip_suffix(".toml").unwrap_or(name).trim();
    if stem.is_empty() {
        bail!("profile name cannot be empty")
    }
    if !stem.chars().all(is_profile_name_character) {
        bail!(
            "profile name may only contain ASCII letters, digits, spaces, '-', '_' and '.' characters"
        )
    }
    Ok(stem)
}

fn list_profile_names_in_dir(dir: &Path) -> Result<Vec<String>> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };

    let mut names = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .filter_map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

fn rename_profile_at_paths(source: &Path, destination: &Path) -> Result<PathBuf> {
    if !source.exists() {
        bail!("profile {} does not exist", source.display())
    }
    if destination.exists() {
        bail!("profile {} already exists", destination.display())
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create profile directory {}", parent.display()))?;
    }
    fs::rename(source, destination).with_context(|| {
        format!(
            "failed to rename profile {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(destination.to_path_buf())
}

fn delete_profile_at_path(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("profile {} does not exist", path.display())
    }
    fs::remove_file(path)
        .with_context(|| format!("failed to delete profile {}", path.display()))?;
    Ok(())
}

fn capture_assignments(state: &AppState) -> Result<Vec<MixerAssignmentEntry>> {
    (0..16)
        .map(|index| {
            let mix1 = state.mixer.channels[MixerSurface::Mix1.index()][index].assignment;
            let mix2 = state.mixer.channels[MixerSurface::Mix2.index()][index].assignment;
            let assignment = match (mix1, mix2) {
                (Some(left), Some(right)) if left != right => {
                    bail!(
                        "assignment mismatch between mix surfaces for CH {:02}",
                        index + 1
                    )
                }
                (Some(value), _) | (_, Some(value)) => value,
                (None, None) => bail!("missing assignment for CH {:02}", index + 1),
            };
            Ok(MixerAssignmentEntry {
                channel: index as u8 + 1,
                source: assignment.into(),
            })
        })
        .collect()
}

fn capture_surface(channels: &[MixerChannelState]) -> Result<Vec<MixerStripProfile>> {
    channels.iter().map(capture_strip).collect()
}

fn capture_strip(channel: &MixerChannelState) -> Result<MixerStripProfile> {
    Ok(MixerStripProfile {
        channel: channel.channel,
        level_raw: channel
            .level
            .ok_or_else(|| anyhow!("missing level for CH {:02}", channel.channel))?,
        pan_raw: channel.pan.raw(),
        muted: channel
            .muted
            .ok_or_else(|| anyhow!("missing mute state for CH {:02}", channel.channel))?,
        soloed: channel
            .soloed
            .ok_or_else(|| anyhow!("missing solo state for CH {:02}", channel.channel))?,
        linked: channel
            .linked
            .ok_or_else(|| anyhow!("missing link state for CH {:02}", channel.channel))?,
    })
}

fn validate_channel_sequence(mut channels: Vec<u8>, label: &str) -> Result<()> {
    channels.sort_unstable();
    let expected: Vec<u8> = (1..=16).collect();
    if channels != expected {
        bail!("{label} must contain channels 1 through 16 exactly once")
    }
    Ok(())
}

fn validate_strip_profiles(strips: &[MixerStripProfile], label: &str) -> Result<()> {
    validate_channel_sequence(strips.iter().map(|strip| strip.channel).collect(), label)?;
    for strip in strips {
        if strip.level_raw > 0x60 {
            bail!(
                "{label} CH {:02} level 0x{:02x} exceeds maximum 0x60",
                strip.channel,
                strip.level_raw
            )
        }
        if strip.pan_raw < PanState::MIN || strip.pan_raw > PanState::MAX {
            bail!(
                "{label} CH {:02} pan 0x{:02x} out of range 0x{:02x}–0x{:02x}",
                strip.channel,
                strip.pan_raw,
                PanState::MIN,
                PanState::MAX
            )
        }
    }
    for pair in strips.chunks(2) {
        if pair.len() == 2 && pair[0].linked != pair[1].linked {
            bail!("{label} linked state must match within each stereo pair")
        }
    }
    Ok(())
}

fn validate_output_profile(output: &OutputProfile, label: &str) -> Result<()> {
    if output.volume_step > 0x60 {
        bail!(
            "{label} volume 0x{:02x} exceeds maximum 0x60",
            output.volume_step
        )
    }
    Ok(())
}

fn validate_preamp_profile(input: &PreampInputProfile, label: &str) -> Result<()> {
    let max_gain = match input.mode {
        PreampModeProfile::Mic => 0x41,
        PreampModeProfile::Line => 0x2d,
        PreampModeProfile::HiZ => 0x2d,
    };
    if input.gain_raw > max_gain {
        bail!(
            "{label} gain 0x{:02x} exceeds maximum 0x{:02x} for {:?} mode",
            input.gain_raw,
            max_gain,
            input.mode
        )
    }
    Ok(())
}

fn apply_preamp_to_state(input: &mut PreampInputState, profile: &PreampInputProfile) {
    input.gain_raw = profile.gain_raw;
    input.mode = profile.mode.into_device();
    input.phantom_on = profile.phantom_on;
    input.mode_raw = preamp_mode_raw(profile.mode, profile.phantom_on, profile.phase_inverted);
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::app::AppState;
    use antelope_protocol::{MixerAssignment, MixerSurface, OutputMode, PanState, PreampMode};

    use super::*;

    #[test]
    fn profile_dir_prefers_xdg_config_home() {
        let path = profile_dir_from(Some(Path::new("/tmp/xdg")), Some(Path::new("/tmp/home")))
            .expect("profile dir");

        assert_eq!(path, Path::new("/tmp/xdg/zen-go-tui/profiles"));
    }

    #[test]
    fn profile_dir_falls_back_to_home_dot_config() {
        let path = profile_dir_from(None, Some(Path::new("/tmp/home"))).expect("profile dir");

        assert_eq!(path, Path::new("/tmp/home/.config/zen-go-tui/profiles"));
    }

    #[test]
    fn device_profile_captures_known_controls_and_round_trips_through_toml() {
        let mut state = AppState::default();
        state.output.states[0].volume = 0x12;
        state.output.states[0].mode = OutputMode::Dim;
        state.output.states[1].volume = 0x21;
        state.output.states[1].mode = OutputMode::Mute;
        state.output.states[2].volume = 0x05;
        state.output.states[2].mode = OutputMode::Normal;

        state.preamp.state.input1.gain_raw = 0x20;
        state.preamp.state.input1.mode = PreampMode::Mic;
        state.preamp.state.input1.mode_raw = 0x50;
        state.preamp.state.input1.phantom_on = true;
        state.preamp.state.input2.gain_raw = 0x11;
        state.preamp.state.input2.mode = PreampMode::Line;
        state.preamp.state.input2.mode_raw = 0x41;

        for index in 0..16 {
            let assignment = if index == 0 {
                MixerAssignment::Preamp(1)
            } else {
                MixerAssignment::Mute
            };
            for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
                let channel = &mut state.mixer.channels[mixer.index()][index];
                channel.assignment = Some(assignment);
                channel.level = Some(index as u8);
                channel.pan = if index == 0 {
                    PanState::right()
                } else {
                    PanState::center()
                };
                channel.muted = Some(index % 2 == 0);
                channel.soloed = Some(index == 1);
                channel.linked = Some(index < 2);
            }
        }

        let profile = DeviceProfile::capture(&state).expect("capture profile");

        assert_eq!(profile.outputs.monitor.volume_step, 0x12);
        assert_eq!(profile.outputs.monitor.mode, OutputModeProfile::Dim);
        assert_eq!(profile.outputs.hp1.mode, OutputModeProfile::Mute);
        assert_eq!(profile.preamps.input1.mode, PreampModeProfile::Mic);
        assert!(profile.preamps.input1.phantom_on);
        assert!(profile.preamps.input1.phase_inverted);
        assert_eq!(
            profile.assignments[0].source,
            MixerAssignmentProfile::Preamp(1)
        );
        assert_eq!(profile.mixers.mix1[0].pan_raw, PanState::right().raw());
        assert!(profile.mixers.mix1[1].soloed);
        assert!(profile.mixers.mix2[0].linked);

        let rendered = toml::to_string(&profile).expect("serialize profile");
        let decoded: DeviceProfile = toml::from_str(&rendered).expect("deserialize profile");

        assert_eq!(decoded, profile);
    }

    #[test]
    fn profile_names_allow_safe_punctuation_and_spaces_with_trimmed_boundaries() {
        assert_eq!(
            validated_profile_stem("  Session 1_A-B.v2  ").expect("valid profile name"),
            "Session 1_A-B.v2"
        );
        assert_eq!(
            validated_profile_stem("  Session 1_A-B.v2.toml  ").expect("valid profile filename"),
            "Session 1_A-B.v2"
        );
        assert!("Az09_-. ".chars().all(is_profile_name_character));

        for invalid in ["bad/name", "bad\tname", "café"] {
            assert!(
                validated_profile_stem(invalid).is_err(),
                "{invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn profile_helpers_list_rename_and_delete_profiles() {
        let dir = temp_profile_dir();
        fs::create_dir_all(&dir).expect("create temp profile dir");
        fs::write(dir.join("b.toml"), "[outputs]\n").expect("write b");
        fs::write(dir.join("a.toml"), "[outputs]\n").expect("write a");
        fs::write(dir.join("mix 1.v2.toml"), "[outputs]\n").expect("write spaced name");
        fs::write(dir.join("ignore.txt"), "x").expect("write ignore");

        assert_eq!(
            list_profile_names_in_dir(&dir).expect("list profiles"),
            vec!["a".to_string(), "b".to_string(), "mix 1.v2".to_string()]
        );

        let renamed = rename_profile_at_paths(&dir.join("a.toml"), &dir.join("c.toml"))
            .expect("rename profile");
        assert_eq!(renamed, dir.join("c.toml"));
        assert!(!dir.join("a.toml").exists());
        assert!(dir.join("c.toml").exists());

        delete_profile_at_path(&dir.join("b.toml")).expect("delete profile");
        assert!(!dir.join("b.toml").exists());

        fs::remove_dir_all(&dir).ok();
    }

    fn temp_profile_dir() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("zen-go-tui-profile-tests-{unique}"))
    }

    #[test]
    fn validate_rejects_output_volume_above_max() {
        let profile = minimal_valid_profile();
        let mut bad = profile.clone();
        bad.outputs.monitor.volume_step = 0x61;
        assert!(bad.validate().is_err());

        bad.outputs.monitor.volume_step = 0x00;
        bad.outputs.hp1.volume_step = 0xff;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn validate_accepts_output_volume_at_max() {
        let profile = minimal_valid_profile();
        let mut ok = profile.clone();
        ok.outputs.monitor.volume_step = 0x60;
        ok.outputs.hp1.volume_step = 0x60;
        ok.outputs.hp2.volume_step = 0x60;
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn validate_rejects_mixer_level_above_max() {
        let profile = minimal_valid_profile();
        let mut bad = profile.clone();
        bad.mixers.mix1[0].level_raw = 0x61;
        assert!(bad.validate().is_err());

        bad.mixers.mix1[0].level_raw = 0x30;
        bad.mixers.mix2[7].level_raw = 0xff;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn validate_accepts_mixer_level_at_max() {
        let profile = minimal_valid_profile();
        let mut ok = profile;
        for strip in ok.mixers.mix1.iter_mut().chain(ok.mixers.mix2.iter_mut()) {
            strip.level_raw = 0x60;
        }
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn validate_rejects_pan_out_of_range() {
        let profile = minimal_valid_profile();
        let mut bad = profile.clone();
        bad.mixers.mix1[0].pan_raw = 0x01;
        assert!(bad.validate().is_err());

        bad.mixers.mix1[0].pan_raw = 0x3f;
        assert!(bad.validate().is_err());

        bad.mixers.mix1[0].pan_raw = 0x00;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn validate_accepts_pan_at_bounds() {
        let profile = minimal_valid_profile();
        let mut ok = profile;
        ok.mixers.mix1[0].pan_raw = PanState::MIN;
        ok.mixers.mix1[1].pan_raw = PanState::MAX;
        ok.mixers.mix1[2].pan_raw = PanState::CENTER;
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn validate_rejects_preamp_gain_above_mode_max() {
        let profile = minimal_valid_profile();
        let mut bad = profile.clone();
        bad.preamps.input1.mode = PreampModeProfile::Mic;
        bad.preamps.input1.gain_raw = 0x42;
        assert!(bad.validate().is_err());

        bad.preamps.input1.mode = PreampModeProfile::Line;
        bad.preamps.input1.gain_raw = 0x2e;
        assert!(bad.validate().is_err());

        bad.preamps.input1.mode = PreampModeProfile::HiZ;
        bad.preamps.input1.gain_raw = 0x2e;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn validate_accepts_preamp_gain_at_mode_max() {
        let profile = minimal_valid_profile();
        let mut ok = profile.clone();
        ok.preamps.input1.mode = PreampModeProfile::Mic;
        ok.preamps.input1.gain_raw = 0x41;
        assert!(ok.validate().is_ok());

        ok.preamps.input2.mode = PreampModeProfile::Line;
        ok.preamps.input2.gain_raw = 0x2d;
        assert!(ok.validate().is_ok());

        ok.preamps.input2.mode = PreampModeProfile::HiZ;
        ok.preamps.input2.gain_raw = 0x2d;
        assert!(ok.validate().is_ok());
    }

    fn minimal_valid_profile() -> DeviceProfile {
        let mut assignments = Vec::new();
        for ch in 1..=16 {
            assignments.push(MixerAssignmentEntry {
                channel: ch,
                source: MixerAssignmentProfile::Mute,
            });
        }
        let mix1: Vec<MixerStripProfile> = (1..=16)
            .map(|ch| MixerStripProfile {
                channel: ch,
                level_raw: 0x30,
                pan_raw: PanState::CENTER,
                muted: false,
                soloed: false,
                linked: (ch - 1) / 2 % 2 == 0,
            })
            .collect();
        let mix2 = mix1.clone();
        DeviceProfile {
            outputs: OutputProfiles {
                monitor: OutputProfile {
                    volume_step: 0x20,
                    mode: OutputModeProfile::Normal,
                },
                hp1: OutputProfile {
                    volume_step: 0x20,
                    mode: OutputModeProfile::Normal,
                },
                hp2: OutputProfile {
                    volume_step: 0x20,
                    mode: OutputModeProfile::Normal,
                },
            },
            preamps: PreampProfiles {
                input1: PreampInputProfile {
                    gain_raw: 0x20,
                    mode: PreampModeProfile::Mic,
                    phantom_on: false,
                    phase_inverted: false,
                },
                input2: PreampInputProfile {
                    gain_raw: 0x10,
                    mode: PreampModeProfile::Line,
                    phantom_on: false,
                    phase_inverted: false,
                },
            },
            assignments,
            mixers: MixerProfiles { mix1, mix2 },
        }
    }
}
