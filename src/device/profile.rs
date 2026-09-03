//! Owned runtime catalog assembled from built-ins and normalized profile packs.

use super::{
    AddressSpaceKind, AddressingMode, DeviceEntry, FrameEndianDefinition, FrameKind,
    FrameOperationDefinition, InputControlKind, LinkDomainKind, ParamValueType, Readiness, Status,
    TransportKind, DEVICE_CATALOG,
};
use antelope_protocol::{
    CandidatePreampMeter, FaderDirection, FaderSemantics, FrameEndian, FrameOperation,
    MixerReadbackLayout, ParamReference, ProfileLoadError, ProfilePack, QueryRequest,
    ReadbackCategory, ReadbackDefinition as RuntimeReadbackDefinition, RuntimeAddressSpace,
    RuntimeConstraint, RuntimeDecoder, RuntimeDriverKind, RuntimeEntry, RuntimeFrame,
    RuntimeHazard, RuntimeIdentity, RuntimeInput, RuntimeInputCapability, RuntimeInputControlKind,
    RuntimeLinkDomain, RuntimeLinkDomainKind, RuntimeMixer, RuntimeOutput, RuntimeParam,
    RuntimeProfile, RuntimeProvenance, RuntimeReadiness, RuntimeRoutingGroup,
    RuntimeRoutingSourceDomain, RuntimeStateReport, RuntimeTransport,
};
use std::collections::HashSet;

/// Deterministically ordered owned runtime device profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCatalog {
    entries: Vec<RuntimeEntry>,
}

impl ProfileCatalog {
    /// Convert checked-in static definitions without filesystem access.
    pub fn builtin() -> Self {
        let mut catalog = Self {
            entries: DEVICE_CATALOG.iter().map(convert_entry).collect(),
        };
        catalog.sort();
        catalog
    }

    /// Merge a validated external pack without shadowing existing identities.
    pub fn add_external(&mut self, pack: ProfilePack) -> Result<(), ProfileLoadError> {
        let pack = ProfilePack::validate(pack)?;
        let mut ids: HashSet<&str> = self.entries.iter().map(|entry| entry.id.as_str()).collect();
        let mut identities: HashSet<(u16, u16)> = self
            .entries
            .iter()
            .map(|entry| (entry.profile.identity.vid, entry.profile.identity.pid))
            .collect();
        for entry in pack.profiles() {
            if !ids.insert(entry.id.as_str()) {
                return Err(ProfileLoadError::DuplicateProfileId {
                    profile_id: entry.id.clone(),
                    field: "external.profiles.id".into(),
                });
            }
            let identity = (entry.profile.identity.vid, entry.profile.identity.pid);
            if !identities.insert(identity) {
                return Err(ProfileLoadError::DuplicateIdentity {
                    profile_id: entry.id.clone(),
                    vid: identity.0,
                    pid: identity.1,
                    field: "external.profiles.identity".into(),
                });
            }
        }
        self.entries.extend(pack.profiles);
        self.sort();
        Ok(())
    }

    pub fn entries(&self) -> &[RuntimeEntry] {
        &self.entries
    }

    pub fn find(&self, vid: u16, pid: u16) -> Option<&RuntimeEntry> {
        self.entries
            .iter()
            .find(|entry| entry.profile.identity.vid == vid && entry.profile.identity.pid == pid)
    }

    fn sort(&mut self) {
        self.entries.sort_by(|left, right| {
            readiness_rank(left.readiness)
                .cmp(&readiness_rank(right.readiness))
                .then_with(|| left.profile.identity.name.cmp(&right.profile.identity.name))
                .then_with(|| left.id.cmp(&right.id))
        });
    }
}

pub fn catalog_readiness(readiness: RuntimeReadiness) -> Readiness {
    match readiness {
        RuntimeReadiness::Supported => Readiness::Supported,
        RuntimeReadiness::Partial => Readiness::Partial,
        RuntimeReadiness::Unverified => Readiness::Unverified,
        RuntimeReadiness::Disabled => Readiness::Disabled,
    }
}

fn readiness_rank(readiness: RuntimeReadiness) -> u8 {
    match readiness {
        RuntimeReadiness::Supported => 0,
        RuntimeReadiness::Partial => 1,
        RuntimeReadiness::Unverified => 2,
        RuntimeReadiness::Disabled => 3,
    }
}

