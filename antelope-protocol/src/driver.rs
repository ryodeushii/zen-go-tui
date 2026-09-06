use crate::mixer::{MixerChannelState, MixerSurface};
use crate::profile::RuntimeMeterTarget;
use crate::query::QueryRequest;
use crate::types::{
    ClockSource, OutputState, PreampInputState, ProtocolError, SampleRate, Surface,
};

/// Owned identity and protocol facts exposed by a device driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverDefinition {
    pub id: String,
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub supported: bool,
}

/// Compatibility spelling retained while callers migrate.
pub type DeviceDefinition = DriverDefinition;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandBatch {
    pub frames: Vec<Vec<u8>>,
    pub refresh_requests: Vec<QueryRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputAddress {
    pub space: u16,
    pub index: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputAddress {
    pub id: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MixerAddress {
    pub surface: u8,
    pub strip: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoutingSource {
    pub bank: u8,
    pub index: u16,
}

/// One named field in a complete profile-defined whole-state operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WholeStateField {
    pub id: u16,
    pub value: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlValue {
    Bool(bool),
    Int(i32),
    Enum(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputControl {
    Mode,
    Gain,
    Phantom,
    Phase,
    Parameter(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputControl {
    Level,
    Mute,
    Dim,
    Parameter(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MixerControl {
    Fader,
    Pan,
    Send,
    Mute,
    Solo,
    Parameter(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobalControl {
    SampleRate,
    ClockSource,
    Surface,
    Parameter(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicGlobalState {
    pub control: GlobalControl,
    pub value: ControlValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicInputState {
    pub address: InputAddress,
    pub name: String,
    pub mode: Option<i32>,
    pub gain: Option<i32>,
    pub phantom: Option<bool>,
    pub phase: Option<bool>,
    pub meter: Option<u8>,
    pub parameters: Vec<(u16, ControlValue)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicOutputState {
    pub address: OutputAddress,
    pub name: String,
    pub level: Option<i32>,
    pub muted: Option<bool>,
    pub dimmed: Option<bool>,
    pub parameters: Vec<(u16, ControlValue)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicMixerStrip {
    pub strip: u16,
    pub name: String,
    pub fader: Option<i32>,
    pub pan: Option<i32>,
    pub send: Option<i32>,
    pub muted: Option<bool>,
    pub soloed: Option<bool>,
    pub linked: Option<bool>,
    pub meter: Option<u8>,
    pub parameters: Vec<(u16, ControlValue)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicMixerSurface {
    pub surface: u8,
    pub name: String,
    pub master: Option<DynamicMixerStrip>,
    pub strips: Vec<DynamicMixerStrip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicMeterState {
    pub target: RuntimeMeterTarget,
    pub target_index: u16,
    pub lane: u8,
    pub value: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicRoutingGroup {
    pub destination: u16,
    pub name: String,
    pub sources: Vec<RoutingSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicDeviceState {
    pub globals: Vec<DynamicGlobalState>,
    pub inputs: Vec<DynamicInputState>,
    pub outputs: Vec<DynamicOutputState>,
    pub mixers: Vec<DynamicMixerSurface>,
    pub meters: Vec<DynamicMeterState>,
    pub routing: Vec<DynamicRoutingGroup>,
    /// Private migration payload used only by the fixed Zen Go application state.
    #[doc(hidden)]
    pub zen_go_compatibility: Option<Box<ZenGoCompatibilityState>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct ZenGoCompatibilityState {
    pub sample_rate: SampleRate,
    pub clock_source: ClockSource,
    pub sample_rate_hz: u32,
    pub status_flags: Vec<u8>,
    pub front_panel_bytes: Vec<u8>,
    pub outputs: Vec<OutputState>,
    pub preamps: Vec<PreampInputState>,
    pub dsp_cluster: Vec<u8>,
    pub surface: Surface,
    pub mixer_surfaces: Vec<(MixerSurface, Vec<MixerChannelState>)>,
    pub late_shadow: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DynamicStatePatch {
    Inputs(Vec<DynamicInputState>),
    Outputs(Vec<DynamicOutputState>),
    Mixer(DynamicMixerSurface),
    /// Partial readback containing multiple mixer surfaces.
    Mixers(Vec<DynamicMixerSurface>),
    Routing(DynamicRoutingGroup),
    Globals(Vec<DynamicGlobalState>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    Snapshot {
        state: DynamicDeviceState,
        raw: Vec<u8>,
    },
    QueryReply {
        query_id: u8,
        sub_id: u8,
        body: Vec<u8>,
        patch: Option<DynamicStatePatch>,
        raw: Vec<u8>,
    },
    /// Confirmed meter values decoded without replacing unrelated snapshot state.
    Meter {
        inputs: Vec<DynamicInputState>,
        meters: Vec<DynamicMeterState>,
        raw: Vec<u8>,
    },
    Auxiliary {
        bytes: Vec<u8>,
        raw: Vec<u8>,
    },
    Notification {
        bytes: Vec<u8>,
        raw: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    SetInput {
        address: InputAddress,
        control: InputControl,
        value: ControlValue,
    },
    SetOutput {
        address: OutputAddress,
        control: OutputControl,
        value: ControlValue,
    },
    SetMixer {
        address: MixerAddress,
        control: MixerControl,
        value: ControlValue,
    },
    /// Complete atomic strip state required by compound-frame devices such as Zen Go.
    SetMixerStripState {
        address: MixerAddress,
        fader: i32,
        pan: i32,
        muted: bool,
        soloed: bool,
        send: Option<i32>,
    },
    SetLink {
        surface: u8,
        pair: u16,
        enabled: bool,
    },
    SetRouting {
        destination: u16,
        channel: u16,
        source: RoutingSource,
    },
    /// Complete ordered routing group required by atomic assignment-table devices.
    SetRoutingGroup {
        destination: u16,
        changed_channel: Option<u16>,
        sources: Vec<RoutingSource>,
    },
    SetGlobal {
        control: GlobalControl,
        value: ControlValue,
    },
    /// Complete generic whole-state command. Profiles define field IDs, ranges,
    /// frame offsets, and fixed operation bytes; partial writes are rejected.
    SetWholeState {
        operation: u16,
        target: u16,
        enabled: bool,
        fields: Vec<WholeStateField>,
    },
    Query(QueryRequest),
}

#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("unsupported driver action: {0}")]
    UnsupportedAction(String),
    #[error("invalid driver action: {0}")]
    InvalidAction(String),
}

pub trait DeviceDriver: Send {
    fn definition(&self) -> &DriverDefinition;
    fn startup_requests(&self) -> &[QueryRequest];
    fn encode(&self, action: Action) -> Result<CommandBatch, DriverError>;
    fn decode(&self, bytes: &[u8]) -> Result<Option<DeviceEvent>, DriverError>;
}
