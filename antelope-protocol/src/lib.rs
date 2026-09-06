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

pub mod driver;
pub use driver::{
    Action, CommandBatch, ControlValue, DeviceDefinition, DeviceDriver, DeviceEvent,
    DriverDefinition, DriverError, DynamicDeviceState, DynamicGlobalState, DynamicInputState,
    DynamicMeterState, DynamicMixerStrip, DynamicMixerSurface, DynamicOutputState,
    DynamicRoutingGroup, DynamicStatePatch, GlobalControl, InputAddress, InputControl,
    MixerAddress, MixerControl, OutputAddress, OutputControl, RoutingSource, WholeStateField,
};
mod encoder;
mod frame;
mod mixer;
pub mod profile;
mod profile_codec;
pub mod profile_driver;
mod query;
mod types;
pub mod zen_go;

pub use profile_driver::ProfileDriver;
pub use zen_go::ZenGoDriver;

// Explicit re-exports for API stability and discoverability
pub use encoder::{
    encode_command, encode_link_companion, encode_mixer_assignment_frames_with_table, encode_query,
    Command, EncodeResult,
};
pub use frame::{DeviceNotification, DeviceSnapshot, Frame};
pub use mixer::{
    decode_passive_mixer_state, MixerAssignment, MixerChannelState, MixerLinkTarget,
    MixerPassiveDecode, MixerPassiveStripState, MixerStrip, MixerStripKind, MixerSurface,
};
pub use profile::{
    load_profile_pack, load_profile_pack_file, CandidatePreampMeter, FaderDirection,
    FaderSemantics, FrameEndian, FrameOperation, MixerReadbackLayout, ParamReference,
    ProfileLoadError, ProfilePack, ReadbackCategory, ReadbackDefinition, RuntimeAddressSpace,
    RuntimeConstraint, RuntimeDecoder, RuntimeDriverKind, RuntimeEntry, RuntimeFrame,
    RuntimeHazard, RuntimeIdentity, RuntimeInput, RuntimeInputCapability, RuntimeInputControlKind,
    RuntimeLinkDomain, RuntimeLinkDomainKind, RuntimeMeterMapping, RuntimeMeterTarget,
    RuntimeMixer, RuntimeOutput, RuntimeParam, RuntimeProfile, RuntimeProvenance, RuntimeReadiness,
    RuntimeRoutingGroup, RuntimeRoutingReadbackSourceDomain, RuntimeRoutingSourceDomain,
    RuntimeStateReport, RuntimeTransport, SafeQuery, PROFILE_PACK_SCHEMA_VERSION,
};
pub use query::{control_panel_startup_queries, DeviceMetadata, QueryRequest, QueryResponse};

/// Encode one query using profile-declared sparse safety or bounded categories.
///
/// This narrow wrapper keeps profile query safety available to callers without
/// exposing the generic profile codec implementation module.
pub fn encode_profile_query(
    profile: &RuntimeProfile,
    query: QueryRequest,
) -> Result<Vec<u8>, DriverError> {
    let readback = profile.readback.as_ref().ok_or_else(|| {
        DriverError::UnsupportedAction("profile has no readback safety data".into())
    })?;
    profile_codec::encode_query(profile, readback, query)
}

pub use types::{
    meter_db_ratio,
    meter_display_db,
    meter_ratio,
    ClockSource,
    DeviceStateSnapshot,
    OutputMode,
    OutputState,
    OutputTarget,
    PanState,
    PreampInputState,
    PreampMode,
    PreampState,
    ProtocolError,
    SampleRate,
    Surface,
    FRAME_TYPE_AUXILIARY,
    FRAME_TYPE_QUERY_REPLY,
    // Frame type identifiers
    FRAME_TYPE_SNAPSHOT,
    // Frame geometry constants
    HID_REPORT_SIZE,
    MIN_SNAPSHOT_FRAME_LEN,
    OFFSET_CLOCK_SOURCE,
    OFFSET_FRONT_PANEL_BYTES_END,
    OFFSET_FRONT_PANEL_BYTES_START,
    OFFSET_HP1_MODE,
    OFFSET_HP1_VOLUME,
    OFFSET_HP2_MODE,
    OFFSET_HP2_VOLUME,
    OFFSET_LATE_SHADOW_END,
    // Snapshot payload offsets — late shadow region
    OFFSET_LATE_SHADOW_START,
    OFFSET_METER_LANES_END,
    // Snapshot payload offsets — meter lanes
    OFFSET_METER_LANES_START,
    OFFSET_MIX1_LANE_A,
    OFFSET_MIX1_LANE_B,
    OFFSET_MIX1_MIRROR_A,
    OFFSET_MIX1_MIRROR_B,
    // Snapshot payload offsets — mute/pan primary bytes
    OFFSET_MIX1_PRIMARY,
    OFFSET_MIX2_LANE_A,
    OFFSET_MIX2_LANE_B,
    OFFSET_MIX2_PRIMARY,
    OFFSET_MONITOR_MODE,
    // Snapshot payload offsets — output state region
    OFFSET_MONITOR_VOLUME,
    // Snapshot payload offsets — DSP/preamp cluster
    OFFSET_PREAMP1_GAIN,
    OFFSET_PREAMP1_METER,
    OFFSET_PREAMP1_MODE,
    OFFSET_PREAMP2_GAIN,
    OFFSET_PREAMP2_METER,
    OFFSET_PREAMP2_MODE,
    OFFSET_SAMPLE_RATE_CODE,
    OFFSET_SAMPLE_RATE_HZ_END,
    OFFSET_SAMPLE_RATE_HZ_START,
    OFFSET_SHARED_SHADOW_0,
    OFFSET_SHARED_SHADOW_1,
    OFFSET_SHARED_SHADOW_2,
    OFFSET_SHARED_SHADOW_3,
    OFFSET_SHARED_SHADOW_4,
    OFFSET_SHARED_SHADOW_5,
    // Snapshot payload offsets — header region
    OFFSET_STATUS_FLAGS_0,
    OFFSET_STATUS_FLAGS_1,
    // Snapshot payload offsets — surface selector
    OFFSET_SURFACE_SELECTOR,
    // Snapshot payload offset — unknown byte between surface selector and meter lanes
    OFFSET_UNKNOWN_6E,
    SNAPSHOT_PAYLOAD_OFFSET,
    SNAPSHOT_PAYLOAD_SIZE,
    SURFACE_CODE_HP2,
    // Surface selector codes
    SURFACE_CODE_MONITOR_HP1,
};
