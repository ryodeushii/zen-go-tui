//! Core protocol types: sample rates, clock sources, output modes, preamp states, pan, meters.

use thiserror::Error;

/// Size of a HID report frame in bytes.
pub const HID_REPORT_SIZE: usize = 320;

/// Offset from frame start to the beginning of the 0x73 snapshot payload.
pub const SNAPSHOT_PAYLOAD_OFFSET: usize = 0x10;

/// Minimum length of a 0x73 snapshot frame (payload offset + payload size).
pub const MIN_SNAPSHOT_FRAME_LEN: usize = SNAPSHOT_PAYLOAD_OFFSET + SNAPSHOT_PAYLOAD_SIZE;

/// Size of the 0x73 snapshot payload in bytes.
pub const SNAPSHOT_PAYLOAD_SIZE: usize = 0xe6;

/// Snapshot payload offsets — header region (bytes 0x00–0x0b).
pub const OFFSET_STATUS_FLAGS_0: usize = 0x00;
pub const OFFSET_STATUS_FLAGS_1: usize = 0x01;
pub const OFFSET_SAMPLE_RATE_CODE: usize = 0x02;
pub const OFFSET_CLOCK_SOURCE: usize = 0x03;
pub const OFFSET_SAMPLE_RATE_HZ_START: usize = 0x04;
pub const OFFSET_SAMPLE_RATE_HZ_END: usize = 0x08;
pub const OFFSET_FRONT_PANEL_BYTES_START: usize = 0x08;
pub const OFFSET_FRONT_PANEL_BYTES_END: usize = 0x0b;

/// Snapshot payload offsets — output state region (bytes 0x0c–0x11).
pub const OFFSET_MONITOR_VOLUME: usize = 0x0c;
pub const OFFSET_MONITOR_MODE: usize = 0x0d;
pub const OFFSET_HP1_VOLUME: usize = 0x0e;
pub const OFFSET_HP1_MODE: usize = 0x0f;
pub const OFFSET_HP2_VOLUME: usize = 0x10;
pub const OFFSET_HP2_MODE: usize = 0x11;

/// Snapshot payload offsets — DSP/preamp cluster (bytes 0x18–0x1b).
#[allow(dead_code)]
pub const OFFSET_DSP_CLUSTER_START: usize = 0x18;
#[allow(dead_code)]
pub const OFFSET_DSP_CLUSTER_END: usize = 0x1c;
pub const OFFSET_PREAMP1_GAIN: usize = 0x18;
pub const OFFSET_PREAMP2_GAIN: usize = 0x19;
pub const OFFSET_PREAMP1_MODE: usize = 0x1a;
pub const OFFSET_PREAMP2_MODE: usize = 0x1b;

/// Snapshot payload offset — surface selector byte.
pub const OFFSET_SURFACE_SELECTOR: usize = 0x6a;

/// Snapshot payload offset — unknown byte between surface selector and meter lanes.
#[allow(dead_code)]
pub const OFFSET_UNKNOWN_6E: usize = 0x6e;

/// Snapshot payload offsets — meter lane region (bytes 0x8e–0x9d).
pub const OFFSET_METER_LANES_START: usize = 0x8e;
pub const OFFSET_METER_LANES_END: usize = 0x9d;

/// Snapshot payload offsets — preamp meter lanes.
pub const OFFSET_PREAMP1_METER: usize = 0xce;
pub const OFFSET_PREAMP2_METER: usize = 0xcf;

/// Snapshot payload offsets — mute/pan primary bytes.
pub const OFFSET_MIX1_PRIMARY: usize = 0x8f;
pub const OFFSET_MIX2_PRIMARY: usize = 0xcf;

/// Snapshot payload offsets — late shadow region (bytes 0xda–0xe5).
pub const OFFSET_LATE_SHADOW_START: usize = 0xda;
pub const OFFSET_LATE_SHADOW_END: usize = 0xe5;

/// Mix1 late shadow lane offsets.
pub const OFFSET_MIX1_LANE_A: usize = 0xda;
pub const OFFSET_MIX1_LANE_B: usize = 0xdb;
pub const OFFSET_MIX1_MIRROR_A: usize = 0xdc;
pub const OFFSET_MIX1_MIRROR_B: usize = 0xdd;

