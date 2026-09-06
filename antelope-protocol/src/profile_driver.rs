//! Generic driver backed by a validated normalized runtime profile.

use std::collections::{HashMap, HashSet};

use crate::driver::{
    Action, CommandBatch, ControlValue, DeviceDriver, DeviceEvent, DriverDefinition, DriverError,
    DynamicDeviceState, DynamicGlobalState, DynamicInputState, DynamicMeterState,
    DynamicMixerStrip, DynamicMixerSurface, DynamicOutputState, DynamicRoutingGroup,
    DynamicStatePatch, GlobalControl, InputAddress, InputControl, MixerAddress, OutputAddress,
    OutputControl, RoutingSource,
};
use crate::profile::{
    FrameOperation, RuntimeDriverKind, RuntimeEntry, RuntimeFrame, RuntimeMeterTarget,
    RuntimeParam, RuntimeProfile, RuntimeReadiness,
};
use crate::profile_codec;
use crate::types::PanState;
use crate::QueryRequest;

#[derive(Debug)]
enum MeterSource {
    MeterReport,
    StateReport,
    Unavailable,
}

#[derive(Debug)]
pub struct ProfileDriver {
    definition: DriverDefinition,
    profile: RuntimeProfile,
    startup_requests: Vec<QueryRequest>,
    frame_index: HashMap<String, usize>,
    mixer_readback_category: u8,
    routing_readback_category: u8,
    routing_source_count: usize,
    meter_source: MeterSource,
    canonical_orion_identity: bool,
}

impl ProfileDriver {
    fn validate_declared_state_meter_layout(
        profile: &RuntimeProfile,
        frame: &RuntimeFrame,
    ) -> Result<bool, DriverError> {
        let mut indexed = frame
            .operations
            .iter()
            .filter_map(|operation| match operation {
                FrameOperation::Indexed { index_field, .. } if index_field == "physical_meter" => {
                    Some(operation)
                }
                _ => None,
            });
        let Some(operation) = indexed.next() else {
            if frame.operations.iter().any(|operation| {
                matches!(
                    operation,
                    FrameOperation::Scalar { field, .. } if field == "physical_meter"
                )
            }) {
                return Err(DriverError::InvalidAction(
                    "state physical meter must use an indexed mapping".into(),
                ));
            }
            return Ok(false);
        };
        if indexed.next().is_some() {
            return Err(DriverError::InvalidAction(
                "state physical meter mapping is ambiguous".into(),
            ));
        }
        let FrameOperation::Indexed {
            width, max_index, ..
        } = operation
        else {
            unreachable!("filtered state meter operation is indexed");
        };
        let physical_space = profile
            .address_spaces
            .iter()
            .find(|space| space.id == "physical_inputs")
            .ok_or_else(|| DriverError::InvalidAction("missing physical meter space".into()))?;
        let physical_count = physical_space.count.ok_or_else(|| {
            DriverError::InvalidAction("physical meter space is unbounded".into())
        })?;
        if physical_count == 0
            || profile
                .inputs
                .iter()
                .filter(|input| input.space_id == physical_space.space_id)
                .count()
                != usize::from(physical_count)
        {
            return Err(DriverError::InvalidAction(
                "physical meter count does not match finite physical inputs".into(),
            ));
        }
        let max_index = max_index.ok_or_else(|| {
            DriverError::InvalidAction("state physical meter mapping is unbounded".into())
        })?;
        if *width != 1 || u32::from(max_index) + 1 != u32::from(physical_count) {
            return Err(DriverError::InvalidAction(
                "state physical meter layout is incomplete".into(),
            ));
        }
        Ok(true)
    }

