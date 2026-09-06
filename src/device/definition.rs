//! Typed, generated-independent representation of an Antelope device profile.
//!
//! `generated.rs` contains only static data.  Keep these definitions free of
//! filesystem or JSON dependencies so normal Cargo builds do not need the
//! Antelope-Ctl checkout that produced the catalog.

/// Status attached to a field in a canonical profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Confirmed,
    Observed,
    Unconfirmed,
    Unavailable,
    Unknown,
}

/// Compatibility alias for callers that prefer the longer name.
pub type DefinitionStatus = Status;

/// Profile/runtime support classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportLevel {
    /// Profile and runtime support are validated.
    Supported,
    /// Some protocol data is known, but normal control is not safe.
    Partial,
    /// Profile exists, but its data has not been confirmed.
    Unverified,
    /// Profile is known but must not be controlled.
    Unsupported,
}

/// Runtime readiness is deliberately separate from canonical profile status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Supported,
    Partial,
    Unverified,
    Disabled,
}

impl Readiness {
    /// Whether a caller may construct a normal device driver for this entry.
    pub const fn is_selectable(self) -> bool {
        matches!(self, Self::Supported)
    }
}

/// Addressing convention used by a device address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressingMode {
    ZeroBased,
    OneBased,
    Unknown,
}

/// Kind of address space represented by a profile section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceKind {
    PhysicalInputs,
    AdatInputs,
    SpdifInputs,
    Outputs,
    Mixer,
    Routing,
    Unknown,
}

/// Closed presentation/action kinds for canonical input capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputControlKind {
    Gain,
    Mode,
    Phantom,
    Phase,
    Link,
    Parameter,
}

/// One canonical typed input control attached to an address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputCapabilityDefinition {
    pub kind: InputControlKind,
    pub parameter: &'static str,
    pub parameter_id: Option<u16>,
    pub label: &'static str,
}

/// Transport family represented by a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Hid,
    Unknown,
}

/// Frame direction/layout category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Command,
    StateReport,
    MeterReport,
    NameReport,
    InitEnumerationReport,
    ErrorResponse,
    Response,
    Decoder,
    Unknown,
}

/// Normalized parameter value representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamValueType {
    Bool,
    Enum,
    Int,
    Int8,
    UInt,
    Unknown,
}

/// Device identity from a canonical profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub name: &'static str,
    pub vid: u16,
    pub pid: u16,
    pub bcd_device: Option<&'static str>,
    pub status: Status,
    pub status_text: &'static str,
    pub notes: &'static str,
    pub evidence: &'static str,
}

/// HID transport geometry and provenance status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportDefinition {
    pub kind: TransportKind,
    pub report_size: Option<u16>,
    pub out_endpoint: Option<u8>,
    pub in_endpoint: Option<u8>,
    pub poll_interval_ms: Option<u16>,
    pub uses_numbered_reports: Option<bool>,
    /// Expected HID control interface, when profile evidence identifies one.
    pub expected_interface_number: Option<i32>,
    pub expected_usage_page: Option<u16>,
    pub expected_usage: Option<u16>,
    pub status: Status,
    pub status_text: &'static str,
    pub notes: &'static str,
    pub evidence: &'static str,
}

/// A normalized input/preamp address-space descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressSpaceDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: AddressSpaceKind,
    pub count: Option<u16>,
    pub addressing: AddressingMode,
    pub status: Status,
    pub status_text: &'static str,
    pub notes: &'static str,
    pub metadata: &'static str,
    pub input_capabilities: &'static [InputCapabilityDefinition],
}

/// One addressable input channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputDefinition {
    pub id: &'static str,
    pub space: &'static str,
    pub index: u16,
    pub name: &'static str,
    pub hiz_capable: bool,
    pub status: Status,
    pub metadata: &'static str,
}

/// One addressable output bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputDefinition {
    pub id: u16,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub verified: bool,
    pub status: Status,
    pub metadata: &'static str,
}

/// Fader domain direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaderDirectionDefinition {
    Direct,
    Attenuation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaderSemanticsDefinition {
    pub min: i32,
    pub max: i32,
    pub direction: FaderDirectionDefinition,
    pub unity: i32,
}

/// One virtual mixer surface and its strip geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub mix_index: u8,
    pub strip_count: u16,
    pub has_master: bool,
    pub fader_range: Option<(i32, i32)>,
    pub fader: Option<FaderSemanticsDefinition>,
    pub pan_range: Option<(i32, i32)>,
    pub pan_center: Option<i32>,
    pub send_range: Option<(i32, i32)>,
    pub status: Status,
    pub status_text: &'static str,
    pub notes: &'static str,
    pub metadata: &'static str,
}

