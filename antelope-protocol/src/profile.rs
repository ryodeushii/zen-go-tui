//! Owned, validated runtime profile packs.

use crate::{types::PanState, QueryRequest, SNAPSHOT_PAYLOAD_OFFSET};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

pub const PROFILE_PACK_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfilePack {
    pub schema_version: u16,
    pub generator_version: String,
    pub profiles: Vec<RuntimeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEntry {
    pub id: String,
    #[serde(flatten)]
    pub profile: RuntimeProfile,
    pub readiness: RuntimeReadiness,
    pub driver_kind: RuntimeDriverKind,
    pub support_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProfile {
    pub identity: RuntimeIdentity,
    pub transport: RuntimeTransport,
    pub address_spaces: Vec<RuntimeAddressSpace>,
    pub inputs: Vec<RuntimeInput>,
    pub outputs: Vec<RuntimeOutput>,
    pub mixers: Vec<RuntimeMixer>,
    #[serde(default)]
    pub meter_mappings: Vec<RuntimeMeterMapping>,
    #[serde(default)]
    pub state_report: Option<RuntimeStateReport>,
    #[serde(default)]
    pub link_domains: Vec<RuntimeLinkDomain>,
    pub routing_groups: Vec<RuntimeRoutingGroup>,
    pub frames: Vec<RuntimeFrame>,
    pub decoders: Vec<RuntimeDecoder>,
    pub params: Vec<RuntimeParam>,
    pub constraints: Vec<RuntimeConstraint>,
    pub hazards: Vec<RuntimeHazard>,
    #[serde(with = "query_requests")]
    pub startup_queries: Vec<QueryRequest>,
    pub readback: Option<ReadbackDefinition>,
    pub provenance: RuntimeProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadbackDefinition {
    pub request_magic: u8,
    pub request_subcommand: u32,
    pub response_magic: u8,
    pub response_discriminator_offset: u16,
    pub response_discriminator: u8,
    pub category_offset: u16,
    pub index_offset: u16,
    pub data_offset: u16,
    #[serde(default)]
    pub category_counts: Vec<ReadbackCategory>,
    #[serde(default)]
    pub safe_queries: Vec<SafeQuery>,
    #[serde(default)]
    pub layouts: Vec<MixerReadbackLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SafeQuery {
    pub category: u8,
    pub index: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MixerReadbackLayout {
    pub category: u8,
    pub index: u8,
    pub body_size: usize,
    pub record_count: usize,
    pub record_stride: usize,
    pub level_offset: usize,
    pub state_offset: usize,
    #[serde(default)]
    pub surface: Option<u8>,
    #[serde(default)]
    pub surface_stride: Option<usize>,
    #[serde(default)]
    pub supported_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidatePreampMeter {
    pub input_index: u16,
    /// Byte offset relative to the state-report payload (after its 0x10-byte header).
    pub offset: usize,
    /// Inclusive raw-byte ranges accepted as a candidate meter value.
    pub raw_value_ranges: Vec<(u8, u8)>,
    pub status: String,
    pub confidence: String,
    pub caveat: String,
}

impl CandidatePreampMeter {
    /// Returns whether a raw byte belongs to this profile-owned candidate lane.
    pub fn accepts(&self, raw: u8) -> bool {
        self.raw_value_ranges
            .iter()
            .any(|(minimum, maximum)| (*minimum..=*maximum).contains(&raw))
    }
}

/// Profile-generated Zen Go candidates retained only for legacy profile-less parsers.
pub(crate) fn legacy_zen_go_candidate_preamp_meters() -> &'static [CandidatePreampMeter] {
    static METERS: OnceLock<Vec<CandidatePreampMeter>> = OnceLock::new();
    METERS
        .get_or_init(|| {
            let meters: Vec<CandidatePreampMeter> = serde_json::from_slice(include_bytes!(
                "legacy_zen_go_candidate_preamp_meters.json"
            ))
            .expect("generated legacy Zen Go candidate meter artifact must be valid JSON");
            assert!(
                meters.len() == 2
                    && meters
                        .iter()
                        .enumerate()
                        .all(|(index, meter)| meter.input_index == index as u16
                            && !meter.raw_value_ranges.is_empty()
                            && !meter.status.trim().is_empty()
                            && !meter.confidence.trim().is_empty()
                            && !meter.caveat.trim().is_empty()),
                "generated legacy Zen Go candidate meter artifact must retain inputs 0 and 1 with provenance"
            );
            meters
        })
        .as_slice()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaderDirection {
    Direct,
    Attenuation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaderSemantics {
    pub min: i32,
    pub max: i32,
    pub direction: FaderDirection,
    pub unity: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadbackCategory {
    pub category: u8,
    pub count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMeterTarget {
    MixMaster,
    PhysicalOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMeterMapping {
    pub frame_id: String,
    pub target: RuntimeMeterTarget,
    pub target_index: u16,
    pub lane: u8,
    pub offset: usize,
    pub status: String,
    pub status_text: String,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStateReport {
    #[serde(default)]
    pub candidate_preamp_meters: Vec<CandidatePreampMeter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub bcd_device: Option<String>,
    pub status: String,
    pub status_text: String,
    pub notes: String,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTransport {
    pub kind: String,
    pub report_size: Option<u16>,
    pub out_endpoint: Option<u8>,
    pub in_endpoint: Option<u8>,
    pub poll_interval_ms: Option<u16>,
    pub uses_numbered_reports: Option<bool>,
    pub expected_interface_number: Option<i32>,
    pub expected_usage_page: Option<u16>,
    pub expected_usage: Option<u16>,
    pub status: String,
    pub status_text: String,
    pub notes: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInputControlKind {
    Gain,
    Mode,
    Phantom,
    Phase,
    Link,
    Parameter,
}

impl RuntimeInputControlKind {
    fn profile_name(self) -> &'static str {
        match self {
            Self::Gain => "gain",
            Self::Mode => "mode",
            Self::Phantom => "phantom",
            Self::Phase => "phase",
            Self::Link => "link",
            Self::Parameter => "parameter",
        }
    }

    fn accepts_parameter(self, parameter: &str) -> bool {
        match self {
            Self::Gain => matches!(parameter, "gain" | "adat_gain" | "spdif_gain"),
            Self::Mode => parameter == "input_mode",
            Self::Phantom => parameter == "phantom",
            Self::Phase => parameter == "phase_invert",
            Self::Link => matches!(
                parameter,
                "channel_link" | "adat_channel_link" | "spdif_channel_link"
            ),
            Self::Parameter => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInputCapability {
    pub kind: RuntimeInputControlKind,
    pub parameter: String,
    pub parameter_id: Option<u16>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAddressSpace {
    pub id: String,
    pub space_id: u16,
    pub name: String,
    pub kind: String,
    pub count: Option<u16>,
    pub addressing: String,
    pub status: String,
    pub status_text: String,
    pub notes: String,
    pub metadata: String,
    #[serde(default)]
    pub input_capabilities: Vec<RuntimeInputCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInput {
    pub id: String,
    pub space: String,
    pub space_id: u16,
    pub index: u16,
    pub name: String,
    pub hiz_capable: bool,
    pub status: String,
    pub metadata: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeOutput {
    pub id: u16,
    pub name: String,
    pub aliases: Vec<String>,
    pub verified: bool,
    pub status: String,
    pub metadata: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeMixer {
    pub id: String,
    pub name: String,
    pub mix_index: u8,
    pub strip_count: u16,
    pub has_master: bool,
    pub fader_range: Option<(i32, i32)>,
    #[serde(default)]
    pub fader: Option<FaderSemantics>,
    pub pan_range: Option<(i32, i32)>,
    pub pan_center: Option<i32>,
    pub send_range: Option<(i32, i32)>,
    pub status: String,
    pub status_text: String,
    pub notes: String,
    pub metadata: String,
}

impl RuntimeMixer {
    /// Convert a raw wire pan position to the profile's semantic value.
    pub fn pan_value_from_raw(&self, pan: PanState) -> Option<i32> {
        let (min, max) = self.pan_range?;
        let center = self.pan_center?;
        if min > max {
            return None;
        }
        let raw_min = center.checked_add(min)?;
        let raw_max = center.checked_add(max)?;
        let raw = i32::from(pan.raw());
        if raw_min < i32::from(PanState::MIN)
            || raw_max > i32::from(PanState::MAX)
            || raw_min > raw_max
            || !(raw_min..=raw_max).contains(&raw)
        {
            return None;
        }
        let value = raw.checked_sub(center)?;
        (min..=max).contains(&value).then_some(value)
    }

    /// Convert a semantic pan value to its raw wire position.
    pub fn pan_raw_from_value(&self, value: i32) -> Option<PanState> {
        let (min, max) = self.pan_range?;
        let center = self.pan_center?;
        if min > max || !((min..=max).contains(&value)) {
            return None;
        }
        let raw = center.checked_add(value)?;
        if !(i32::from(PanState::MIN)..=i32::from(PanState::MAX)).contains(&raw) {
            return None;
        }
        Some(PanState::from_raw(u8::try_from(raw).ok()?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLinkDomainKind {
    Mixer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeLinkDomain {
    pub protocol_space: u8,
    pub kind: RuntimeLinkDomainKind,
    pub pair_count: u16,
    pub status: String,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRoutingSourceDomain {
    pub bank: u8,
    pub index_count: u16,
    pub status: String,
    pub evidence: String,
}

/// Explicit source indices accepted for inbound routing readback only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRoutingReadbackSourceDomain {
    pub bank: u8,
    pub indices: Vec<u8>,
    pub status: String,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRoutingGroup {
    pub destination: u16,
    pub name: String,
    pub channel_count: u16,
    #[serde(default)]
    pub source_domains: Vec<RuntimeRoutingSourceDomain>,
    #[serde(default)]
    pub readback_source_domains: Vec<RuntimeRoutingReadbackSourceDomain>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFrame {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub report_size: Option<u16>,
    pub operations: Vec<FrameOperation>,
    pub metadata: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDecoder {
    pub id: String,
    pub frame_id: String,
    pub kind: String,
    pub status: String,
    pub metadata: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeParam {
    pub name: String,
    pub id: Option<u16>,
    pub value_type: String,
    pub status: String,
    pub applies_to: String,
    pub range: Option<(i32, i32)>,
    #[serde(default)]
    pub range_by_mode: Vec<(String, (i32, i32))>,
    #[serde(default)]
    pub direction: Option<FaderDirection>,
    #[serde(default)]
    pub unity: Option<i32>,
    pub values: Vec<(i32, String)>,
    pub frame: ParamReference,
    pub readback: ParamReference,
    pub metadata: String,
}

impl RuntimeParam {
    pub fn scalar_semantics(&self) -> Option<FaderSemantics> {
        let (min, max) = self.range?;
        Some(FaderSemantics {
            min,
            max,
            direction: self.direction?,
            unity: self.unity?,
        })
    }
}

#[cfg(test)]
mod runtime_param_tests {
    use super::{FaderDirection, FaderSemantics, ParamReference, RuntimeParam};

    #[test]
    fn runtime_param_scalar_semantics() {
        let parameter = RuntimeParam {
            name: "bus_level".into(),
            id: Some(0x47),
            value_type: "int".into(),
            status: "confirmed".into(),
            applies_to: "output".into(),
            range: Some((0, 96)),
            range_by_mode: Vec::new(),
            direction: Some(FaderDirection::Attenuation),
            unity: Some(0),
            values: Vec::new(),
            frame: ParamReference {
                text: String::new(),
                formula: String::new(),
                offsets: Vec::new(),
            },
            readback: ParamReference {
                text: String::new(),
                formula: String::new(),
                offsets: Vec::new(),
            },
            metadata: String::new(),
        };

        assert_eq!(
            parameter.scalar_semantics(),
            Some(FaderSemantics {
                min: 0,
                max: 96,
                direction: FaderDirection::Attenuation,
                unity: 0,
            })
        );
    }
}

#[cfg(test)]
mod runtime_mixer_tests {
    use super::RuntimeMixer;
    use crate::types::PanState;

    fn mixer() -> RuntimeMixer {
        RuntimeMixer {
            id: "mix_1".into(),
            name: "Mix 1".into(),
            mix_index: 0,
            strip_count: 16,
            has_master: false,
            fader_range: Some((0, 90)),
            fader: None,
            pan_range: Some((-30, 30)),
            pan_center: Some(32),
            send_range: None,
            status: "confirmed".into(),
            status_text: "confirmed".into(),
            notes: String::new(),
            metadata: String::new(),
        }
    }

    #[test]
    fn runtime_mixer_pan_conversion() {
        let mixer = mixer();

        assert_eq!(mixer.pan_value_from_raw(PanState::left()), Some(-30));
        assert_eq!(mixer.pan_value_from_raw(PanState::center()), Some(0));
        assert_eq!(mixer.pan_value_from_raw(PanState::right()), Some(30));
        assert_eq!(mixer.pan_raw_from_value(-30), Some(PanState::left()));
        assert_eq!(mixer.pan_raw_from_value(0), Some(PanState::center()));
        assert_eq!(mixer.pan_raw_from_value(30), Some(PanState::right()));
    }

    #[test]
    fn runtime_mixer_pan_conversion_rejects_out_of_domain_values() {
        let mut mixer = mixer();
        mixer.pan_range = Some((-29, 29));

        assert_eq!(mixer.pan_value_from_raw(PanState::left()), None);
        assert_eq!(mixer.pan_value_from_raw(PanState::right()), None);
        assert_eq!(mixer.pan_raw_from_value(-30), None);
        assert_eq!(mixer.pan_raw_from_value(30), None);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConstraint {
    pub name: String,
    pub status: String,
    pub range: Option<(i32, i32)>,
    pub values: Vec<i32>,
    pub scalar: Option<i32>,
    pub text: String,
    pub metadata: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHazard {
    pub name: String,
    pub status: String,
    pub rule: String,
    pub effect: String,
    pub notes: String,
    pub opcodes: Vec<u8>,
    pub metadata: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProvenance {
    pub source_path: String,
    pub source_sha256: String,
    pub generator_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamReference {
    pub text: String,
    pub formula: String,
    pub offsets: Vec<(String, u16)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameEndian {
    NotApplicable,
    Little,
    Big,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FrameOperation {
    FixedByte {
        offset: u16,
        value: u8,
    },
    Scalar {
        field: String,
        offset: u16,
        width: u8,
        endian: FrameEndian,
    },
    Indexed {
        base: u16,
        stride: u16,
        index_field: String,
        width: u8,
        max_index: Option<u16>,
    },
    BitField {
        field: String,
        offset: u16,
        mask: u8,
        shift: u8,
    },
    PairIndex {
        base: u16,
        stride: u16,
        pair_field: String,
        width: u8,
        max_index: Option<u16>,
    },
    AllowedValues {
        values: Vec<i32>,
    },
    UncompiledFormula {
        formula: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeReadiness {
    Supported,
    Partial,
    Unverified,
    Disabled,
}

impl RuntimeReadiness {
    pub const fn is_selectable(self) -> bool {
        matches!(self, Self::Supported)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDriverKind {
    ZenGo,
    Profile,
    None,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileLoadError {
    #[error("profile pack JSON is invalid: {source}")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot read profile pack {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("unsupported profile pack schema version {version}; expected {expected}")]
    UnsupportedSchemaVersion { version: u16, expected: u16 },
    #[error("profile id is empty at {field}")]
    EmptyProfileId { field: String },
    #[error("duplicate profile id {profile_id} at {field}")]
    DuplicateProfileId { profile_id: String, field: String },
    #[error("duplicate VID/PID {vid:#06x}:{pid:#06x} for {profile_id} at {field}")]
    DuplicateIdentity {
        profile_id: String,
        vid: u16,
        pid: u16,
        field: String,
    },
    #[error("profile {profile_id} has unsupported transport at {field}: {kind}")]
    UnsupportedTransport {
        profile_id: String,
        field: String,
        kind: String,
    },
    #[error("profile {profile_id} is missing required field {field}")]
    MissingRequiredField { profile_id: String, field: String },
    #[error("profile {profile_id} has invalid driver/readiness combination at {field}: {detail}")]
    InvalidDriverReadiness {
        profile_id: String,
        field: String,
        detail: String,
    },
    #[error("profile {profile_id} has no proven finite operation domain at {field}: {domain}")]
    MissingOperationDomain {
        profile_id: String,
        field: String,
        domain: String,
    },
    #[error("profile {profile_id} has invalid report geometry at {field}: {detail}")]
    InvalidReportGeometry {
        profile_id: String,
        field: String,
        detail: String,
    },
    #[error("profile {profile_id} has invalid frame operation at {field}: {detail}")]
    InvalidFrameOperation {
        profile_id: String,
        field: String,
        detail: String,
    },
    #[error("profile {profile_id} has duplicate parameter id {param_id} at {field}")]
    DuplicateParameterId {
        profile_id: String,
        param_id: u16,
        field: String,
    },
    #[error("profile {profile_id} has invalid enum values at {field}: {detail}")]
    InvalidEnumValues {
        profile_id: String,
        field: String,
        detail: String,
    },
    #[error("profile {profile_id} has unsafe readback bounds at {field}: {detail}")]
    InvalidReadbackBounds {
        profile_id: String,
        field: String,
        detail: String,
    },
    #[error("profile {profile_id} has unconfirmed command field {field}")]
    UnconfirmedCommand { profile_id: String, field: String },
    #[error("profile {profile_id} has uncompiled formula at {field}: {formula}")]
    UncompiledFormula {
        profile_id: String,
        field: String,
        formula: String,
    },
    #[error("profile {profile_id} has invalid provenance at {field}: {detail}")]
    InvalidProvenance {
        profile_id: String,
        field: String,
        detail: String,
    },
}

impl ProfilePack {
    pub fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn profiles(&self) -> &[RuntimeEntry] {
        &self.profiles
    }

    pub fn validate(pack: ProfilePack) -> Result<ProfilePack, ProfileLoadError> {
        if pack.schema_version != PROFILE_PACK_SCHEMA_VERSION {
            return Err(ProfileLoadError::UnsupportedSchemaVersion {
                version: pack.schema_version,
                expected: PROFILE_PACK_SCHEMA_VERSION,
            });
        }
        let mut ids = HashSet::new();
        let mut identities = HashSet::new();
        for (entry_index, entry) in pack.profiles.iter().enumerate() {
            let profile_id = entry.id.as_str();
            if profile_id.trim().is_empty() {
                return Err(ProfileLoadError::EmptyProfileId {
                    field: format!("profiles[{entry_index}].id"),
                });
            }
            if !ids.insert(profile_id) {
                return Err(ProfileLoadError::DuplicateProfileId {
                    profile_id: profile_id.to_owned(),
                    field: format!("profiles[{entry_index}].id"),
                });
            }
            let identity = (entry.profile.identity.vid, entry.profile.identity.pid);
            if !identities.insert(identity) {
                return Err(ProfileLoadError::DuplicateIdentity {
                    profile_id: profile_id.to_owned(),
                    vid: identity.0,
                    pid: identity.1,
                    field: format!("profiles[{entry_index}].identity"),
                });
            }
            validate_entry(entry, entry_index)?;
        }
        Ok(pack)
    }
}

impl RuntimeEntry {
    pub fn profile(&self) -> &RuntimeProfile {
        &self.profile
    }
}

impl ReadbackDefinition {
    pub fn allows(&self, query: QueryRequest) -> bool {
        self.safe_queries
            .iter()
            .any(|safe| safe.category == query.query_id && safe.index == query.sub_id)
            || self.category_counts.iter().any(|category| {
                category.category == query.query_id && u16::from(query.sub_id) < category.count
            })
    }

    pub fn layout_for(&self, query: QueryRequest) -> Option<&MixerReadbackLayout> {
        self.layouts
            .iter()
            .find(|layout| layout.category == query.query_id && layout.index == query.sub_id)
    }
}

impl RuntimeProfile {
    pub fn identity(&self) -> &RuntimeIdentity {
        &self.identity
    }

    pub fn inputs_in(&self, space: &str) -> usize {
        self.inputs
            .iter()
            .filter(|input| input.space == space)
            .count()
    }

    pub fn outputs(&self) -> &[RuntimeOutput] {
        &self.outputs
    }

    pub fn mixers(&self) -> &[RuntimeMixer] {
        &self.mixers
    }

    pub fn candidate_preamp_meters(&self) -> &[CandidatePreampMeter] {
        self.state_report.as_ref().map_or(&[], |state_report| {
            state_report.candidate_preamp_meters.as_slice()
        })
    }

    pub fn candidate_preamp_meter(&self, input_index: u16) -> Option<usize> {
        self.candidate_preamp_meters()
            .iter()
            .find(|meter| meter.input_index == input_index)
            .map(|meter| meter.offset)
    }

    pub fn mixer(&self, surface: u8) -> Option<&RuntimeMixer> {
        self.mixers.iter().find(|mixer| mixer.mix_index == surface)
    }

    pub fn mixer_fader(&self, surface: u8) -> Option<FaderSemantics> {
        self.mixer(surface).and_then(|mixer| mixer.fader)
    }
}

pub fn load_profile_pack(bytes: &[u8]) -> Result<ProfilePack, ProfileLoadError> {
    #[derive(Deserialize)]
    struct SchemaHeader {
        schema_version: u16,
    }
    let header: SchemaHeader =
        serde_json::from_slice(bytes).map_err(|source| ProfileLoadError::InvalidJson { source })?;
    if header.schema_version != PROFILE_PACK_SCHEMA_VERSION {
        return Err(ProfileLoadError::UnsupportedSchemaVersion {
            version: header.schema_version,
            expected: PROFILE_PACK_SCHEMA_VERSION,
        });
    }
    let pack =
        serde_json::from_slice(bytes).map_err(|source| ProfileLoadError::InvalidJson { source })?;
    ProfilePack::validate(pack)
}

pub fn load_profile_pack_file(path: &Path) -> Result<ProfilePack, ProfileLoadError> {
    let bytes = std::fs::read(path).map_err(|source| ProfileLoadError::Io {
        path: path.display().to_string(),
        source,
    })?;
    load_profile_pack(&bytes)
}

fn validate_entry(entry: &RuntimeEntry, entry_index: usize) -> Result<(), ProfileLoadError> {
    let profile_id = entry.id.as_str();
    let profile = &entry.profile;
    if !profile.transport.kind.eq_ignore_ascii_case("hid") {
        return Err(ProfileLoadError::UnsupportedTransport {
            profile_id: profile_id.to_owned(),
            field: format!("profiles[{entry_index}].transport.kind"),
            kind: profile.transport.kind.clone(),
        });
    }
    if profile.provenance.source_path.trim().is_empty()
        || profile.provenance.generator_version.trim().is_empty()
        || profile.provenance.source_sha256.len() != 64
        || !profile
            .provenance
            .source_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ProfileLoadError::InvalidProvenance {
            profile_id: profile_id.to_owned(),
            field: format!("profiles[{entry_index}].provenance"),
            detail: "source path, generator version, and 64-digit lowercase SHA-256 are required"
                .into(),
        });
    }

    if entry.readiness.is_selectable() {
        let valid_driver = match entry.driver_kind {
            RuntimeDriverKind::ZenGo => {
                profile.identity.vid == 0x23e5 && profile.identity.pid == 0xa015
            }
            RuntimeDriverKind::Profile => {
                !(profile.identity.vid == 0x23e5 && profile.identity.pid == 0xa015)
            }
            RuntimeDriverKind::None => false,
        };
        if !valid_driver {
            return Err(ProfileLoadError::InvalidDriverReadiness {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].driver_kind"),
                detail: format!(
                    "supported identity {:#06x}:{:#06x} cannot use {:?}",
                    profile.identity.vid, profile.identity.pid, entry.driver_kind
                ),
            });
        }
    }
    let selectable_profile =
        entry.readiness.is_selectable() && matches!(entry.driver_kind, RuntimeDriverKind::Profile);
    if selectable_profile && profile.transport.report_size.is_none() {
        return Err(ProfileLoadError::MissingRequiredField {
            profile_id: profile_id.to_owned(),
            field: format!("profiles[{entry_index}].transport.report_size"),
        });
    }

    let spaces: HashMap<&str, u16> = profile
        .address_spaces
        .iter()
        .map(|space| (space.id.as_str(), space.space_id))
        .collect();
    let params_by_name: HashMap<&str, Option<u16>> = profile
        .params
        .iter()
        .map(|param| (param.name.as_str(), param.id))
        .collect();
    for (space_index, space) in profile.address_spaces.iter().enumerate() {
        let mut kinds = HashSet::new();
        let mut parameter_ids = HashSet::new();
        for (capability_index, capability) in space.input_capabilities.iter().enumerate() {
            let field = format!(
                "profiles[{entry_index}].address_spaces[{space_index}].input_capabilities[{capability_index}]"
            );
            let capability_context = format!(
                "address space {:?}, kind {:?}, parameter key {:?}",
                space.id,
                capability.kind.profile_name(),
                capability.parameter
            );
            if capability.label.trim().is_empty() || !kinds.insert(capability.kind) {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field,
                    detail: format!(
                        "{capability_context} requires a non-blank label and unique kind per space"
                    ),
                });
            }
            let Some(actual_parameter_id) = params_by_name.get(capability.parameter.as_str())
            else {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field,
                    detail: format!("{capability_context} references an unknown parameter"),
                });
            };
            if !capability.kind.accepts_parameter(&capability.parameter) {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field,
                    detail: format!("{capability_context} is not a legal input capability"),
                });
            }
            if capability.parameter_id != *actual_parameter_id {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field,
                    detail: format!(
                        "{capability_context} has parameter id {:?}, expected {:?}",
                        capability.parameter_id, actual_parameter_id
                    ),
                });
            }
            if capability
                .parameter_id
                .is_some_and(|id| !parameter_ids.insert(id))
            {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field,
                    detail: format!("{capability_context} has a duplicate parameter id"),
                });
            }
        }
    }
    for (input_index, input) in profile.inputs.iter().enumerate() {
        if spaces.get(input.space.as_str()) != Some(&input.space_id) {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].inputs[{input_index}].space_id"),
                detail: "input space id does not match address space".into(),
            });
        }
    }

    let mixer_ids: HashSet<u8> = profile.mixers.iter().map(|mixer| mixer.mix_index).collect();
    if mixer_ids.len() != profile.mixers.len() {
        return Err(ProfileLoadError::InvalidReportGeometry {
            profile_id: profile_id.to_owned(),
            field: format!("profiles[{entry_index}].mixers"),
            detail: "mixer surface addresses must be unique".into(),
        });
    }
    let mut link_spaces = HashSet::new();
    for (domain_index, domain) in profile.link_domains.iter().enumerate() {
        if !link_spaces.insert(domain.protocol_space)
            || domain.pair_count == 0
            || domain.pair_count > 256
            || !is_confirmed(&domain.status)
            || domain.evidence.trim().is_empty()
        {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].link_domains[{domain_index}]"),
                detail: "link domains require unique spaces, confirmed evidence, and finite pair counts within 1..=256".into(),
            });
        }
    }
    if selectable_profile && profile.link_domains.is_empty() {
        return Err(ProfileLoadError::MissingRequiredField {
            profile_id: profile_id.to_owned(),
            field: format!("profiles[{entry_index}].link_domains"),
        });
    }

    let mut routing_destinations = HashSet::new();
    for (group_index, group) in profile.routing_groups.iter().enumerate() {
        if group.name.trim().is_empty()
            || group.channel_count == 0
            || !routing_destinations.insert(group.destination)
        {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].routing_groups[{group_index}]"),
                detail:
                    "routing groups require unique destinations, names, and positive channel counts"
                        .into(),
            });
        }
        let mut banks = HashSet::new();
        for (domain_index, domain) in group.source_domains.iter().enumerate() {
            if !banks.insert(domain.bank)
                || domain.index_count == 0
                || domain.index_count > 256
                || !is_confirmed(&domain.status)
                || domain.evidence.trim().is_empty()
            {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field: format!("profiles[{entry_index}].routing_groups[{group_index}].source_domains[{domain_index}]"),
                    detail: "routing source domains require unique banks, confirmed evidence, and finite index counts within 1..=256".into(),
                });
            }
        }
        let mut readback_banks = HashSet::new();
        for (domain_index, domain) in group.readback_source_domains.iter().enumerate() {
            if !readback_banks.insert(domain.bank)
                || domain.bank == 0x0c
                || banks.contains(&domain.bank)
                || domain.indices.is_empty()
                || domain.indices.len() > 256
                || domain.indices.windows(2).any(|pair| pair[0] >= pair[1])
                || !is_observed(&domain.status)
                || domain.evidence.trim().is_empty()
            {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field: format!("profiles[{entry_index}].routing_groups[{group_index}].readback_source_domains[{domain_index}]"),
                    detail: "routing readback source domains require unique write-distinct banks, sorted finite indices within 1..=256, observed evidence, and non-empty provenance".into(),
                });
            }
        }
        if selectable_profile && group.source_domains.is_empty() {
            return Err(ProfileLoadError::MissingRequiredField {
                profile_id: profile_id.to_owned(),
                field: format!(
                    "profiles[{entry_index}].routing_groups[{group_index}].source_domains"
                ),
            });
        }
    }
    if entry.readiness.is_selectable()
        && (profile.mixers.is_empty() || profile.routing_groups.is_empty())
    {
        return Err(ProfileLoadError::MissingRequiredField {
            profile_id: profile_id.to_owned(),
            field: format!("profiles[{entry_index}].runtime_topology"),
        });
    }

    for (mixer_index, mixer) in profile.mixers.iter().enumerate() {
        if let Some(fader) = mixer.fader {
            if fader.min > fader.max || !(fader.min..=fader.max).contains(&fader.unity) {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field: format!("profiles[{entry_index}].mixers[{mixer_index}].fader"),
                    detail: "fader min must not exceed max and unity must fit the domain".into(),
                });
            }
        }
    }
    let mut meter_mapping_keys = HashSet::new();
    for (mapping_index, mapping) in profile.meter_mappings.iter().enumerate() {
        let field = format!("profiles[{entry_index}].meter_mappings[{mapping_index}]");
        let Some(frame) = profile
            .frames
            .iter()
            .find(|frame| frame.id == mapping.frame_id)
        else {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field,
                detail: format!("meter frame {} is not declared", mapping.frame_id),
            });
        };
        if !matches!(mapping.frame_id.as_str(), "state_report" | "meter_report")
            || !matches!(frame.kind.as_str(), "state_report" | "meter_report")
        {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field,
                detail: "meter mappings must target state_report or meter_report".into(),
            });
        }
        if mapping.status.trim().is_empty() || mapping.evidence.trim().is_empty() {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field,
                detail: "meter mappings require status and evidence".into(),
            });
        }
        if profile
            .transport
            .report_size
            .is_some_and(|size| mapping.offset >= usize::from(size))
        {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field,
                detail: format!("meter offset {} exceeds report size", mapping.offset),
            });
        }
        let target_exists = match mapping.target {
            RuntimeMeterTarget::MixMaster => profile
                .mixers
                .iter()
                .any(|mixer| u16::from(mixer.mix_index) == mapping.target_index),
            RuntimeMeterTarget::PhysicalOutput => profile
                .outputs
                .iter()
                .any(|output| output.id == mapping.target_index),
        };
        if !target_exists {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field,
                detail: format!(
                    "meter target index {} is not in profile topology",
                    mapping.target_index
                ),
            });
        }
        let key = (&mapping.target, mapping.target_index, mapping.lane);
        if !meter_mapping_keys.insert(key) {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field,
                detail: "meter target lane is declared more than once across frames".into(),
            });
        }
    }
    if let Some(state_report) = profile.state_report.as_ref() {
        if !state_report.candidate_preamp_meters.is_empty()
            && profile.transport.report_size.is_none()
        {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].state_report.candidate_preamp_meters"),
                detail: "candidate meters require a finite transport.report_size".into(),
            });
        }
        let mut input_indices = HashSet::new();
        for (meter_index, meter) in state_report.candidate_preamp_meters.iter().enumerate() {
            let field = format!(
                "profiles[{entry_index}].state_report.candidate_preamp_meters[{meter_index}]"
            );
            if !input_indices.insert(meter.input_index) {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field: format!("{field}.input_index"),
                    detail: "candidate input index is declared more than once".into(),
                });
            }
            if !profile
                .inputs
                .iter()
                .any(|input| input.space == "physical_inputs" && input.index == meter.input_index)
            {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field: format!("{field}.input_index"),
                    detail: "candidate meter input is not a declared physical input".into(),
                });
            }
            if let Some(report_size) = profile.transport.report_size {
                let full_offset = SNAPSHOT_PAYLOAD_OFFSET
                    .checked_add(meter.offset)
                    .ok_or_else(|| ProfileLoadError::InvalidReportGeometry {
                        profile_id: profile_id.to_owned(),
                        field: format!("{field}.offset"),
                        detail: "payload offset overflows report geometry".into(),
                    })?;
                if full_offset >= usize::from(report_size) {
                    return Err(ProfileLoadError::InvalidReportGeometry {
                        profile_id: profile_id.to_owned(),
                        field: format!("{field}.offset"),
                        detail: format!(
                            "payload offset {} (full report {}) exceeds state report size {report_size}",
                            meter.offset, full_offset
                        ),
                    });
                }
            }
            if meter.status.trim().is_empty()
                || meter.confidence.trim().is_empty()
                || meter.caveat.trim().is_empty()
            {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field,
                    detail: "candidate meters require status, confidence, and caveat".into(),
                });
            }
            if meter.raw_value_ranges.is_empty()
                || meter
                    .raw_value_ranges
                    .iter()
                    .any(|(minimum, maximum)| minimum > maximum)
                || meter
                    .raw_value_ranges
                    .windows(2)
                    .any(|ranges| ranges[0].1 >= ranges[1].0)
            {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field: format!("{field}.raw_value_ranges"),
                    detail:
                        "candidate raw value ranges must be non-empty, ordered, and non-overlapping"
                            .into(),
                });
            }
        }
    }

    let mut parameter_ids = HashSet::new();
    for (param_index, param) in profile.params.iter().enumerate() {
        if let Some(param_id) = param.id {
            if !parameter_ids.insert(param_id) {
                return Err(ProfileLoadError::DuplicateParameterId {
                    profile_id: profile_id.to_owned(),
                    param_id,
                    field: format!("profiles[{entry_index}].params[{param_index}].id"),
                });
            }
        }
        if selectable_profile && param.id.is_some() {
            let field = format!("profiles[{entry_index}].params[{param_index}]");
            let numeric_type = !param.value_type.eq_ignore_ascii_case("bool")
                && !param.value_type.eq_ignore_ascii_case("enum");
            if !is_confirmed(&param.status)
                || param.applies_to.trim().is_empty()
                || param.frame.text.trim().is_empty()
                || param.frame.offsets.is_empty()
                || !param.frame.formula.trim().is_empty()
                || param.readback.text.trim().is_empty()
                || param.readback.offsets.is_empty()
                || !param.readback.formula.trim().is_empty()
                || (numeric_type && param.range.is_none())
            {
                return Err(ProfileLoadError::UnconfirmedCommand {
                    profile_id: profile_id.to_owned(),
                    field,
                });
            }
        }
        if param.value_type.eq_ignore_ascii_case("enum") {
            let unique: HashSet<i32> = param.values.iter().map(|(value, _)| *value).collect();
            if unique.len() != param.values.len() || (selectable_profile && param.values.is_empty())
            {
                return Err(ProfileLoadError::InvalidEnumValues {
                    profile_id: profile_id.to_owned(),
                    field: format!("profiles[{entry_index}].params[{param_index}].values"),
                    detail: "selectable enum values must be non-empty and all enum values must be unique".into(),
                });
            }
        }
        if let Some(report_size) = profile.transport.report_size {
            for (reference_name, reference) in
                [("frame", &param.frame), ("readback", &param.readback)]
            {
                for (offset_name, offset) in &reference.offsets {
                    if *offset >= report_size {
                        return Err(ProfileLoadError::InvalidReportGeometry {
                            profile_id: profile_id.to_owned(),
                            field: format!(
                                "profiles[{entry_index}].params[{param_index}].{reference_name}.offsets.{offset_name}"
                            ),
                            detail: format!("offset {offset} exceeds report size {report_size}"),
                        });
                    }
                }
            }
        }
    }

    for (frame_index, frame) in profile.frames.iter().enumerate() {
        let report_size = frame.report_size.or(profile.transport.report_size);
        if selectable_profile && report_size.is_none() {
            return Err(ProfileLoadError::MissingRequiredField {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].frames[{frame_index}].report_size"),
            });
        }
        if selectable_profile
            && frame.kind.eq_ignore_ascii_case("command")
            && !is_confirmed(&frame.status)
        {
            return Err(ProfileLoadError::UnconfirmedCommand {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].frames[{frame_index}].status"),
            });
        }
        for (operation_index, operation) in frame.operations.iter().enumerate() {
            let field = format!(
                "profiles[{entry_index}].frames[{frame_index}].operations[{operation_index}]"
            );
            validate_operation(profile_id, &field, operation, report_size)?;
            if selectable_profile && frame.kind.eq_ignore_ascii_case("command") {
                if let FrameOperation::UncompiledFormula { formula } = operation {
                    return Err(ProfileLoadError::UncompiledFormula {
                        profile_id: profile_id.to_owned(),
                        field,
                        formula: formula.clone(),
                    });
                }
            }
        }
    }

    if selectable_profile {
        let readback =
            profile
                .readback
                .as_ref()
                .ok_or_else(|| ProfileLoadError::MissingRequiredField {
                    profile_id: profile_id.to_owned(),
                    field: format!("profiles[{entry_index}].readback"),
                })?;
        if profile.startup_queries.is_empty() {
            return Err(ProfileLoadError::MissingRequiredField {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].startup_queries"),
            });
        }
        validate_readback(profile_id, entry_index, profile, readback)?;
    } else if let Some(readback) = &profile.readback {
        validate_readback(profile_id, entry_index, profile, readback)?;
    }
    Ok(())
}