/// Mix2 late shadow lane offsets.
pub const OFFSET_MIX2_LANE_A: usize = 0xde;
pub const OFFSET_MIX2_LANE_B: usize = 0xdf;

/// Shared late shadow offsets used by both mixes.
pub const OFFSET_SHARED_SHADOW_0: usize = 0xe0;
pub const OFFSET_SHARED_SHADOW_1: usize = 0xe1;
pub const OFFSET_SHARED_SHADOW_2: usize = 0xe2;
pub const OFFSET_SHARED_SHADOW_3: usize = 0xe3;
pub const OFFSET_SHARED_SHADOW_4: usize = 0xe4;
pub const OFFSET_SHARED_SHADOW_5: usize = 0xe5;

/// Frame type identifiers.
pub const FRAME_TYPE_SNAPSHOT: u32 = 0x73;
pub const FRAME_TYPE_QUERY_REPLY: u32 = 0x75;
pub const FRAME_TYPE_AUXILIARY: u32 = 0x83;

/// Surface selector codes.
pub const SURFACE_CODE_MONITOR_HP1: u8 = 0x0f;
pub const SURFACE_CODE_HP2: u8 = 0x0c;

/// Errors that can occur during protocol frame parsing.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// The frame is shorter than the minimum required length.
    #[error("frame too short: {0}")]
    FrameTooShort(usize),
    /// The frame type identifier is not recognized.
    #[error("unsupported frame type: 0x{0:02x}")]
    UnsupportedFrame(u32),
    /// Failed to parse a fixed-size field from the frame payload.
    #[error("invalid frame field: {0}")]
    InvalidField(&'static str),
}

/// Supported sample rates for the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRate {
    /// 32 kHz.
    Hz32000,
    /// 44.1 kHz.
    Hz44100,
    /// 48 kHz.
    Hz48000,
    /// 88.2 kHz.
    Hz88200,
    /// 96 kHz.
    Hz96000,
    /// 176.4 kHz.
    Hz176400,
    /// 192 kHz.
    Hz192000,
    /// An unrecognized sample rate code.
    Unknown(u8),
}

impl SampleRate {
    /// Creates a `SampleRate` from the device's raw code byte.
    pub fn from_code(code: u8) -> Self {
        match code {
            0x00 => Self::Hz32000,
            0x01 => Self::Hz44100,
            0x02 => Self::Hz48000,
            0x03 => Self::Hz88200,
            0x04 => Self::Hz96000,
            0x05 => Self::Hz176400,
            0x06 => Self::Hz192000,
            value => Self::Unknown(value),
        }
    }

    /// Returns the raw protocol code for this sample rate.
    pub fn code(self) -> u8 {
        match self {
            Self::Hz32000 => 0x00,
            Self::Hz44100 => 0x01,
            Self::Hz48000 => 0x02,
            Self::Hz88200 => 0x03,
            Self::Hz96000 => 0x04,
            Self::Hz176400 => 0x05,
            Self::Hz192000 => 0x06,
            Self::Unknown(value) => value,
        }
    }

    /// Returns the sample rate in hertz, or `None` for unknown codes.
    pub fn hz(self) -> Option<u32> {
        match self {
            Self::Hz32000 => Some(32_000),
            Self::Hz44100 => Some(44_100),
            Self::Hz48000 => Some(48_000),
            Self::Hz88200 => Some(88_200),
            Self::Hz96000 => Some(96_000),
            Self::Hz176400 => Some(176_400),
            Self::Hz192000 => Some(192_000),
            Self::Unknown(_) => None,
        }
    }

    /// Returns a human-readable label (e.g. `"48000 Hz"` or `"Unknown (0x07)"`).
    pub fn label(self) -> String {
        self.hz()
            .map(|hz| format!("{} Hz", hz))
            .unwrap_or_else(|| format!("Unknown (0x{:02x})", self.code()))
    }

