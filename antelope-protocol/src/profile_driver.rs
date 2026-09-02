//! Generic driver backed by a validated normalized runtime profile.

use std::collections::{HashMap, HashSet};

use crate::driver::{
    Action, CommandBatch, ControlValue, DeviceDriver, DeviceEvent, DriverDefinition, DriverError,
    DynamicDeviceState, DynamicGlobalState, DynamicInputState, DynamicMixerStrip,
    DynamicMixerSurface, DynamicOutputState, DynamicRoutingGroup, DynamicStatePatch, GlobalControl,
    InputAddress, InputControl, MixerAddress, OutputAddress, OutputControl, RoutingSource,
};
use crate::profile::{
    FrameOperation, RuntimeDriverKind, RuntimeEntry, RuntimeFrame, RuntimeParam, RuntimeProfile,
    RuntimeReadiness,
};
use crate::profile_codec;
use crate::QueryRequest;

#[derive(Debug)]
pub struct ProfileDriver {
    definition: DriverDefinition,
    profile: RuntimeProfile,
    startup_requests: Vec<QueryRequest>,
    parameter_index: HashMap<(String, String), usize>,
    frame_index: HashMap<String, usize>,
    mixer_readback_category: u8,
    routing_readback_category: u8,
    routing_source_count: usize,
}