fn validate_operation(
    profile_id: &str,
    field: &str,
    operation: &FrameOperation,
    report_size: Option<u16>,
) -> Result<(), ProfileLoadError> {
    let span = |offset: u16, width: u16| -> Result<(), ProfileLoadError> {
        let end = u32::from(offset) + u32::from(width);
        if width == 0 || report_size.is_some_and(|size| end > u32::from(size)) {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field: field.to_owned(),
                detail: format!("offset {offset} width {width} exceeds report {report_size:?}"),
            });
        }
        Ok(())
    };
    match operation {
        FrameOperation::FixedByte { offset, .. } => span(*offset, 1),
        FrameOperation::Scalar {
            field,
            offset,
            width,
            endian,
        } => {
            if field.trim().is_empty()
                || (*width == 1 && *endian != FrameEndian::NotApplicable)
                || (*width > 1 && *endian == FrameEndian::NotApplicable)
            {
                return Err(ProfileLoadError::InvalidFrameOperation {
                    profile_id: profile_id.to_owned(),
                    field: field.to_owned(),
                    detail:
                        "scalar requires semantic field and width-appropriate declared endianness"
                            .into(),
                });
            }
            span(*offset, u16::from(*width))
        }
        FrameOperation::Indexed {
            base,
            stride,
            width,
            index_field,
            max_index,
        } => {
            if *stride == 0 || *width == 0 || index_field.trim().is_empty() {
                return Err(ProfileLoadError::InvalidFrameOperation {
                    profile_id: profile_id.to_owned(),
                    field: field.to_owned(),
                    detail: "indexed operation requires positive stride/width and index field"
                        .into(),
                });
            }
            let max_index = max_index.ok_or_else(|| ProfileLoadError::MissingOperationDomain {
                profile_id: profile_id.to_owned(),
                field: field.to_owned(),
                domain: index_field.clone(),
            })?;
            let end = u32::from(*base)
                .checked_add(u32::from(*stride) * u32::from(max_index))
                .and_then(|offset| offset.checked_add(u32::from(*width)))
                .ok_or_else(|| ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field: field.to_owned(),
                    detail: "indexed reachable span overflows".into(),
                })?;
            if report_size.is_none_or(|size| end > u32::from(size)) {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field: field.to_owned(),
                    detail: format!("indexed reachable end {end} exceeds report {report_size:?}"),
                });
            }
            Ok(())
        }
        FrameOperation::BitField {
            field: semantic_field,
            offset,
            mask,
            shift,
        } => {
            if semantic_field.trim().is_empty()
                || *mask == 0
                || *shift >= 8
                || (*mask >> *shift) == 0
            {
                return Err(ProfileLoadError::InvalidFrameOperation {
                    profile_id: profile_id.to_owned(),
                    field: field.to_owned(),
                    detail: format!("invalid mask {mask:#04x} or shift {shift}"),
                });
            }
            span(*offset, 1)
        }
        FrameOperation::PairIndex {
            base,
            stride,
            pair_field,
            width,
            max_index,
        } => {
            if *stride == 0 || *width == 0 || pair_field.trim().is_empty() {
                return Err(ProfileLoadError::InvalidFrameOperation {
                    profile_id: profile_id.to_owned(),
                    field: field.to_owned(),
                    detail: "pair operation requires positive stride/width and pair field".into(),
                });
            }
            let max_index = max_index.ok_or_else(|| ProfileLoadError::MissingOperationDomain {
                profile_id: profile_id.to_owned(),
                field: field.to_owned(),
                domain: pair_field.clone(),
            })?;
            let end = u32::from(*base)
                .checked_add(u32::from(*stride) * u32::from(max_index))
                .and_then(|offset| offset.checked_add(u32::from(*width)))
                .ok_or_else(|| ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field: field.to_owned(),
                    detail: "pair reachable span overflows".into(),
                })?;
            if report_size.is_none_or(|size| end > u32::from(size)) {
                return Err(ProfileLoadError::InvalidReportGeometry {
                    profile_id: profile_id.to_owned(),
                    field: field.to_owned(),
                    detail: format!("pair reachable end {end} exceeds report {report_size:?}"),
                });
            }
            Ok(())
        }
        FrameOperation::AllowedValues { values } => {
            let unique: HashSet<i32> = values.iter().copied().collect();
            if values.is_empty() || unique.len() != values.len() {
                return Err(ProfileLoadError::InvalidFrameOperation {
                    profile_id: profile_id.to_owned(),
                    field: field.to_owned(),
                    detail: "allowed values must be non-empty and unique".into(),
                });
            }
            Ok(())
        }
        FrameOperation::UncompiledFormula { formula } => {
            if formula.trim().is_empty() {
                return Err(ProfileLoadError::InvalidFrameOperation {
                    profile_id: profile_id.to_owned(),
                    field: field.to_owned(),
                    detail: "uncompiled formula text must not be empty".into(),
                });
            }
            Ok(())
        }
    }
}