    /// Returns the complete list of confirmed sample rate variants.
    pub fn all_confirmed() -> &'static [SampleRate] {
        const ALL: [SampleRate; 7] = [
            SampleRate::Hz32000,
            SampleRate::Hz44100,
            SampleRate::Hz48000,
            SampleRate::Hz88200,
            SampleRate::Hz96000,
            SampleRate::Hz176400,
            SampleRate::Hz192000,
        ];
        &ALL
    }
}

/// Clock synchronization source for the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSource {
    /// Internal oscillator.
    Internal,
    /// S/PDIF digital input.
    Spdif,
    /// USB host clock.
    Usb,
    /// An unrecognized clock source code.
    Unknown(u8),
}

impl ClockSource {
    /// Creates a `ClockSource` from the device's raw code byte.
    pub fn from_code(code: u8) -> Self {
        match code {
            0x00 => Self::Internal,
            0x01 => Self::Spdif,
            0x02 => Self::Usb,
            value => Self::Unknown(value),
        }
    }

    /// Returns the raw protocol code for this clock source.
    pub fn code(self) -> u8 {
        match self {
            Self::Internal => 0x00,
            Self::Spdif => 0x01,
            Self::Usb => 0x02,
            Self::Unknown(value) => value,
        }
    }

    /// Returns a human-readable label (e.g. `"Internal"`, `"S/PDIF"`, `"USB"`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Internal => "Internal",
            Self::Spdif => "S/PDIF",
            Self::Usb => "USB",
            Self::Unknown(_) => "Unknown",
        }
    }

    /// Returns the complete list of confirmed clock source variants.
    pub fn all_confirmed() -> &'static [ClockSource] {
        const ALL: [ClockSource; 3] = [ClockSource::Internal, ClockSource::Spdif, ClockSource::Usb];
        &ALL
    }
}

/// Output signal mode (normal, muted, or dimmed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Signal passes through at the configured volume.
    Normal,
    /// Signal is silenced.
    Mute,
    /// Signal is attenuated by a fixed dim amount.
    Dim,
    /// An unrecognized output mode code.
    Unknown(u8),
}

impl OutputMode {
    /// Creates an `OutputMode` from the device's raw code byte.
    pub fn from_code(code: u8) -> Self {
        match code {
            0x00 => Self::Normal,
            0x01 => Self::Mute,
            0x02 => Self::Dim,
            value => Self::Unknown(value),
        }
    }

    /// Returns a human-readable label (e.g. `"Normal"`, `"Mute"`, `"Dim"`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Mute => "Mute",
            Self::Dim => "Dim",
            Self::Unknown(_) => "Unknown",
        }
    }
}

/// Physical output destination on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTarget {
    /// Main monitor outputs.
    Monitor,
    /// Headphone output 1.
    Hp1,
    /// Headphone output 2.
    Hp2,
}

impl OutputTarget {
    /// Returns the zero-based index used in protocol frames.
    pub fn index(self) -> u8 {
        match self {
            Self::Monitor => 0x00,
            Self::Hp1 => 0x01,
            Self::Hp2 => 0x02,
        }
    }

    /// Creates an `OutputTarget` from a zero-based index.
    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Monitor,
            1 => Self::Hp1,
            _ => Self::Hp2,
        }
    }

    /// Returns a human-readable label (e.g. `"Monitor"`, `"HP1"`, `"HP2"`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Monitor => "Monitor",
            Self::Hp1 => "HP1",
            Self::Hp2 => "HP2",
        }
    }
}

/// Front-panel surface selection, determining which outputs are controlled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// Monitor and HP1 share the same surface controls.
    MonitorHp1,
    /// HP2 has its own surface controls.
    Hp2,
    /// An unrecognized surface code.
    Unknown(u8),
}

impl Surface {
    /// Creates a `Surface` from the device's raw code byte.
    pub fn from_code(code: u8) -> Self {
        match code {
            0x0f => Self::MonitorHp1,
            0x0c => Self::Hp2,
            value => Self::Unknown(value),
        }
    }

    /// Returns the raw protocol code for this surface.
    pub fn code(self) -> u8 {
        match self {
            Self::MonitorHp1 => 0x0f,
            Self::Hp2 => 0x0c,
            Self::Unknown(value) => value,
        }
    }