fn convert_entry(entry: &DeviceEntry) -> RuntimeEntry {
    let definition = &entry.definition;
    let readiness = convert_readiness(entry.readiness);
    let id = definition
        .provenance
        .source_path
        .rsplit('/')
        .next()
        .unwrap_or(definition.identity.name)
        .strip_suffix(".json")
        .unwrap_or(definition.identity.name)
        .to_owned();
    let spaces: Vec<RuntimeAddressSpace> = definition
        .address_spaces
        .iter()
        .enumerate()
        .map(|(space_id, space)| RuntimeAddressSpace {
            id: space.id.into(),
            space_id: u16::try_from(space_id).expect("generated address-space count fits u16"),
            name: space.name.into(),
            kind: address_space_kind(space.kind).into(),
            count: space.count,
            addressing: addressing(space.addressing).into(),
            status: status(space.status).into(),
            status_text: space.status_text.into(),
            notes: space.notes.into(),
            metadata: space.metadata.into(),
            input_capabilities: space
                .input_capabilities
                .iter()
                .map(|capability| RuntimeInputCapability {
                    kind: match capability.kind {
                        InputControlKind::Gain => RuntimeInputControlKind::Gain,
                        InputControlKind::Mode => RuntimeInputControlKind::Mode,
                        InputControlKind::Phantom => RuntimeInputControlKind::Phantom,
                        InputControlKind::Phase => RuntimeInputControlKind::Phase,
                        InputControlKind::Link => RuntimeInputControlKind::Link,
                        InputControlKind::Parameter => RuntimeInputControlKind::Parameter,
                    },
                    parameter: capability.parameter.into(),
                    parameter_id: capability.parameter_id,
                    label: capability.label.into(),
                })
                .collect(),
        })
        .collect();
    let runtime_inputs = definition
        .inputs
        .iter()
        .map(|input| RuntimeInput {
            id: input.id.into(),
            space: input.space.into(),
            space_id: spaces
                .iter()
                .find(|space| space.id == input.space)
                .map(|space| space.space_id)
                .expect("generated input references generated address space"),
            index: input.index,
            name: input.name.into(),
            hiz_capable: input.hiz_capable,
            status: status(input.status).into(),
            metadata: input.metadata.into(),
        })
        .collect();
    let is_orion = id == "orion_studio_3";
    let mut hazards: Vec<RuntimeHazard> = definition
        .hazards
        .iter()
        .map(|hazard| RuntimeHazard {
            name: hazard.name.into(),
            status: status(hazard.status).into(),
            rule: hazard.rule.into(),
            effect: hazard.effect.into(),
            notes: hazard.notes.into(),
            opcodes: hazard.opcodes.to_vec(),
            metadata: hazard.metadata.into(),
        })
        .collect();
    if is_orion {
        hazards.push(RuntimeHazard {
            name: "orion_framing_assumption".into(),
            status: "unknown".into(),
            rule: "uses_numbered_reports=false".into(),
            effect: "hardware verification pending".into(),
            notes: "Source-backed runtime assumption; verify Orion HID report framing on hardware."
                .into(),
            opcodes: Vec::new(),
            metadata: "{\"raw\":\"transport.uses_numbered_reports is absent\",\"verification\":\"pending\"}".into(),
        });
    }
    RuntimeEntry {
        id,
        profile: RuntimeProfile {
            identity: RuntimeIdentity {
                name: definition.identity.name.into(),
                vid: definition.identity.vid,
                pid: definition.identity.pid,
                bcd_device: definition.identity.bcd_device.map(str::to_owned),
                status: status(definition.identity.status).into(),
                status_text: definition.identity.status_text.into(),
                notes: definition.identity.notes.into(),
                evidence: definition.identity.evidence.into(),
            },
            transport: RuntimeTransport {
                kind: transport_kind(definition.transport.kind).into(),
                report_size: definition.transport.report_size,
                out_endpoint: definition.transport.out_endpoint,
                in_endpoint: definition.transport.in_endpoint,
                poll_interval_ms: definition.transport.poll_interval_ms,
                uses_numbered_reports: definition.transport.uses_numbered_reports,
                expected_interface_number: definition.transport.expected_interface_number,
                expected_usage_page: definition.transport.expected_usage_page,
                expected_usage: definition.transport.expected_usage,
                status: status(definition.transport.status).into(),
                status_text: definition.transport.status_text.into(),
                notes: definition.transport.notes.into(),
                evidence: definition.transport.evidence.into(),
            },
            address_spaces: spaces,
            inputs: runtime_inputs,
            outputs: definition
                .outputs
                .iter()
                .map(|output| RuntimeOutput {
                    id: output.id,
                    name: output.name.into(),
                    aliases: output.aliases.iter().map(|alias| (*alias).into()).collect(),
                    verified: output.verified,
                    status: status(output.status).into(),
                    metadata: output.metadata.into(),
                })
                .collect(),
            mixers: definition
                .mixers
                .iter()
                .map(|mixer| RuntimeMixer {
                    id: mixer.id.into(),
                    name: mixer.name.into(),
                    mix_index: mixer.mix_index,
                    strip_count: mixer.strip_count,
                    has_master: mixer.has_master,
                    fader_range: mixer.fader_range,
                    fader: mixer.fader.map(|fader| FaderSemantics {
                        min: fader.min,
                        max: fader.max,
                        direction: match fader.direction {
                            super::FaderDirectionDefinition::Direct => FaderDirection::Direct,
                            super::FaderDirectionDefinition::Attenuation => FaderDirection::Attenuation,
                        },
                        unity: fader.unity,
                    }),
                    pan_range: mixer.pan_range,
                    pan_center: mixer.pan_center,
                    send_range: mixer.send_range,
                    status: status(mixer.status).into(),
                    status_text: mixer.status_text.into(),
                    notes: mixer.notes.into(),
                    metadata: mixer.metadata.into(),
                })
                .collect(),
            state_report: definition.state_report.map(|state_report| RuntimeStateReport {
                candidate_preamp_meters: state_report
                    .candidate_preamp_meters
                    .iter()
                    .map(|meter| CandidatePreampMeter {
                        input_index: meter.input_index,
                        offset: meter.offset,
                    })
                    .collect(),
            }),
            link_domains: definition
                .link_domains
                .iter()
                .map(|domain| RuntimeLinkDomain {
                    protocol_space: domain.protocol_space,
                    kind: match domain.kind {
                        LinkDomainKind::Mixer => RuntimeLinkDomainKind::Mixer,
                    },
                    pair_count: domain.pair_count,
                    status: status(domain.status).into(),
                    evidence: domain.evidence.into(),
                })
                .collect(),
            routing_groups: definition
                .routing_groups
                .iter()
                .map(|group| RuntimeRoutingGroup {
                    destination: group.destination,
                    name: group.name.into(),
                    channel_count: group.channel_count,
                    source_domains: group
                        .source_domains
                        .iter()
                        .map(|domain| RuntimeRoutingSourceDomain {
                            bank: domain.bank,
                            index_count: domain.index_count,
                            status: status(domain.status).into(),
                            evidence: domain.evidence.into(),
                        })
                        .collect(),
                })
                .collect(),
            frames: definition
                .frames
                .iter()
                .map(|frame| {
                    let operations = frame.operations.iter().map(convert_operation).collect();
                    RuntimeFrame {
                        id: frame.id.into(),
                        kind: frame_kind(frame.kind).into(),
                        status: status(frame.status).into(),
                        report_size: definition.transport.report_size,
                        operations,
                        metadata: frame.metadata.into(),
                    }
                })
                .collect(),
            decoders: definition
                .decoders
                .iter()
                .map(|decoder| RuntimeDecoder {
                    id: decoder.id.into(),
                    frame_id: decoder.frame_id.into(),
                    kind: frame_kind(decoder.kind).into(),
                    status: status(decoder.status).into(),
                    metadata: decoder.metadata.into(),
                })
                .collect(),
            params: definition
                .params
                .iter()
                .map(|param| RuntimeParam {
                    name: param.name.into(),
                    id: param.id,
                    value_type: param_value_type(param.value_type).into(),
                    status: status(param.status).into(),
                    applies_to: param.applies_to.into(),
                    range: param.range,
                    range_by_mode: param
                        .range_by_mode
                        .iter()
                        .filter_map(|range| range.range.map(|value| (range.name.into(), value)))
                        .collect(),
                    values: param
                        .values
                        .iter()
                        .map(|value| (value.value, value.name.into()))
                        .collect(),
                    frame: convert_reference(param.frame),
                    readback: convert_reference(param.readback),
                    metadata: param.metadata.into(),
                })
                .collect(),
            constraints: definition
                .constraints
                .iter()
                .map(|constraint| RuntimeConstraint {
                    name: constraint.name.into(),
                    status: status(constraint.status).into(),
                    range: constraint.range,
                    values: constraint.values.to_vec(),
                    scalar: constraint.scalar,
                    text: constraint.text.into(),
                    metadata: constraint.metadata.into(),
                })
                .collect(),
            hazards,
            startup_queries: definition
                .startup_queries
                .iter()
                .map(|query| QueryRequest::new(query.query_id, query.sub_id))
                .collect(),
            readback: definition
                .readback
                .map(|readback| RuntimeReadbackDefinition {
                    request_magic: readback.request_magic,
                    request_subcommand: readback.request_subcommand,
                    response_magic: readback.response_magic,
                    response_discriminator_offset: readback.response_discriminator_offset,
                    response_discriminator: readback.response_discriminator,
                    category_offset: readback.category_offset,
                    index_offset: readback.index_offset,
                    data_offset: readback.data_offset,
                    category_counts: readback
                        .category_counts
                        .iter()
                        .map(|category| ReadbackCategory {
                            category: category.category,
                            count: category.count,
                        })
                        .collect(),
                    safe_queries: readback
                        .safe_queries
                        .iter()
                        .map(|query| antelope_protocol::SafeQuery {
                            category: query.category,
                            index: query.index,
                        })
                        .collect(),
                    layouts: readback
                        .layouts
                        .iter()
                        .map(|layout| MixerReadbackLayout {
                            category: layout.category,
                            index: layout.index,
                            body_size: layout.body_size,
                            record_count: layout.record_count,
                            record_stride: layout.record_stride,
                            level_offset: layout.level_offset,
                            state_offset: layout.state_offset,
                            surface: layout.surface,
                            surface_stride: layout.surface_stride,
                            supported_fields: layout.supported_fields.iter().map(|field| (*field).into()).collect(),
                        })
                        .collect(),
                }),
            provenance: RuntimeProvenance {
                source_path: definition.provenance.source_path.into(),
                source_sha256: definition.provenance.source_sha256.into(),
                generator_version: definition.provenance.generator_version.into(),
            },
        },
        readiness,
        driver_kind: match (readiness, definition.identity.pid) {
            (RuntimeReadiness::Supported, 0xa015) => RuntimeDriverKind::ZenGo,
            (RuntimeReadiness::Supported, _) => RuntimeDriverKind::Profile,
            _ => RuntimeDriverKind::None,
        },
        support_reason: if is_orion {
            "validated source-backed profile; assumes unnumbered HID reports pending hardware verification"
        } else {
            match readiness {
                RuntimeReadiness::Supported => "validated built-in driver",
                RuntimeReadiness::Partial => "profile data is incomplete for safe read/write control",
            RuntimeReadiness::Unverified => "transport or frame geometry is unverified",
                RuntimeReadiness::Disabled => "profile is not enabled for control",
            }
        }
        .into(),
    }
}