/// Closed semantic kind for one confirmed generic link protocol domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkDomainKind {
    Mixer,
}

/// One confirmed finite link protocol domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkDomainDefinition {
    pub protocol_space: u8,
    pub kind: LinkDomainKind,
    pub pair_count: u16,
    pub status: Status,
    pub evidence: &'static str,
}

/// One finite source-bank domain allowed for a routing destination's writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingSourceDomainDefinition {
    pub bank: u8,
    pub index_count: u16,
    pub kind: &'static str,
    pub name: &'static str,
    pub display_index_base: u16,
    pub status: Status,
    pub evidence: &'static str,
}

/// One explicitly observed source-bank/index set accepted by a routing readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingReadbackSourceDomainDefinition {
    pub bank: u8,
    pub indices: &'static [u8],
    pub status: Status,
    pub evidence: &'static str,
}

/// One finite logical routing destination group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingGroupDefinition {
    pub destination: u16,
    pub name: &'static str,
    pub channel_count: u16,
    pub mixer_surface: Option<u8>,
    pub source_domains: &'static [RoutingSourceDomainDefinition],
    pub readback_source_domains: &'static [RoutingReadbackSourceDomainDefinition],
}

/// A scalar field in a command/report layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFieldDefinition {
    pub name: &'static str,
    pub offset: Option<u16>,
    pub stride: Option<u16>,
    pub width: Option<u16>,
    pub value: Option<i32>,
    pub mask: Option<u8>,
    pub values: &'static [i32],
    pub formula: &'static str,
    pub text: &'static str,
    pub children: &'static [FrameFieldDefinition],
}

/// Byte order for a typed scalar operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameEndianDefinition {
    NotApplicable,
    Little,
    Big,
}

/// One finite-domain typed operation compiled from canonical frame geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOperationDefinition {
    FixedByte {
        offset: u16,
        value: u8,
    },
    Scalar {
        field: &'static str,
        offset: u16,
        width: u8,
        endian: FrameEndianDefinition,
    },
    Indexed {
        base: u16,
        stride: u16,
        index_field: &'static str,
        width: u8,
        max_index: Option<u16>,
    },
    BitField {
        field: &'static str,
        offset: u16,
        mask: u8,
        shift: u8,
    },
    PairIndex {
        base: u16,
        stride: u16,
        pair_field: &'static str,
        width: u8,
        max_index: Option<u16>,
    },
    AllowedValues {
        values: &'static [i32],
    },
    UncompiledFormula {
        formula: &'static str,
    },
}

/// A command, report, or decoder frame layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameDefinition {
    pub id: &'static str,
    pub kind: FrameKind,
    pub status: Status,
    pub status_text: &'static str,
    pub magic_offset: Option<u16>,
    pub magic: Option<u8>,
    pub opcode_offset: Option<u16>,
    pub opcode: Option<u8>,
    pub opcode_name: &'static str,
    pub fields: &'static [FrameFieldDefinition],
    pub operations: &'static [FrameOperationDefinition],
    pub metadata: &'static str,
}

/// Extra decoder metadata associated with an incoming frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoderDefinition {
    pub id: &'static str,
    pub frame_id: &'static str,
    pub kind: FrameKind,
    pub status: Status,
    pub metadata: &'static str,
}

/// An enum value or symbolic parameter value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamValueDefinition {
    pub value: i32,
    pub name: &'static str,
}

/// A numeric offset extracted from a parameter's profile reference text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamOffsetDefinition {
    pub name: &'static str,
    pub offset: u16,
    pub formula: &'static str,
}

/// A parameter frame or readback reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamReference {
    pub text: &'static str,
    pub formula: &'static str,
    pub offsets: &'static [ParamOffsetDefinition],
}

/// One named numeric or explanatory parameter range form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamRangeDefinition {
    pub name: &'static str,
    pub range: Option<(i32, i32)>,
    pub text: &'static str,
}

/// A parameter/range/readback description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamDefinition {
    pub name: &'static str,
    pub id: Option<u16>,
    pub value_type: ParamValueType,
    pub status: Status,
    pub status_text: &'static str,
    pub applies_to: &'static str,
    pub range: Option<(i32, i32)>,
    pub direction: Option<FaderDirectionDefinition>,
    pub unity: Option<i32>,
    pub range_by_mode: &'static [ParamRangeDefinition],
    pub range_forms: &'static [ParamRangeDefinition],
    pub values: &'static [ParamValueDefinition],
    pub frame: ParamReference,
    pub readback: ParamReference,
    pub encoding: &'static str,
    pub metadata: &'static str,
}