    /// Returns a human-readable label (e.g. `"Monitor / HP1"`, `"HP2"`).
    pub fn label(self) -> &'static str {
        match self {
            Self::MonitorHp1 => "Monitor / HP1",
            Self::Hp2 => "HP2",
            Self::Unknown(_) => "Unknown",
        }
    }
}

/// Preamp input signal type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreampMode {
    /// Microphone-level input.
    Mic,
    /// Line-level input.
    Line,
    /// High-impedance instrument input.
    HiZ,
    /// An unrecognized preamp mode code.
    Unknown(u8),
}

impl PreampMode {
    /// Creates a `PreampMode` from the raw mode byte (masked to lower nibble).
    pub fn from_raw(mode: u8) -> Self {
        match mode & 0x0f {
            0x00 => Self::Mic,
            0x01 => Self::Line,
            0x02 => Self::HiZ,
            value => Self::Unknown(value),
        }
    }

    /// Returns a human-readable label (e.g. `"Mic"`, `"Line"`, `"Hi-Z"`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Mic => "Mic",
            Self::Line => "Line",
            Self::HiZ => "Hi-Z",
            Self::Unknown(_) => "Unknown",
        }
    }

    /// Returns the raw protocol code for this preamp mode.
    pub fn code(self) -> u8 {
        match self {
            Self::Mic => 0x00,
            Self::Line => 0x01,
            Self::HiZ => 0x02,
            Self::Unknown(value) => value,
        }
    }
}

/// Complete state of a single preamp input channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreampInputState {
    /// Raw gain value from the protocol frame.
    pub gain_raw: u8,
    /// Input signal type (Mic, Line, Hi-Z).
    pub mode: PreampMode,
    /// Whether +48V phantom power is enabled (only meaningful in Mic mode).
    pub phantom_on: bool,
    /// Unprocessed mode byte, preserving the phantom power flag bit.
    pub mode_raw: u8,
    /// Meter reading observed passively from snapshot frames, if any.
    pub observed_meter: Option<u8>,
}

impl PreampInputState {
    /// Decodes a preamp input from raw gain and mode bytes.
    pub fn from_raw(gain_raw: u8, mode_raw: u8) -> Self {
        let mode = PreampMode::from_raw(mode_raw);
        Self {
            gain_raw,
            mode,
            phantom_on: matches!(mode, PreampMode::Mic) && mode_raw & 0x10 != 0,
            mode_raw,
            observed_meter: None,
        }
    }

    /// Returns a human-readable gain label in dB, formatted per mode.
    pub fn gain_db_label(self) -> String {
        match self.mode {
            PreampMode::Mic => format!("{} dB", self.gain_raw.min(0x41)),
            PreampMode::Line => format!("{:+} dB", i8::from_ne_bytes([self.gain_raw])),
            PreampMode::HiZ => format!("{} dB", self.gain_raw.min(0x2d)),
            PreampMode::Unknown(_) => format!("raw {:02x}", self.gain_raw),
        }
    }

    /// Returns the gain as a normalized 0.0–1.0 ratio for UI display.
    pub fn gain_ratio(self) -> f64 {
        match self.mode {
            PreampMode::Mic => (self.gain_raw.min(0x41) as f64 / 65.0).clamp(0.0, 1.0),
            PreampMode::Line => {
                let db = i8::from_ne_bytes([self.gain_raw]).clamp(-6, 20) as f64;
                ((db + 6.0) / 26.0).clamp(0.0, 1.0)
            }
            PreampMode::HiZ => (self.gain_raw.min(0x2d) as f64 / 45.0).clamp(0.0, 1.0),
            PreampMode::Unknown(_) => 0.0,
        }
    }

    /// Converts the observed meter reading to a display-friendly dB value.
    pub fn observed_meter_db(self) -> Option<i16> {
        self.observed_meter.and_then(meter_display_db)
    }

    /// Converts the observed meter reading to a normalized 0.0–1.0 ratio.
    pub fn observed_meter_ratio(self) -> Option<f64> {
        self.observed_meter.map(meter_ratio)
    }
}