    pub fn new(entry: RuntimeEntry) -> Result<Self, DriverError> {
        if entry.readiness != RuntimeReadiness::Supported {
            return Err(DriverError::UnsupportedAction(format!(
                "profile {} readiness is {:?}",
                entry.id, entry.readiness
            )));
        }
        if entry.driver_kind != RuntimeDriverKind::Profile {
            return Err(DriverError::UnsupportedAction(format!(
                "profile {} does not use profile driver",
                entry.id
            )));
        }
        match entry.profile.transport.uses_numbered_reports {
            None => {
                return Err(DriverError::UnsupportedAction(format!(
                    "profile {} has unconfirmed report framing",
                    entry.id
                )))
            }
            Some(true) => {
                return Err(DriverError::UnsupportedAction(format!(
                    "profile {} numbered report framing is not representable by the generic profile codec",
                    entry.id
                )))
            }
            Some(false) => {}
        }
        let report_size = profile_codec::report_size(&entry.profile)?;
        let mut frame_index = HashMap::new();
        for (index, frame) in entry.profile.frames.iter().enumerate() {
            if frame_index.insert(frame.id.clone(), index).is_some() {
                return Err(DriverError::InvalidAction(format!(
                    "ambiguous duplicate frame {}",
                    frame.id
                )));
            }
            // Observation-only decoder frames may contain alternative wire maps
            // for one byte. They are not used by this command driver.
            if profile_codec::is_confirmed(&frame.status) || frame.kind != "decoder" {
                profile_codec::validate_operations(frame, report_size)?;
            }
            if frame.report_size.map(usize::from).unwrap_or(report_size) != report_size {
                return Err(DriverError::InvalidAction(format!(
                    "frame {} report geometry differs from transport",
                    frame.id
                )));
            }
        }
        let mut meter_mapping_keys = HashSet::new();
        for mapping in &entry.profile.meter_mappings {
            if !matches!(mapping.frame_id.as_str(), "state_report" | "meter_report")
                || !frame_index.contains_key(&mapping.frame_id)
            {
                return Err(DriverError::InvalidAction(format!(
                    "meter mapping references undeclared frame {}",
                    mapping.frame_id
                )));
            }
            if mapping.offset >= report_size
                || mapping.status.trim().is_empty()
                || mapping.evidence.trim().is_empty()
            {
                return Err(DriverError::InvalidAction(
                    "meter mapping is malformed or outside report geometry".into(),
                ));
            }
            let target_exists = match mapping.target {
                RuntimeMeterTarget::MixMaster => entry
                    .profile
                    .mixers
                    .iter()
                    .any(|mixer| u16::from(mixer.mix_index) == mapping.target_index),
                RuntimeMeterTarget::PhysicalOutput => entry
                    .profile
                    .outputs
                    .iter()
                    .any(|output| output.id == mapping.target_index),
            };
            if !target_exists {
                return Err(DriverError::InvalidAction(format!(
                    "meter mapping target {} is not in profile topology",
                    mapping.target_index
                )));
            }
            if !meter_mapping_keys.insert((mapping.target, mapping.target_index, mapping.lane)) {
                return Err(DriverError::InvalidAction(
                    "meter target lane is declared more than once across frames".into(),
                ));
            }
        }
        let confirmed_meter_decoder_count = entry
            .profile
            .decoders
            .iter()
            .filter(|decoder| {
                decoder.frame_id == "meter_report"
                    && decoder.kind == "meter_report"
                    && profile_codec::is_confirmed(&decoder.status)
            })
            .count();
        let state_has_meter_layout = match frame_index
            .get("state_report")
            .and_then(|index| entry.profile.frames.get(*index))
        {
            Some(frame) => Self::validate_declared_state_meter_layout(&entry.profile, frame)?,
            None => false,
        };
        let meter_frame_is_confirmed = frame_index
            .get("meter_report")
            .and_then(|index| entry.profile.frames.get(*index))
            .is_some_and(|frame| profile_codec::is_confirmed(&frame.status));
        let meter_source = if meter_frame_is_confirmed && confirmed_meter_decoder_count == 1 {
            MeterSource::MeterReport
        } else if meter_frame_is_confirmed {
            return Err(DriverError::InvalidAction(
                "confirmed meter_report requires exactly one confirmed decoder mapping".into(),
            ));
        } else if state_has_meter_layout {
            MeterSource::StateReport
        } else {
            MeterSource::Unavailable
        };
        for required in [
            "command",
            "global_command",
            "mix_command",
            "link_command",
            "routing_command",
            "state_report",
            "readback",
        ] {
            let frame = frame_index
                .get(required)
                .and_then(|index| entry.profile.frames.get(*index))
                .ok_or_else(|| {
                    DriverError::UnsupportedAction(format!("required {required} frame"))
                })?;
            if !profile_codec::is_confirmed(&frame.status) {
                return Err(DriverError::UnsupportedAction(format!(
                    "required {required} frame is unconfirmed"
                )));
            }
            if frame
                .operations
                .iter()
                .any(|operation| matches!(operation, FrameOperation::UncompiledFormula { .. }))
            {
                return Err(DriverError::UnsupportedAction(format!(
                    "required {required} frame has uncompiled formula"
                )));
            }
        }
        for frame_id in [
            "command",
            "global_command",
            "mix_command",
            "link_command",
            "routing_command",
        ] {
            let frame = &entry.profile.frames[*frame_index.get(frame_id).expect("required frame")];
            if !frame
                .operations
                .iter()
                .any(|operation| matches!(operation, FrameOperation::FixedByte { .. }))
            {
                return Err(DriverError::InvalidAction(format!(
                    "required {frame_id} frame has no fixed command mapping"
                )));
            }
        }
        for (decoder_kind, frame_id) in [("state_report", "state_report"), ("readback", "readback")]
        {
            let count = entry
                .profile
                .decoders
                .iter()
                .filter(|decoder| {
                    decoder.frame_id == frame_id
                        && (decoder.kind == decoder_kind
                            || (frame_id == "readback" && decoder.kind == "decoder"))
                        && profile_codec::is_confirmed(&decoder.status)
                })
                .count();
            if count != 1 {
                return Err(DriverError::InvalidAction(format!(
                    "required {decoder_kind} decoder mapping count is {count}"
                )));
            }
        }
        if matches!(meter_source, MeterSource::MeterReport) && confirmed_meter_decoder_count != 1 {
            return Err(DriverError::InvalidAction(format!(
                "required meter_report decoder mapping count is {confirmed_meter_decoder_count}"
            )));
        }

        let readback = entry.profile.readback.as_ref().ok_or_else(|| {
            DriverError::UnsupportedAction("profile has no confirmed readback definition".into())
        })?;
        for (name, offset) in [
            ("discriminator", readback.response_discriminator_offset),
            ("category", readback.category_offset),
            ("index", readback.index_offset),
            ("data", readback.data_offset),
        ] {
            if usize::from(offset) >= report_size {
                return Err(DriverError::InvalidAction(format!(
                    "readback {name} offset outside report"
                )));
            }
        }
        let fixed = |frame_id: &str, offset: u16| {
            frame_index
                .get(frame_id)
                .and_then(|index| entry.profile.frames.get(*index))
                .and_then(|frame| profile_codec::fixed_byte(frame, offset))
        };
        let state_magic = fixed("state_report", 0);
        let readback_magic = fixed("readback", 0);
        let readback_discriminator = fixed("readback", readback.response_discriminator_offset);
        let meter_magic = fixed("meter_report", 0);
        let meter_discriminator = fixed("meter_report", readback.response_discriminator_offset);
        let meter_mapping_valid = meter_magic == Some(readback.response_magic)
            && meter_discriminator.is_some()
            && meter_discriminator != Some(readback.response_discriminator);
        let canonical_orion_identity = entry.id == "orion_studio_3"
            && entry.profile.identity.vid == 0x23e5
            && entry.profile.identity.pid == 0xa221;
        let readback_magic_valid = readback_magic == Some(readback.response_magic)
            || profile_codec::scalar_offset(
                &entry.profile.frames[*frame_index.get("readback").expect("required frame")],
                "magic",
            )
            .is_ok_and(|offset| offset == 0);
        if state_magic.is_none()
            || !readback_magic_valid
            || readback_discriminator != Some(readback.response_discriminator)
            || (matches!(meter_source, MeterSource::MeterReport) && !meter_mapping_valid)
        {
            return Err(DriverError::InvalidAction(
                "state/meter/readback discriminator mappings are incomplete or ambiguous".into(),
            ));
        }
        if entry.profile.startup_queries.is_empty() {
            return Err(DriverError::InvalidAction(
                "profile has no explicit startup readback walk".into(),
            ));
        }
        let mut category_counts = HashSet::new();
        for category in &readback.category_counts {
            if category.count == 0 || !category_counts.insert(category.category) {
                return Err(DriverError::InvalidAction(
                    "readback category bounds are empty or ambiguous".into(),
                ));
            }
        }
        for query in &entry.profile.startup_queries {
            let count = readback
                .category_counts
                .iter()
                .find(|category| category.category == query.query_id)
                .map(|category| category.count)
                .ok_or_else(|| {
                    DriverError::InvalidAction(format!(
                        "startup readback category {:#04x} has no bound",
                        query.query_id
                    ))
                })?;
            if u16::from(query.sub_id) >= count {
                return Err(DriverError::InvalidAction(format!(
                    "startup readback category {:#04x} index {} outside count {count}",
                    query.query_id, query.sub_id
                )));
            }
        }

        let scalar_constraint = |name: &str| -> Option<i32> {
            entry
                .profile
                .constraints
                .iter()
                .find(|constraint| {
                    constraint.name == name && profile_codec::is_confirmed(&constraint.status)
                })
                .and_then(|constraint| constraint.scalar)
        };
        // Older normalized packs carried these as scalar constraints.  Newer
        // packs prove them through finite topology and readback bounds.
        let category_for_count = |count: usize, name: &str| -> Result<u8, DriverError> {
            let matches: Vec<_> = readback
                .category_counts
                .iter()
                .filter(|category| usize::from(category.count) == count)
                .map(|category| category.category)
                .collect();
            if matches.len() != 1 {
                return Err(DriverError::InvalidAction(format!(
                    "{name} readback category count is {}",
                    matches.len()
                )));
            }
            Ok(matches[0])
        };
        let mixer_readback_category = match scalar_constraint("mixer_readback_category") {
            Some(value) => u8::try_from(value).map_err(|_| {
                DriverError::InvalidAction("mixer readback category outside byte".into())
            })?,
            None if canonical_orion_identity => {
                category_for_count(entry.profile.mixers.len(), "mixer")?
            }
            None => {
                return Err(DriverError::InvalidAction(
                    "mixer readback category has no confirmed scalar constraint".into(),
                ))
            }
        };
        let routing_readback_category = match scalar_constraint("routing_readback_category") {
            Some(value) => u8::try_from(value).map_err(|_| {
                DriverError::InvalidAction("routing readback category outside byte".into())
            })?,
            None if canonical_orion_identity => {
                category_for_count(entry.profile.routing_groups.len(), "routing")?
            }
            None => {
                return Err(DriverError::InvalidAction(
                    "routing readback category has no confirmed scalar constraint".into(),
                ))
            }
        };
        let routing_source_count = match scalar_constraint("routing_source_count") {
            Some(value) => usize::try_from(value).map_err(|_| {
                DriverError::InvalidAction("routing source count is invalid".into())
            })?,
            None if canonical_orion_identity => entry
                .profile
                .routing_groups
                .iter()
                .map(|group| usize::from(group.channel_count))
                .max()
                .ok_or_else(|| {
                    DriverError::InvalidAction("routing source count is invalid".into())
                })?,
            None => {
                return Err(DriverError::InvalidAction(
                    "routing source count has no confirmed scalar constraint".into(),
                ))
            }
        };
        let routing_destination_count = match scalar_constraint("routing_destination_count") {
            Some(value) => u16::try_from(value).map_err(|_| {
                DriverError::InvalidAction("routing destination count is invalid".into())
            })?,
            None if canonical_orion_identity => entry.profile.routing_groups.len() as u16,
            None => {
                return Err(DriverError::InvalidAction(
                    "routing destination count has no confirmed scalar constraint".into(),
                ))
            }
        };
        if routing_source_count == 0
            || routing_destination_count == 0
            || usize::from(routing_destination_count) != entry.profile.routing_groups.len()
            || mixer_readback_category == routing_readback_category
        {
            return Err(DriverError::InvalidAction(
                "invalid profile routing/readback capability domain".into(),
            ));
        }
        for group in &entry.profile.routing_groups {
            let mut banks = HashSet::new();
            if group.source_domains.is_empty()
                || group.source_domains.iter().any(|domain| {
                    !banks.insert(domain.bank)
                        || domain.index_count == 0
                        || domain.index_count > 256
                        || !profile_codec::is_confirmed(&domain.status)
                        || domain.evidence.trim().is_empty()
                })
            {
                return Err(DriverError::InvalidAction(format!(
                    "routing destination {} has invalid confirmed source domains",
                    group.destination
                )));
            }
            let mut readback_banks = HashSet::new();
            if group.readback_source_domains.iter().any(|domain| {
                !readback_banks.insert(domain.bank)
                    || domain.bank == 0x0c
                    || banks.contains(&domain.bank)
                    || domain.indices.is_empty()
                    || domain.indices.len() > 256
                    || domain.indices.windows(2).any(|pair| pair[0] >= pair[1])
                    || !domain
                        .status
                        .trim()
                        .to_ascii_lowercase()
                        .starts_with("observ")
                    || domain.evidence.trim().is_empty()
            }) {
                return Err(DriverError::InvalidAction(format!(
                    "routing destination {} has invalid observed readback source domains",
                    group.destination
                )));
            }
        }
        let mut link_spaces = HashSet::new();
        if entry.profile.link_domains.is_empty()
            || entry.profile.link_domains.iter().any(|domain| {
                !link_spaces.insert(domain.protocol_space)
                    || domain.pair_count == 0
                    || domain.pair_count > 256
                    || !profile_codec::is_confirmed(&domain.status)
                    || domain.evidence.trim().is_empty()
            })
        {
            return Err(DriverError::InvalidAction(
                "profile has invalid confirmed link domains".into(),
            ));
        }
        let counts: HashSet<u8> = readback
            .category_counts
            .iter()
            .map(|category| category.category)
            .collect();
        let readback_frame =
            &entry.profile.frames[*frame_index.get("readback").expect("required frame")];
        let has_mixer_layout = readback_frame.operations.iter().any(|operation| {
            matches!(operation, FrameOperation::Indexed { index_field, .. } if index_field == "mixer_slot")
        });
        let has_routing_layout = readback_frame.operations.iter().any(|operation| {
            matches!(operation, FrameOperation::Indexed { index_field, .. } if index_field == "routing_source_pair")
        });
        if (!has_mixer_layout || !has_routing_layout)
            && (readback.data_offset != 16
                || (!has_mixer_layout && mixer_readback_category != 0x04)
                || (!has_routing_layout && routing_readback_category != 0x03))
        {
            return Err(DriverError::InvalidAction(
                "implicit readback layout lacks confirmed canonical offsets".into(),
            ));
        }
        if !counts.contains(&mixer_readback_category)
            || !counts.contains(&routing_readback_category)
        {
            return Err(DriverError::InvalidAction(
                "readback categories lack required confirmed decoder mappings".into(),
            ));
        }

        let driver = Self {
            definition: DriverDefinition {
                id: entry.id,
                name: entry.profile.identity.name.clone(),
                vid: entry.profile.identity.vid,
                pid: entry.profile.identity.pid,
                supported: true,
            },
            startup_requests: entry.profile.startup_queries.clone(),
            profile: entry.profile,
            frame_index,
            mixer_readback_category,
            routing_readback_category,
            routing_source_count,
            meter_source,
            canonical_orion_identity,
        };
        driver.validate_capabilities()?;
        let zero_report = vec![0; report_size];
        driver.decode_state(&zero_report)?;
        if matches!(driver.meter_source, MeterSource::MeterReport) {
            driver.decode_meter(&zero_report)?;
        }
        Ok(driver)
    }