fn convert_operation(operation: &FrameOperationDefinition) -> FrameOperation {
    match operation {
        FrameOperationDefinition::FixedByte { offset, value } => FrameOperation::FixedByte {
            offset: *offset,
            value: *value,
        },
        FrameOperationDefinition::Scalar {
            field,
            offset,
            width,
            endian,
        } => FrameOperation::Scalar {
            field: (*field).into(),
            offset: *offset,
            width: *width,
            endian: match endian {
                FrameEndianDefinition::NotApplicable => FrameEndian::NotApplicable,
                FrameEndianDefinition::Little => FrameEndian::Little,
                FrameEndianDefinition::Big => FrameEndian::Big,
            },
        },
        FrameOperationDefinition::Indexed {
            base,
            stride,
            index_field,
            width,
            max_index,
        } => FrameOperation::Indexed {
            base: *base,
            stride: *stride,
            index_field: (*index_field).into(),
            width: *width,
            max_index: *max_index,
        },
        FrameOperationDefinition::BitField {
            field,
            offset,
            mask,
            shift,
        } => FrameOperation::BitField {
            field: (*field).into(),
            offset: *offset,
            mask: *mask,
            shift: *shift,
        },
        FrameOperationDefinition::PairIndex {
            base,
            stride,
            pair_field,
            width,
            max_index,
        } => FrameOperation::PairIndex {
            base: *base,
            stride: *stride,
            pair_field: (*pair_field).into(),
            width: *width,
            max_index: *max_index,
        },
        FrameOperationDefinition::AllowedValues { values } => FrameOperation::AllowedValues {
            values: values.to_vec(),
        },
        FrameOperationDefinition::UncompiledFormula { formula } => {
            FrameOperation::UncompiledFormula {
                formula: (*formula).into(),
            }
        }
    }
}