/// Combined state of both preamp inputs, decoded from a 4-byte DSP cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreampState {
    /// State of preamp input 1.
    pub input1: PreampInputState,
    /// State of preamp input 2.
    pub input2: PreampInputState,
    /// Raw 4-byte cluster from the protocol frame.
    pub cluster: [u8; 4],
}

impl Default for PreampState {
    fn default() -> Self {
        Self::from_cluster([0; 4])
    }
}

impl PreampState {
    /// Decodes both preamp inputs from a 4-byte DSP cluster.
    pub fn from_cluster(cluster: [u8; 4]) -> Self {
        Self {
            input1: PreampInputState::from_raw(cluster[0], cluster[2]),
            input2: PreampInputState::from_raw(cluster[1], cluster[3]),
            cluster,
        }
    }
}

/// Stereo pan position with mute and solo flags.
///
/// The raw value ranges from [`MIN`](Self::MIN) (fully left) to [`MAX`](Self::MAX) (fully right),
/// with [`CENTER`](Self::CENTER) at the midpoint. The upper two bits encode mute (0x40)
/// and solo (0x80) flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanState(u8);

impl PanState {
    /// Minimum pan value (fully left).
    pub const MIN: u8 = 0x02;
    /// Center pan position.
    pub const CENTER: u8 = 0x20;
    /// Maximum pan value (fully right).
    pub const MAX: u8 = 0x3e;
    const PAN_MASK: u8 = 0x3f;
    const MUTE_FLAG: u8 = 0x40;
    const SOLO_FLAG: u8 = 0x80;

    /// Creates a `PanState` from a raw pan value, clamped to [`MIN`](Self::MIN)–[`MAX`](Self::MAX).
    pub fn from_raw(raw: u8) -> Self {
        Self(raw.clamp(Self::MIN, Self::MAX))
    }

    /// Extracts the pan position from a state code that may include mute/solo flags.
    pub fn from_state_code(code: u8) -> Self {
        Self::from_raw(code & Self::PAN_MASK)
    }

    /// Returns a pan state positioned fully left.
    pub fn left() -> Self {
        Self(Self::MIN)
    }

    /// Returns a pan state positioned at center.
    pub fn center() -> Self {
        Self(Self::CENTER)
    }

    /// Returns a pan state positioned fully right.
    pub fn right() -> Self {
        Self(Self::MAX)
    }

    /// Returns the masked pan code without mute/solo flags.
    pub fn code(self) -> u8 {
        self.0
    }

    /// Returns the raw internal pan value.
    pub fn raw(self) -> u8 {
        self.0
    }

    /// Encodes the pan position with mute and solo flags into a state code.
    pub fn state_code(self, muted: bool, soloed: bool) -> u8 {
        self.code()
            | if muted { Self::MUTE_FLAG } else { 0x00 }
            | if soloed { Self::SOLO_FLAG } else { 0x00 }
    }

    /// Encodes the pan position with only the mute flag.
    pub fn muted_code(self, muted: bool) -> u8 {
        self.state_code(muted, false)
    }

    /// Checks whether the mute flag is set in a state code.
    pub fn state_code_is_muted(code: u8) -> bool {
        code & Self::MUTE_FLAG != 0
    }

    /// Checks whether the solo flag is set in a state code.
    pub fn state_code_is_soloed(code: u8) -> bool {
        code & Self::SOLO_FLAG != 0
    }

    /// Returns the pan as a normalized 0.0 (left) to 1.0 (right) ratio.
    pub fn ratio(self) -> f64 {
        (self.raw().saturating_sub(Self::MIN) as f64 / (Self::MAX - Self::MIN) as f64)
            .clamp(0.0, 1.0)
    }

    /// Returns a signed display offset from center in device steps (-30 to +30).
    pub fn display_percent(self) -> i16 {
        self.raw() as i16 - Self::CENTER as i16
    }
}

impl Default for PanState {
    fn default() -> Self {
        Self::center()
    }
}