impl ProfileDriver {
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
            profile_codec::validate_operations(frame, report_size)?;
            if frame.report_size.map(usize::from).unwrap_or(report_size) != report_size {
                return Err(DriverError::InvalidAction(format!(
                    "frame {} report geometry differs from transport",
                    frame.id
                )));
            }
        }
        for required in [
            "command",
            "global_command",
            "mix_command",
            "link_command",
            "routing_command",
            "state_report",
            "meter_report",
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
        for (decoder_kind, frame_id) in [
            ("state_report", "state_report"),
            ("meter_report", "meter_report"),
            ("readback", "readback"),
        ] {
            let count = entry
                .profile
                .decoders
                .iter()
                .filter(|decoder| {
                    decoder.frame_id == frame_id
                        && decoder.kind == decoder_kind
                        && profile_codec::is_confirmed(&decoder.status)
                })
                .count();
            if count != 1 {
                return Err(DriverError::InvalidAction(format!(
                    "required {decoder_kind} decoder mapping count is {count}"
                )));
            }
        }

        let mut parameter_index = HashMap::new();
        for (index, parameter) in entry.profile.params.iter().enumerate() {
            let key = (parameter.applies_to.clone(), parameter.name.clone());
            if parameter_index.insert(key.clone(), index).is_some() {
                return Err(DriverError::InvalidAction(format!(
                    "ambiguous parameter mapping {}/{}",
                    key.0, key.1
                )));
            }
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
        if fixed("state_report", 0).is_none()
            || fixed("readback", 0) != Some(readback.response_magic)
            || fixed("readback", readback.response_discriminator_offset)
                != Some(readback.response_discriminator)
            || fixed("meter_report", 0) != Some(readback.response_magic)
            || fixed("meter_report", readback.response_discriminator_offset).is_none()
            || fixed("meter_report", readback.response_discriminator_offset)
                == Some(readback.response_discriminator)
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

        let scalar_constraint = |name: &str| -> Result<i32, DriverError> {
            entry
                .profile
                .constraints
                .iter()
                .find(|constraint| {
                    constraint.name == name && profile_codec::is_confirmed(&constraint.status)
                })
                .and_then(|constraint| constraint.scalar)
                .ok_or_else(|| DriverError::InvalidAction(format!("missing confirmed {name}")))
        };
        let mixer_readback_category = u8::try_from(scalar_constraint("mixer_readback_category")?)
            .map_err(|_| {
            DriverError::InvalidAction("mixer readback category outside byte".into())
        })?;
        let routing_readback_category =
            u8::try_from(scalar_constraint("routing_readback_category")?).map_err(|_| {
                DriverError::InvalidAction("routing readback category outside byte".into())
            })?;
        let routing_source_count = usize::try_from(scalar_constraint("routing_source_count")?)
            .map_err(|_| DriverError::InvalidAction("routing source count is invalid".into()))?;
        let routing_destination_count =
            u16::try_from(scalar_constraint("routing_destination_count")?).map_err(|_| {
                DriverError::InvalidAction("routing destination count is invalid".into())
            })?;
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
            parameter_index,
            frame_index,
            mixer_readback_category,
            routing_readback_category,
            routing_source_count,
        };
        driver.validate_capabilities()?;
        let zero_report = vec![0; report_size];
        driver.decode_state(&zero_report)?;
        driver.decode_meter(&zero_report)?;
        Ok(driver)
    }

    fn frame(&self, id: &str) -> Result<&RuntimeFrame, DriverError> {
        self.frame_index
            .get(id)
            .and_then(|index| self.profile.frames.get(*index))
            .ok_or_else(|| DriverError::UnsupportedAction(format!("profile frame {id}")))
    }

    fn parameter(&self, target: &str, name: &str) -> Result<&RuntimeParam, DriverError> {
        self.parameter_index
            .get(&(target.to_owned(), name.to_owned()))
            .and_then(|index| self.profile.params.get(*index))
            .filter(|parameter| profile_codec::is_confirmed(&parameter.status))
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
        if parameter.applies_to != applies_to {
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
            let declared = profile_codec::scalar_offset(frame, field)?;
            if declared != *offset {
                return Err(DriverError::InvalidAction(format!(
                    "parameter {} semantic {field} offset {offset} differs from typed operation {declared}",
                    parameter.name
                )));
            }
        }
        for field in required {
            if !seen.contains(field) {
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
            let required: &[&str] = if parameter.applies_to == "globals" {
                &["parameter", "value"]
            } else {
                &["parameter", "target", "value"]
            };
            self.validate_reference(frame, parameter, required)?;
        }
        let mix = self.frame("mix_command")?;
        for field in ["surface", "strip", "fader"] {
            profile_codec::scalar_offset(mix, field)?;
        }
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
        for field in ["surface", "enabled"] {
            profile_codec::scalar_offset(link, field)?;
        }
        let mut pair = link
            .operations
            .iter()
            .filter_map(|operation| match operation {
                FrameOperation::PairIndex {
                    pair_field,
                    max_index: Some(max_index),
                    ..
                } if pair_field == "pair" => Some(*max_index),
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
        profile_codec::scalar_offset(self.frame("routing_command")?, "destination")?;
        let routing_ops: Vec<_> = self
            .frame("routing_command")?
            .operations
            .iter()
            .filter(|operation| {
                matches!(operation, FrameOperation::Indexed { index_field, width: 2, max_index: Some(max), .. } if index_field == "source_pair" && usize::from(*max) + 1 == self.routing_source_count)
            })
            .collect();
        if routing_ops.len() != 1 {
            return Err(DriverError::InvalidAction(
                "routing frame missing or ambiguous complete source-pair mapping".into(),
            ));
        }
        for (field, expected_width, expected_count) in [
            (
                "mixer_slot",
                if send_parameter { 3_u8 } else { 2_u8 },
                33_usize,
            ),
            ("routing_source_pair", 2_u8, self.routing_source_count),
        ] {
            let count = self
                .frame("readback")?
                .operations
                .iter()
                .filter(|operation| {
                    matches!(operation, FrameOperation::Indexed { index_field, width, max_index: Some(max), .. } if index_field == field && *width == expected_width && usize::from(*max) + 1 == expected_count)
                })
                .count();
            if count != 1 {
                return Err(DriverError::InvalidAction(format!(
                    "readback frame mapping {field} count is {count}"
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
        profile_codec::write_scalar(frame, &mut bytes, "parameter", i32::from(id))?;
        if let Some(target) = target {
            profile_codec::write_scalar(frame, &mut bytes, "target", i32::from(target))?;
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
        profile_codec::write_scalar(frame, &mut bytes, "surface", i32::from(address.surface))?;
        profile_codec::write_scalar(frame, &mut bytes, "strip", i32::from(address.strip))?;
        profile_codec::write_scalar(frame, &mut bytes, "fader", fader)?;
        profile_codec::write_bit_field(frame, &mut bytes, "pan", pan)?;
        profile_codec::write_bit_field(frame, &mut bytes, "mute", muted)?;
        profile_codec::write_bit_field(frame, &mut bytes, "solo", soloed)?;
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
            profile_codec::write_indexed_bytes(
                frame,
                &mut bytes,
                "source_pair",
                u16::try_from(channel)
                    .map_err(|_| DriverError::InvalidAction("routing channel overflow".into()))?,
                &[
                    source.bank,
                    u8::try_from(source.index).map_err(|_| {
                        DriverError::InvalidAction("routing source index exceeds byte".into())
                    })?,
                ],
            )?;
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
                } if index_field == field => Some((
                    usize::from(*base),
                    usize::from(*stride),
                    usize::from(*width),
                    *max,
                )),
                _ => None,
            });
        let result = matches.next().ok_or_else(|| {
            DriverError::InvalidAction(format!("readback missing indexed layout {field}"))
        })?;
        if matches.next().is_some() {
            return Err(DriverError::InvalidAction(format!(
                "readback ambiguous indexed layout {field}"
            )));
        }
        Ok(result)
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
        let offset = profile_codec::scalar_offset(frame, field)?;
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
        let layouts = [
            ("physical_gain", "physical_inputs"),
            ("physical_status", "physical_inputs"),
            ("adat_gain", "adat_inputs"),
            ("spdif_gain", "spdif_inputs"),
        ];
        for (field, space_name) in layouts {
            let (base, stride, width, max) = self.indexed_layout(frame, field)?;
            if width != 1 {
                return Err(DriverError::InvalidAction(format!(
                    "state field {field} record width must be one"
                )));
            }
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
                        input.mode = Some(Self::bit_value_from(frame, "input_mode", raw)?);
                        input.phantom =
                            Some(Self::bit_value_from(frame, "input_phantom", raw)? != 0);
                        input.phase = Some(Self::bit_value_from(frame, "input_phase", raw)? != 0);
                    }
                    _ => unreachable!(),
                }
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
                    .then(|| Self::bit_value_from(frame, "output_mute", status).map(|v| v != 0))
                    .transpose()?;
            }
            if dim_targets.is_some_and(|targets| targets.contains(&i32::from(output.address.id))) {
                output.dimmed = Some(Self::bit_value_from(frame, "output_dim", status)? != 0);
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
        let (base, stride, width, max) = self.indexed_layout(frame, "physical_meter")?;
        let space = self
            .profile
            .address_spaces
            .iter()
            .find(|space| space.id == "physical_inputs")
            .ok_or_else(|| DriverError::InvalidAction("missing physical meter space".into()))?;
        let count = space.count.ok_or_else(|| {
            DriverError::InvalidAction("physical meter space is unbounded".into())
        })?;
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
        let (base, stride, width, max) =
            self.indexed_layout(self.frame("readback")?, "mixer_slot")?;
        let required_slots = usize::from(mixer.strip_count) + 1;
        let has_send = self.parameter("mixers", "mix_send").is_ok();
        let expected_width = if has_send { 3 } else { 2 };
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
            decoded.push(DynamicMixerStrip {
                strip: slot as u16,
                name: if slot == 0 {
                    "Master".into()
                } else {
                    format!("CH {slot:02}")
                },
                fader: Some(i32::from(record[0])),
                pan: Some(self.bit_value("pan", record[1])?),
                send: has_send.then(|| i32::from(record[2])),
                muted: Some(self.bit_value("mute", record[1])? != 0),
                soloed: Some(self.bit_value("solo", record[1])? != 0),
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
        let (base, stride, width, max) =
            self.indexed_layout(self.frame("readback")?, "routing_source_pair")?;
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
            let Some(domain) = group
                .source_domains
                .iter()
                .find(|domain| domain.bank == source.bank)
            else {
                return Err(DriverError::InvalidAction(format!("routing readback category {:#04x} index {destination} channel {channel} source bank {} is unavailable", self.routing_readback_category, source.bank)));
            };
            if source.index >= domain.index_count {
                return Err(DriverError::InvalidAction(format!("routing readback category {:#04x} index {destination} channel {channel} source {}:{} outside 0..{}", self.routing_readback_category, source.bank, source.index, domain.index_count - 1)));
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
                profile_codec::write_scalar(frame, &mut bytes, "surface", i32::from(surface))?;
                profile_codec::write_pair_index(frame, &mut bytes, "pair", pair)?;
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
            let meter_discriminator = profile_codec::fixed_byte(
                self.frame("meter_report")?,
                readback.response_discriminator_offset,
            );
            if meter_discriminator == Some(discriminator) {
                return Ok(Some(DeviceEvent::Meter {
                    inputs: self.decode_meter(bytes)?,
                    raw: bytes.to_vec(),
                }));
            }
            if discriminator != readback.response_discriminator {
                return Err(DriverError::InvalidAction(format!(
                    "invalid readback discriminator {discriminator:#04x}"
                )));
            }
            for operation in &self.frame("readback")?.operations {
                if let FrameOperation::FixedByte { offset, value } = operation {
                    if bytes.get(usize::from(*offset)) != Some(value) {
                        return Err(DriverError::InvalidAction(format!(
                            "invalid readback fixed byte at {offset}"
                        )));
                    }
                }
            }
            let category = bytes[usize::from(readback.category_offset)];
            let index = bytes[usize::from(readback.index_offset)];
            let count = readback
                .category_counts
                .iter()
                .find(|bound| bound.category == category)
                .map(|bound| bound.count)
                .ok_or_else(|| {
                    DriverError::InvalidAction(format!(
                        "unknown inbound readback category {category:#04x} index {index}"
                    ))
                })?;
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