fn validate_readback(
    profile_id: &str,
    entry_index: usize,
    profile: &RuntimeProfile,
    readback: &ReadbackDefinition,
) -> Result<(), ProfileLoadError> {
    let report_size =
        profile
            .transport
            .report_size
            .ok_or_else(|| ProfileLoadError::MissingRequiredField {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].transport.report_size"),
            })?;
    for (name, offset) in [
        (
            "response_discriminator_offset",
            readback.response_discriminator_offset,
        ),
        ("category_offset", readback.category_offset),
        ("index_offset", readback.index_offset),
        ("data_offset", readback.data_offset),
    ] {
        if offset >= report_size {
            return Err(ProfileLoadError::InvalidReportGeometry {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].readback.{name}"),
                detail: format!("offset {offset} exceeds report size {report_size}"),
            });
        }
    }
    let mut counts = HashMap::new();
    for category in &readback.category_counts {
        if category.count == 0 || counts.insert(category.category, category.count).is_some() {
            return Err(ProfileLoadError::InvalidReadbackBounds {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].readback.category_counts"),
                detail: "category counts must be positive and unique".into(),
            });
        }
    }
    // Safe queries also preserve ordered startup walks, so repeated pairs are
    // valid. Membership set below is used only to validate layout references.
    let safe_pairs: HashSet<(u8, u8)> = readback
        .safe_queries
        .iter()
        .map(|safe| (safe.category, safe.index))
        .collect();
    let mut layouts = HashSet::new();
    for (layout_index, layout) in readback.layouts.iter().enumerate() {
        let field = format!("profiles[{entry_index}].readback.layouts[{layout_index}]");
        if !safe_pairs.contains(&(layout.category, layout.index)) {
            return Err(ProfileLoadError::InvalidReadbackBounds {
                profile_id: profile_id.to_owned(),
                field,
                detail: "layout query is not present in safe_queries".into(),
            });
        }
        if !layouts.insert((layout.category, layout.index))
            || layout.body_size == 0
            || layout.record_count == 0
            || layout.record_stride == 0
            || layout
                .record_count
                .checked_mul(layout.record_stride)
                .is_none_or(|span| span > layout.body_size)
        {
            return Err(ProfileLoadError::InvalidReadbackBounds {
                profile_id: profile_id.to_owned(),
                field,
                detail: "layout query must be unique and records must fit body_size".into(),
            });
        }
        for (name, offset) in [
            ("level_offset", layout.level_offset),
            ("state_offset", layout.state_offset),
        ] {
            let Some(last) = (layout.record_count - 1)
                .checked_mul(layout.record_stride)
                .and_then(|base| base.checked_add(offset))
                .and_then(|end| end.checked_add(1))
            else {
                return Err(ProfileLoadError::InvalidReadbackBounds {
                    profile_id: profile_id.to_owned(),
                    field: format!("{field}.{name}"),
                    detail: "layout offset span overflows".into(),
                });
            };
            if last > layout.body_size {
                return Err(ProfileLoadError::InvalidReadbackBounds {
                    profile_id: profile_id.to_owned(),
                    field: format!("{field}.{name}"),
                    detail: format!(
                        "last record ends at {last}, beyond body_size {}",
                        layout.body_size
                    ),
                });
            }
        }
        if let Some(surface_stride) = layout.surface_stride {
            if surface_stride == 0
                || surface_stride > layout.record_count
                || surface_stride
                    .checked_mul(layout.record_stride)
                    .is_none_or(|span| span > layout.body_size)
            {
                return Err(ProfileLoadError::InvalidReadbackBounds {
                    profile_id: profile_id.to_owned(),
                    field: format!("{field}.surface_stride"),
                    detail: "surface stride exceeds layout body_size".into(),
                });
            }
        }
    }
    for (query_index, query) in profile.startup_queries.iter().enumerate() {
        if !readback.allows(*query) {
            return Err(ProfileLoadError::InvalidReadbackBounds {
                profile_id: profile_id.to_owned(),
                field: format!("profiles[{entry_index}].startup_queries[{query_index}]"),
                detail: format!(
                    "query {:#04x}:{} is not in readback safety data",
                    query.query_id, query.sub_id
                ),
            });
        }
    }
    Ok(())
}

fn is_confirmed(status: &str) -> bool {
    status.trim().to_ascii_lowercase().starts_with("confirm")
}

fn is_observed(status: &str) -> bool {
    status.trim().to_ascii_lowercase().starts_with("observ")
}

mod query_requests {
    use super::*;

    #[derive(Serialize, Deserialize)]
    struct QueryRecord {
        query_id: u8,
        sub_id: u8,
    }

    pub fn serialize<S>(queries: &[QueryRequest], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        queries
            .iter()
            .map(|query| QueryRecord {
                query_id: query.query_id,
                sub_id: query.sub_id,
            })
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<QueryRequest>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Vec::<QueryRecord>::deserialize(deserializer)?
            .into_iter()
            .map(|query| QueryRequest::new(query.query_id, query.sub_id))
            .collect())
    }
}