fn convert_reference(reference: super::ParamReference) -> ParamReference {
    ParamReference {
        text: reference.text.into(),
        formula: reference.formula.into(),
        offsets: reference
            .offsets
            .iter()
            .map(|offset| (offset.name.into(), offset.offset))
            .collect(),
    }
}

fn convert_readiness(readiness: Readiness) -> RuntimeReadiness {
    match readiness {
        Readiness::Supported => RuntimeReadiness::Supported,
        Readiness::Partial => RuntimeReadiness::Partial,
        Readiness::Unverified => RuntimeReadiness::Unverified,
        Readiness::Disabled => RuntimeReadiness::Disabled,
    }
}

fn status(value: Status) -> &'static str {
    match value {
        Status::Confirmed => "confirmed",
        Status::Observed => "observed",
        Status::Unconfirmed => "unconfirmed",
        Status::Unavailable => "unavailable",
        Status::Unknown => "unknown",
    }
}

fn transport_kind(value: TransportKind) -> &'static str {
    match value {
        TransportKind::Hid => "hid",
        TransportKind::Unknown => "unknown",
    }
}

fn address_space_kind(value: AddressSpaceKind) -> &'static str {
    match value {
        AddressSpaceKind::PhysicalInputs => "physical_inputs",
        AddressSpaceKind::AdatInputs => "adat_inputs",
        AddressSpaceKind::SpdifInputs => "spdif_inputs",
        AddressSpaceKind::Outputs => "outputs",
        AddressSpaceKind::Mixer => "mixer",
        AddressSpaceKind::Routing => "routing",
        AddressSpaceKind::Unknown => "unknown",
    }
}

