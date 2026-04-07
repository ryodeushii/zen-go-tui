use thiserror::Error;

pub const HID_REPORT_SIZE: usize = 320;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("frame too short: {0}")]
    FrameTooShort(usize),
    #[error("unsupported frame type: 0x{0:02x}")]
    UnsupportedFrame(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRate {
    Hz32000,
    Hz44100,
    Hz48000,
    Hz88200,
    Hz96000,
    Hz176400,
    Hz192000,
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

    pub fn observed_meter_ratio(self) -> Option<f64> {
        self.observed_meter
            .map(|raw| (1.0 - (raw.min(0x60) as f64 / 96.0)).clamp(0.0, 1.0))
    }
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

    pub fn muted_code(self, muted: bool) -> u8 {
        self.code() | if muted { Self::MUTE_FLAG } else { 0x00 }
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
        let center = Self::CENTER as i16;
        let span = (Self::MAX - Self::CENTER) as i16;
        (((self.raw() as i16 - center) as f64 / span as f64) * 100.0).round() as i16
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

    pub fn assignment_entry_index(self) -> Option<usize> {
        match self.kind {
            MixerStripKind::Ordinary => Some((self.channel - 1) as usize),
            MixerStripKind::EarlyAfxAdjacent => None,
        }
    }

    pub fn assignment_write_is_grounded(channel: u8) -> bool {
        channel == 11
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
            (MixerSurface::Mix1, 0x03) => Some(Self {
                mixer,
                left_channel: 7,
                right_channel: 8,
                selector,
            }),
            (MixerSurface::Mix2, 0x01) => Some(Self {
                mixer,
                left_channel: 1,
                right_channel: 2,
                selector,
            }),
            _ => None,
        }
    }

    pub fn from_channel(mixer: MixerSurface, channel: u8) -> Option<Self> {
        match (mixer, channel) {
            (MixerSurface::Mix1, 1 | 2) => Self::from_selector(mixer, 0x00),
            (MixerSurface::Mix1, 7 | 8) => Self::from_selector(mixer, 0x03),
            (MixerSurface::Mix2, 1 | 2) => Self::from_selector(mixer, 0x01),
            _ => None,
        }
    }

    pub fn companion_bank(self) -> Option<u8> {
        match (self.mixer, self.selector) {
            (MixerSurface::Mix1, 0x00) => Some(0x00),
            (MixerSurface::Mix2, 0x01) => Some(0x01),
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
pub struct Snapshot73 {
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

impl Snapshot73 {
    pub fn output(&self, target: OutputTarget) -> OutputState {
        self.outputs[target.index() as usize]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceMetadata {
    pub product_name: String,
    pub serial: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupQueryKind {
    Metadata,
    CapabilityDefaults,
    StatusValue,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryRequest74 {
    pub query_id: u8,
    pub sub_id: u8,
}

impl QueryRequest74 {
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
pub struct QueryReply75 {
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

impl QueryReply75 {
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
            version: parts[2].clone(),
        })
    }

    pub fn summary_label(&self) -> String {
        match self.kind() {
            StartupQueryKind::Metadata => self
                .metadata()
                .map(|metadata| {
                    format!(
                        "{}: {} ({}, serial {})",
                        self.kind().label(),
                        metadata.product_name,
                        metadata.version,
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

    pub fn mixer_strip_readback(&self) -> Option<[QueriedMixerStripState; 16]> {
        if self.query_id != 0x18 || self.sub_id != 0x00 || self.body.len() < 32 {
            return None;
        }

        let mut strips = [QueriedMixerStripState::default(); 16];
        for (index, chunk) in self.body[..32].chunks_exact(2).enumerate() {
            strips[index] = QueriedMixerStripState {
                level: chunk[0],
                pan: PanState::from_state_code(chunk[1]),
                muted: PanState::state_code_is_muted(chunk[1]),
                soloed: PanState::state_code_is_soloed(chunk[1]),
            };
        }

        Some(strips)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification81 {
    pub bytes: [u8; 6],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Snapshot {
        snapshot: Snapshot73,
        raw: Vec<u8>,
    },
    QueryReply {
        reply: QueryReply75,
        raw: Vec<u8>,
    },
    Auxiliary83 {
        bytes: Vec<u8>,
        raw: Vec<u8>,
    },
    Notification {
        notification: Notification81,
        raw: Vec<u8>,
    },
}

impl Frame {
    pub fn parse(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() < 6 {
            return Err(ProtocolError::FrameTooShort(bytes.len()));
        }

        if bytes.len() == 6 {
            let mut raw = [0_u8; 6];
            raw.copy_from_slice(bytes);
            return Ok(Self::Notification {
                notification: Notification81 { bytes: raw },
                raw: bytes.to_vec(),
            });
        }

        if bytes.len() < 0x12 {
            return Err(ProtocolError::FrameTooShort(bytes.len()));
        }

        let frame_type = u32::from_le_bytes(bytes[0..4].try_into().expect("type header"));
        match frame_type {
            0x73 => Ok(Self::Snapshot {
                snapshot: parse_snapshot73(bytes)?,
                raw: bytes.to_vec(),
            }),
            0x75 => Ok(Self::QueryReply {
                reply: QueryReply75 {
                    query_id: bytes[0x08],
                    sub_id: bytes[0x0c],
                    body: bytes[0x10..].to_vec(),
                },
                raw: bytes.to_vec(),
            }),
            0x83 => Ok(Self::Auxiliary83 {
                bytes: bytes[0x10..].to_vec(),
                raw: bytes.to_vec(),
            }),
            other => Err(ProtocolError::UnsupportedFrame(other)),
        }
    }

    pub fn as_snapshot(&self) -> Option<&Snapshot73> {
        match self {
            Self::Snapshot { snapshot, .. } => Some(snapshot),
            _ => None,
        }
    }

    pub fn as_query_reply(&self) -> Option<&QueryReply75> {
        match self {
            Self::QueryReply { reply, .. } => Some(reply),
            _ => None,
        }
    }

    pub fn raw_bytes(&self) -> &[u8] {
        match self {
            Self::Snapshot { raw, .. } => raw,
            Self::QueryReply { raw, .. } => raw,
            Self::Auxiliary83 { raw, .. } => raw,
            Self::Notification { raw, .. } => raw,
        }
    }
}

fn parse_snapshot73(bytes: &[u8]) -> Result<Snapshot73, ProtocolError> {
    if bytes.len() < 0x10 + 0xe6 {
        return Err(ProtocolError::FrameTooShort(bytes.len()));
    }

    let payload = &bytes[0x10..];
    Ok(Snapshot73 {
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

    let shared_meter = observe_meter_from_group(payload, 0x8f, 0xcf, 0xda, 0xdd, 0xde, 0xdf)
        .or_else(|| observe_meter_from_group(payload, 0x6f, 0x8f, 0xda, 0xdd, 0xde, 0xdf));
    let shared_mute = decode_mute_from_group(payload, 0x8f, 0xcf, 0xda, 0xdb, 0xdc, 0xdd);
    let shared_pan = decode_pan_from_group(payload, 0x8f, 0xcf, 0xda, 0xdd, 0xde, 0xdf);

    let active_mixer = MixerSurface::from_surface(Surface::from_code(payload[0x6a]));
    decode.observed_preamp2_meter = shared_meter;
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

fn observe_meter_from_group(
    payload: &[u8],
    primary_a: usize,
    primary_b: usize,
    shadow_a: usize,
    shadow_b: usize,
    tail_a: usize,
    tail_b: usize,
) -> Option<u8> {
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
    matches!(value, 0x43..=0x4e).then_some(value)
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
    let values = [
        payload.get(0x8f).copied()?,
        payload.get(0xcf).copied()?,
        payload.get(0xda).copied()?,
        payload.get(0xdb).copied()?,
        payload.get(0xdc).copied()?,
        payload.get(0xdd).copied()?,
        payload.get(0xde).copied()?,
        payload.get(0xdf).copied()?,
    ];
    if values
        .iter()
        .all(|value| matches!(*value, 0x4c | 0x5a | 0x51))
    {
        Some(true)
    } else if values.iter().all(|value| matches!(*value, 0x4e | 0x51)) {
        Some(false)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSnapshot {
    Snapshot(Snapshot73),
    Auxiliary83(Vec<u8>),
    QueryReply(QueryReply75),
    Notification(Notification81),
}

impl From<Frame> for DeviceSnapshot {
    fn from(frame: Frame) -> Self {
        match frame {
            Frame::Snapshot { snapshot, .. } => Self::Snapshot(snapshot),
            Frame::Auxiliary83 { bytes, .. } => Self::Auxiliary83(bytes),
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
    },
    SetMixerMute {
        mixer: MixerSurface,
        channel: u8,
        muted: bool,
        pan_state: PanState,
    },
    SetMixerPan {
        mixer: MixerSurface,
        channel: u8,
        pan: PanState,
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
    pub observed_preamp2_meter: Option<u8>,
}

impl Default for MixerPassiveDecode {
    fn default() -> Self {
        Self {
            surfaces: [[MixerPassiveStripState::unresolved(); 16]; 2],
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
            pan,
            assignment,
            linked,
        }
    }

    pub fn display_db(self) -> Option<i16> {
        self.level.map(|raw| -(raw.min(0x60) as i16))
    }

    pub fn gain_ratio(self) -> Option<f64> {
        self.level
            .map(|raw| (1.0 - (raw.min(0x60) as f64 / 96.0)).clamp(0.0, 1.0))
    }

    pub fn meter_ratio(self) -> Option<f64> {
        self.meter
            .map(|raw| (1.0 - (raw.min(0x60) as f64 / 96.0)).clamp(0.0, 1.0))
    }
}

pub fn control_panel_startup_queries() -> &'static [QueryRequest74] {
    const QUERIES: [QueryRequest74; 46] = [
        QueryRequest74::new(0x11, 0x00),
        QueryRequest74::new(0x0a, 0x00),
        QueryRequest74::new(0x17, 0x00),
        QueryRequest74::new(0x18, 0x00),
        QueryRequest74::new(0x11, 0x01),
        QueryRequest74::new(0x03, 0x00),
        QueryRequest74::new(0x03, 0x01),
        QueryRequest74::new(0x03, 0x02),
        QueryRequest74::new(0x03, 0x03),
        QueryRequest74::new(0x03, 0x04),
        QueryRequest74::new(0x03, 0x05),
        QueryRequest74::new(0x03, 0x06),
        QueryRequest74::new(0x03, 0x07),
        QueryRequest74::new(0x03, 0x08),
        QueryRequest74::new(0x03, 0x09),
        QueryRequest74::new(0x0b, 0x00),
        QueryRequest74::new(0x16, 0x00),
        QueryRequest74::new(0x0a, 0x00),
        QueryRequest74::new(0x04, 0x00),
        QueryRequest74::new(0x0b, 0x03),
        QueryRequest74::new(0x04, 0x01),
        QueryRequest74::new(0x0b, 0x03),
        QueryRequest74::new(0x04, 0x02),
        QueryRequest74::new(0x0b, 0x03),
        QueryRequest74::new(0x04, 0x03),
        QueryRequest74::new(0x0b, 0x03),
        QueryRequest74::new(0x15, 0x00),
        QueryRequest74::new(0x19, 0x00),
        QueryRequest74::new(0x19, 0x01),
        QueryRequest74::new(0x07, 0x27),
        QueryRequest74::new(0x07, 0x2c),
        QueryRequest74::new(0x07, 0x09),
        QueryRequest74::new(0x07, 0x14),
        QueryRequest74::new(0x07, 0x4c),
        QueryRequest74::new(0x19, 0x02),
        QueryRequest74::new(0x19, 0x03),
        QueryRequest74::new(0x19, 0x04),
        QueryRequest74::new(0x19, 0x05),
        QueryRequest74::new(0x19, 0x06),
        QueryRequest74::new(0x19, 0x07),
        QueryRequest74::new(0x19, 0x08),
        QueryRequest74::new(0x19, 0x09),
        QueryRequest74::new(0x19, 0x0a),
        QueryRequest74::new(0x19, 0x0b),
        QueryRequest74::new(0x0b, 0x04),
        QueryRequest74::new(0x12, 0x00),
    ];
    &QUERIES
}

pub fn encode_query(query: QueryRequest74) -> Vec<u8> {
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
        } => host_frame(
            0x16,
            &[0xd4, 0x04, mixer.code(), channel, level, pan_state.code()],
        ),
        Command::SetMixerMute {
            mixer,
            channel,
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
                pan_state.muted_code(muted),
            ],
        ),
        Command::SetMixerPan {
            mixer,
            channel,
            pan,
        } => host_frame(0x16, &[0xd4, 0x04, mixer.code(), channel, 0x00, pan.code()]),
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
    let entry_index = MixerStrip::ordinary(strip)
        .and_then(|value| value.assignment_entry_index())
        .expect("ordinary strip assignment write requires grounded ordinary-strip mapping");
    let tuple_offset = 0x03 + entry_index * 2;
    let [a, b] = assignment.ordinary_strip_bytes();

    [0x06_u8, 0x07, 0x08, 0x09]
        .into_iter()
        .map(|bank| {
            let mut frame = vec![0_u8; HID_REPORT_SIZE];
            frame[0..4].copy_from_slice(&0x70_u32.to_le_bytes());
            frame[4..8].copy_from_slice(&0x53_u32.to_le_bytes());
            frame[0x10..0x13].copy_from_slice(&[0xd3, 0x41, bank]);
            frame[0x10 + tuple_offset] = a;
            frame[0x10 + tuple_offset + 1] = b;
            frame
        })
        .collect()
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
    fn ordinary_strip_index_map_stays_out_of_early_afx_range() {
        assert_eq!(MixerStrip::ordinary(4), None);
        assert_eq!(
            MixerStrip::ordinary(5).map(|strip| strip.assignment_entry_index()),
            Some(Some(4))
        );
        assert_eq!(
            MixerStrip::ordinary(16).map(|strip| strip.assignment_entry_index()),
            Some(Some(15))
        );
    }

    #[test]
    fn link_target_mapping_stays_limited_to_grounded_selectors() {
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
            None
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 2).map(|target| target.selector),
            Some(0x01)
        );
        assert_eq!(MixerLinkTarget::from_channel(MixerSurface::Mix2, 7), None);
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
            Some(0x01)
        );
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
        assert_eq!(metadata.version, "6.6");
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
        let defaults = QueryReply75 {
            query_id: 0x00,
            sub_id: 0x00,
            body: vec![0xaa, 0xbb, 0xcc],
        };
        let status = QueryReply75 {
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
    fn decodes_assignment_readback_from_grounded_0x75_banks() {
        let early_bank = QueryReply75 {
            query_id: 0x03,
            sub_id: 0x05,
            body: vec![0x05, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x01, 0x01],
        };
        let ordinary_bank = QueryReply75 {
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
        let reply = QueryReply75 {
            query_id: 0x18,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x02, 0x60, 0x3e, 0x2e, 0x02, 0x60, 0x3e,
                0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                0x60, 0x3e, 0x60, 0x02,
            ],
        };

        let strips = reply.mixer_strip_readback().expect("strip state readback");
        assert_eq!(strips[0].level, 0x00);
        assert_eq!(strips[0].pan, PanState::center());
        assert!(!strips[0].muted);
        assert_eq!(strips[3].level, 0x60);
        assert_eq!(strips[3].pan, PanState::left());
        assert!(!strips[3].muted);
        assert_eq!(strips[4].level, 0x60);
        assert_eq!(strips[4].pan, PanState::right());
        assert_eq!(strips[5].level, 0x2e);
        assert_eq!(strips[5].pan, PanState::left());
        assert!(strips.iter().all(|strip| !strip.soloed));
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
        assert_eq!(silence.display_db(), Some(-96));
        assert_eq!(unity.gain_ratio(), Some(1.0));
        assert_eq!(silence.gain_ratio(), Some(0.0));
    }

    #[test]
    fn decodes_passive_mix1_strip1_meter_from_late_row_cluster() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0x6f] = 0x49;
        payload[0x8f] = 0x49;
        payload[0xcf] = 0x49;
        payload[0xda] = 0x49;
        payload[0xdd] = 0x49;
        payload[0xde] = 0x49;
        payload[0xdf] = 0x49;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        let strip = snapshot
            .mixer_decode
            .strip(MixerSurface::Mix1, 1)
            .expect("mix1 strip 1");
        assert_eq!(strip.meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, Some(0x49));
        assert_eq!(strip.pan, Some(PanState::center()));
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
        payload[0x8f] = 0x5a;
        payload[0xcf] = 0x4c;
        payload[0xda] = 0x51;
        payload[0xdb] = 0x51;
        payload[0xdc] = 0x51;
        payload[0xdd] = 0x51;
        payload[0xde] = 0x51;
        payload[0xdf] = 0x51;

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