    fn frame(&self, id: &str) -> Result<&RuntimeFrame, DriverError> {
        self.frame_index
            .get(id)
            .and_then(|index| self.profile.frames.get(*index))
            .ok_or_else(|| DriverError::UnsupportedAction(format!("profile frame {id}")))
    }

    fn scalar_alias<'a>(
        frame: &'a RuntimeFrame,
        canonical: &'static str,
        legacy: &'static str,
    ) -> Result<&'static str, DriverError> {
        if profile_codec::scalar_offset(frame, canonical).is_ok() {
            Ok(canonical)
        } else if profile_codec::scalar_offset(frame, legacy).is_ok() {
            Ok(legacy)
        } else {
            Err(DriverError::InvalidAction(format!(
                "frame {} missing semantic {canonical} or {legacy}",
                frame.id
            )))
        }
    }

    fn parameter_target_matches(
        parameter: &RuntimeParam,
        target: &str,
        canonical_orion_identity: bool,
    ) -> bool {
        parameter.applies_to == target
            || (canonical_orion_identity
                && ((target == "outputs" && parameter.name.starts_with("bus_"))
                    || (target == "adat_inputs" && parameter.name == "adat_gain")
                    || (target == "spdif_inputs" && parameter.name == "spdif_gain")
                    || (target == "physical_inputs" && parameter.name == "gain")
                    || (target == "mixers" && parameter.name.starts_with("mix_"))))
    }

    fn parameter(&self, target: &str, name: &str) -> Result<&RuntimeParam, DriverError> {
        self.profile
            .params
            .iter()
            .find(|parameter| {
                parameter.name == name
                    && Self::parameter_target_matches(
                        parameter,
                        target,
                        self.canonical_orion_identity,
                    )
                    && profile_codec::is_confirmed(&parameter.status)
            })
            .ok_or_else(|| {
                DriverError::UnsupportedAction(format!("confirmed parameter {target}/{name}"))
            })
    }

    fn parameter_by_id_for(&self, id: u16, applies_to: &str) -> Result<&RuntimeParam, DriverError> {
        let mut matches = self.profile.params.iter().filter(|parameter| {
            parameter.id == Some(id) && profile_codec::is_confirmed(&parameter.status)
        });
        let parameter = matches.next().ok_or_else(|| {
            DriverError::UnsupportedAction(format!("confirmed parameter id {id}"))
        })?;
        if matches.next().is_some() {
            return Err(DriverError::InvalidAction(format!(
                "ambiguous confirmed parameter id {id}"
            )));
        }
        if !Self::parameter_target_matches(parameter, applies_to, self.canonical_orion_identity) {
            return Err(DriverError::InvalidAction(format!(
                "parameter id {id} applies to {}, not {applies_to}",
                parameter.applies_to
            )));
        }
        Ok(parameter)
    }

    fn validate_reference(
        &self,
        frame: &RuntimeFrame,
        parameter: &RuntimeParam,
        required: &[&str],
    ) -> Result<(), DriverError> {
        if !parameter.frame.formula.trim().is_empty() {
            return Err(DriverError::UnsupportedAction(format!(
                "parameter {} has uncompiled formula",
                parameter.name
            )));
        }
        let mut seen = HashSet::new();
        for (field, offset) in &parameter.frame.offsets {
            if !seen.insert(field.as_str()) {
                return Err(DriverError::InvalidAction(format!(
                    "parameter {} has ambiguous offset semantic {field}",
                    parameter.name
                )));
            }
            let declared = match profile_codec::scalar_offset(frame, field) {
                Ok(declared) => declared,
                Err(_) if self.canonical_orion_identity && field.starts_with("offset_") => continue,
                Err(error) => return Err(error),
            };
            if declared != *offset {
                return Err(DriverError::InvalidAction(format!(
                    "parameter {} semantic {field} offset {offset} differs from typed operation {declared}",
                    parameter.name
                )));
            }
        }
        for field in required {
            if !seen.contains(field)
                && !(self.canonical_orion_identity
                    && field == &"value"
                    && seen
                        .iter()
                        .any(|candidate| candidate.starts_with("offset_")))
            {
                return Err(DriverError::InvalidAction(format!(
                    "parameter {} missing semantic offset {field}",
                    parameter.name
                )));
            }
        }
        Ok(())
    }

    fn validate_capabilities(&self) -> Result<(), DriverError> {
        let command = self.frame("command")?;
        let global = self.frame("global_command")?;
        for parameter in self
            .profile
            .params
            .iter()
            .filter(|parameter| parameter.id.is_some())
        {
            let frame = if parameter.applies_to == "globals" {
                global
            } else if parameter.applies_to == "mixers" {
                self.frame("mix_command")?
            } else {
                command
            };
            let parameter_field = Self::scalar_alias(frame, "param_id", "parameter")?;
            let target_field = Self::scalar_alias(frame, "channel", "target").ok();
            let required: Vec<&str> = if parameter.applies_to == "globals" {
                // Older global references used offset_0 for parameter id;
                // command frame remains authoritative for its actual fields.
                vec!["value"]
            } else {
                vec![
                    parameter_field,
                    target_field.ok_or_else(|| {
                        DriverError::InvalidAction(format!(
                            "frame {} missing channel or target semantic",
                            frame.id
                        ))
                    })?,
                    "value",
                ]
            };
            if !(self.canonical_orion_identity
                && parameter
                    .frame
                    .offsets
                    .iter()
                    .all(|(field, _)| field.starts_with("offset_")))
            {
                self.validate_reference(frame, parameter, &required)?;
            }
        }
        let mix = self.frame("mix_command")?;
        Self::scalar_alias(mix, "mix", "surface")?;
        Self::scalar_alias(mix, "channel", "strip")?;
        profile_codec::scalar_offset(mix, "fader")?;
        if profile_codec::scalar_offset(mix, "pan_flags").is_err() {
            for field in ["pan", "mute", "solo"] {
                let mut matches = mix.operations.iter().filter(|operation| {
                    matches!(operation, FrameOperation::BitField { field: candidate, .. } if candidate == field)
                });
                if matches.next().is_none() || matches.next().is_some() {
                    return Err(DriverError::InvalidAction(format!(
                        "mix frame missing or ambiguous bit semantic {field}"
                    )));
                }
            }
        }
        for parameter in ["mix_fader", "mix_pan", "mix_mute", "mix_solo"] {
            self.parameter("mixers", parameter)?;
        }
        let send_parameter = self.parameter("mixers", "mix_send").is_ok();
        let send_operation = profile_codec::scalar_offset(mix, "send").is_ok();
        if send_parameter != send_operation {
            return Err(DriverError::InvalidAction(
                "atomic mixer send parameter and operation availability differ".into(),
            ));
        }
        let link = self.frame("link_command")?;
        Self::scalar_alias(link, "space", "surface")?;
        profile_codec::scalar_offset(link, "enabled")?;
        let pair_field = if link.operations.iter().any(|operation| {
            matches!(operation, FrameOperation::PairIndex { pair_field, .. } if pair_field == "pair_index")
        }) {
            "pair_index"
        } else {
            "pair"
        };
        let mut pair = link
            .operations
            .iter()
            .filter_map(|operation| match operation {
                FrameOperation::PairIndex {
                    pair_field: candidate,
                    max_index: Some(max_index),
                    ..
                } if candidate == pair_field => Some(*max_index),
                _ => None,
            });
        let pair_max = pair.next().ok_or_else(|| {
            DriverError::InvalidAction("link frame missing finite pair mapping".into())
        })?;
        if pair.next().is_some() {
            return Err(DriverError::InvalidAction(
                "link frame has ambiguous pair mapping".into(),
            ));
        }
        let declared_max = self
            .profile
            .link_domains
            .iter()
            .map(|domain| domain.pair_count - 1)
            .max()
            .ok_or_else(|| DriverError::InvalidAction("profile has no link domain".into()))?;
        if declared_max != pair_max {
            return Err(DriverError::InvalidAction(format!(
                "link frame pair mapping 0..={pair_max} differs from declared domain 0..={declared_max}"
            )));
        }
        let routing = self.frame("routing_command")?;
        profile_codec::scalar_offset(routing, "destination")?;
        let legacy_routing = routing.operations.iter().any(|operation| {
            matches!(operation, FrameOperation::Indexed { index_field, width: 2, max_index: Some(max), .. } if index_field == "source_pair" && usize::from(*max) + 1 == self.routing_source_count)
        });
        let canonical_routing = routing.operations.iter().any(|operation| {
            matches!(operation, FrameOperation::Indexed { index_field, width: 1, max_index: Some(max), .. } if index_field == "channel" && usize::from(*max) + 1 == self.routing_source_count)
        });
        if legacy_routing == canonical_routing {
            return Err(DriverError::InvalidAction(
                "routing frame missing or ambiguous complete source mapping".into(),
            ));
        }
        let readback_frame = self.frame("readback")?;
        for (field, expected_width, expected_count) in [
            (
                "mixer_slot",
                if send_parameter { 3_u8 } else { 2_u8 },
                33_usize,
            ),
            ("routing_source_pair", 2_u8, self.routing_source_count),
        ] {
            let typed_count = readback_frame
                .operations
                .iter()
                .filter(|operation| {
                    matches!(operation, FrameOperation::Indexed { index_field, .. } if index_field == field)
                })
                .count();
            let valid_count = readback_frame
                .operations
                .iter()
                .filter(|operation| {
                    matches!(operation, FrameOperation::Indexed { index_field, width, max_index: Some(max), .. } if index_field == field && *width == expected_width && usize::from(*max) + 1 == expected_count)
                })
                .count();
            let implicit = (field == "mixer_slot" && expected_count == 33)
                || (field == "routing_source_pair" && expected_count == self.routing_source_count);
            if (typed_count == 0 && !implicit)
                || (typed_count > 0 && (typed_count != 1 || valid_count != 1))
            {
                return Err(DriverError::InvalidAction(format!(
                    "readback frame mapping {field} count is {valid_count}"
                )));
            }
        }
        if let Some(required) = self.profile.constraints.iter().find(|constraint| {
            constraint.name == "required_whole_state_operations"
                && profile_codec::is_confirmed(&constraint.status)
        }) {
            for operation in &required.values {
                let operation = u16::try_from(*operation).map_err(|_| {
                    DriverError::InvalidAction("whole-state operation outside u16".into())
                })?;
                self.validate_whole_state_operation(operation)?;
            }
        }
        Ok(())
    }

    fn whole_state_frame(&self, operation: u16) -> Result<&RuntimeFrame, DriverError> {
        let operation = u8::try_from(operation).map_err(|_| {
            DriverError::InvalidAction(format!("whole-state operation {operation} exceeds byte"))
        })?;
        let mut frames = self.profile.frames.iter().filter(|frame| {
            frame.kind.eq_ignore_ascii_case("command")
                && profile_codec::is_confirmed(&frame.status)
                && profile_codec::fixed_byte(frame, 16) == Some(operation)
        });
        let frame = frames.next().ok_or_else(|| {
            DriverError::UnsupportedAction(format!(
                "confirmed whole-state operation {operation:#04x}"
            ))
        })?;
        if frames.next().is_some() {
            return Err(DriverError::InvalidAction(format!(
                "ambiguous whole-state operation {operation:#04x}"
            )));
        }
        Ok(frame)
    }

    fn whole_state_field_ids(&self, operation: u16) -> Result<Vec<u16>, DriverError> {
        let name = format!("whole_state.{operation}.field_ids");
        let constraint = self
            .profile
            .constraints
            .iter()
            .find(|constraint| {
                constraint.name == name && profile_codec::is_confirmed(&constraint.status)
            })
            .ok_or_else(|| {
                DriverError::InvalidAction(format!(
                    "whole-state operation {operation:#04x} has no confirmed complete field set"
                ))
            })?;
        let fields: Vec<u16> = constraint
            .values
            .iter()
            .map(|value| {
                u16::try_from(*value).map_err(|_| {
                    DriverError::InvalidAction(format!(
                        "whole-state operation {operation:#04x} field id outside u16"
                    ))
                })
            })
            .collect::<Result<_, _>>()?;
        let unique: HashSet<_> = fields.iter().copied().collect();
        if fields.is_empty() || fields.len() != unique.len() {
            return Err(DriverError::InvalidAction(format!(
                "whole-state operation {operation:#04x} field set is empty or ambiguous"
            )));
        }
        Ok(fields)
    }

    fn validate_whole_state_operation(&self, operation: u16) -> Result<(), DriverError> {
        let frame = self.whole_state_frame(operation)?;
        for semantic in ["target", "enabled"] {
            profile_codec::scalar_offset(frame, semantic)?;
        }
        for field in self.whole_state_field_ids(operation)? {
            profile_codec::scalar_offset(frame, &format!("field_{field}"))?;
            let name = format!("whole_state.{operation}.field.{field}");
            let constraint = self.profile.constraints.iter().find(|constraint| {
                constraint.name == name && profile_codec::is_confirmed(&constraint.status)
            });
            if constraint.and_then(|constraint| constraint.range).is_none() {
                return Err(DriverError::InvalidAction(format!(
                    "whole-state operation {operation:#04x} field {field} has no confirmed range"
                )));
            }
        }
        Ok(())
    }

    fn input(&self, address: InputAddress) -> Result<&str, DriverError> {
        self.profile
            .inputs
            .iter()
            .find(|input| input.space_id == address.space && input.index == address.index)
            .map(|input| input.space.as_str())
            .ok_or_else(|| {
                DriverError::InvalidAction(format!(
                    "input address {}:{} outside profile",
                    address.space, address.index
                ))
            })
    }

    fn output(&self, address: OutputAddress) -> Result<(), DriverError> {
        self.profile
            .outputs
            .iter()
            .any(|output| output.id == address.id)
            .then_some(())
            .ok_or_else(|| {
                DriverError::InvalidAction(format!("output id {} outside profile", address.id))
            })
    }

    fn mixer(&self, address: MixerAddress) -> Result<(), DriverError> {
        let mixer = self
            .profile
            .mixers
            .iter()
            .find(|mixer| mixer.mix_index == address.surface)
            .ok_or_else(|| {
                DriverError::InvalidAction(format!(
                    "mixer surface {} outside profile",
                    address.surface
                ))
            })?;
        if address.strip <= mixer.strip_count {
            Ok(())
        } else {
            Err(DriverError::InvalidAction(format!(
                "mixer strip {} outside 0..={}",
                address.strip, mixer.strip_count
            )))
        }
    }

    fn checked_value(
        &self,
        parameter: &RuntimeParam,
        value: ControlValue,
    ) -> Result<i32, DriverError> {
        profile_codec::value_i32(
            value,
            &parameter.value_type,
            parameter.range,
            &parameter.values,
            &parameter.name,
        )
    }

    fn encode_parameter(
        &self,
        frame_id: &str,
        parameter: &RuntimeParam,
        target: Option<u16>,
        value: ControlValue,
    ) -> Result<CommandBatch, DriverError> {
        let id = parameter.id.ok_or_else(|| {
            DriverError::UnsupportedAction(format!(
                "parameter {} has no command id",
                parameter.name
            ))
        })?;
        let frame = self.frame(frame_id)?;
        let parameter_field = Self::scalar_alias(frame, "param_id", "parameter")?;
        let target_field = Self::scalar_alias(frame, "channel", "target").ok();
        let value = self.checked_value(parameter, value)?;
        if let Some(target) = target {
            if let Some(domain) = self.profile.constraints.iter().find(|constraint| {
                constraint.name == format!("parameter_target.{}", parameter.name)
                    && profile_codec::is_confirmed(&constraint.status)
            }) {
                if !domain.values.contains(&i32::from(target)) {
                    return Err(DriverError::InvalidAction(format!(
                        "parameter {} target {target} outside confirmed domain",
                        parameter.name
                    )));
                }
            }
        }
        let mut bytes = profile_codec::allocate(&self.profile, frame)?;
        profile_codec::write_scalar(frame, &mut bytes, parameter_field, i32::from(id))?;
        if let Some(target) = target {
            let target_field = target_field.ok_or_else(|| {
                DriverError::InvalidAction(format!(
                    "frame {} missing channel or target semantic",
                    frame.id
                ))
            })?;
            profile_codec::write_scalar(frame, &mut bytes, target_field, i32::from(target))?;
        }
        profile_codec::write_scalar(frame, &mut bytes, "value", value)?;
        Ok(CommandBatch {
            frames: vec![bytes],
            refresh_requests: Vec::new(),
        })
    }

    fn encode_complete_mixer(
        &self,
        address: MixerAddress,
        fader: i32,
        pan: i32,
        muted: bool,
        soloed: bool,
        send: Option<i32>,
    ) -> Result<CommandBatch, DriverError> {
        self.mixer(address)?;
        let send_parameter = self.parameter("mixers", "mix_send").ok();
        let send = match (send_parameter, send) {
            (Some(parameter), Some(value)) => {
                Some(self.checked_value(parameter, ControlValue::Int(value))?)
            }
            (Some(_), None) => {
                return Err(DriverError::InvalidAction(
                    "atomic mixer frame requires complete send value".into(),
                ))
            }
            (None, Some(_)) => {
                return Err(DriverError::InvalidAction(
                    "atomic mixer frame has no send field".into(),
                ))
            }
            (None, None) => None,
        };
        let fader = self.checked_value(
            self.parameter("mixers", "mix_fader")?,
            ControlValue::Int(fader),
        )?;
        let pan =
            self.checked_value(self.parameter("mixers", "mix_pan")?, ControlValue::Int(pan))?;
        let muted = self.checked_value(
            self.parameter("mixers", "mix_mute")?,
            ControlValue::Bool(muted),
        )?;
        let soloed = self.checked_value(
            self.parameter("mixers", "mix_solo")?,
            ControlValue::Bool(soloed),
        )?;
        let frame = self.frame("mix_command")?;
        let mut bytes = profile_codec::allocate(&self.profile, frame)?;
        let mix_field = Self::scalar_alias(frame, "mix", "surface")?;
        let channel_field = Self::scalar_alias(frame, "channel", "strip")?;
        profile_codec::write_scalar(frame, &mut bytes, mix_field, i32::from(address.surface))?;
        profile_codec::write_scalar(frame, &mut bytes, channel_field, i32::from(address.strip))?;
        profile_codec::write_scalar(frame, &mut bytes, "fader", fader)?;
        if profile_codec::scalar_offset(frame, "pan_flags").is_ok() {
            let pan = self
                .profile
                .mixer(address.surface)
                .and_then(|mixer| mixer.pan_raw_from_value(pan))
                .ok_or_else(|| {
                    DriverError::InvalidAction(format!(
                        "mixer surface {} pan value {} outside profile domain",
                        address.surface, pan
                    ))
                })?;
            let pan_flags =
                i32::from(pan.raw()) | (i32::from(muted) << 6) | (i32::from(soloed) << 7);
            profile_codec::write_scalar(frame, &mut bytes, "pan_flags", pan_flags)?;
        } else {
            profile_codec::write_bit_field(frame, &mut bytes, "pan", pan)?;
            profile_codec::write_bit_field(frame, &mut bytes, "mute", muted)?;
            profile_codec::write_bit_field(frame, &mut bytes, "solo", soloed)?;
        }
        if let Some(send) = send {
            profile_codec::write_scalar(frame, &mut bytes, "send", send)?;
        }
        Ok(CommandBatch {
            frames: vec![bytes],
            refresh_requests: Vec::new(),
        })
    }

    fn encode_routing_group(
        &self,
        destination: u16,
        changed_channel: Option<u16>,
        sources: Vec<RoutingSource>,
    ) -> Result<CommandBatch, DriverError> {
        if changed_channel.is_some() {
            return Err(DriverError::InvalidAction(
                "profile routing does not use a changed-channel hint".into(),
            ));
        }
        let group = self
            .profile
            .routing_groups
            .iter()
            .find(|group| group.destination == destination)
            .ok_or_else(|| {
                DriverError::InvalidAction(format!(
                    "routing destination {destination} outside profile"
                ))
            })?;
        if sources.len() != usize::from(group.channel_count) {
            return Err(DriverError::InvalidAction(format!(
                "routing destination {destination} requires exactly {} sources",
                group.channel_count
            )));
        }
        let frame = self.frame("routing_command")?;
        let mut bytes = profile_codec::allocate(&self.profile, frame)?;
        profile_codec::write_scalar(frame, &mut bytes, "destination", i32::from(destination))?;
        for (channel, source) in sources.into_iter().enumerate() {
            let domain = group
                .source_domains
                .iter()
                .find(|domain| domain.bank == source.bank)
                .ok_or_else(|| {
                    DriverError::InvalidAction(format!(
                        "routing destination {destination} channel {channel} source bank {} is unavailable",
                        source.bank
                    ))
                })?;
            if source.index >= domain.index_count {
                return Err(DriverError::InvalidAction(format!(
                    "routing destination {destination} channel {channel} source {}:{} outside 0..{}",
                    source.bank,
                    source.index,
                    domain.index_count - 1
                )));
            }
            let channel = u16::try_from(channel)
                .map_err(|_| DriverError::InvalidAction("routing channel overflow".into()))?;
            let source_index = u8::try_from(source.index).map_err(|_| {
                DriverError::InvalidAction("routing source index exceeds byte".into())
            })?;
            if frame.operations.iter().any(|operation| {
                matches!(operation, FrameOperation::Indexed { index_field, width: 1, .. } if index_field == "channel")
            }) {
                profile_codec::write_indexed_bytes(
                    frame,
                    &mut bytes,
                    "channel",
                    channel,
                    &[source.bank],
                )?;
                let (base, stride) = frame
                    .operations
                    .iter()
                    .find_map(|operation| match operation {
                        FrameOperation::Indexed { base, stride, index_field, width: 1, .. }
                            if index_field == "channel" => Some((*base, *stride)),
                        _ => None,
                    })
                    .ok_or_else(|| DriverError::InvalidAction("routing channel mapping missing".into()))?;
                let offset = usize::from(base)
                    .checked_add(usize::from(stride) * usize::from(channel))
                    .and_then(|offset| offset.checked_add(1))
                    .ok_or_else(|| DriverError::InvalidAction("routing channel offset overflow".into()))?;
                *bytes.get_mut(offset).ok_or_else(|| {
                    DriverError::InvalidAction("routing source index outside report".into())
                })? = source_index;
            } else {
                profile_codec::write_indexed_bytes(frame, &mut bytes, "source_pair", channel, &[source.bank, source_index])?;
            }
        }
        Ok(CommandBatch {
            frames: vec![bytes],
            refresh_requests: Vec::new(),
        })
    }

    fn encode_whole_state(
        &self,
        operation: u16,
        target: u16,
        enabled: bool,
        fields: Vec<crate::driver::WholeStateField>,
    ) -> Result<CommandBatch, DriverError> {
        self.validate_whole_state_operation(operation)?;
        let frame = self.whole_state_frame(operation)?;
        let required = self.whole_state_field_ids(operation)?;
        let supplied: HashSet<_> = fields.iter().map(|field| field.id).collect();
        let required_set: HashSet<_> = required.iter().copied().collect();
        if supplied.len() != fields.len() || supplied != required_set {
            return Err(DriverError::InvalidAction(format!(
                "whole-state operation {operation:#04x} requires exactly fields {required:?}"
            )));
        }
        let mut bytes = profile_codec::allocate(&self.profile, frame)?;
        profile_codec::write_scalar(frame, &mut bytes, "target", i32::from(target))?;
        profile_codec::write_scalar(frame, &mut bytes, "enabled", i32::from(enabled))?;
        for field in fields {
            let constraint_name = format!("whole_state.{operation}.field.{}", field.id);
            let (minimum, maximum) = self
                .profile
                .constraints
                .iter()
                .find(|constraint| {
                    constraint.name == constraint_name
                        && profile_codec::is_confirmed(&constraint.status)
                })
                .and_then(|constraint| constraint.range)
                .ok_or_else(|| {
                    DriverError::InvalidAction(format!(
                        "whole-state operation {operation:#04x} field {} has no range",
                        field.id
                    ))
                })?;
            if !(minimum..=maximum).contains(&field.value) {
                return Err(DriverError::InvalidAction(format!(
                    "whole-state operation {operation:#04x} field {} value {} outside {minimum}..={maximum}",
                    field.id, field.value
                )));
            }
            profile_codec::write_scalar(
                frame,
                &mut bytes,
                &format!("field_{}", field.id),
                field.value,
            )?;
        }
        Ok(CommandBatch {
            frames: vec![bytes],
            refresh_requests: Vec::new(),
        })
    }

    fn empty_strip(strip: u16, name: String) -> DynamicMixerStrip {
        DynamicMixerStrip {
            strip,
            name,
            fader: None,
            pan: None,
            send: None,
            muted: None,
            soloed: None,
            linked: None,
            meter: None,
            parameters: Vec::new(),
        }
    }

    fn mapped_meters(&self, frame_id: &str, bytes: &[u8]) -> Vec<DynamicMeterState> {
        self.profile
            .meter_mappings
            .iter()
            .filter(|mapping| mapping.frame_id == frame_id)
            .filter_map(|mapping| {
                bytes
                    .get(mapping.offset)
                    .copied()
                    .map(|value| DynamicMeterState {
                        target: mapping.target,
                        target_index: mapping.target_index,
                        lane: mapping.lane,
                        value,
                    })
            })
            .collect()
    }

    fn topology_state(&self) -> DynamicDeviceState {
        DynamicDeviceState {
            globals: Vec::new(),
            inputs: self
                .profile
                .inputs
                .iter()
                .map(|input| DynamicInputState {
                    address: InputAddress {
                        space: input.space_id,
                        index: input.index,
                    },
                    name: input.name.clone(),
                    mode: None,
                    gain: None,
                    phantom: None,
                    phase: None,
                    meter: None,
                    parameters: Vec::new(),
                })
                .collect(),
            outputs: self
                .profile
                .outputs
                .iter()
                .map(|output| DynamicOutputState {
                    address: OutputAddress { id: output.id },
                    name: output.name.clone(),
                    level: None,
                    muted: None,
                    dimmed: None,
                    parameters: Vec::new(),
                })
                .collect(),
            mixers: self
                .profile
                .mixers
                .iter()
                .map(|mixer| DynamicMixerSurface {
                    surface: mixer.mix_index,
                    name: mixer.name.clone(),
                    master: mixer
                        .has_master
                        .then(|| Self::empty_strip(0, "Master".into())),
                    strips: (1..=mixer.strip_count)
                        .map(|strip| Self::empty_strip(strip, format!("CH {strip:02}")))
                        .collect(),
                })
                .collect(),
            meters: Vec::new(),
            routing: Vec::new(),
            zen_go_compatibility: None,
        }
    }

    fn indexed_layout(
        &self,
        frame: &RuntimeFrame,
        field: &str,
    ) -> Result<(usize, usize, usize, u16), DriverError> {
        let mut matches = frame
            .operations
            .iter()
            .filter_map(|operation| match operation {
                FrameOperation::Indexed {
                    base,
                    stride,
                    index_field,
                    width,
                    max_index: Some(max),
                } if index_field == field
                    || (field == "output_bus" && index_field == "bytes_per_bus") =>
                {
                    Some((
                        usize::from(*base),
                        usize::from(*stride),
                        if field == "output_bus" && index_field == "bytes_per_bus" {
                            3
                        } else {
                            usize::from(*width)
                        },
                        *max,
                    ))
                }
                _ => None,
            });
        let result = match matches.next() {
            Some(result) => result,
            None if frame.id == "readback" && field == "mixer_slot" => (16, 3, 3, 32),
            None if frame.id == "readback" && field == "routing_source_pair" => {
                // Readback record starts with destination id at data_offset;
                // source pairs follow that one-byte record header.
                (17, 2, 2, self.routing_source_count.saturating_sub(1) as u16)
            }
            None => {
                return Err(DriverError::InvalidAction(format!(
                    "{} missing indexed layout {field}",
                    frame.id
                )))
            }
        };
        if matches.next().is_some() {
            return Err(DriverError::InvalidAction(format!(
                "readback ambiguous indexed layout {field}"
            )));
        }
        Ok(result)
    }

    fn state_layout(
        frame: &RuntimeFrame,
        field: &str,
        base_field: &str,
        count: u16,
    ) -> Result<(usize, usize, usize, u16), DriverError> {
        let indexed = frame
            .operations
            .iter()
            .filter_map(|operation| match operation {
                FrameOperation::Indexed {
                    base,
                    stride,
                    index_field,
                    width,
                    max_index: Some(max),
                } if index_field == field => Some((
                    usize::from(*base),
                    usize::from(*stride),
                    usize::from(*width),
                    *max,
                )),
                _ => None,
            });
        let mut indexed = indexed;
        if let Some(layout) = indexed.next() {
            if indexed.next().is_some() {
                return Err(DriverError::InvalidAction(format!(
                    "{} ambiguous indexed layout {field}",
                    frame.id
                )));
            }
            return Ok(layout);
        }
        let base = profile_codec::scalar_offset(frame, base_field)?;
        count.checked_sub(1).map_or_else(
            || {
                Err(DriverError::InvalidAction(format!(
                    "{field} has empty finite domain"
                )))
            },
            |max| Ok((usize::from(base), 1, 1, max)),
        )
    }

    fn state_bit_value(frame: &RuntimeFrame, field: &str, byte: u8) -> Result<i32, DriverError> {
        if let Ok(value) = Self::bit_value_from(frame, field, byte) {
            return Ok(value);
        }
        let (mask, shift) = match field {
            "input_mode" => (0x03, 0),
            "input_phantom" => (0x10, 4),
            "input_phase" => (0x40, 6),
            "output_mute" => (0x04, 2),
            "output_dim" => (0x08, 3),
            _ => {
                return Err(DriverError::InvalidAction(format!(
                    "missing state bit mapping {field}"
                )))
            }
        };
        Ok(i32::from((byte & mask) >> shift))
    }

    fn bit_value_from(frame: &RuntimeFrame, field: &str, byte: u8) -> Result<i32, DriverError> {
        let mut matches = frame
            .operations
            .iter()
            .filter_map(|operation| match operation {
                FrameOperation::BitField {
                    field: candidate,
                    mask,
                    shift,
                    ..
                } if candidate == field => Some((*mask, *shift)),
                _ => None,
            });
        let (mask, shift) = matches.next().ok_or_else(|| {
            DriverError::InvalidAction(format!("missing mixer bit mapping {field}"))
        })?;
        if matches.next().is_some() {
            return Err(DriverError::InvalidAction(format!(
                "ambiguous mixer bit mapping {field}"
            )));
        }
        Ok(i32::from((byte & mask) >> shift))
    }

    fn bit_value(&self, field: &str, byte: u8) -> Result<i32, DriverError> {
        Self::bit_value_from(self.frame("mix_command")?, field, byte)
    }

    fn scalar_value(frame: &RuntimeFrame, bytes: &[u8], field: &str) -> Result<u8, DriverError> {
        let semantic = match field {
            "sample_rate" if profile_codec::scalar_offset(frame, field).is_err() => {
                "sample_rate_byte_offset"
            }
            "brightness" if profile_codec::scalar_offset(frame, field).is_err() => {
                "screen_brightness_byte_offset"
            }
            _ => field,
        };
        let offset = profile_codec::scalar_offset(frame, semantic)?;
        bytes
            .get(usize::from(offset))
            .copied()
            .ok_or_else(|| DriverError::InvalidAction(format!("state scalar {field} is truncated")))
    }

    fn constraint_values(&self, name: &str) -> Option<&[i32]> {
        self.profile
            .constraints
            .iter()
            .find(|constraint| {
                constraint.name == name && profile_codec::is_confirmed(&constraint.status)
            })
            .map(|constraint| constraint.values.as_slice())
    }

    fn decode_state(&self, bytes: &[u8]) -> Result<DynamicDeviceState, DriverError> {
        let frame = self.frame("state_report")?;
        let mut state = self.topology_state();
        state.meters = self.mapped_meters("state_report", bytes);
        let layouts = [
            ("physical_gain", "gain_base", "physical_inputs"),
            ("physical_status", "status_base", "physical_inputs"),
            ("adat_gain", "adat_gain_base", "adat_inputs"),
            ("spdif_gain", "spdif_gain_base", "spdif_inputs"),
        ];
        for (field, base_field, space_name) in layouts {
            let space = self
                .profile
                .address_spaces
                .iter()
                .find(|space| space.id == space_name)
                .ok_or_else(|| {
                    DriverError::InvalidAction(format!("missing state address space {space_name}"))
                })?;
            let expected = space.count.ok_or_else(|| {
                DriverError::InvalidAction(format!("state address space {space_name} is unbounded"))
            })?;
            let (base, stride, width, max) =
                Self::state_layout(frame, field, base_field, expected)?;
            if width != 1 {
                return Err(DriverError::InvalidAction(format!(
                    "state field {field} record width must be one"
                )));
            }
            if u32::from(max) + 1 != u32::from(expected) {
                return Err(DriverError::InvalidAction(format!(
                    "state field {field} does not cover {space_name}"
                )));
            }
            for index in 0..expected {
                let offset = base + stride * usize::from(index);
                let raw = *bytes.get(offset).ok_or_else(|| {
                    DriverError::InvalidAction(format!(
                        "state field {field} index {index} is truncated"
                    ))
                })?;
                let input = state
                    .inputs
                    .iter_mut()
                    .find(|input| {
                        input.address.space == space.space_id && input.address.index == index
                    })
                    .ok_or_else(|| {
                        DriverError::InvalidAction(format!(
                            "state field {field} index {index} has no input"
                        ))
                    })?;
                match field {
                    "physical_gain" | "adat_gain" | "spdif_gain" => {
                        input.gain = Some(i32::from(raw as i8));
                    }
                    "physical_status" => {
                        input.mode = Some(Self::state_bit_value(frame, "input_mode", raw)?);
                        input.phantom =
                            Some(Self::state_bit_value(frame, "input_phantom", raw)? != 0);
                        input.phase = Some(Self::state_bit_value(frame, "input_phase", raw)? != 0);
                    }
                    _ => unreachable!(),
                }
            }
        }

        if matches!(self.meter_source, MeterSource::StateReport) {
            let physical_space = self
                .profile
                .address_spaces
                .iter()
                .find(|space| space.id == "physical_inputs")
                .ok_or_else(|| DriverError::InvalidAction("missing physical meter space".into()))?;
            let physical_count = physical_space.count.ok_or_else(|| {
                DriverError::InvalidAction("physical meter space is unbounded".into())
            })?;
            if physical_count == 0
                || self
                    .profile
                    .inputs
                    .iter()
                    .filter(|input| input.space_id == physical_space.space_id)
                    .count()
                    != usize::from(physical_count)
            {
                return Err(DriverError::InvalidAction(
                    "physical meter count does not match finite physical inputs".into(),
                ));
            }
            let (meter_base, meter_stride, meter_width, meter_max) =
                self.indexed_layout(frame, "physical_meter")?;
            if meter_width != 1 || u32::from(meter_max) + 1 != u32::from(physical_count) {
                return Err(DriverError::InvalidAction(
                    "state physical meter layout is incomplete".into(),
                ));
            }
            for index in 0..physical_count {
                let offset = meter_base + meter_stride * usize::from(index);
                let meter = *bytes.get(offset).ok_or_else(|| {
                    DriverError::InvalidAction(format!("state physical meter {index} is truncated"))
                })?;
                let input = state
                    .inputs
                    .iter_mut()
                    .find(|input| {
                        input.address.space == physical_space.space_id
                            && input.address.index == index
                    })
                    .ok_or_else(|| {
                        DriverError::InvalidAction(format!(
                            "state physical meter {index} has no input"
                        ))
                    })?;
                input.meter = Some(meter);
            }
        }

        let (base, stride, width, max) = self.indexed_layout(frame, "output_bus")?;
        if width != 3 || usize::from(max) + 1 != state.outputs.len() {
            return Err(DriverError::InvalidAction(
                "state output-bus layout is incomplete".into(),
            ));
        }
        let mute_targets = self.constraint_values("state_output_mute_targets");
        let dim_targets = self.constraint_values("state_output_dim_targets");
        for output in &mut state.outputs {
            let start = base + stride * usize::from(output.address.id);
            let record = bytes.get(start..start + width).ok_or_else(|| {
                DriverError::InvalidAction(format!(
                    "state output bus {} record is truncated",
                    output.address.id
                ))
            })?;
            output.level = Some(i32::from(record[0]));
            let status = record[1];
            if mute_targets.is_some_and(|targets| targets.contains(&i32::from(output.address.id))) {
                // Canonical status bit is ambiguous at maximum level; never report
                // a false mute value from that state.
                output.muted = (record[0] != 96)
                    .then(|| Self::state_bit_value(frame, "output_mute", status).map(|v| v != 0))
                    .transpose()?;
            }
            if dim_targets.is_some_and(|targets| targets.contains(&i32::from(output.address.id))) {
                output.dimmed = Some(Self::state_bit_value(frame, "output_dim", status)? != 0);
            }
        }
        state.globals = vec![
            DynamicGlobalState {
                control: GlobalControl::SampleRate,
                value: ControlValue::Enum(i32::from(Self::scalar_value(
                    frame,
                    bytes,
                    "sample_rate",
                )?)),
            },
            DynamicGlobalState {
                control: GlobalControl::Parameter(0x0e),
                value: ControlValue::Int(i32::from(Self::scalar_value(
                    frame,
                    bytes,
                    "brightness",
                )?)),
            },
        ];
        Ok(state)
    }

    fn decode_meter(&self, bytes: &[u8]) -> Result<Vec<DynamicInputState>, DriverError> {
        let frame = self.frame("meter_report")?;
        let space = self
            .profile
            .address_spaces
            .iter()
            .find(|space| space.id == "physical_inputs")
            .ok_or_else(|| DriverError::InvalidAction("missing physical meter space".into()))?;
        let count = space.count.ok_or_else(|| {
            DriverError::InvalidAction("physical meter space is unbounded".into())
        })?;
        let (base, stride, width, max) =
            Self::state_layout(frame, "physical_meter", "channel_meter_base", count)?;
        if width != 1 || u32::from(max) + 1 != u32::from(count) {
            return Err(DriverError::InvalidAction(
                "physical meter layout is incomplete".into(),
            ));
        }
        (0..count)
            .map(|index| {
                let offset = base + stride * usize::from(index);
                let meter = bytes.get(offset).copied().ok_or_else(|| {
                    DriverError::InvalidAction(format!("physical meter {index} is truncated"))
                })?;
                Ok(DynamicInputState {
                    address: InputAddress {
                        space: space.space_id,
                        index,
                    },
                    name: self
                        .profile
                        .inputs
                        .iter()
                        .find(|input| input.space_id == space.space_id && input.index == index)
                        .map(|input| input.name.clone())
                        .ok_or_else(|| {
                            DriverError::InvalidAction(format!(
                                "physical meter {index} has no input"
                            ))
                        })?,
                    mode: None,
                    gain: None,
                    phantom: None,
                    phase: None,
                    meter: Some(meter),
                    parameters: Vec::new(),
                })
            })
            .collect()
    }

    fn decode_mixer(
        &self,
        bytes: &[u8],
        surface_index: u8,
    ) -> Result<DynamicMixerSurface, DriverError> {
        let mixer = self
            .profile
            .mixers
            .iter()
            .find(|mixer| mixer.mix_index == surface_index)
            .ok_or_else(|| {
                DriverError::InvalidAction(format!(
                    "mixer readback category {:#04x} index {surface_index} has no surface",
                    self.mixer_readback_category
                ))
            })?;
        let readback_frame = self.frame("readback")?;
        let has_typed_layout = readback_frame.operations.iter().any(|operation| {
            matches!(operation, FrameOperation::Indexed { index_field, .. } if index_field == "mixer_slot")
        });
        let mix_command = self.frame("mix_command")?;
        let has_pan_flags = profile_codec::scalar_offset(mix_command, "pan_flags").is_ok();
        let (base, stride, width, max) = self.indexed_layout(readback_frame, "mixer_slot")?;
        let required_slots = usize::from(mixer.strip_count) + 1;
        let has_send = self.parameter("mixers", "mix_send").is_ok();
        let expected_width = if has_send || !has_typed_layout { 3 } else { 2 };
        if width != expected_width || usize::from(max) + 1 < required_slots {
            return Err(DriverError::InvalidAction(
                "mixer readback layout cannot cover complete surface".into(),
            ));
        }
        let mut decoded = Vec::with_capacity(required_slots);
        for slot in 0..required_slots {
            let start = base
                .checked_add(stride.checked_mul(slot).ok_or_else(|| {
                    DriverError::InvalidAction("mixer readback offset overflow".into())
                })?)
                .ok_or_else(|| {
                    DriverError::InvalidAction("mixer readback offset overflow".into())
                })?;
            let record = bytes.get(start..start + width).ok_or_else(|| {
                DriverError::InvalidAction(format!(
                    "truncated mixer readback category {:#04x} index {surface_index} slot {slot}",
                    self.mixer_readback_category
                ))
            })?;
            let pan = if has_pan_flags {
                mixer.pan_value_from_raw(PanState::from_raw(record[1] & 0x3f))
            } else {
                Some(self.bit_value("pan", record[1])?)
            };
            decoded.push(DynamicMixerStrip {
                strip: slot as u16,
                name: if slot == 0 {
                    "Master".into()
                } else {
                    format!("CH {slot:02}")
                },
                fader: Some(i32::from(record[0])),
                pan,
                send: has_send.then(|| i32::from(record[2])),
                muted: if has_pan_flags {
                    Some(record[1] & 0x40 != 0)
                } else {
                    Some(self.bit_value("mute", record[1])? != 0)
                },
                soloed: if has_pan_flags {
                    Some(record[1] & 0x80 != 0)
                } else {
                    Some(self.bit_value("solo", record[1])? != 0)
                },
                linked: None,
                meter: None,
                parameters: Vec::new(),
            });
        }
        let master = Some(decoded.remove(0));
        Ok(DynamicMixerSurface {
            surface: mixer.mix_index,
            name: mixer.name.clone(),
            master,
            strips: decoded,
        })
    }

    fn decode_routing(
        &self,
        bytes: &[u8],
        destination: u8,
    ) -> Result<DynamicRoutingGroup, DriverError> {
        let group = self
            .profile
            .routing_groups
            .iter()
            .find(|group| group.destination == u16::from(destination))
            .ok_or_else(|| {
                DriverError::InvalidAction(format!(
                    "routing readback category {:#04x} destination {destination} outside profile",
                    self.routing_readback_category
                ))
            })?;
        let readback_frame = self.frame("readback")?;
        let data_offset = self
            .profile
            .readback
            .as_ref()
            .ok_or_else(|| {
                DriverError::InvalidAction("routing readback definition missing".into())
            })?
            .data_offset;
        let has_typed_layout = readback_frame.operations.iter().any(|operation| {
            matches!(operation, FrameOperation::Indexed { index_field, .. } if index_field == "routing_source_pair")
        });
        let (base, stride, width, max) =
            self.indexed_layout(readback_frame, "routing_source_pair")?;
        if !has_typed_layout && bytes.get(usize::from(data_offset)) != Some(&destination) {
            return Err(DriverError::InvalidAction(
                "routing readback destination header mismatch".into(),
            ));
        }
        if width != 2 || usize::from(max) + 1 < usize::from(group.channel_count) {
            return Err(DriverError::InvalidAction(
                "routing readback layout is incomplete".into(),
            ));
        }
        let mut sources = Vec::with_capacity(usize::from(group.channel_count));
        for channel in 0..usize::from(group.channel_count) {
            let start = base + stride * channel;
            let record = bytes.get(start..start + width).ok_or_else(|| {
                DriverError::InvalidAction(format!("truncated routing readback category {:#04x} index {destination} channel {channel}", self.routing_readback_category))
            })?;
            let source = RoutingSource {
                bank: record[0],
                index: u16::from(record[1]),
            };
            if let Some(domain) = group
                .source_domains
                .iter()
                .find(|domain| domain.bank == source.bank)
            {
                if source.index >= domain.index_count {
                    return Err(DriverError::InvalidAction(format!("routing readback category {:#04x} index {destination} channel {channel} source {}:{} outside 0..{}", self.routing_readback_category, source.bank, source.index, domain.index_count - 1)));
                }
            } else if let Some(domain) = group
                .readback_source_domains
                .iter()
                .find(|domain| domain.bank == source.bank)
            {
                if !domain.indices.contains(&(source.index as u8)) {
                    return Err(DriverError::InvalidAction(format!("routing readback category {:#04x} index {destination} channel {channel} observed source {}:{} is not in the profile readback domain", self.routing_readback_category, source.bank, source.index)));
                }
            } else {
                return Err(DriverError::InvalidAction(format!("routing readback category {:#04x} index {destination} channel {channel} source bank {} is unavailable", self.routing_readback_category, source.bank)));
            }
            sources.push(source);
        }
        Ok(DynamicRoutingGroup {
            destination: u16::from(destination),
            name: group.name.clone(),
            sources,
        })
    }
}