/// A normalized safety constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstraintDefinition {
    pub name: &'static str,
    pub status: Status,
    pub range: Option<(i32, i32)>,
    pub scalar: Option<i32>,
    pub values: &'static [i32],
    pub text: &'static str,
    pub metadata: &'static str,
}

/// A hazard that must be visible to driver/runtime safety checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HazardDefinition {
    pub name: &'static str,
    pub status: Status,
    pub rule: &'static str,
    pub effect: &'static str,
    pub notes: &'static str,
    pub opcodes: &'static [u8],
    pub metadata: &'static str,
}

/// Source provenance retained in generated catalogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provenance {
    pub source_path: &'static str,
    pub source_sha256: &'static str,
    pub generator_version: &'static str,
}

/// One startup category/index query compiled from confirmed readback bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupQueryDefinition {
    pub query_id: u8,
    pub sub_id: u8,
}

/// One confirmed safe readback category bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadbackCategoryDefinition {
    pub category: u8,
    pub count: u16,
}

/// One explicitly safe readback query pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafeQueryDefinition {
    pub category: u8,
    pub index: u8,
}

/// One captured readback body layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerReadbackLayoutDefinition {
    pub category: u8,
    pub index: u8,
    pub body_size: usize,
    pub record_count: usize,
    pub record_stride: usize,
    pub level_offset: usize,
    pub state_offset: usize,
    pub surface: Option<u8>,
    pub surface_stride: Option<usize>,
    pub supported_fields: &'static [&'static str],
}

/// Candidate meter byte retained with state-report metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidatePreampMeterDefinition {
    pub input_index: u16,
    /// Payload-relative state-report byte offset.
    pub offset: usize,
    /// Inclusive raw-byte ranges accepted by the candidate decoder.
    pub raw_value_ranges: &'static [(u8, u8)],
    pub status: Status,
    pub status_text: &'static str,
    pub confidence: &'static str,
    pub caveat: &'static str,
}

/// Typed target kind for an explicit one-byte meter lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterTargetDefinition {
    MixMaster,
    PhysicalOutput,
}

/// One explicit profile-owned meter lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeterMappingDefinition {
    pub frame_id: &'static str,
    pub target: MeterTargetDefinition,
    pub target_index: u16,
    pub lane: u8,
    pub offset: usize,
    pub status: Status,
    pub status_text: &'static str,
    pub evidence: &'static str,
}

/// State-report metadata retained in the built-in artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateReportDefinition {
    pub candidate_preamp_meters: &'static [CandidatePreampMeterDefinition],
}

/// Optional generic readback layout retained in the built-in artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadbackDefinition {
    pub request_magic: u8,
    pub request_subcommand: u32,
    pub response_magic: u8,
    pub response_discriminator_offset: u16,
    pub response_discriminator: u8,
    pub category_offset: u16,
    pub index_offset: u16,
    pub data_offset: u16,
    pub category_counts: &'static [ReadbackCategoryDefinition],
    pub safe_queries: &'static [SafeQueryDefinition],
    pub layouts: &'static [MixerReadbackLayoutDefinition],
}

/// Complete typed definition for one canonical hardware profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceDefinition {
    pub identity: DeviceIdentity,
    pub transport: TransportDefinition,
    pub address_spaces: &'static [AddressSpaceDefinition],
    pub inputs: &'static [InputDefinition],
    pub outputs: &'static [OutputDefinition],
    pub mixers: &'static [MixerDefinition],
    pub link_domains: &'static [LinkDomainDefinition],
    pub routing_groups: &'static [RoutingGroupDefinition],
    pub frames: &'static [FrameDefinition],
    pub decoders: &'static [DecoderDefinition],
    pub params: &'static [ParamDefinition],
    pub constraints: &'static [ConstraintDefinition],
    pub hazards: &'static [HazardDefinition],
    pub meter_mappings: &'static [MeterMappingDefinition],
    pub state_report: Option<StateReportDefinition>,
    pub startup_queries: &'static [StartupQueryDefinition],
    pub readback: Option<ReadbackDefinition>,
    pub status: Status,
    pub status_text: &'static str,
    pub support_level: SupportLevel,
    pub readiness: Readiness,
    pub provenance: Provenance,
    /// Canonical JSON retained for forward-compatible metadata inspection.
    pub raw_profile: &'static str,
}

/// Catalog entry keyed by USB identity and carrying one typed definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceEntry {
    pub definition: DeviceDefinition,
    pub support_level: SupportLevel,
    pub readiness: Readiness,
}

impl DeviceEntry {
    pub const fn vid(self) -> u16 {
        self.definition.identity.vid
    }

    pub const fn pid(self) -> u16 {
        self.definition.identity.pid
    }

    pub const fn is_selectable(self) -> bool {
        self.readiness.is_selectable()
    }
}
