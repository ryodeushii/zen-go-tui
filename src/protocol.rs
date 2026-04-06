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
}

impl PreampInputState {
    pub fn from_raw(gain_raw: u8, mode_raw: u8) -> Self {
        let mode = PreampMode::from_raw(mode_raw);
        Self {
            gain_raw,
            mode,
            phantom_on: matches!(mode, PreampMode::Mic) && mode_raw & 0x10 != 0,
            mode_raw,
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

    pub fn from_raw(raw: u8) -> Self {
        Self(raw.clamp(Self::MIN, Self::MAX))
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
        let base = self.code();
        if muted {
            base | 0x40
        } else {
            base
        }
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

impl MixerAssignment {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryReply75 {
    pub query_id: u8,
    pub body: Vec<u8>,
}

impl QueryReply75 {
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
        include_companion: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerChannelState {
    pub channel: u8,
    pub level: Option<u8>,
    pub muted: Option<bool>,
    pub pan: PanState,
    pub assignment: Option<MixerAssignment>,
    pub linked: Option<bool>,
}

impl MixerChannelState {
    pub fn unknown(channel: u8) -> Self {
        Self {
            channel,
            level: None,
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
}

pub fn encode_query(query_id: u8) -> Vec<u8> {
    let mut frame = vec![0_u8; HID_REPORT_SIZE];
    frame[0..4].copy_from_slice(&0x74_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x10_u32.to_le_bytes());
    frame[0x08] = query_id;
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
            include_companion,
        } => {
            if include_companion {
                host_frame(0x14, &[0xa2, 0x04, 0x01, u8::from(enabled)])
            } else {
                host_frame(0x14, &[0xa2, 0x03, selector, u8::from(enabled)])
            }
        }
    }
}

fn encode_mixer_assignment(strip: u8, assignment: MixerAssignment) -> Vec<u8> {
    let mut frame = vec![0_u8; HID_REPORT_SIZE];
    frame[0..4].copy_from_slice(&0x70_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x53_u32.to_le_bytes());
    frame[0x10..0x13].copy_from_slice(&[0xd3, 0x41, 0xbb]);
    let entry_index = strip.saturating_sub(1) as usize;
    let tuple_offset = 0x03 + 0x17 + entry_index * 2;
    let [a, b] = assignment.ordinary_strip_bytes();
    frame[0x10 + tuple_offset] = a;
    frame[0x10 + tuple_offset + 1] = b;
    frame
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
    fn encodes_ordinary_strip_assignment_write() {
        let frame = encode_command(Command::SetMixerAssignment {
            strip: 11,
            assignment: MixerAssignment::EmuMic(2),
        });

        assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
        assert_eq!(&frame[4..8], &0x53_u32.to_le_bytes());
        assert_eq!(&frame[0x10..0x13], &[0xd3, 0x41, 0xbb]);
        assert_eq!(&frame[0x3e..0x40], &[0x0a, 0x01]);
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
}
