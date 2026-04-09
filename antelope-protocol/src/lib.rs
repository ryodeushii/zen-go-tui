//! Protocol definitions and encoding/decoding for Antelope Audio Zen Go Synergy Core.
//!
//! This crate provides types and functions for communicating with the Zen Go Synergy Core
//! audio interface over USB HID. It covers:
//!
//! - **Frame parsing**: Decode incoming HID reports into typed [`Frame`] variants
//! - **Command encoding**: Build outgoing HID frames via [`encode_command`]
//! - **State types**: Strongly-typed representations of device state (sample rate, clock source,
//!   preamp settings, mixer strips, etc.)
//! - **Startup queries**: The sequence of queries sent during device initialization via
//!   [`control_panel_startup_queries`]
//!
//! # Example
//!
//! ```no_run
//! use antelope_protocol::{Frame, Command, encode_command, SampleRate};
//!
//! // Parse an incoming frame
//! let raw = vec![0u8; 320];
//! let frame = Frame::parse(&raw).unwrap();
//!
//! // Encode a command
//! let cmd = Command::SetSampleRate(SampleRate::Hz48000);
//! let encoded = encode_command(cmd);
//! ```

use thiserror::Error;

/// Size of a HID report frame in bytes.
pub const HID_REPORT_SIZE: usize = 320;

/// Errors that can occur during protocol frame parsing.
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// The frame is shorter than the minimum required length.
    #[error("frame too short: {0}")]
    FrameTooShort(usize),
    /// The frame type identifier is not recognized.
    #[error("unsupported frame type: 0x{0:02x}")]
    UnsupportedFrame(u32),
}

/// Supported sample rates for the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRate {
    /// 32,000 Hz
    Hz32000,
    /// 44,100 Hz
    Hz44100,
    /// 48,000 Hz
    Hz48000,
    /// 88,200 Hz
    Hz88200,
    /// 96,000 Hz
    Hz96000,
    /// 176,400 Hz
    Hz176400,
    /// 192,000 Hz
    Hz192000,
    /// An unrecognized sample rate code.
    Unknown(u8),
}

impl SampleRate {
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

