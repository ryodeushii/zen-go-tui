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

/// One virtual mixer surface and its strip geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub mix_index: u8,
    pub strip_count: u16,
    pub fader_range: Option<(i32, i32)>,
    pub pan_range: Option<(i32, i32)>,
    pub pan_center: Option<i32>,
    pub send_range: Option<(i32, i32)>,
    pub status: Status,
    pub status_text: &'static str,
    pub notes: &'static str,
    pub metadata: &'static str,
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

/// Complete typed definition for one canonical hardware profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceDefinition {
    pub identity: DeviceIdentity,
    pub transport: TransportDefinition,
    pub address_spaces: &'static [AddressSpaceDefinition],
    pub inputs: &'static [InputDefinition],
    pub outputs: &'static [OutputDefinition],
    pub mixers: &'static [MixerDefinition],
    pub frames: &'static [FrameDefinition],
    pub decoders: &'static [DecoderDefinition],
    pub params: &'static [ParamDefinition],
    pub constraints: &'static [ConstraintDefinition],
    pub hazards: &'static [HazardDefinition],
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