impl DeviceDriver for ProfileDriver {
    fn definition(&self) -> &DriverDefinition {
        &self.definition
    }
    fn startup_requests(&self) -> &[QueryRequest] {
        &self.startup_requests
    }

    fn encode(&self, action: Action) -> Result<CommandBatch, DriverError> {
        match action {
            Action::Query(query) => {
                let readback = self
                    .profile
                    .readback
                    .as_ref()
                    .ok_or_else(|| DriverError::UnsupportedAction("profile readback".into()))?;
                Ok(CommandBatch {
                    frames: vec![profile_codec::encode_query(&self.profile, readback, query)?],
                    refresh_requests: Vec::new(),
                })
            }
            Action::SetInput {
                address,
                control,
                value,
            } => {
                let target = self.input(address)?.to_owned();
                let name = match control {
                    InputControl::Mode => "input_mode",
                    InputControl::Gain => match target.as_str() {
                        "physical_inputs" => "gain",
                        "adat_inputs" => "adat_gain",
                        "spdif_inputs" => "spdif_gain",
                        _ => "gain",
                    },
                    InputControl::Phantom => "phantom",
                    InputControl::Phase => "phase_invert",
                    InputControl::Parameter(id) => {
                        return self.encode_parameter(
                            "command",
                            self.parameter_by_id_for(id, &target)?,
                            Some(address.index),
                            value,
                        )
                    }
                };
                self.encode_parameter(
                    "command",
                    self.parameter(&target, name)?,
                    Some(address.index),
                    value,
                )
            }
            Action::SetOutput {
                address,
                control,
                value,
            } => {
                self.output(address)?;
                let name = match control {
                    OutputControl::Level => "bus_level",
                    OutputControl::Mute => "bus_mute",
                    OutputControl::Dim => "bus_dim",
                    OutputControl::Parameter(id) => {
                        return self.encode_parameter(
                            "command",
                            self.parameter_by_id_for(id, "outputs")?,
                            Some(address.id),
                            value,
                        )
                    }
                };
                self.encode_parameter(
                    "command",
                    self.parameter("outputs", name)?,
                    Some(address.id),
                    value,
                )
            }
            Action::SetMixer { .. } => Err(DriverError::UnsupportedAction(
                "compound profile mixer frame requires complete SetMixerStripState".into(),
            )),
            Action::SetMixerStripState {
                address,
                fader,
                pan,
                muted,
                soloed,
                send,
            } => self.encode_complete_mixer(address, fader, pan, muted, soloed, send),
            Action::SetLink {
                surface,
                pair,
                enabled,
            } => {
                let protocol_space = surface;
                let domain = self
                    .profile
                    .link_domains
                    .iter()
                    .find(|domain| domain.protocol_space == protocol_space)
                    .ok_or_else(|| {
                        DriverError::InvalidAction(format!(
                            "link domain {surface} is not declared as confirmed"
                        ))
                    })?;
                if pair >= domain.pair_count {
                    return Err(DriverError::InvalidAction(format!(
                        "link domain {surface} pair {pair} outside 0..{}",
                        domain.pair_count - 1
                    )));
                }
                let frame = self.frame("link_command")?;
                let mut bytes = profile_codec::allocate(&self.profile, frame)?;
                let space_field = Self::scalar_alias(frame, "space", "surface")?;
                profile_codec::write_scalar(frame, &mut bytes, space_field, i32::from(surface))?;
                let pair_field = if frame.operations.iter().any(|operation| {
                    matches!(operation, FrameOperation::PairIndex { pair_field, .. } if pair_field == "pair_index")
                }) {
                    "pair_index"
                } else {
                    "pair"
                };
                profile_codec::write_pair_index(frame, &mut bytes, pair_field, pair)?;
                profile_codec::write_scalar(frame, &mut bytes, "enabled", i32::from(enabled))?;
                Ok(CommandBatch {
                    frames: vec![bytes],
                    refresh_requests: Vec::new(),
                })
            }
            Action::SetRouting { .. } => Err(DriverError::UnsupportedAction(
                "atomic routing frame requires complete SetRoutingGroup".into(),
            )),
            Action::SetRoutingGroup {
                destination,
                changed_channel,
                sources,
            } => self.encode_routing_group(destination, changed_channel, sources),
            Action::SetGlobal { control, value } => {
                let name = match control {
                    GlobalControl::SampleRate => "sample_rate",
                    GlobalControl::ClockSource => "clock_source",
                    GlobalControl::Surface => "surface",
                    GlobalControl::Parameter(id) => {
                        return self.encode_parameter(
                            "global_command",
                            self.parameter_by_id_for(id, "globals")?,
                            None,
                            value,
                        )
                    }
                };
                self.encode_parameter(
                    "global_command",
                    self.parameter("globals", name)?,
                    None,
                    value,
                )
            }
            Action::SetWholeState {
                operation,
                target,
                enabled,
                fields,
            } => self.encode_whole_state(operation, target, enabled, fields),
        }
    }