    pub fn label(self) -> String {
        self.hz()
            .map(|hz| format!("{} Hz", hz))
            .unwrap_or_else(|| format!("Unknown (0x{:02x})", self.code()))
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSource {
    Internal,
    Spdif,
    Usb,
    Unknown(u8),
}

impl ClockSource {
    pub fn from_code(code: u8) -> Self {
        match code {
            0x00 => Self::Internal,
            0x01 => Self::Spdif,
            0x02 => Self::Usb,
            value => Self::Unknown(value),
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Internal => 0x00,
            Self::Spdif => 0x01,
            Self::Usb => 0x02,
            Self::Unknown(value) => value,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Internal => "Internal",
            Self::Spdif => "S/PDIF",
            Self::Usb => "USB",
            Self::Unknown(_) => "Unknown",
        }
    }

    pub fn all_confirmed() -> &'static [ClockSource] {
        const ALL: [ClockSource; 3] = [ClockSource::Internal, ClockSource::Spdif, ClockSource::Usb];
        &ALL
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Normal,
    Mute,
    Dim,
    Unknown(u8),
}

impl OutputMode {
    pub fn from_code(code: u8) -> Self {
        match code {
            0x00 => Self::Normal,
            0x01 => Self::Mute,
            0x02 => Self::Dim,
            value => Self::Unknown(value),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Mute => "Mute",
            Self::Dim => "Dim",
            Self::Unknown(_) => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTarget {
    Monitor,
    Hp1,
    Hp2,
}

impl OutputTarget {
    pub fn index(self) -> u8 {
        match self {
            Self::Monitor => 0x00,
            Self::Hp1 => 0x01,
            Self::Hp2 => 0x02,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Monitor,
            1 => Self::Hp1,
            _ => Self::Hp2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Monitor => "Monitor",
            Self::Hp1 => "HP1",
            Self::Hp2 => "HP2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    MonitorHp1,
    Hp2,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreampMode {
    Mic,
    Line,
    HiZ,
    Unknown(u8),
}

impl PreampMode {
    pub fn from_raw(mode: u8) -> Self {
        match mode & 0x0f {
            0x00 => Self::Mic,
            0x01 => Self::Line,
            0x02 => Self::HiZ,
            value => Self::Unknown(value),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mic => "Mic",
            Self::Line => "Line",
            Self::HiZ => "Hi-Z",
            Self::Unknown(_) => "Unknown",
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Mic => 0x00,
            Self::Line => 0x01,
            Self::HiZ => 0x02,
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreampInputState {
    pub gain_raw: u8,
    pub mode: PreampMode,
    pub phantom_on: bool,
    pub mode_raw: u8,
    pub observed_meter: Option<u8>,
}

impl PreampInputState {
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

    pub fn gain_db_label(self) -> String {
        match self.mode {
            PreampMode::Mic => format!("{} dB", self.gain_raw.min(0x41)),
            PreampMode::Line => format!("{:+} dB", i8::from_ne_bytes([self.gain_raw])),
            PreampMode::HiZ => format!("{} dB", self.gain_raw.min(0x2d)),
            PreampMode::Unknown(_) => format!("raw {:02x}", self.gain_raw),
        }
    }

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

    pub fn observed_meter_db(self) -> Option<i16> {
        self.observed_meter.and_then(meter_display_db)
    }

    pub fn observed_meter_ratio(self) -> Option<f64> {
        self.observed_meter.map(meter_ratio)
    }
}

pub fn meter_display_db(raw: u8) -> Option<i16> {
    (raw <= 0x3c).then_some(-(raw as i16))
}

pub fn meter_db_ratio(db: i16) -> f64 {
    let db = db.clamp(-60, 0) as f64;
    let min_amplitude = 10_f64.powf(-60.0 / 20.0);
    let amplitude = 10_f64.powf(db / 20.0);
    ((amplitude - min_amplitude) / (1.0 - min_amplitude)).clamp(0.0, 1.0)
}

pub fn meter_ratio(raw: u8) -> f64 {
    meter_display_db(raw).map(meter_db_ratio).unwrap_or(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreampState {
    pub input1: PreampInputState,
    pub input2: PreampInputState,
    pub cluster: [u8; 4],
}

impl Default for PreampState {
    fn default() -> Self {
        Self::from_cluster([0; 4])
    }
}

impl PreampState {
    pub fn from_cluster(cluster: [u8; 4]) -> Self {
        Self {
            input1: PreampInputState::from_raw(cluster[0], cluster[2]),
            input2: PreampInputState::from_raw(cluster[1], cluster[3]),
            cluster,
        }
    }
}

impl Surface {
    pub fn from_code(code: u8) -> Self {
        match code {
            0x0f => Self::MonitorHp1,
            0x0c => Self::Hp2,
            value => Self::Unknown(value),
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::MonitorHp1 => 0x0f,
            Self::Hp2 => 0x0c,
            Self::Unknown(value) => value,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MonitorHp1 => "Monitor / HP1",
            Self::Hp2 => "HP2",
            Self::Unknown(_) => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerSurface {
    Mix1,
    Mix2,
}

impl MixerSurface {
    pub fn index(self) -> usize {
        match self {
            Self::Mix1 => 0,
            Self::Mix2 => 1,
        }
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Mix1 => 0x00,
            Self::Mix2 => 0x01,
        }
    }

    pub fn from_surface(surface: Surface) -> Self {
        match surface {
            Surface::Hp2 => Self::Mix2,
            Surface::MonitorHp1 | Surface::Unknown(_) => Self::Mix1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanState(u8);

impl PanState {
    pub const MIN: u8 = 0x02;
    pub const CENTER: u8 = 0x20;
    pub const MAX: u8 = 0x3e;
    const PAN_MASK: u8 = 0x3f;
    const MUTE_FLAG: u8 = 0x40;
    const SOLO_FLAG: u8 = 0x80;

    pub fn from_raw(raw: u8) -> Self {
        Self(raw.clamp(Self::MIN, Self::MAX))
    }

    pub fn from_state_code(code: u8) -> Self {
        Self::from_raw(code & Self::PAN_MASK)
    }

    pub fn left() -> Self {
        Self(Self::MIN)
    }

    pub fn center() -> Self {
        Self(Self::CENTER)
    }

    pub fn right() -> Self {
        Self(Self::MAX)
    }

    pub fn code(self) -> u8 {
        self.0
    }

    pub fn raw(self) -> u8 {
        self.0
    }

    pub fn state_code(self, muted: bool, soloed: bool) -> u8 {
        self.code()
            | if muted { Self::MUTE_FLAG } else { 0x00 }
            | if soloed { Self::SOLO_FLAG } else { 0x00 }
    }

    pub fn muted_code(self, muted: bool) -> u8 {
        self.state_code(muted, false)
    }

    pub fn state_code_is_muted(code: u8) -> bool {
        code & Self::MUTE_FLAG != 0
    }

    pub fn state_code_is_soloed(code: u8) -> bool {
        code & Self::SOLO_FLAG != 0
    }

    pub fn ratio(self) -> f64 {
        (self.raw().saturating_sub(Self::MIN) as f64 / (Self::MAX - Self::MIN) as f64)
            .clamp(0.0, 1.0)
    }

    pub fn display_percent(self) -> i16 {
        self.raw() as i16 - Self::CENTER as i16
    }
}

impl Default for PanState {
    fn default() -> Self {
        Self::center()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerAssignment {
    Preamp(u8),
    ComputerPlay(u8),
    SpdifIn(u8),
    Mute,
    Oscillator(u8),
    EmuMic(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerStripKind {
    EarlyAfxAdjacent,
    Ordinary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerStrip {
    pub channel: u8,
    pub kind: MixerStripKind,
}

impl MixerStrip {
    pub fn new(channel: u8) -> Option<Self> {
        if !(1..=16).contains(&channel) {
            return None;
        }

        Some(Self {
            channel,
            kind: if channel <= 4 {
                MixerStripKind::EarlyAfxAdjacent
            } else {
                MixerStripKind::Ordinary
            },
        })
    }

    pub fn ordinary(channel: u8) -> Option<Self> {
        let strip = Self::new(channel)?;
        matches!(strip.kind, MixerStripKind::Ordinary).then_some(strip)
    }

    pub fn assignment_entry_index(self) -> usize {
        (self.channel - 1) as usize
    }

    pub fn assignment_write_banks(self) -> &'static [u8] {
        match self.kind {
            MixerStripKind::EarlyAfxAdjacent => &[0x05],
            MixerStripKind::Ordinary if self.channel <= 8 => &[0x03, 0x06, 0x07, 0x08, 0x09],
            MixerStripKind::Ordinary => &[0x06, 0x07, 0x08, 0x09],
        }
    }

    pub fn assignment_write_is_grounded(channel: u8) -> bool {
        Self::new(channel).is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerLinkTarget {
    pub mixer: MixerSurface,
    pub left_channel: u8,
    pub right_channel: u8,
    pub selector: u8,
}

impl MixerLinkTarget {
    pub fn from_selector(mixer: MixerSurface, selector: u8) -> Option<Self> {
        match (mixer, selector) {
            (MixerSurface::Mix1, 0x00) => Some(Self {
                mixer,
                left_channel: 1,
                right_channel: 2,
                selector,
            }),
            (MixerSurface::Mix1, 0x01) => Some(Self {
                mixer,
                left_channel: 3,
                right_channel: 4,
                selector,
            }),
            (MixerSurface::Mix1, 0x02) => Some(Self {
                mixer,
                left_channel: 5,
                right_channel: 6,
                selector,
            }),
            (MixerSurface::Mix1, 0x03) => Some(Self {
                mixer,
                left_channel: 7,
                right_channel: 8,
                selector,
            }),
            (MixerSurface::Mix1, 0x04) => Some(Self {
                mixer,
                left_channel: 9,
                right_channel: 10,
                selector,
            }),
            (MixerSurface::Mix1, 0x05) => Some(Self {
                mixer,
                left_channel: 11,
                right_channel: 12,
                selector,
            }),
            (MixerSurface::Mix1, 0x06) => Some(Self {
                mixer,
                left_channel: 13,
                right_channel: 14,
                selector,
            }),
            (MixerSurface::Mix1, 0x07) => Some(Self {
                mixer,
                left_channel: 15,
                right_channel: 16,
                selector,
            }),
            (MixerSurface::Mix2, 0x10) => Some(Self {
                mixer,
                left_channel: 1,
                right_channel: 2,
                selector,
            }),
            (MixerSurface::Mix2, 0x11) => Some(Self {
                mixer,
                left_channel: 3,
                right_channel: 4,
                selector,
            }),
            (MixerSurface::Mix2, 0x12) => Some(Self {
                mixer,
                left_channel: 5,
                right_channel: 6,
                selector,
            }),
            (MixerSurface::Mix2, 0x13) => Some(Self {
                mixer,
                left_channel: 7,
                right_channel: 8,
                selector,
            }),
            (MixerSurface::Mix2, 0x14) => Some(Self {
                mixer,
                left_channel: 9,
                right_channel: 10,
                selector,
            }),
            (MixerSurface::Mix2, 0x15) => Some(Self {
                mixer,
                left_channel: 11,
                right_channel: 12,
                selector,
            }),
            (MixerSurface::Mix2, 0x16) => Some(Self {
                mixer,
                left_channel: 13,
                right_channel: 14,
                selector,
            }),
            (MixerSurface::Mix2, 0x17) => Some(Self {
                mixer,
                left_channel: 15,
                right_channel: 16,
                selector,
            }),
            _ => None,
        }
    }

    pub fn from_channel(mixer: MixerSurface, channel: u8) -> Option<Self> {
        match (mixer, channel) {
            (MixerSurface::Mix1, 1 | 2) => Self::from_selector(mixer, 0x00),
            (MixerSurface::Mix1, 3 | 4) => Self::from_selector(mixer, 0x01),
            (MixerSurface::Mix1, 5 | 6) => Self::from_selector(mixer, 0x02),
            (MixerSurface::Mix1, 7 | 8) => Self::from_selector(mixer, 0x03),
            (MixerSurface::Mix1, 9 | 10) => Self::from_selector(mixer, 0x04),
            (MixerSurface::Mix1, 11 | 12) => Self::from_selector(mixer, 0x05),
            (MixerSurface::Mix1, 13 | 14) => Self::from_selector(mixer, 0x06),
            (MixerSurface::Mix1, 15 | 16) => Self::from_selector(mixer, 0x07),
            (MixerSurface::Mix2, 1 | 2) => Self::from_selector(mixer, 0x10),
            (MixerSurface::Mix2, 3 | 4) => Self::from_selector(mixer, 0x11),
            (MixerSurface::Mix2, 5 | 6) => Self::from_selector(mixer, 0x12),
            (MixerSurface::Mix2, 7 | 8) => Self::from_selector(mixer, 0x13),
            (MixerSurface::Mix2, 9 | 10) => Self::from_selector(mixer, 0x14),
            (MixerSurface::Mix2, 11 | 12) => Self::from_selector(mixer, 0x15),
            (MixerSurface::Mix2, 13 | 14) => Self::from_selector(mixer, 0x16),
            (MixerSurface::Mix2, 15 | 16) => Self::from_selector(mixer, 0x17),
            _ => None,
        }
    }

    pub fn companion_bank(self) -> Option<u8> {
        match (self.mixer, self.selector) {
            (MixerSurface::Mix1, 0x00) => Some(0x00),
            (MixerSurface::Mix1, 0x01) => Some(0x01),
            (MixerSurface::Mix2, 0x10) => Some(0x00),
            (MixerSurface::Mix2, 0x11) => Some(0x01),
            _ => None,
        }
    }
}

impl MixerAssignment {
    pub fn grounded_choices() -> &'static [MixerAssignment] {
        const CHOICES: [MixerAssignment; 17] = [
            MixerAssignment::Mute,
            MixerAssignment::Preamp(1),
            MixerAssignment::Preamp(2),
            MixerAssignment::ComputerPlay(1),
            MixerAssignment::ComputerPlay(2),
            MixerAssignment::ComputerPlay(3),
            MixerAssignment::ComputerPlay(4),
            MixerAssignment::ComputerPlay(5),
            MixerAssignment::ComputerPlay(6),
            MixerAssignment::ComputerPlay(7),
            MixerAssignment::ComputerPlay(8),
            MixerAssignment::SpdifIn(1),
            MixerAssignment::SpdifIn(2),
            MixerAssignment::Oscillator(1),
            MixerAssignment::Oscillator(2),
            MixerAssignment::EmuMic(1),
            MixerAssignment::EmuMic(2),
        ];
        &CHOICES
    }

    pub fn from_ordinary_strip_bytes(bytes: [u8; 2]) -> Option<Self> {
        match bytes {
            [0x00, 0x00] => Some(Self::Preamp(1)),
            [0x00, 0x01] => Some(Self::Preamp(2)),
            [0x01, 0x00..=0x07] => Some(Self::ComputerPlay(bytes[1] + 1)),
            [0x02, 0x00] => Some(Self::SpdifIn(1)),
            [0x02, 0x01] => Some(Self::SpdifIn(2)),
            [0x08, 0x00] => Some(Self::Mute),
            [0x09, 0x00] => Some(Self::Oscillator(1)),
            [0x09, 0x01] => Some(Self::Oscillator(2)),
            [0x0a, 0x00] => Some(Self::EmuMic(1)),
            [0x0a, 0x01] => Some(Self::EmuMic(2)),
            _ => None,
        }
    }

    pub fn ordinary_strip_bytes(self) -> [u8; 2] {
        match self {
            Self::Preamp(1) => [0x00, 0x00],
            Self::Preamp(2) => [0x00, 0x01],
            Self::ComputerPlay(channel @ 1..=8) => [0x01, channel - 1],
            Self::SpdifIn(1) => [0x02, 0x00],
            Self::SpdifIn(2) => [0x02, 0x01],
            Self::Mute => [0x08, 0x00],
            Self::Oscillator(1) => [0x09, 0x00],
            Self::Oscillator(2) => [0x09, 0x01],
            Self::EmuMic(1) => [0x0a, 0x00],
            Self::EmuMic(2) => [0x0a, 0x01],
            _ => panic!("unsupported grounded assignment variant"),
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Preamp(index) => format!("Preamp {}", index),
            Self::ComputerPlay(index) => format!("Computer Play {}", index),
            Self::SpdifIn(index) => format!("SPDIF In {}", index),
            Self::Mute => "Mute".to_string(),
            Self::Oscillator(index) => format!("Oscillator {}", index),
            Self::EmuMic(index) => format!("Emu Mic {}", index),
        }
    }

    pub fn short_label(self) -> String {
        match self {
            Self::Preamp(index) => format!("P{}", index),
            Self::ComputerPlay(index) => format!("C{}", index),
            Self::SpdifIn(index) => format!("S{}", index),
            Self::Mute => "M".to_string(),
            Self::Oscillator(index) => format!("O{}", index),
            Self::EmuMic(index) => format!("E{}", index),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputState {
    pub target: OutputTarget,
    pub volume: u8,
    pub mode: OutputMode,
}

impl OutputState {
    pub fn new(target: OutputTarget, volume: u8, mode: OutputMode) -> Self {
        Self {
            target,
            volume,
            mode,
        }
    }

    pub fn attenuation_steps(self) -> u8 {
        self.volume.min(0x60)
    }

    pub fn display_db(self) -> i16 {
        -(self.attenuation_steps() as i16)
    }

    pub fn gain_ratio(self) -> f64 {
        let attenuation = self.attenuation_steps() as f64;
        (1.0 - attenuation / 96.0).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStateSnapshot {
    pub sample_rate: SampleRate,
    pub clock_source: ClockSource,
    pub sample_rate_hz: u32,
    pub status_flags: [u8; 2],
    pub front_panel_bytes: [u8; 3],
    pub outputs: [OutputState; 3],
    pub dsp_cluster: [u8; 4],
    pub preamp: PreampState,
    pub surface: Surface,
    pub mixer_decode: MixerPassiveDecode,
    pub late_shadow: [u8; 12],
}

impl DeviceStateSnapshot {
    pub fn output(&self, target: OutputTarget) -> OutputState {
        self.outputs[target.index() as usize]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceMetadata {
    pub product_name: String,
    pub serial: String,
    pub hardware_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupQueryKind {
    Metadata,
    CapabilityDefaults,
    StatusValue,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryRequest {
    pub query_id: u8,
    pub sub_id: u8,
}

impl QueryRequest {
    pub const fn new(query_id: u8, sub_id: u8) -> Self {
        Self { query_id, sub_id }
    }
}

impl StartupQueryKind {
    pub fn from_query_id(query_id: u8) -> Self {
        match query_id {
            0x01 => Self::Metadata,
            0x00 => Self::CapabilityDefaults,
            0x11 => Self::StatusValue,
            value => Self::Unknown(value),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Metadata => "Metadata",
            Self::CapabilityDefaults => "Capability/default block",
            Self::StatusValue => "Status/capability value",
            Self::Unknown(_) => "Unknown query reply",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryResponse {
    pub query_id: u8,
    pub sub_id: u8,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueriedMixerStripState {
    pub level: u8,
    pub pan: PanState,
    pub muted: bool,
    pub soloed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueriedMixerSurfaceReadback {
    pub surfaces: [[QueriedMixerStripState; 16]; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPanCategory {
    Center,
    Left,
    Right,
}

impl StartupPanCategory {
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Center => "C",
            Self::Left => "L",
            Self::Right => "R",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupMixerStripState {
    pub level: u8,
    pub pan: PanState,
    pub muted: bool,
    pub soloed: bool,
}

impl Default for QueriedMixerStripState {
    fn default() -> Self {
        Self {
            level: 0,
            pan: PanState::center(),
            muted: false,
            soloed: false,
        }
    }
}

impl QueryResponse {
    pub fn kind(&self) -> StartupQueryKind {
        StartupQueryKind::from_query_id(self.query_id)
    }

    pub fn metadata(&self) -> Option<DeviceMetadata> {
        if self.query_id != 0x01 {
            return None;
        }

        let parts: Vec<String> = self
            .body
            .split(|byte| *byte == 0)
            .filter(|chunk| !chunk.is_empty())
            .map(|chunk| String::from_utf8_lossy(chunk).trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();

        if parts.len() < 3 {
            return None;
        }

        Some(DeviceMetadata {
            product_name: parts[0].clone(),
            serial: parts[1].clone(),
            hardware_version: parts[2].clone(),
        })
    }

    pub fn summary_label(&self) -> String {
        if let Some(entries) = self.startup_indexed_code_table() {
            let preview = entries
                .iter()
                .take(10)
                .map(|(index, code)| format!("{index:02x}:{code:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            return format!("Startup indexed code table [{}]", preview);
        }

        if let Some(bytes) = self.startup_quad_state() {
            return format!(
                "Startup quad state [{:02x} {:02x} {:02x} {:02x}]",
                bytes[0], bytes[1], bytes[2], bytes[3]
            );
        }

        if let Some((surface, categories)) = self.startup_pan_category_readback() {
            let preview = categories
                .iter()
                .map(|category| category.map(|value| value.short_label()).unwrap_or("?"))
                .collect::<Vec<_>>()
                .join(" ");
            return format!("Startup {:?} pan categories [{}]", surface, preview);
        }

        if let Some(bitmap) = self.selector_bitmap() {
            let asserted = bitmap
                .iter()
                .enumerate()
                .filter_map(|(index, enabled)| enabled.then_some(format!("{index:02x}")))
                .collect::<Vec<_>>();
            return format!(
                "Selector bitmap: {} asserted [{}]",
                asserted.len(),
                asserted.join(" ")
            );
        }

        if let Some(pairs) = self.selector_pair_bank() {
            let preview = pairs
                .iter()
                .take(8)
                .map(|(left, right)| format!("{left:02x}/{right:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            return format!(
                "Selector pair bank 0x{:02x}: {} pairs [{}]",
                self.sub_id,
                pairs.len(),
                preview
            );
        }

        match self.kind() {
            StartupQueryKind::Metadata => self
                .metadata()
                .map(|metadata| {
                    format!(
                        "{}: {} (hw {}, serial {})",
                        self.kind().label(),
                        metadata.product_name,
                        metadata.hardware_version,
                        metadata.serial
                    )
                })
                .unwrap_or_else(|| format!("{}: undecoded", self.kind().label())),
            StartupQueryKind::CapabilityDefaults | StartupQueryKind::StatusValue => format!(
                "{}: {} bytes [{}]",
                self.kind().label(),
                self.body.len(),
                self.body
                    .iter()
                    .take(8)
                    .map(|byte| format!("{:02x}", byte))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            StartupQueryKind::Unknown(id) => format!(
                "{} 0x{id:02x}/0x{:02x}: {} bytes",
                self.kind().label(),
                self.sub_id,
                self.body.len()
            ),
        }
    }

    pub fn assignment_readback(&self) -> Option<[Option<MixerAssignment>; 16]> {
        if self.query_id != 0x03 || self.body.is_empty() || self.body[0] != self.sub_id {
            return None;
        }

        let mut assignments = [None; 16];
        match self.sub_id {
            0x05 if self.body.len() >= 9 => {
                for (index, chunk) in self.body[1..9].chunks_exact(2).enumerate() {
                    assignments[index] =
                        MixerAssignment::from_ordinary_strip_bytes([chunk[0], chunk[1]]);
                }
                Some(assignments)
            }
            0x06..=0x09 if self.body.len() >= 33 => {
                for (index, chunk) in self.body[9..33].chunks_exact(2).enumerate() {
                    assignments[index + 4] =
                        MixerAssignment::from_ordinary_strip_bytes([chunk[0], chunk[1]]);
                }
                Some(assignments)
            }
            _ => None,
        }
    }

    pub fn selector_bitmap(&self) -> Option<[bool; 24]> {
        if self.query_id != 0x0b || self.sub_id != 0x03 || self.body.len() < 24 {
            return None;
        }

        let mut selectors = [false; 24];
        for (index, value) in self.body.iter().take(24).copied().enumerate() {
            selectors[index] = value != 0;
        }
        Some(selectors)
    }

    pub fn startup_link_readback_from_bitmap(
        &self,
    ) -> Option<[(MixerSurface, [Option<bool>; 16]); 2]> {
        let bitmap = self.selector_bitmap()?;
        let mut mix1 = [None; 16];
        let mut mix2 = [None; 16];

        for (bit, pair_start) in [
            (0_usize, 0_usize),
            (1, 2),
            (2, 4),
            (3, 6),
            (4, 8),
            (5, 10),
            (6, 12),
            (7, 14),
        ] {
            mix1[pair_start] = Some(bitmap[bit]);
            mix1[pair_start + 1] = Some(bitmap[bit]);
        }
        for (bit, pair_start) in [
            (16_usize, 0_usize),
            (17, 2),
            (18, 4),
            (19, 6),
            (20, 8),
            (21, 10),
            (22, 12),
            (23, 14),
        ] {
            mix2[pair_start] = Some(bitmap[bit]);
            mix2[pair_start + 1] = Some(bitmap[bit]);
        }

        Some([(MixerSurface::Mix1, mix1), (MixerSurface::Mix2, mix2)])
    }

    pub fn selector_pair_bank(&self) -> Option<Vec<(u8, u8)>> {
        if self.query_id != 0x04 || self.body.len() < 64 {
            return None;
        }

        Some(
            self.body[..64]
                .chunks_exact(2)
                .map(|chunk| (chunk[0], chunk[1]))
                .collect(),
        )
    }

    pub fn startup_pan_state_readback(
        &self,
    ) -> Option<(MixerSurface, [Option<StartupMixerStripState>; 16])> {
        let surface = match self.sub_id {
            0x00 => MixerSurface::Mix1,
            0x01 => MixerSurface::Mix2,
            _ => return None,
        };
        if self.query_id != 0x04 || self.body.len() < 34 {
            return None;
        }

        let mut states = [None; 16];
        for (index, code) in self.body.iter().skip(3).step_by(2).take(16).enumerate() {
            let level = self.body.get(2 + index * 2).copied().unwrap_or(0);
            states[index] = Some(StartupMixerStripState {
                level,
                pan: PanState::from_state_code(*code),
                muted: PanState::state_code_is_muted(*code),
                soloed: PanState::state_code_is_soloed(*code),
            });
        }

        Some((surface, states))
    }

    pub fn startup_pan_category_readback(
        &self,
    ) -> Option<(MixerSurface, [Option<StartupPanCategory>; 16])> {
        let (surface, states) = self.startup_pan_state_readback()?;
        let mut categories = [None; 16];
        for (index, state) in states.into_iter().enumerate() {
            let Some(state) = state else {
                continue;
            };
            categories[index] = Some(match state.pan.raw() {
                0x20 => StartupPanCategory::Center,
                0x02 => StartupPanCategory::Left,
                0x3e => StartupPanCategory::Right,
                raw if raw < 0x20 => StartupPanCategory::Left,
                _ => StartupPanCategory::Right,
            });
        }

        Some((surface, categories))
    }

    pub fn startup_indexed_code_table(&self) -> Option<Vec<(u8, u8)>> {
        if self.query_id != 0x15 || self.sub_id != 0x00 || self.body.len() < 64 {
            return None;
        }

        Some(
            self.body[..64]
                .chunks_exact(2)
                .map(|chunk| (chunk[0], chunk[1]))
                .collect(),
        )
    }

    pub fn startup_quad_state(&self) -> Option<[u8; 4]> {
        if self.query_id != 0x17 || self.sub_id != 0x00 || self.body.len() < 4 {
            return None;
        }

        Some(self.body[..4].try_into().ok()?)
    }

    pub fn mixer_strip_readback(&self) -> Option<QueriedMixerSurfaceReadback> {
        if self.query_id != 0x18 || self.sub_id != 0x00 || self.body.len() < 64 {
            return None;
        }

        let mut surfaces = [[QueriedMixerStripState::default(); 16]; 2];
        for (index, chunk) in self.body[..64].chunks_exact(2).enumerate() {
            let surface = index / 16;
            let channel = index % 16;
            surfaces[surface][channel] = QueriedMixerStripState {
                level: chunk[0].min(0x5a),
                pan: PanState::from_state_code(chunk[1]),
                muted: PanState::state_code_is_muted(chunk[1]),
                soloed: PanState::state_code_is_soloed(chunk[1]),
            };
        }

        Some(QueriedMixerSurfaceReadback { surfaces })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceNotification {
    pub bytes: [u8; 6],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Snapshot {
        snapshot: DeviceStateSnapshot,
        raw: Vec<u8>,
    },
    QueryReply {
        reply: QueryResponse,
        raw: Vec<u8>,
    },
    Auxiliary {
        bytes: Vec<u8>,
        raw: Vec<u8>,
    },
    Notification {
        notification: DeviceNotification,
        raw: Vec<u8>,
    },
}

impl Frame {
    pub fn parse(bytes: &[u8]) -> Result<Self, ProtocolError> {
        Self::parse_owned(bytes.to_vec())
    }

    pub fn parse_owned(bytes: Vec<u8>) -> Result<Self, ProtocolError> {
        if bytes.len() < 6 {
            return Err(ProtocolError::FrameTooShort(bytes.len()));
        }

        if bytes.len() == 6 {
            let mut raw = [0_u8; 6];
            raw.copy_from_slice(&bytes);
            return Ok(Self::Notification {
                notification: DeviceNotification { bytes: raw },
                raw: bytes,
            });
        }

        if bytes.len() < 0x12 {
            return Err(ProtocolError::FrameTooShort(bytes.len()));
        }

        let frame_type = u32::from_le_bytes(bytes[0..4].try_into().expect("type header"));
        match frame_type {
            0x73 => Ok(Self::Snapshot {
                snapshot: parse_snapshot73(&bytes)?,
                raw: bytes,
            }),
            0x75 => Ok(Self::QueryReply {
                reply: QueryResponse {
                    query_id: bytes[0x08],
                    sub_id: bytes[0x0c],
                    body: bytes[0x10..].to_vec(),
                },
                raw: bytes,
            }),
            0x83 => Ok(Self::Auxiliary {
                bytes: bytes[0x10..].to_vec(),
                raw: bytes,
            }),
            other => Err(ProtocolError::UnsupportedFrame(other)),
        }
    }

    pub fn as_snapshot(&self) -> Option<&DeviceStateSnapshot> {
        match self {
            Self::Snapshot { snapshot, .. } => Some(snapshot),
            _ => None,
        }
    }

    pub fn as_query_reply(&self) -> Option<&QueryResponse> {
        match self {
            Self::QueryReply { reply, .. } => Some(reply),
            _ => None,
        }
    }

    pub fn raw_bytes(&self) -> &[u8] {
        match self {
            Self::Snapshot { raw, .. } => raw,
            Self::QueryReply { raw, .. } => raw,
            Self::Auxiliary { raw, .. } => raw,
            Self::Notification { raw, .. } => raw,
        }
    }

    pub fn into_snapshot_and_raw(self) -> (DeviceSnapshot, Vec<u8>) {
        match self {
            Self::Snapshot { snapshot, raw } => (DeviceSnapshot::Snapshot(snapshot), raw),
            Self::QueryReply { reply, raw } => (DeviceSnapshot::QueryReply(reply), raw),
            Self::Auxiliary { bytes, raw } => (DeviceSnapshot::Auxiliary(bytes), raw),
            Self::Notification { notification, raw } => {
                (DeviceSnapshot::Notification(notification), raw)
            }
        }
    }
}

fn parse_snapshot73(bytes: &[u8]) -> Result<DeviceStateSnapshot, ProtocolError> {
    if bytes.len() < 0x10 + 0xe6 {
        return Err(ProtocolError::FrameTooShort(bytes.len()));
    }

    let payload = &bytes[0x10..];
    Ok(DeviceStateSnapshot {
        sample_rate: SampleRate::from_code(payload[0x02]),
        clock_source: ClockSource::from_code(payload[0x03]),
        sample_rate_hz: u32::from_be_bytes(payload[0x04..0x08].try_into().expect("sample rate")),
        status_flags: [payload[0x00], payload[0x01]],
        front_panel_bytes: [payload[0x08], payload[0x09], payload[0x0a]],
        outputs: [
            OutputState::new(
                OutputTarget::Monitor,
                payload[0x0c],
                OutputMode::from_code(payload[0x0d]),
            ),
            OutputState::new(
                OutputTarget::Hp1,
                payload[0x0e],
                OutputMode::from_code(payload[0x0f]),
            ),
            OutputState::new(
                OutputTarget::Hp2,
                payload[0x10],
                OutputMode::from_code(payload[0x11]),
            ),
        ],
        dsp_cluster: [payload[0x18], payload[0x19], payload[0x1a], payload[0x1b]],
        preamp: PreampState::from_cluster([
            payload[0x18],
            payload[0x19],
            payload[0x1a],
            payload[0x1b],
        ]),
        surface: Surface::from_code(payload[0x6a]),
        mixer_decode: decode_passive_mixer_state(payload),
        late_shadow: [
            payload[0xda],
            payload[0xdb],
            payload[0xdc],
            payload[0xdd],
            payload[0xde],
            payload[0xdf],
            payload[0xe0],
            payload[0xe1],
            payload[0xe2],
            payload[0xe3],
            payload[0xe4],
            payload[0xe5],
        ],
    })
}

fn decode_passive_mixer_state(payload: &[u8]) -> MixerPassiveDecode {
    let mut decode = MixerPassiveDecode::default();

    let shared_mute = decode_mute_from_group(payload, 0x8f, 0xcf, 0xda, 0xdb, 0xdc, 0xdd);
    let shared_pan = decode_pan_from_group(payload, 0x8f, 0xcf, 0xda, 0xdd, 0xde, 0xdf);

    let active_mixer = MixerSurface::from_surface(Surface::from_code(payload[0x6a]));
    decode.observed_preamp1_meter = decode_preamp_meter(payload, 0xce);
    decode.observed_preamp2_meter = decode_preamp_meter(payload, 0xcf);
    for channel in 1..=16 {
        let meter = decode_strip_meter(payload, channel);
        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            if let Some(slot) = decode.surfaces[mixer.index()].get_mut(channel as usize - 1) {
                slot.meter = meter;
            }
        }
    }
    if let Some(slot) = decode.surfaces[active_mixer.index()].get_mut(0) {
        slot.muted = shared_mute;
        slot.pan = shared_pan;
    }

    if let Some(linked) = decode_link_state(payload) {
        let targets: &[(MixerSurface, u8)] = match active_mixer {
            MixerSurface::Mix1 => &[(MixerSurface::Mix1, 1), (MixerSurface::Mix1, 2)],
            MixerSurface::Mix2 => &[(MixerSurface::Mix2, 1), (MixerSurface::Mix2, 2)],
        };
        for (mixer, channel) in targets {
            if let Some(slot) = decode.surfaces[mixer.index()].get_mut(*channel as usize - 1) {
                slot.linked = Some(linked);
            }
        }
    }

    decode
}

fn decode_strip_meter(payload: &[u8], channel: u8) -> Option<u8> {
    let meter_lanes = payload.get(0x8e..=0x9d)?;
    if meter_lanes.iter().all(|lane| *lane == 0x00) {
        return None;
    }

    meter_lanes
        .get(channel.checked_sub(1)? as usize)
        .copied()
        .filter(|raw| *raw <= 0x60)
}

fn decode_preamp_meter(payload: &[u8], offset: usize) -> Option<u8> {
    payload
        .get(offset)
        .copied()
        .filter(|raw| *raw != 0x00 && *raw <= 0x49)
}

fn decode_mute_from_group(
    payload: &[u8],
    primary_a: usize,
    primary_b: usize,
    shadow_a: usize,
    shadow_b: usize,
    shadow_c: usize,
    shadow_d: usize,
) -> Option<bool> {
    let values = [
        payload.get(primary_a).copied()?,
        payload.get(primary_b).copied()?,
        payload.get(shadow_a).copied()?,
        payload.get(shadow_b).copied()?,
        payload.get(shadow_c).copied()?,
        payload.get(shadow_d).copied()?,
    ];

    let all_51 = values.iter().all(|value| *value == 0x51);
    let all_active = values
        .iter()
        .all(|value| matches!(*value, 0x49 | 0x4b | 0x4c | 0x4e | 0x51));
    if all_51 {
        Some(true)
    } else if all_active {
        Some(false)
    } else {
        None
    }
}

fn decode_pan_from_group(
    payload: &[u8],
    primary_a: usize,
    primary_b: usize,
    shadow_a: usize,
    shadow_b: usize,
    tail_a: usize,
    tail_b: usize,
) -> Option<PanState> {
    let samples = [
        payload.get(primary_a).copied()?,
        payload.get(primary_b).copied()?,
        payload.get(shadow_a).copied()?,
        payload.get(shadow_b).copied()?,
        payload.get(tail_a).copied()?,
        payload.get(tail_b).copied()?,
    ];
    let value =
        (samples.iter().map(|&sample| sample as u16).sum::<u16>() / samples.len() as u16) as u8;
    let centered = match value {
        0x49..=0x4c => 0x20,
        0x4d..=0x4e => 0x1e,
        _ => return None,
    };
    Some(PanState::from_raw(centered))
}

fn decode_link_state(payload: &[u8]) -> Option<bool> {
    let head_a = payload.get(0x8f).copied()?;
    let head_b = payload.get(0xcf).copied()?;
    let tails = [
        payload.get(0xda).copied()?,
        payload.get(0xdb).copied()?,
        payload.get(0xdc).copied()?,
        payload.get(0xdd).copied()?,
        payload.get(0xde).copied()?,
        payload.get(0xdf).copied()?,
    ];
    let values = [
        head_a, head_b, tails[0], tails[1], tails[2], tails[3], tails[4], tails[5],
    ];

    if values.iter().all(|value| *value == 0x49) {
        Some(true)
    } else if head_a == 0x51 && head_b == 0x51 && tails.iter().all(|value| *value == 0x4e) {
        Some(true)
    } else if head_a == 0x4e
        && head_b == 0x4e
        && tails.iter().all(|value| matches!(*value, 0x4c | 0x4e))
    {
        Some(false)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSnapshot {
    Snapshot(DeviceStateSnapshot),
    Auxiliary(Vec<u8>),
    QueryReply(QueryResponse),
    Notification(DeviceNotification),
}

impl From<Frame> for DeviceSnapshot {
    fn from(frame: Frame) -> Self {
        match frame {
            Frame::Snapshot { snapshot, .. } => Self::Snapshot(snapshot),
            Frame::Auxiliary { bytes, .. } => Self::Auxiliary(bytes),
            Frame::QueryReply { reply, .. } => Self::QueryReply(reply),
            Frame::Notification { notification, .. } => Self::Notification(notification),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SetSampleRate(SampleRate),
    SetClockSource(ClockSource),
    SelectSurface(Surface),
    SetPreampMode {
        input: u8,
        mode: PreampMode,
    },
    SetPreampGain {
        input: u8,
        raw: u8,
    },
    SetPreampPhantom {
        input: u8,
        enabled: bool,
    },
    SetPreampPhase {
        input: u8,
        enabled: bool,
    },
    SetOutputVolume {
        target: OutputTarget,
        step: u8,
    },
    SetOutputMute {
        target: OutputTarget,
        enabled: bool,
    },
    SetOutputDim {
        target: OutputTarget,
        enabled: bool,
    },
    SetMixerLevel {
        mixer: MixerSurface,
        channel: u8,
        level: u8,
        pan_state: PanState,
        muted: bool,
        soloed: bool,
    },
    SetMixerMute {
        mixer: MixerSurface,
        channel: u8,
        muted: bool,
        pan_state: PanState,
        soloed: bool,
    },
    SetMixerSolo {
        mixer: MixerSurface,
        channel: u8,
        soloed: bool,
        muted: bool,
        pan_state: PanState,
    },
    SetMixerPan {
        mixer: MixerSurface,
        channel: u8,
        pan: PanState,
        muted: bool,
        soloed: bool,
    },
    SetMixerAssignment {
        strip: u8,
        assignment: MixerAssignment,
    },
    SetLinkState {
        selector: u8,
        enabled: bool,
        companion_bank: Option<u8>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerChannelState {
    pub channel: u8,
    pub level: Option<u8>,
    pub meter: Option<u8>,
    pub muted: Option<bool>,
    pub soloed: Option<bool>,
    pub pan: PanState,
    pub assignment: Option<MixerAssignment>,
    pub linked: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerPassiveStripState {
    pub meter: Option<u8>,
    pub muted: Option<bool>,
    pub pan: Option<PanState>,
    pub linked: Option<bool>,
}

impl MixerPassiveStripState {
    pub const fn unresolved() -> Self {
        Self {
            meter: None,
            muted: None,
            pan: None,
            linked: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixerPassiveDecode {
    pub surfaces: [[MixerPassiveStripState; 16]; 2],
    pub observed_preamp1_meter: Option<u8>,
    pub observed_preamp2_meter: Option<u8>,
}

impl Default for MixerPassiveDecode {
    fn default() -> Self {
        Self {
            surfaces: [[MixerPassiveStripState::unresolved(); 16]; 2],
            observed_preamp1_meter: None,
            observed_preamp2_meter: None,
        }
    }
}

impl MixerPassiveDecode {
    pub fn strip(&self, mixer: MixerSurface, channel: u8) -> Option<MixerPassiveStripState> {
        let index = channel.checked_sub(1)? as usize;
        self.surfaces
            .get(mixer.index())
            .and_then(|surface| surface.get(index))
            .copied()
    }
}

impl MixerChannelState {
    pub fn unknown(channel: u8) -> Self {
        Self {
            channel,
            level: None,
            meter: None,
            muted: None,
            soloed: None,
            pan: PanState::center(),
            assignment: None,
            linked: None,
        }
    }

    pub fn known(
        channel: u8,
        level: Option<u8>,
        muted: Option<bool>,
        pan: PanState,
        assignment: Option<MixerAssignment>,
        linked: Option<bool>,
    ) -> Self {
        Self {
            channel,
            level,
            meter: None,
            muted,
            soloed: None,
            pan,
            assignment,
            linked,
        }
    }

    pub fn display_db(self) -> Option<i16> {
        self.level.map(|raw| -(raw.min(0x5a) as i16))
    }

    pub fn gain_ratio(self) -> Option<f64> {
        self.level
            .map(|raw| (1.0 - (raw.min(0x5a) as f64 / 90.0)).clamp(0.0, 1.0))
    }

    pub fn meter_ratio(self) -> Option<f64> {
        self.meter.map(meter_ratio)
    }

    pub fn meter_db(self) -> Option<i16> {
        self.meter.and_then(meter_display_db)
    }
}

pub fn control_panel_startup_queries() -> &'static [QueryRequest] {
    const QUERIES: [QueryRequest; 47] = [
        QueryRequest::new(0x01, 0x00),
        QueryRequest::new(0x11, 0x00),
        QueryRequest::new(0x0a, 0x00),
        QueryRequest::new(0x17, 0x00),
        QueryRequest::new(0x18, 0x00),
        QueryRequest::new(0x11, 0x01),
        QueryRequest::new(0x03, 0x00),
        QueryRequest::new(0x03, 0x01),
        QueryRequest::new(0x03, 0x02),
        QueryRequest::new(0x03, 0x03),
        QueryRequest::new(0x03, 0x04),
        QueryRequest::new(0x03, 0x05),
        QueryRequest::new(0x03, 0x06),
        QueryRequest::new(0x03, 0x07),
        QueryRequest::new(0x03, 0x08),
        QueryRequest::new(0x03, 0x09),
        QueryRequest::new(0x0b, 0x00),
        QueryRequest::new(0x16, 0x00),
        QueryRequest::new(0x0a, 0x00),
        QueryRequest::new(0x04, 0x00),
        QueryRequest::new(0x0b, 0x03),
        QueryRequest::new(0x04, 0x01),
        QueryRequest::new(0x0b, 0x03),
        QueryRequest::new(0x04, 0x02),
        QueryRequest::new(0x0b, 0x03),
        QueryRequest::new(0x04, 0x03),
        QueryRequest::new(0x0b, 0x03),
        QueryRequest::new(0x15, 0x00),
        QueryRequest::new(0x19, 0x00),
        QueryRequest::new(0x19, 0x01),
        QueryRequest::new(0x07, 0x27),
        QueryRequest::new(0x07, 0x2c),
        QueryRequest::new(0x07, 0x09),
        QueryRequest::new(0x07, 0x14),
        QueryRequest::new(0x07, 0x4c),
        QueryRequest::new(0x19, 0x02),
        QueryRequest::new(0x19, 0x03),
        QueryRequest::new(0x19, 0x04),
        QueryRequest::new(0x19, 0x05),
        QueryRequest::new(0x19, 0x06),
        QueryRequest::new(0x19, 0x07),
        QueryRequest::new(0x19, 0x08),
        QueryRequest::new(0x19, 0x09),
        QueryRequest::new(0x19, 0x0a),
        QueryRequest::new(0x19, 0x0b),
        QueryRequest::new(0x0b, 0x04),
        QueryRequest::new(0x12, 0x00),
    ];
    &QUERIES
}

pub fn encode_query(query: QueryRequest) -> Vec<u8> {
    let mut frame = vec![0_u8; HID_REPORT_SIZE];
    frame[0..4].copy_from_slice(&0x74_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x10_u32.to_le_bytes());
    frame[0x08] = query.query_id;
    frame[0x0c] = query.sub_id;
    frame
}

pub fn encode_command(command: Command) -> Vec<u8> {
    match command {
        Command::SetSampleRate(rate) => host_frame(0x12, &[0x03, rate.code()]),
        Command::SetClockSource(source) => host_frame(0x12, &[0x04, source.code()]),
        Command::SelectSurface(surface) => host_frame(0x13, &[0x49, 0x00, surface.code()]),
        Command::SetPreampMode { input, mode } => {
            host_frame(0x13, &[0x4f, input.min(1), mode.code()])
        }
        Command::SetPreampGain { input, raw } => host_frame(0x13, &[0x50, input.min(1), raw]),
        Command::SetPreampPhantom { input, enabled } => {
            host_frame(0x13, &[0x51, input.min(1), u8::from(enabled)])
        }
        Command::SetPreampPhase { input, enabled } => {
            host_frame(0x13, &[0x52, input.min(1), u8::from(enabled)])
        }
        Command::SetOutputVolume { target, step } => {
            host_frame(0x13, &[0x47, target.index(), step])
        }
        Command::SetOutputMute { target, enabled } => {
            host_frame(0x13, &[0x48, target.index(), u8::from(enabled)])
        }
        Command::SetOutputDim { target, enabled } => {
            host_frame(0x13, &[0x66, target.index(), u8::from(enabled)])
        }
        Command::SetMixerLevel {
            mixer,
            channel,
            level,
            pan_state,
            muted,
            soloed,
        } => host_frame(
            0x16,
            &[
                0xd4,
                0x04,
                mixer.code(),
                channel,
                level,
                pan_state.state_code(muted, soloed),
            ],
        ),
        Command::SetMixerMute {
            mixer,
            channel,
            muted,
            pan_state,
            soloed,
        } => host_frame(
            0x16,
            &[
                0xd4,
                0x04,
                mixer.code(),
                channel,
                0x00,
                pan_state.state_code(muted, soloed),
            ],
        ),
        Command::SetMixerSolo {
            mixer,
            channel,
            soloed,
            muted,
            pan_state,
        } => host_frame(
            0x16,
            &[
                0xd4,
                0x04,
                mixer.code(),
                channel,
                0x00,
                pan_state.state_code(muted, soloed),
            ],
        ),
        Command::SetMixerPan {
            mixer,
            channel,
            pan,
            muted,
            soloed,
        } => host_frame(
            0x16,
            &[
                0xd4,
                0x04,
                mixer.code(),
                channel,
                0x00,
                pan.state_code(muted, soloed),
            ],
        ),
        Command::SetMixerAssignment { strip, assignment } => {
            encode_mixer_assignment(strip, assignment)
        }
        Command::SetLinkState {
            selector,
            enabled,
            companion_bank: _,
        } => host_frame(0x14, &[0xa2, 0x03, selector, u8::from(enabled)]),
    }
}

pub fn encode_link_companion(bank: u8, enabled: bool) -> Vec<u8> {
    host_frame(0x14, &[0xa2, 0x04, bank, u8::from(enabled)])
}

fn encode_mixer_assignment(strip: u8, assignment: MixerAssignment) -> Vec<u8> {
    encode_mixer_assignment_frames(strip, assignment)
        .into_iter()
        .next()
        .expect("assignment write must emit at least one frame")
}

pub fn encode_mixer_assignment_frames(strip: u8, assignment: MixerAssignment) -> Vec<Vec<u8>> {
    let strip = MixerStrip::new(strip).expect("assignment write requires grounded strip mapping");
    let entry_index = strip.assignment_entry_index();
    let assignment_bytes = assignment.ordinary_strip_bytes();

    strip
        .assignment_write_banks()
        .iter()
        .copied()
        .map(|bank| {
            let mut frame = assignment_frame(bank);
            write_assignment_entry(&mut frame, entry_index, assignment_bytes);
            frame
        })
        .collect()
}

pub fn encode_mixer_assignment_frames_with_table(
    strip: u8,
    assignment: MixerAssignment,
    assignments: &[MixerAssignment; 16],
) -> Vec<Vec<u8>> {
    let strip = MixerStrip::new(strip).expect("assignment write requires grounded strip mapping");
    let mut full_assignments = *assignments;
    full_assignments[strip.assignment_entry_index()] = assignment;

    strip
        .assignment_write_banks()
        .iter()
        .copied()
        .map(|bank| {
            let mut frame = assignment_frame(bank);

            for entry_index in assignment_entries_for_bank(bank) {
                write_assignment_entry(
                    &mut frame,
                    entry_index,
                    assignment_entry_bytes(bank, entry_index, &full_assignments),
                );
            }

            frame
        })
        .collect()
}

fn assignment_frame(bank: u8) -> Vec<u8> {
    let mut frame = vec![0_u8; HID_REPORT_SIZE];
    frame[0..4].copy_from_slice(&0x70_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x53_u32.to_le_bytes());
    frame[0x10..0x13].copy_from_slice(&[0xd3, 0x41, bank]);
    frame
}

fn write_assignment_entry(frame: &mut [u8], entry_index: usize, assignment: [u8; 2]) {
    let tuple_offset = 0x03 + entry_index * 2;
    frame[0x10 + tuple_offset] = assignment[0];
    frame[0x10 + tuple_offset + 1] = assignment[1];
}

fn assignment_entries_for_bank(bank: u8) -> std::ops::Range<usize> {
    match bank {
        0x05 => 0..4,
        0x03 => 0..8,
        0x06 | 0x07 | 0x08 | 0x09 => 0..16,
        _ => 0..0,
    }
}

fn assignment_entry_bytes(
    bank: u8,
    entry_index: usize,
    assignments: &[MixerAssignment; 16],
) -> [u8; 2] {
    match (bank, entry_index) {
        (0x03 | 0x06 | 0x07 | 0x08 | 0x09, 0..=3) => [0x03, entry_index as u8],
        _ => assignments[entry_index].ordinary_strip_bytes(),
    }
}

fn host_frame(length: u32, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0_u8; HID_REPORT_SIZE];
    frame[0..4].copy_from_slice(&0x70_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&length.to_le_bytes());
    frame[0x10..0x10 + payload.len()].copy_from_slice(payload);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct ExperimentalSurfacePairLanes {
        mixer: MixerSurface,
        lane_a: u8,
        lane_b: u8,
        mirrored: bool,
    }

    fn experimental_surface_pair_lanes(payload: &[u8]) -> Option<ExperimentalSurfacePairLanes> {
        let mixer = MixerSurface::from_surface(Surface::from_code(*payload.get(0x6a)?));
        match mixer {
            MixerSurface::Mix1 => {
                let lane_a = *payload.get(0xda)?;
                let lane_b = *payload.get(0xdb)?;
                Some(ExperimentalSurfacePairLanes {
                    mixer,
                    lane_a,
                    lane_b,
                    mirrored: payload.get(0xdc) == Some(&lane_a)
                        && payload.get(0xdd) == Some(&lane_b),
                })
            }
            MixerSurface::Mix2 => Some(ExperimentalSurfacePairLanes {
                mixer,
                lane_a: *payload.get(0xde)?,
                lane_b: *payload.get(0xdf)?,
                mirrored: false,
            }),
        }
    }

    fn snapshot_payload(frame: &[u8]) -> &[u8] {
        &frame[0x10..]
    }

    fn empty_snapshot_frame() -> Vec<u8> {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame
    }

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
    fn ordinary_strip_index_map_stays_out_of_early_afx_range() {
        assert_eq!(MixerStrip::ordinary(4), None);
        assert_eq!(
            MixerStrip::ordinary(5).map(|strip| strip.assignment_entry_index()),
            Some(4)
        );
        assert_eq!(
            MixerStrip::ordinary(16).map(|strip| strip.assignment_entry_index()),
            Some(15)
        );
    }

    #[test]
    fn assignment_write_is_grounded_for_all_visible_strips() {
        for channel in 1..=16 {
            assert!(MixerStrip::assignment_write_is_grounded(channel));
        }
    }

    #[test]
    fn link_target_mapping_covers_full_visible_pair_map() {
        assert_eq!(
            MixerLinkTarget::from_selector(MixerSurface::Mix1, 0x03),
            Some(MixerLinkTarget {
                mixer: MixerSurface::Mix1,
                left_channel: 7,
                right_channel: 8,
                selector: 0x03,
            })
        );
        assert_eq!(
            MixerLinkTarget::from_selector(MixerSurface::Mix1, 0x01),
            Some(MixerLinkTarget {
                mixer: MixerSurface::Mix1,
                left_channel: 3,
                right_channel: 4,
                selector: 0x01,
            })
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 2).map(|target| target.selector),
            Some(0x10)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 4).map(|target| target.selector),
            Some(0x11)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 12).map(|target| target.selector),
            Some(0x05)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 7).map(|target| target.selector),
            Some(0x13)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 1)
                .and_then(|target| target.companion_bank()),
            Some(0x00)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 7)
                .and_then(|target| target.companion_bank()),
            None
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 1)
                .and_then(|target| target.companion_bank()),
            Some(0x00)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 3)
                .and_then(|target| target.companion_bank()),
            Some(0x01)
        );
    }

    #[test]
    fn passive_link_state_decode_matches_grounded_mix1_and_mix2_signatures() {
        let mut payload = vec![0_u8; 0xe6];
        payload[0x8f] = 0x51;
        payload[0xcf] = 0x51;
        payload[0xda] = 0x4e;
        payload[0xdb] = 0x4e;
        payload[0xdc] = 0x4e;
        payload[0xdd] = 0x4e;
        payload[0xde] = 0x4e;
        payload[0xdf] = 0x4e;
        assert_eq!(decode_link_state(&payload), Some(true));

        payload[0x8f] = 0x4e;
        payload[0xcf] = 0x4e;
        payload[0xda] = 0x4c;
        payload[0xdb] = 0x4c;
        payload[0xdc] = 0x4c;
        payload[0xdd] = 0x4c;
        payload[0xde] = 0x4c;
        payload[0xdf] = 0x4c;
        assert_eq!(decode_link_state(&payload), Some(false));

        payload[0x8f] = 0x49;
        payload[0xcf] = 0x49;
        payload[0xda] = 0x49;
        payload[0xdb] = 0x49;
        payload[0xdc] = 0x49;
        payload[0xdd] = 0x49;
        payload[0xde] = 0x49;
        payload[0xdf] = 0x49;
        assert_eq!(decode_link_state(&payload), Some(true));
    }

    #[test]
    fn mixer_channel_state_tracks_assignment_pan_and_link() {
        let state = MixerChannelState::known(
            16,
            Some(0x22),
            Some(true),
            PanState::from_raw(0x3e),
            Some(MixerAssignment::ComputerPlay(8)),
            Some(false),
        );

        assert_eq!(state.channel, 16);
        assert_eq!(state.assignment, Some(MixerAssignment::ComputerPlay(8)));
        assert_eq!(state.pan, PanState::from_raw(0x3e));
        assert_eq!(state.linked, Some(false));
    }

    #[test]
    fn mixer_assignment_decodes_grounded_ordinary_strip_values() {
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x00, 0x00]),
            Some(MixerAssignment::Preamp(1))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x00, 0x01]),
            Some(MixerAssignment::Preamp(2))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x01, 0x00]),
            Some(MixerAssignment::ComputerPlay(1))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x01, 0x06]),
            Some(MixerAssignment::ComputerPlay(7))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x01, 0x07]),
            Some(MixerAssignment::ComputerPlay(8))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x02, 0x00]),
            Some(MixerAssignment::SpdifIn(1))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x02, 0x01]),
            Some(MixerAssignment::SpdifIn(2))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x08, 0x00]),
            Some(MixerAssignment::Mute)
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x09, 0x00]),
            Some(MixerAssignment::Oscillator(1))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x09, 0x01]),
            Some(MixerAssignment::Oscillator(2))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x0a, 0x00]),
            Some(MixerAssignment::EmuMic(1))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x0a, 0x01]),
            Some(MixerAssignment::EmuMic(2))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x03, 0x00]),
            None
        );
    }

    #[test]
    fn encodes_ordinary_strip_assignment_write_sequence_for_strip_11() {
        let frames = encode_mixer_assignment_frames(11, MixerAssignment::EmuMic(2));

        assert_eq!(frames.len(), 4);
        for (frame, bank) in frames.iter().zip([0x06_u8, 0x07, 0x08, 0x09]) {
            assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
            assert_eq!(&frame[4..8], &0x53_u32.to_le_bytes());
            assert_eq!(&frame[0x10..0x13], &[0xd3, 0x41, bank]);
            assert_eq!(&frame[0x10 + 0x17..0x10 + 0x19], &[0x0a, 0x01]);
        }
    }

    #[test]
    fn encodes_ordinary_assignment_write_sequence_for_strip_5_with_bank_03() {
        let frames = encode_mixer_assignment_frames(5, MixerAssignment::Oscillator(1));

        assert_eq!(frames.len(), 5);
        for (frame, bank) in frames.iter().zip([0x03_u8, 0x06, 0x07, 0x08, 0x09]) {
            assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
            assert_eq!(&frame[4..8], &0x53_u32.to_le_bytes());
            assert_eq!(&frame[0x10..0x13], &[0xd3, 0x41, bank]);
            assert_eq!(&frame[0x10 + 0x0b..0x10 + 0x0d], &[0x09, 0x00]);
        }
    }

    #[test]
    fn encodes_early_assignment_write_sequence_for_strip_1_with_bank_05() {
        let frames = encode_mixer_assignment_frames(1, MixerAssignment::Oscillator(1));

        assert_eq!(frames.len(), 1);
        let frame = &frames[0];
        assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
        assert_eq!(&frame[4..8], &0x53_u32.to_le_bytes());
        assert_eq!(&frame[0x10..0x13], &[0xd3, 0x41, 0x05]);
        assert_eq!(&frame[0x10 + 0x03..0x10 + 0x05], &[0x09, 0x00]);
    }

    #[test]
    fn encodes_grounded_link_selector_write() {
        let frame = encode_command(Command::SetLinkState {
            selector: 0x00,
            enabled: true,
            companion_bank: Some(0x00),
        });

        assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
        assert_eq!(&frame[4..8], &0x14_u32.to_le_bytes());
        assert_eq!(&frame[0x10..0x14], &[0xa2, 0x03, 0x00, 0x01]);
    }

    #[test]
    fn encodes_grounded_link_companion_write() {
        let frame = encode_link_companion(0x01, true);

        assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
        assert_eq!(&frame[4..8], &0x14_u32.to_le_bytes());
        assert_eq!(&frame[0x10..0x14], &[0xa2, 0x04, 0x01, 0x01]);
    }

    #[test]
    fn decodes_snapshot_global_fields_and_outputs() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x20_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x00] = 0x08;
        payload[0x02] = 0x02;
        payload[0x03] = 0x01;
        payload[0x04..0x08].copy_from_slice(&48_000_u32.to_be_bytes());
        payload[0x0c] = 0x40;
        payload[0x0d] = 0x02;
        payload[0x0e] = 0x30;
        payload[0x0f] = 0x01;
        payload[0x10] = 0x20;
        payload[0x11] = 0x00;
        payload[0x6a] = 0x0c;
        payload[0x18..0x1c].copy_from_slice(&[0x2f, 0x34, 0x50, 0x10]);

        let parsed = Frame::parse(&frame).expect("frame should parse");
        let snapshot = parsed.as_snapshot().expect("snapshot");

        assert_eq!(snapshot.sample_rate, SampleRate::Hz48000);
        assert_eq!(snapshot.clock_source, ClockSource::Spdif);
        assert_eq!(snapshot.sample_rate_hz, 48_000);
        assert_eq!(snapshot.surface, Surface::Hp2);
        assert_eq!(snapshot.output(OutputTarget::Monitor).volume, 0x40);
        assert_eq!(snapshot.output(OutputTarget::Monitor).mode, OutputMode::Dim);
        assert_eq!(snapshot.output(OutputTarget::Hp1).mode, OutputMode::Mute);
        assert_eq!(snapshot.output(OutputTarget::Hp2).mode, OutputMode::Normal);
        assert_eq!(snapshot.dsp_cluster, [0x2f, 0x34, 0x50, 0x10]);
        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix2, 1)
                .unwrap()
                .meter,
            None
        );
    }

    #[test]
    fn snapshot_frame_preserves_raw_bytes() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x8e] = 0x5a;
        frame[0x10 + 0xcf] = 0x4c;
        frame[0x10 + 0xde] = 0x11;

        let parsed = Frame::parse(&frame).expect("frame should parse");
        assert_eq!(parsed.raw_bytes()[0x10 + 0x8e], 0x5a);
        assert_eq!(parsed.raw_bytes()[0x10 + 0xcf], 0x4c);
        assert_eq!(parsed.raw_bytes()[0x10 + 0xde], 0x11);
    }

    #[test]
    fn snapshot_frame_parse_owned_preserves_raw_bytes() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x8e] = 0x5a;
        frame[0x10 + 0xcf] = 0x4c;
        frame[0x10 + 0xde] = 0x11;

        let parsed = Frame::parse_owned(frame).expect("frame should parse");
        assert_eq!(parsed.raw_bytes()[0x10 + 0x8e], 0x5a);
        assert_eq!(parsed.raw_bytes()[0x10 + 0xcf], 0x4c);
        assert_eq!(parsed.raw_bytes()[0x10 + 0xde], 0x11);
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

    #[test]
    fn encodes_confirmed_commands() {
        let sample = encode_command(Command::SetSampleRate(SampleRate::Hz44100));
        assert_eq!(&sample[0..4], &0x70_u32.to_le_bytes());
        assert_eq!(sample[4], 0x12);
        assert_eq!(&sample[0x10..0x12], &[0x03, 0x01]);

        let output = encode_command(Command::SetOutputVolume {
            target: OutputTarget::Hp2,
            step: 0x33,
        });
        assert_eq!(output[4], 0x13);
        assert_eq!(&output[0x10..0x13], &[0x47, 0x02, 0x33]);

        let mixer = encode_command(Command::SetMixerLevel {
            mixer: MixerSurface::Mix2,
            channel: 4,
            level: 0x28,
            pan_state: PanState::right(),
            muted: false,
            soloed: false,
        });
        assert_eq!(mixer[4], 0x16);
        assert_eq!(&mixer[0x10..0x16], &[0xd4, 0x04, 0x01, 0x04, 0x28, 0x3e]);

        let preamp_mode = encode_command(Command::SetPreampMode {
            input: 1,
            mode: PreampMode::HiZ,
        });
        assert_eq!(preamp_mode[4], 0x13);
        assert_eq!(&preamp_mode[0x10..0x13], &[0x4f, 0x01, 0x02]);

        let preamp_gain = encode_command(Command::SetPreampGain {
            input: 0,
            raw: 0x2d,
        });
        assert_eq!(preamp_gain[4], 0x13);
        assert_eq!(&preamp_gain[0x10..0x13], &[0x50, 0x00, 0x2d]);

        let preamp_phantom = encode_command(Command::SetPreampPhantom {
            input: 1,
            enabled: true,
        });
        assert_eq!(preamp_phantom[4], 0x13);
        assert_eq!(&preamp_phantom[0x10..0x13], &[0x51, 0x01, 0x01]);

        let preamp_phase = encode_command(Command::SetPreampPhase {
            input: 0,
            enabled: false,
        });
        assert_eq!(preamp_phase[4], 0x13);
        assert_eq!(&preamp_phase[0x10..0x13], &[0x52, 0x00, 0x00]);
    }

    #[test]
    fn parses_metadata_query_reply() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x75_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x30_u32.to_le_bytes());
        frame[0x08] = 0x01;
        let body = [
            b'Z', b'e', b'n', b' ', b'G', b'o', b' ', b'S', b'y', b'n', b'e', b'r', b'g', b'y',
            b' ', b'C', b'o', b'r', b'e', 0x00, 0x00, b'4', b'5', b'0', b'2', b'7', b'2', b'1',
            b'0', b'0', b'1', b'3', b'0', b'0', 0x00, 0x00, b'6', b'.', b'6', 0x00,
        ];
        frame[0x10..0x10 + body.len()].copy_from_slice(&body);

        let parsed = Frame::parse(&frame).expect("reply should parse");
        let metadata = parsed
            .as_query_reply()
            .and_then(|reply| reply.metadata())
            .expect("metadata");

        assert_eq!(metadata.product_name, "Zen Go Synergy Core");
        assert_eq!(metadata.serial, "4502721001300");
        assert_eq!(metadata.hardware_version, "6.6");
    }

    #[test]
    fn classifies_grounded_startup_query_reply_kinds() {
        assert_eq!(
            StartupQueryKind::from_query_id(0x01),
            StartupQueryKind::Metadata
        );
        assert_eq!(
            StartupQueryKind::from_query_id(0x00),
            StartupQueryKind::CapabilityDefaults
        );
        assert_eq!(
            StartupQueryKind::from_query_id(0x11),
            StartupQueryKind::StatusValue
        );
        assert_eq!(
            StartupQueryKind::from_query_id(0x7f),
            StartupQueryKind::Unknown(0x7f)
        );
    }

    #[test]
    fn summarizes_non_metadata_query_replies_without_over_decoding() {
        let defaults = QueryResponse {
            query_id: 0x00,
            sub_id: 0x00,
            body: vec![0xaa, 0xbb, 0xcc],
        };
        let status = QueryResponse {
            query_id: 0x11,
            sub_id: 0x00,
            body: vec![0x12],
        };

        assert_eq!(
            defaults.summary_label(),
            "Capability/default block: 3 bytes [aa bb cc]"
        );
        assert_eq!(
            status.summary_label(),
            "Status/capability value: 1 bytes [12]"
        );
    }

    #[test]
    fn decodes_selector_bitmap_from_0x75_0b_03() {
        let reply = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            ],
        };

        let bitmap = reply.selector_bitmap().expect("selector bitmap");
        let asserted = bitmap
            .iter()
            .enumerate()
            .filter_map(|(index, enabled)| enabled.then_some(index as u8))
            .collect::<Vec<_>>();
        assert_eq!(
            asserted,
            vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x11, 0x12, 0x13, 0x14]
        );
        assert_eq!(
            reply.summary_label(),
            "Selector bitmap: 9 asserted [00 01 02 03 04 11 12 13 14]"
        );
    }

    #[test]
    fn decodes_startup_visible_link_pairs_from_0x75_0b_03() {
        let mix1_linked = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            ],
        };
        let unlinked = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            ],
        };
        let mix2_linked = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            ],
        };
        let mix1_high = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            ],
        };
        let mix2_high = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            ],
        };

        assert_eq!(
            mix1_linked
                .startup_link_readback_from_bitmap()
                .map(|maps| maps[0].1),
            Some([
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            ])
        );
        assert_eq!(
            unlinked
                .startup_link_readback_from_bitmap()
                .map(|maps| maps[0].1),
            Some([
                Some(false),
                Some(false),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            ])
        );
        assert_eq!(
            mix2_linked
                .startup_link_readback_from_bitmap()
                .map(|maps| maps[1].1),
            Some([
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            ])
        );
        assert_eq!(
            mix1_high
                .startup_link_readback_from_bitmap()
                .map(|maps| maps[0].1),
            Some([
                Some(false),
                Some(false),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
            ])
        );
        assert_eq!(
            mix2_high
                .startup_link_readback_from_bitmap()
                .map(|maps| maps[1].1),
            Some([
                Some(false),
                Some(false),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
            ])
        );
    }

    #[test]
    fn summarizes_selector_pair_bank_conservatively() {
        let reply = QueryResponse {
            query_id: 0x04,
            sub_id: 0x01,
            body: vec![
                0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
            ],
        };

        let pairs = reply.selector_pair_bank().expect("selector pair bank");
        assert_eq!(pairs.len(), 32);
        assert_eq!(pairs[0], (0x00, 0x20));
        assert_eq!(pairs[1], (0x00, 0x60));
        assert_eq!(pairs[2], (0x00, 0x60));
        assert_eq!(pairs[3], (0x00, 0x02));
        assert_eq!(
            reply.summary_label(),
            "Startup Mix2 pan categories [C C L R L R L R L R L R L R L R]"
        );
    }

    #[test]
    fn decodes_startup_pan_categories_from_grounded_0x75_04_mix_banks() {
        let mix1 = QueryResponse {
            query_id: 0x04,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
            ],
        };
        let mix2 = QueryResponse {
            query_id: 0x04,
            sub_id: 0x01,
            body: vec![
                0x00, 0x20, 0x60, 0x20, 0x60, 0x20, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x60, 0x02, 0x60, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
            ],
        };

        let (surface1, pans1) = mix1.startup_pan_category_readback().expect("mix1 pans");
        let (surface2, pans2) = mix2.startup_pan_category_readback().expect("mix2 pans");
        assert_eq!(surface1, MixerSurface::Mix1);
        assert_eq!(surface2, MixerSurface::Mix2);
        assert_eq!(pans1[0], Some(StartupPanCategory::Center));
        assert_eq!(pans1[1], Some(StartupPanCategory::Center));
        assert_eq!(pans1[2], Some(StartupPanCategory::Left));
        assert_eq!(pans1[3], Some(StartupPanCategory::Right));
        assert_eq!(pans2[0], Some(StartupPanCategory::Center));
        assert_eq!(pans2[1], Some(StartupPanCategory::Center));
        assert_eq!(pans2[2], Some(StartupPanCategory::Left));
        assert_eq!(pans2[3], Some(StartupPanCategory::Right));
        assert_eq!(
            mix1.summary_label(),
            "Startup Mix1 pan categories [C C L R L R L R L R L R L R L R]"
        );
    }

    #[test]
    fn decodes_startup_pan_state_from_grounded_0x75_04_mix_banks() {
        let mix1_ch1 = QueryResponse {
            query_id: 0x04,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x00, 0x5e, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
            ],
        };
        let mix1_pair = QueryResponse {
            query_id: 0x04,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
            ],
        };

        let (surface1, states1) = mix1_ch1.startup_pan_state_readback().expect("mix1 state");
        assert_eq!(surface1, MixerSurface::Mix1);
        assert_eq!(
            states1[0],
            Some(StartupMixerStripState {
                level: 0x00,
                pan: PanState::from_raw(0x1e),
                muted: true,
                soloed: false,
            })
        );
        assert_eq!(
            states1[1],
            Some(StartupMixerStripState {
                level: 0x00,
                pan: PanState::center(),
                muted: true,
                soloed: false,
            })
        );

        let (_, states2) = mix1_pair
            .startup_pan_state_readback()
            .expect("mix1 pair state");
        assert_eq!(
            states2[2],
            Some(StartupMixerStripState {
                level: 0x00,
                pan: PanState::left(),
                muted: false,
                soloed: false,
            })
        );
        assert_eq!(
            states2[3],
            Some(StartupMixerStripState {
                level: 0x00,
                pan: PanState::right(),
                muted: false,
                soloed: false,
            })
        );
    }

    #[test]
    fn startup_pan_state_readback_decodes_solo_flag() {
        let reply = QueryResponse {
            query_id: 0x04,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x00, 0xa0, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
            ],
        };

        let (_, states) = reply
            .startup_pan_state_readback()
            .expect("startup pan state");

        assert!(states[0].expect("ch1").soloed);
    }

    #[test]
    fn pan_display_uses_device_step_scale() {
        assert_eq!(PanState::center().display_percent(), 0);
        assert_eq!(PanState::from_raw(0x1e).display_percent(), -2);
        assert_eq!(PanState::left().display_percent(), -30);
        assert_eq!(PanState::right().display_percent(), 30);
    }

    #[test]
    fn decodes_startup_level_from_grounded_0x75_04_mix_banks() {
        let mix1_ch1 = QueryResponse {
            query_id: 0x04,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x12, 0x5e, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
            ],
        };
        let mix2_pair = QueryResponse {
            query_id: 0x04,
            sub_id: 0x01,
            body: vec![
                0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x1e, 0x02, 0x1e, 0x3e, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
            ],
        };

        let (_, states1) = mix1_ch1
            .startup_pan_state_readback()
            .expect("mix1 level state");
        let (_, states2) = mix2_pair
            .startup_pan_state_readback()
            .expect("mix2 pair level state");
        assert_eq!(states1[0].map(|state| state.level), Some(0x12));
        assert_eq!(states2[10].map(|state| state.level), Some(0x1e));
        assert_eq!(states2[11].map(|state| state.level), Some(0x1e));
    }

    #[test]
    fn summarizes_startup_indexed_code_table() {
        let reply = QueryResponse {
            query_id: 0x15,
            sub_id: 0x00,
            body: vec![
                0x00, 0x00, 0x01, 0x10, 0x02, 0x10, 0x03, 0x04, 0x04, 0x04, 0x05, 0x10, 0x06, 0x10,
                0x07, 0x10, 0x08, 0x00, 0x09, 0x0f, 0x0a, 0x00, 0x0b, 0x10, 0x0c, 0x10, 0x0d, 0x10,
                0x0e, 0x00, 0x0f, 0x10, 0x10, 0x00, 0x11, 0x10, 0x12, 0x10, 0x13, 0x00, 0x14, 0x0f,
                0x15, 0x10, 0x16, 0x00, 0x17, 0x00, 0x18, 0x00, 0x19, 0x00, 0x1a, 0x10, 0x1b, 0x00,
                0x1c, 0x10, 0x1d, 0x10, 0x1e, 0x10, 0x1f, 0x10,
            ],
        };

        let entries = reply
            .startup_indexed_code_table()
            .expect("startup indexed code table");
        assert_eq!(entries.len(), 32);
        assert_eq!(entries[0], (0x00, 0x00));
        assert_eq!(entries[3], (0x03, 0x04));
        assert_eq!(entries[9], (0x09, 0x0f));
        assert_eq!(
            reply.summary_label(),
            "Startup indexed code table [00:00 01:10 02:10 03:04 04:04 05:10 06:10 07:10 08:00 09:0f]"
        );
    }

    #[test]
    fn summarizes_startup_quad_state() {
        let reply = QueryResponse {
            query_id: 0x17,
            sub_id: 0x00,
            body: vec![0x5a, 0x00, 0x60, 0x60],
        };

        assert_eq!(reply.startup_quad_state(), Some([0x5a, 0x00, 0x60, 0x60]));
        assert_eq!(reply.summary_label(), "Startup quad state [5a 00 60 60]");
    }

    #[test]
    fn decodes_assignment_readback_from_grounded_0x75_banks() {
        let early_bank = QueryResponse {
            query_id: 0x03,
            sub_id: 0x05,
            body: vec![0x05, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x01, 0x01],
        };
        let ordinary_bank = QueryResponse {
            query_id: 0x03,
            sub_id: 0x06,
            body: vec![
                0x06, 0x03, 0x00, 0x03, 0x01, 0x03, 0x02, 0x03, 0x03, 0x01, 0x02, 0x01, 0x03, 0x01,
                0x04, 0x01, 0x05, 0x01, 0x06, 0x01, 0x07, 0x08, 0x00, 0x08, 0x00, 0x08, 0x00, 0x08,
                0x00, 0x08, 0x00, 0x08, 0x00,
            ],
        };

        let early = early_bank
            .assignment_readback()
            .expect("early assignment bank");
        assert_eq!(early[0], Some(MixerAssignment::Preamp(1)));
        assert_eq!(early[1], Some(MixerAssignment::Preamp(2)));
        assert_eq!(early[2], Some(MixerAssignment::ComputerPlay(1)));
        assert_eq!(early[3], Some(MixerAssignment::ComputerPlay(2)));
        assert!(early[4..].iter().all(|slot| slot.is_none()));

        let ordinary = ordinary_bank
            .assignment_readback()
            .expect("ordinary assignment bank");
        assert!(ordinary[0..4].iter().all(|slot| slot.is_none()));
        assert_eq!(ordinary[4], Some(MixerAssignment::ComputerPlay(3)));
        assert_eq!(ordinary[5], Some(MixerAssignment::ComputerPlay(4)));
        assert_eq!(ordinary[6], Some(MixerAssignment::ComputerPlay(5)));
        assert_eq!(ordinary[7], Some(MixerAssignment::ComputerPlay(6)));
        assert_eq!(ordinary[8], Some(MixerAssignment::ComputerPlay(7)));
        assert_eq!(ordinary[9], Some(MixerAssignment::ComputerPlay(8)));
        assert!(ordinary[10..]
            .iter()
            .all(|slot| *slot == Some(MixerAssignment::Mute)));
    }

    #[test]
    fn decodes_mixer_strip_readback_from_0x75_18_00() {
        let reply = QueryResponse {
            query_id: 0x18,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x02, 0x60, 0x3e, 0x2e, 0x02, 0x60, 0x3e,
                0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e,
                0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
            ],
        };

        assert_eq!(reply.body.len(), 64);
        let readback = reply.mixer_strip_readback().expect("strip state readback");
        let mix1 = &readback.surfaces[MixerSurface::Mix1.index()];
        let mix2 = &readback.surfaces[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, 0x00);
        assert_eq!(mix1[0].pan, PanState::center());
        assert!(!mix1[0].muted);
        assert_eq!(mix1[1].level, 0x5a);
        assert_eq!(mix1[1].pan, PanState::center());
        assert_eq!(mix1[2].level, 0x5a);
        assert_eq!(mix1[2].pan, PanState::center());
        assert_eq!(mix1[3].level, 0x5a);
        assert_eq!(mix1[3].pan, PanState::left());
        assert_eq!(mix1[5].level, 0x2e);
        assert_eq!(mix1[5].pan, PanState::left());
        assert_eq!(mix2[0].level, 0x5a);
        assert_eq!(mix2[0].pan, PanState::right());
        assert_eq!(mix2[1].level, 0x5a);
        assert_eq!(mix2[1].pan, PanState::left());
        assert!(mix1.iter().chain(mix2.iter()).all(|strip| !strip.soloed));
    }

    #[test]
    fn mixer_strip_readback_requires_full_dual_surface_payload() {
        let reply = QueryResponse {
            query_id: 0x18,
            sub_id: 0x00,
            body: vec![
                0x12, 0x3e, 0x60, 0x60, 0x60, 0x60, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x20, 0x60, 0x20,
                0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                0x60, 0x20, 0x60, 0x20,
            ],
        };

        assert!(reply.mixer_strip_readback().is_none());
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
    fn mixer_level_display_uses_inverse_db_scale() {
        let unity =
            MixerChannelState::known(1, Some(0x00), Some(false), PanState::center(), None, None);
        let silence =
            MixerChannelState::known(1, Some(0x60), Some(false), PanState::center(), None, None);

        assert_eq!(unity.display_db(), Some(0));
        assert_eq!(silence.display_db(), Some(-90));
        assert_eq!(unity.gain_ratio(), Some(1.0));
        assert_eq!(silence.gain_ratio(), Some(0.0));
    }

    #[test]
    fn meter_display_uses_logarithmic_minus_60_to_0_db_ui_scale() {
        let mut active =
            MixerChannelState::known(1, Some(0x00), Some(false), PanState::center(), None, None);
        active.meter = Some(0x00);
        let mut moderate =
            MixerChannelState::known(1, Some(0x00), Some(false), PanState::center(), None, None);
        moderate.meter = Some(0x14);
        let mut quiet =
            MixerChannelState::known(1, Some(0x00), Some(false), PanState::center(), None, None);
        quiet.meter = Some(0x1e);
        let mut floor =
            MixerChannelState::known(1, Some(0x00), Some(false), PanState::center(), None, None);
        floor.meter = Some(0x3c);
        let mut hidden =
            MixerChannelState::known(1, Some(0x00), Some(false), PanState::center(), None, None);
        hidden.meter = Some(0x60);
        let mut preamp = PreampInputState::from_raw(0x2a, 0x00);
        preamp.observed_meter = Some(0x60);

        assert_eq!(active.meter_db(), Some(0));
        assert_eq!(floor.meter_db(), Some(-60));
        assert_eq!(hidden.meter_db(), None);
        assert_eq!(active.meter_ratio(), Some(1.0));
        assert!(moderate
            .meter_ratio()
            .is_some_and(|ratio| (ratio - 0.099).abs() < 0.01));
        assert!(quiet
            .meter_ratio()
            .is_some_and(|ratio| (ratio - 0.031).abs() < 0.01));
        assert_eq!(floor.meter_ratio(), Some(0.0));
        assert_eq!(hidden.meter_ratio(), Some(0.0));
        assert_eq!(preamp.observed_meter_db(), None);
        assert_eq!(preamp.observed_meter_ratio(), Some(0.0));
    }

    #[test]
    fn does_not_decode_observed_preamp1_meter_from_untrusted_lane() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0x7e] = 0x18;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, None);
    }

    #[test]
    fn does_not_decode_observed_preamp_meters_from_row_status_values() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xce] = 0x54;
        payload[0xcf] = 0x4e;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, None);
    }

    #[test]
    fn decodes_observed_preamp1_meter_from_direct_lane() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xce] = 0x38;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, Some(0x38));
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, None);
    }

    #[test]
    fn decodes_observed_preamp2_meter_from_direct_lane() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xcf] = 0x49;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, Some(0x49));
    }

    #[test]
    fn observed_preamp1_meter_can_coexist_with_strip1_meter() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xce] = 0x2a;
        payload[0x8e] = 0x12;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 1)
                .expect("mix1 strip 1")
                .meter,
            Some(0x12)
        );
        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, Some(0x2a));
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, None);
    }

    #[test]
    fn observed_preamp2_meter_can_coexist_with_strip_meter() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xcf] = 0x22;
        payload[0x8e] = 0x12;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 1)
                .expect("mix1 strip 1")
                .meter,
            Some(0x12)
        );
        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, Some(0x22));
    }

    #[test]
    fn decodes_passive_shared_strip1_meter_from_slot_byte() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0x8e] = 0x12;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 1)
                .expect("mix1 strip 1")
                .meter,
            Some(0x12)
        );
        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix2, 1)
                .expect("mix2 strip 1")
                .meter,
            Some(0x12)
        );
        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, None);
    }

    #[test]
    fn decodes_passive_shared_strip11_meter_from_slot_byte() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0x98] = 0x05;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 11)
                .expect("mix1 strip 11")
                .meter,
            Some(0x05)
        );
        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix2, 11)
                .expect("mix2 strip 11")
                .meter,
            Some(0x05)
        );
    }

    #[test]
    fn decodes_passive_mix1_strip1_mute_from_late_row_cluster() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0x8f] = 0x51;
        payload[0xcf] = 0x51;
        payload[0xda] = 0x51;
        payload[0xdb] = 0x51;
        payload[0xdc] = 0x51;
        payload[0xdd] = 0x51;
        payload[0xde] = 0x4e;
        payload[0xdf] = 0x4e;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        let strip = snapshot
            .mixer_decode
            .strip(MixerSurface::Mix1, 1)
            .expect("mix1 strip 1");
        assert_eq!(strip.muted, Some(true));
    }

    #[test]
    fn decodes_passive_mix1_link_pair_from_late_row_cluster() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0x8f] = 0x51;
        payload[0xcf] = 0x51;
        payload[0xda] = 0x4e;
        payload[0xdb] = 0x4e;
        payload[0xdc] = 0x4e;
        payload[0xdd] = 0x4e;
        payload[0xde] = 0x4e;
        payload[0xdf] = 0x4e;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 1)
                .unwrap()
                .linked,
            Some(true)
        );
        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 2)
                .unwrap()
                .linked,
            Some(true)
        );
    }

    #[test]
    fn experimental_pair_state_lanes_extract_mix1_mirrored_codebook() {
        let mut frame = empty_snapshot_frame();
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xda] = 0x0a;
        payload[0xdb] = 0x05;
        payload[0xdc] = 0x0a;
        payload[0xdd] = 0x05;
        payload[0xe0] = 0x60;
        payload[0xe1] = 0x60;

        assert_eq!(
            experimental_surface_pair_lanes(snapshot_payload(&frame)),
            Some(ExperimentalSurfacePairLanes {
                mixer: MixerSurface::Mix1,
                lane_a: 0x0a,
                lane_b: 0x05,
                mirrored: true,
            })
        );
    }

    #[test]
    fn experimental_pair_state_lanes_extract_mix2_compact_codebook() {
        let mut frame = empty_snapshot_frame();
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0c;
        payload[0xde] = 0x00;
        payload[0xdf] = 0x06;
        payload[0xe0] = 0x60;
        payload[0xe1] = 0x60;

        assert_eq!(
            experimental_surface_pair_lanes(snapshot_payload(&frame)),
            Some(ExperimentalSurfacePairLanes {
                mixer: MixerSurface::Mix2,
                lane_a: 0x00,
                lane_b: 0x06,
                mirrored: false,
            })
        );
    }

    #[test]
    fn experimental_pair_state_lanes_preserve_both_mute_idle_form() {
        let mut frame = empty_snapshot_frame();
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0c;
        payload[0xde] = 0x60;
        payload[0xdf] = 0x60;

        assert_eq!(
            experimental_surface_pair_lanes(snapshot_payload(&frame)),
            Some(ExperimentalSurfacePairLanes {
                mixer: MixerSurface::Mix2,
                lane_a: 0x60,
                lane_b: 0x60,
                mirrored: false,
            })
        );
    }
}