/// Volume and mode state for a single physical output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputState {
    /// Which physical output this state belongs to.
    pub target: OutputTarget,
    /// Raw volume byte (0x00 = unity, higher = more attenuation).
    pub volume: u8,
    /// Current output mode (Normal, Mute, Dim).
    pub mode: OutputMode,
}

impl OutputState {
    /// Creates a new `OutputState`.
    pub fn new(target: OutputTarget, volume: u8, mode: OutputMode) -> Self {
        Self {
            target,
            volume,
            mode,
        }
    }

    /// Returns the attenuation in device steps, capped at the maximum (0x60).
    pub fn attenuation_steps(self) -> u8 {
        self.volume.min(0x60)
    }

    /// Returns the display-friendly dB value (0 to -96 dB).
    pub fn display_db(self) -> i16 {
        -(self.attenuation_steps() as i16)
    }

    /// Returns the gain as a normalized 0.0 (silent) to 1.0 (unity) ratio.
    pub fn gain_ratio(self) -> f64 {
        let attenuation = self.attenuation_steps() as f64;
        (1.0 - attenuation / 96.0).clamp(0.0, 1.0)
    }
}

/// Converts a raw meter byte to a display-friendly dB value.
///
/// Returns `Some(-dB)` for values ≤ 0x3c (0 to -60 dB), `None` otherwise.
pub fn meter_display_db(raw: u8) -> Option<i16> {
    (raw <= 0x3c).then_some(-(raw as i16))
}

/// Converts a display dB value to a normalized 0.0–1.0 ratio.
///
/// The raw meter byte is already a logarithmic dB value (0 = 0 dB, 0x3c = -60 dB).
/// Maps -60..0 dB linearly to 0.0..1.0.
pub fn meter_db_ratio(db: i16) -> f64 {
    let db = db.clamp(-60, 0) as f64;
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// Converts a raw meter byte directly to a normalized 0.0–1.0 ratio.
pub fn meter_ratio(raw: u8) -> f64 {
    meter_display_db(raw).map(meter_db_ratio).unwrap_or(0.0)
}

/// Immutable snapshot of the entire device state, decoded from a snapshot frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStateSnapshot {
    /// Current sample rate setting.
    pub sample_rate: SampleRate,
    /// Current clock source setting.
    pub clock_source: ClockSource,
    /// Sample rate in hertz (redundant with `sample_rate`, provided directly from frame).
    pub sample_rate_hz: u32,
    /// Raw status flag bytes from the frame.
    pub status_flags: [u8; 2],
    /// Raw front-panel LED/button bytes from the frame.
    pub front_panel_bytes: [u8; 3],
    /// State of all three physical outputs (Monitor, HP1, HP2).
    pub outputs: [OutputState; 3],
    /// Raw 4-byte DSP cluster (shared with preamp decoding).
    pub dsp_cluster: [u8; 4],
    /// Decoded preamp state for both inputs.
    pub preamp: PreampState,
    /// Which surface is currently selected on the front panel.
    pub surface: Surface,
    /// Passive mixer state decoded from the snapshot payload.
    pub mixer_decode: crate::mixer::MixerPassiveDecode,
    /// Late-row shadow bytes from the frame payload (offsets 0xda–0xe5).
    pub late_shadow: [u8; 12],
}