fn addressing(value: AddressingMode) -> &'static str {
    match value {
        AddressingMode::ZeroBased => "zero_based",
        AddressingMode::OneBased => "one_based",
        AddressingMode::Unknown => "unknown",
    }
}

fn frame_kind(value: FrameKind) -> &'static str {
    match value {
        FrameKind::Command => "command",
        FrameKind::StateReport => "state_report",
        FrameKind::MeterReport => "meter_report",
        FrameKind::NameReport => "name_report",
        FrameKind::InitEnumerationReport => "init_enumeration_report",
        FrameKind::ErrorResponse => "error_response",
        FrameKind::Response => "response",
        FrameKind::Decoder => "decoder",
        FrameKind::Unknown => "unknown",
    }
}

fn param_value_type(value: ParamValueType) -> &'static str {
    match value {
        ParamValueType::Bool => "bool",
        ParamValueType::Enum => "enum",
        ParamValueType::Int => "int",
        ParamValueType::Int8 => "int8",
        ParamValueType::UInt => "uint",
        ParamValueType::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use antelope_protocol::{load_profile_pack, RuntimeReadiness};

    #[test]
    fn builtin_catalog_is_owned_sorted_and_promotes_orion_to_profile_support() {
        let catalog = ProfileCatalog::builtin();
        assert_eq!(catalog.entries().len(), DEVICE_CATALOG.len());
        assert_eq!(catalog.entries()[0].readiness, RuntimeReadiness::Supported);
        let orion = catalog.find(0x23e5, 0xa221).expect("Orion entry");
        assert_eq!(orion.readiness, RuntimeReadiness::Supported);
        assert_eq!(orion.driver_kind, RuntimeDriverKind::Profile);
        assert_eq!(orion.profile.inputs_in("physical_inputs"), 12);
    }

    #[test]
    fn builtin_catalog_has_safe_support_matrix_and_validation_metadata() {
        let catalog = ProfileCatalog::builtin();
        let expected = [
            (
                "Antelope Zen Go Synergy Core",
                RuntimeReadiness::Supported,
                RuntimeDriverKind::ZenGo,
            ),
            (
                "Antelope Orion Studio III",
                RuntimeReadiness::Supported,
                RuntimeDriverKind::Profile,
            ),
            (
                "Antelope Discrete 8 Pro Synergy Core",
                RuntimeReadiness::Partial,
                RuntimeDriverKind::None,
            ),
            (
                "Antelope Discrete 4 Synergy Core",
                RuntimeReadiness::Unverified,
                RuntimeDriverKind::None,
            ),
            (
                "Antelope Discrete 4 Pro Synergy Core",
                RuntimeReadiness::Unverified,
                RuntimeDriverKind::None,
            ),
        ];

        for (name, readiness, driver_kind) in expected {
            let entry = catalog
                .entries()
                .iter()
                .find(|entry| entry.profile.identity.name == name)
                .expect("support-matrix entry");
            assert_eq!(entry.readiness, readiness, "{name} readiness");
            assert_eq!(entry.driver_kind, driver_kind, "{name} driver");
            assert!(!entry.support_reason.trim().is_empty(), "{name} reason");

            let provenance = &entry.profile.provenance;
            assert!(
                provenance.source_path.starts_with("profiles/")
                    && provenance.source_path.ends_with(".json"),
                "{name} stable source path"
            );
            assert_eq!(provenance.source_sha256.len(), 64, "{name} SHA-256");
            assert!(
                provenance
                    .source_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "{name} lowercase SHA-256"
            );
            assert!(
                !provenance.generator_version.trim().is_empty(),
                "{name} generator version"
            );
        }
    }

    #[test]
    fn checked_in_normalized_pack_loads_promoted_orion_support() {
        let pack = load_profile_pack(include_bytes!("generated_profiles.json"))
            .expect("checked-in normalized pack");
        assert_eq!(pack.profiles().len(), 5);
        let orion = pack
            .profiles()
            .iter()
            .find(|entry| entry.profile.identity.pid == 0xa221)
            .expect("Orion pack entry");
        assert_eq!(orion.readiness, RuntimeReadiness::Supported);
        assert_eq!(orion.driver_kind, RuntimeDriverKind::Profile);
    }

    #[test]
    fn builtin_entries_match_checked_in_normalized_pack_profiles() {
        let builtin = ProfileCatalog::builtin();
        let pack = load_profile_pack(include_bytes!("generated_profiles.json"))
            .expect("checked-in normalized pack");
        assert_eq!(builtin.entries().len(), pack.profiles().len());
        for built_in in builtin.entries() {
            let packed = pack
                .profiles()
                .iter()
                .find(|entry| entry.id == built_in.id)
                .expect("matching normalized entry");
            assert_eq!(
                built_in, packed,
                "builtin and normalized-pack RuntimeEntry mismatch for {}",
                built_in.id
            );
        }
    }

    #[test]
    fn external_pack_cannot_shadow_builtin_identity() {
        let mut catalog = ProfileCatalog::builtin();
        let pack = load_profile_pack(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/antelope-protocol/tests/fixtures/profile_pack_v1.json"
        )))
        .expect("fixture pack");
        let mut pack = pack;
        pack.profiles[0].id = "external_orion".into();
        let error = catalog.add_external(pack).expect_err("identity collision");
        assert!(matches!(error, ProfileLoadError::DuplicateIdentity { .. }));
    }

    #[test]
    fn external_pack_is_sorted_and_findable() {
        let mut catalog = ProfileCatalog::builtin();
        let mut pack = load_profile_pack(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/antelope-protocol/tests/fixtures/profile_pack_v1.json"
        )))
        .expect("fixture pack");
        pack.profiles[0].id = "fixture_unique".into();
        pack.profiles[0].profile.identity.pid = 0xff01;
        pack.profiles[0].readiness = RuntimeReadiness::Disabled;
        pack.profiles[0].driver_kind = RuntimeDriverKind::None;
        catalog.add_external(pack).expect("unique external profile");
        assert_eq!(catalog.find(0x23e5, 0xff01).unwrap().id, "fixture_unique");
    }
}