    fn decode(&self, bytes: &[u8]) -> Result<Option<DeviceEvent>, DriverError> {
        let expected = profile_codec::report_size(&self.profile)?;
        if bytes.len() != expected {
            return Err(DriverError::InvalidAction(format!(
                "report length {} does not match {expected}; known record is truncated",
                bytes.len()
            )));
        }
        let state_frame = self.frame("state_report")?;
        let state_magic = profile_codec::fixed_byte(state_frame, 0);
        if state_magic == bytes.first().copied() {
            return Ok(Some(DeviceEvent::Snapshot {
                state: self.decode_state(bytes)?,
                raw: bytes.to_vec(),
            }));
        }
        let readback = self
            .profile
            .readback
            .as_ref()
            .ok_or_else(|| DriverError::UnsupportedAction("profile readback".into()))?;
        if bytes[0] == readback.response_magic {
            let discriminator = bytes[usize::from(readback.response_discriminator_offset)];
            let meter_discriminator = match self.meter_source {
                MeterSource::MeterReport => profile_codec::fixed_byte(
                    self.frame("meter_report")?,
                    readback.response_discriminator_offset,
                ),
                MeterSource::StateReport | MeterSource::Unavailable => {
                    self.frame_index
                        .get("meter_report")
                        .and_then(|index| self.profile.frames.get(*index))
                        .and_then(|frame| {
                            profile_codec::fixed_byte(frame, readback.response_discriminator_offset)
                                .or((self.canonical_orion_identity
                                    && profile_codec::fixed_byte(frame, 0)
                                        == Some(readback.response_magic)
                                    && frame
                                        .metadata
                                        .contains("\"status\":\"superseded_for_per_channel\""))
                                .then_some(0x1f))
                        })
                }
            };
            if matches!(self.meter_source, MeterSource::MeterReport)
                && meter_discriminator == Some(discriminator)
            {
                return Ok(Some(DeviceEvent::Meter {
                    inputs: self.decode_meter(bytes)?,
                    meters: self.mapped_meters("meter_report", bytes),
                    raw: bytes.to_vec(),
                }));
            }
            if discriminator != readback.response_discriminator {
                // State-report profiles may suppress only a superseded meter
                // response whose discriminator is proven by meter_report.
                if matches!(
                    self.meter_source,
                    MeterSource::StateReport | MeterSource::Unavailable
                ) && meter_discriminator == Some(discriminator)
                {
                    return Ok(None);
                }

                return Err(DriverError::InvalidAction(format!(
                    "invalid readback discriminator {discriminator:#04x}"
                )));
            }
            let readback_matches =
                self.frame("readback")?
                    .operations
                    .iter()
                    .all(|operation| match operation {
                        FrameOperation::FixedByte { offset, value } if *offset == 1 => {
                            bytes.get(usize::from(*offset)) == Some(value)
                        }
                        _ => true,
                    });
            if !readback_matches {
                for operation in &self.frame("readback")?.operations {
                    if let FrameOperation::FixedByte { offset, .. } = operation {
                        return Err(DriverError::InvalidAction(format!(
                            "invalid readback fixed byte at {offset}"
                        )));
                    }
                }
            }
            let category = bytes[usize::from(readback.category_offset)];
            let index = bytes[usize::from(readback.index_offset)];
            let count = match readback
                .category_counts
                .iter()
                .find(|bound| bound.category == category)
                .map(|bound| bound.count)
            {
                Some(count) => count,
                None => {
                    return Err(DriverError::InvalidAction(format!(
                        "unknown inbound readback category {category:#04x} index {index}"
                    )))
                }
            };
            if u16::from(index) >= count {
                return Err(DriverError::InvalidAction(format!(
                    "inbound readback category {category:#04x} index {index} outside count {count}"
                )));
            }
            let patch = if category == self.mixer_readback_category {
                Some(DynamicStatePatch::Mixer(self.decode_mixer(bytes, index)?))
            } else if category == self.routing_readback_category {
                Some(DynamicStatePatch::Routing(
                    self.decode_routing(bytes, index)?,
                ))
            } else {
                None
            };
            let body = bytes[usize::from(readback.data_offset)..].to_vec();
            return Ok(Some(DeviceEvent::QueryReply {
                query_id: category,
                sub_id: index,
                body,
                patch,
                raw: bytes.to_vec(),
            }));
        }
        Ok(None)
    }
}