impl DeviceStateSnapshot {
    /// Returns the output state for the given target.
    pub fn output(&self, target: OutputTarget) -> OutputState {
        self.outputs[target.index() as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_supports_scalar_raw_values_and_ui_ratio() {
        let pan = PanState::from_raw(0x10);

        assert_eq!(pan.code(), 0x10);
        assert_eq!(pan.raw(), 0x10);
        assert!((pan.ratio() - ((0x10 - 0x02) as f64 / (0x3e - 0x02) as f64)).abs() < 1e-9);
        assert_eq!(PanState::center().raw(), 0x20);
        assert_eq!(PanState::left().raw(), 0x02);
        assert_eq!(PanState::right().raw(), 0x3e);
    }

    #[test]
    fn pan_state_decodes_mute_and_solo_flags_from_state_code() {
        assert_eq!(PanState::from_state_code(0x42), PanState::left());
        assert!(PanState::state_code_is_muted(0x42));
        assert!(!PanState::state_code_is_soloed(0x42));

        assert_eq!(PanState::from_state_code(0x60), PanState::center());
        assert!(PanState::state_code_is_muted(0x60));
        assert!(!PanState::state_code_is_soloed(0x60));

        assert_eq!(PanState::from_state_code(0xe0), PanState::center());
        assert!(PanState::state_code_is_muted(0xe0));
        assert!(PanState::state_code_is_soloed(0xe0));
    }

    #[test]
    fn pan_state_encodes_mute_and_solo_flags_into_state_code() {
        assert_eq!(PanState::center().state_code(false, false), 0x20);
        assert_eq!(PanState::left().state_code(true, false), 0x42);
        assert_eq!(PanState::right().state_code(false, true), 0xbe);
        assert_eq!(PanState::center().state_code(true, true), 0xe0);
    }

    #[test]
    fn pan_display_uses_device_step_scale() {
        assert_eq!(PanState::center().display_percent(), 0);
        assert_eq!(PanState::from_raw(0x1e).display_percent(), -2);
        assert_eq!(PanState::left().display_percent(), -30);
        assert_eq!(PanState::right().display_percent(), 30);
    }

    #[test]
    fn output_volume_display_uses_inverse_db_scale() {
        let unity = OutputState::new(OutputTarget::Monitor, 0x00, OutputMode::Normal);
        let silence = OutputState::new(OutputTarget::Monitor, 0x60, OutputMode::Normal);

        assert_eq!(unity.display_db(), 0);
        assert_eq!(silence.display_db(), -96);
        assert_eq!(unity.gain_ratio(), 1.0);
        assert_eq!(silence.gain_ratio(), 0.0);
    }

    #[test]
    fn meter_display_uses_logarithmic_minus_60_to_0_db_ui_scale() {
        assert_eq!(meter_display_db(0x00), Some(0));
        assert_eq!(meter_display_db(0x3c), Some(-60));
        assert_eq!(meter_display_db(0x60), None);
        assert!((meter_ratio(0x00) - 1.0).abs() < 1e-9);
        assert!((meter_ratio(0x1e) - 0.5).abs() < 0.01);
        assert!((meter_ratio(0x3c) - 0.0).abs() < 1e-9);
        assert_eq!(meter_ratio(0x60), 0.0);
    }

    #[test]
    fn decodes_preamp_cluster_for_both_inputs() {
        let state = PreampState::from_cluster([0x41, 0x2a, 0x10, 0x00]);

        assert_eq!(state.input1.gain_raw, 0x41);
        assert_eq!(state.input1.mode, PreampMode::Mic);
        assert!(state.input1.phantom_on);

        assert_eq!(state.input2.gain_raw, 0x2a);
        assert_eq!(state.input2.mode, PreampMode::Mic);
        assert!(!state.input2.phantom_on);
    }

    #[test]
    fn decodes_preamp_line_and_hiz_modes() {
        let state = PreampState::from_cluster([0x14, 0x2d, 0x11, 0x02]);

        assert_eq!(state.input1.mode, PreampMode::Line);
        assert!(!state.input1.phantom_on);
        assert_eq!(state.input1.gain_raw, 0x14);

        assert_eq!(state.input2.mode, PreampMode::HiZ);
        assert!(!state.input2.phantom_on);
        assert_eq!(state.input2.gain_raw, 0x2d);
    }

    #[test]
    fn preamp_gain_db_ranges_follow_mode() {
        let mic = PreampInputState::from_raw(0x41, 0x10);
        let line_negative = PreampInputState::from_raw(0xfa, 0x11);
        let line = PreampInputState::from_raw(0x14, 0x11);
        let hiz = PreampInputState::from_raw(0x2d, 0x12);

        assert_eq!(mic.gain_db_label(), "65 dB");
        assert_eq!(line_negative.gain_db_label(), "-6 dB");
        assert_eq!(line.gain_db_label(), "+20 dB");
        assert_eq!(hiz.gain_db_label(), "45 dB");
        assert_eq!(mic.gain_ratio(), 1.0);
    }
}
