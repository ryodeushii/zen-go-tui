//! Zen Go implementation of the driver-neutral protocol interface.

use crate::driver::{
    Action, CommandBatch, ControlValue, DeviceDriver, DeviceEvent, DriverDefinition, DriverError,
    DynamicDeviceState, DynamicGlobalState, DynamicInputState, DynamicMeterState,
    DynamicMixerStrip, DynamicMixerSurface, DynamicOutputState, GlobalControl, InputAddress,
    MixerAddress, OutputAddress, OutputControl, RoutingSource, ZenGoCompatibilityState,
};
use crate::encoder::{encode_command, encode_query, Command, EncodeResult};
use crate::frame::Frame;
use crate::mixer::{MixerAssignment, MixerChannelState, MixerLinkTarget, MixerSurface};
use crate::profile::{RuntimeMeterTarget, RuntimeProfile};
use crate::query::QueryRequest;
use crate::types::{ClockSource, DeviceStateSnapshot, PanState, PreampMode, SampleRate, Surface};

#[derive(Debug, Clone)]
pub struct ZenGoDriver {
    definition: DriverDefinition,
    profile: RuntimeProfile,
    startup_requests: Vec<QueryRequest>,
}

impl ZenGoDriver {
    pub fn new(profile: RuntimeProfile) -> Result<Self, DriverError> {
        let physical_inputs = profile
            .address_spaces
            .iter()
            .find(|space| space.kind == "physical_inputs")
            .and_then(|space| space.count)
            .unwrap_or_else(|| profile.inputs_in("physical_inputs") as u16);
        let physical_input_indices = profile
            .inputs
            .iter()
            .filter(|input| input.space == "physical_inputs")
            .map(|input| input.index)
            .collect::<std::collections::HashSet<_>>();
        let valid_mixers = profile.mixers.len() == 2
            && profile
                .mixers
                .iter()
                .all(|mixer| mixer.mix_index < 2 && mixer.strip_count == 16 && !mixer.has_master)
            && profile
                .mixers
                .iter()
                .map(|mixer| mixer.mix_index)
                .collect::<std::collections::HashSet<_>>()
                .len()
                == 2;
        if (profile.identity.vid, profile.identity.pid) != (0x23e5, 0xa015) {
            return Err(DriverError::InvalidAction(
                "Zen Go driver requires identity 0x23e5:0xa015".into(),
            ));
        }
        if physical_inputs != 2
            || profile.inputs_in("physical_inputs") != 2
            || physical_input_indices != std::collections::HashSet::from([0, 1])
            || profile.outputs.len() != 3
            || !valid_mixers
        {
            return Err(DriverError::InvalidAction(
                "Zen Go driver requires physical input indices 0 and 1, three outputs, and two 16-strip mixer surfaces".into(),
            ));
        }
        if !profile.meter_mappings.is_empty() && profile.transport.report_size.is_none() {
            return Err(DriverError::InvalidAction(
                "Zen Go meter mappings require a finite report size".into(),
            ));
        }
        let candidate_preamp_meters = profile.candidate_preamp_meters();
        if !candidate_preamp_meters.is_empty() && profile.transport.report_size.is_none() {
            return Err(DriverError::InvalidAction(
                "Zen Go candidate preamp meters require a finite report size".into(),
            ));
        }
        let mut candidate_input_indices = std::collections::HashSet::new();
        for meter in candidate_preamp_meters {
            if meter.input_index > 1
                || !candidate_input_indices.insert(meter.input_index)
                || !profile.inputs.iter().any(|input| {
                    input.space == "physical_inputs" && input.index == meter.input_index
                })
            {
                return Err(DriverError::InvalidAction(
                    "Zen Go candidate preamp meter must target one declared physical input".into(),
                ));
            }
            let full_offset = crate::SNAPSHOT_PAYLOAD_OFFSET
                .checked_add(meter.offset)
                .ok_or_else(|| {
                    DriverError::InvalidAction(
                        "Zen Go candidate preamp meter payload offset overflows report geometry"
                            .into(),
                    )
                })?;
            if profile
                .transport
                .report_size
                .is_some_and(|size| full_offset >= usize::from(size))
            {
                return Err(DriverError::InvalidAction(
                    "Zen Go candidate preamp meter payload offset exceeds report geometry".into(),
                ));
            }
            if meter.status.trim().is_empty()
                || meter.confidence.trim().is_empty()
                || meter.caveat.trim().is_empty()
                || meter.raw_value_ranges.is_empty()
                || meter
                    .raw_value_ranges
                    .iter()
                    .any(|(minimum, maximum)| minimum > maximum)
                || meter
                    .raw_value_ranges
                    .windows(2)
                    .any(|ranges| ranges[0].1 >= ranges[1].0)
            {
                return Err(DriverError::InvalidAction(
                    "Zen Go candidate preamp meter requires ordered value ranges and provenance"
                        .into(),
                ));
            }
        }
        let mut meter_mapping_keys = std::collections::HashSet::new();
        for mapping in &profile.meter_mappings {
            if mapping.target != RuntimeMeterTarget::MixMaster
                || mapping.target_index >= 2
                || mapping.frame_id != "state_report"
                || mapping.status.trim().is_empty()
                || mapping.evidence.trim().is_empty()
                || profile
                    .transport
                    .report_size
                    .is_some_and(|size| mapping.offset >= usize::from(size))
                || !meter_mapping_keys.insert((mapping.target_index, mapping.lane))
            {
                return Err(DriverError::InvalidAction(
                    "Zen Go meter mapping must be a unique state-report mix-master lane within the report".into(),
                ));
            }
        }
        let readback = profile.readback.as_ref().ok_or_else(|| {
            DriverError::InvalidAction("Zen Go driver requires profile readback metadata".into())
        })?;
        let startup_requests = readback
            .safe_queries
            .iter()
            .map(|query| QueryRequest {
                query_id: query.category,
                sub_id: query.index,
            })
            .collect();
        Ok(Self {
            definition: DriverDefinition {
                id: "zen-go-synergy-core".into(),
                name: "Antelope Zen Go Synergy Core".into(),
                vid: 0x23e5,
                pid: 0xa015,
                supported: true,
            },
            profile,
            startup_requests,
        })
    }

    pub fn profile(&self) -> &RuntimeProfile {
        &self.profile
    }

    fn validate_fader(&self, surface: u8, value: i32) -> Result<u8, DriverError> {
        let semantics = self.profile.mixer_fader(surface).ok_or_else(|| {
            DriverError::InvalidAction(format!(
                "Zen Go mixer surface {surface} has no fader semantics"
            ))
        })?;
        if !(semantics.min..=semantics.max).contains(&value) {
            return Err(DriverError::InvalidAction(format!(
                "Zen Go mixer fader {value} outside {}..={}",
                semantics.min, semantics.max
            )));
        }
        Self::byte(value, "mixer fader")
    }

    fn encode_command_result(&self, result: EncodeResult) -> CommandBatch {
        match result {
            EncodeResult::Single(frame) => CommandBatch {
                frames: vec![frame.to_vec()],
                refresh_requests: Vec::new(),
            },
            EncodeResult::Multi(frames) => CommandBatch {
                frames: (*frames).into_iter().map(|frame| frame.to_vec()).collect(),
                refresh_requests: Vec::new(),
            },
            EncodeResult::WithCompanion { companion, main } => CommandBatch {
                frames: vec![companion.to_vec(), main.to_vec()],
                refresh_requests: Vec::new(),
            },
            EncodeResult::WithRefresh(frame) => CommandBatch {
                frames: vec![frame.to_vec()],
                refresh_requests: self.startup_requests().to_vec(),
            },
            EncodeResult::MixerAssignment { .. } => unreachable!("assignment needs complete table"),
        }
    }

    fn mixer(surface: u8) -> Result<MixerSurface, DriverError> {
        match surface {
            0 => Ok(MixerSurface::Mix1),
            1 => Ok(MixerSurface::Mix2),
            _ => Err(DriverError::InvalidAction(format!(
                "Zen Go mixer surface {surface} outside 0..2"
            ))),
        }
    }

    fn byte(value: i32, field: &str) -> Result<u8, DriverError> {
        u8::try_from(value)
            .map_err(|_| DriverError::InvalidAction(format!("{field} {value} outside byte range")))
    }

    fn bool_value(value: ControlValue, field: &str) -> Result<bool, DriverError> {
        match value {
            ControlValue::Bool(value) => Ok(value),
            _ => Err(DriverError::InvalidAction(format!("{field} requires bool"))),
        }
    }

    fn int_value(value: ControlValue, field: &str) -> Result<i32, DriverError> {
        match value {
            ControlValue::Int(value) | ControlValue::Enum(value) => Ok(value),
            ControlValue::Bool(_) => Err(DriverError::InvalidAction(format!(
                "{field} requires integer"
            ))),
        }
    }

    fn assignment(source: RoutingSource) -> Result<MixerAssignment, DriverError> {
        let channel =
            u8::try_from(source.index.checked_add(1).ok_or_else(|| {
                DriverError::InvalidAction("routing source index overflow".into())
            })?)
            .map_err(|_| {
                DriverError::InvalidAction("routing source index outside Zen Go range".into())
            })?;
        let assignment = match (source.bank, source.index) {
            (0x00, 0..=1) => MixerAssignment::Preamp(channel),
            (0x01, 0..=7) => MixerAssignment::ComputerPlay(channel),
            (0x02, 0..=1) => MixerAssignment::SpdifIn(channel),
            (0x08, 0) => MixerAssignment::Mute,
            (0x09, 0..=1) => MixerAssignment::Oscillator(channel),
            (0x0a, 0..=1) => MixerAssignment::EmuMic(channel),
            _ => {
                return Err(DriverError::InvalidAction(format!(
                    "unsupported Zen Go routing source {:#04x}:{}",
                    source.bank, source.index
                )))
            }
        };
        Ok(assignment)
    }

    fn mapped_meters(
        profile: &RuntimeProfile,
        frame_id: &str,
        bytes: &[u8],
    ) -> Vec<DynamicMeterState> {
        profile
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

    fn state_from_snapshot(
        snapshot: DeviceStateSnapshot,
        profile: &RuntimeProfile,
        raw: &[u8],
    ) -> DynamicDeviceState {
        let compatibility_surfaces: Vec<_> = snapshot
            .mixer_decode
            .surfaces
            .into_iter()
            .enumerate()
            .map(|(surface_index, strips)| {
                let mixer = if surface_index == 0 {
                    MixerSurface::Mix1
                } else {
                    MixerSurface::Mix2
                };
                let states = strips
                    .into_iter()
                    .enumerate()
                    .map(|(channel_index, passive)| {
                        let mut state = MixerChannelState::unknown((channel_index + 1) as u8);
                        state.meter = passive.meter;
                        state.muted = passive.muted;
                        state.pan = passive.pan.unwrap_or_default();
                        state.linked = passive.linked;
                        state
                    })
                    .collect::<Vec<_>>();
                (mixer, states)
            })
            .collect();
        let mixers = profile
            .mixers
            .iter()
            .map(|mixer| {
                let passive = snapshot
                    .mixer_decode
                    .surfaces
                    .get(usize::from(mixer.mix_index));
                DynamicMixerSurface {
                    // Capture-scoped readback patches intentionally carry no
                    // official names; snapshots use profile-owned names.
                    surface: mixer.mix_index,
                    name: mixer.name.clone(),
                    master: None,
                    strips: (0..usize::from(mixer.strip_count))
                        .map(|index| {
                            let strip = passive.and_then(|surface| surface.get(index));
                            DynamicMixerStrip {
                                strip: (index + 1) as u16,
                                name: format!("CH {:02}", index + 1),
                                fader: None,
                                pan: strip
                                    .and_then(|state| state.pan)
                                    .and_then(|pan| mixer.pan_value_from_raw(pan)),
                                send: None,
                                muted: strip.and_then(|state| state.muted),
                                soloed: None,
                                linked: strip.and_then(|state| state.linked),
                                meter: strip.and_then(|state| state.meter),
                                parameters: Vec::new(),
                            }
                        })
                        .collect(),
                }
            })
            .collect();
        let preamps = [snapshot.preamp.input1, snapshot.preamp.input2];
        let inputs = profile
            .inputs
            .iter()
            .map(|input| {
                let physical = input.space == "physical_inputs";
                let legacy = physical
                    .then(|| preamps.get(usize::from(input.index)))
                    .flatten();
                let meter = if physical && profile.candidate_preamp_meter(input.index).is_some() {
                    match input.index {
                        0 => snapshot.mixer_decode.observed_preamp1_meter,
                        1 => snapshot.mixer_decode.observed_preamp2_meter,
                        _ => None,
                    }
                } else {
                    None
                };
                DynamicInputState {
                    address: InputAddress {
                        space: input.space_id,
                        index: input.index,
                    },
                    name: input.name.clone(),
                    mode: legacy.map(|value| i32::from(value.mode.code())),
                    gain: legacy.map(|value| i32::from(value.gain_raw)),
                    phantom: legacy.map(|value| value.phantom_on),
                    phase: legacy.map(|value| value.mode_raw & 0x40 != 0),
                    meter,
                    parameters: Vec::new(),
                }
            })
            .collect();
        let outputs = profile
            .outputs
            .iter()
            .map(|declared| {
                let output = snapshot
                    .outputs
                    .iter()
                    .find(|output| u16::from(output.target.index()) == declared.id);
                let (muted, dimmed) = match output.map(|value| value.mode) {
                    Some(crate::types::OutputMode::Normal) => (Some(false), Some(false)),
                    Some(crate::types::OutputMode::Mute) => (Some(true), Some(false)),
                    Some(crate::types::OutputMode::Dim) => (Some(false), Some(true)),
                    Some(crate::types::OutputMode::Unknown(_)) | None => (None, None),
                };
                DynamicOutputState {
                    address: OutputAddress { id: declared.id },
                    name: declared.name.clone(),
                    level: output.map(|value| i32::from(value.volume)),
                    muted,
                    dimmed,
                    parameters: Vec::new(),
                }
            })
            .collect();
        DynamicDeviceState {
            globals: vec![
                DynamicGlobalState {
                    control: GlobalControl::SampleRate,
                    value: ControlValue::Enum(i32::from(snapshot.sample_rate.code())),
                },
                DynamicGlobalState {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(i32::from(snapshot.clock_source.code())),
                },
                DynamicGlobalState {
                    control: GlobalControl::Surface,
                    value: ControlValue::Enum(i32::from(snapshot.surface.code())),
                },
            ],
            inputs,
            outputs,
            mixers,
            meters: Self::mapped_meters(profile, "state_report", raw),
            routing: Vec::new(),
            zen_go_compatibility: Some(Box::new(ZenGoCompatibilityState {
                sample_rate: snapshot.sample_rate,
                clock_source: snapshot.clock_source,
                sample_rate_hz: snapshot.sample_rate_hz,
                status_flags: snapshot.status_flags.to_vec(),
                front_panel_bytes: snapshot.front_panel_bytes.to_vec(),
                outputs: snapshot.outputs.to_vec(),
                preamps: preamps.to_vec(),
                dsp_cluster: snapshot.dsp_cluster.to_vec(),
                surface: snapshot.surface,
                mixer_surfaces: compatibility_surfaces,
                late_shadow: snapshot.late_shadow.to_vec(),
            })),
        }
    }

    fn patch_for_query_reply(
        &self,
        query_id: u8,
        sub_id: u8,
        reply: &crate::query::QueryResponse,
    ) -> Option<crate::driver::DynamicStatePatch> {
        let query = QueryRequest { query_id, sub_id };
        let layout = self.profile.readback.as_ref()?.layout_for(query)?;
        if reply.body.len() < layout.body_size {
            return None;
        }
        let is_q04 = query_id == 0x04;
        let is_q18 = query_id == 0x18;
        if !is_q04 && !is_q18 {
            return None;
        }
        let supports = |field: &str| layout.supported_fields.iter().any(|value| value == field);
        let decode_surface = |surface: u8, records: Vec<usize>| {
            let mixer = self.profile.mixer(surface);
            let fader = mixer.and_then(|mixer| mixer.fader);
            let strips = records
                .into_iter()
                .map(|index| {
                    let (level, state_code) = reply.readback_record(layout, index)?;
                    let pan = PanState::from_state_code(state_code);
                    Some(DynamicMixerStrip {
                        strip: (index % layout.surface_stride.unwrap_or(layout.record_count) + 1)
                            as u16,
                        name: String::new(),
                        fader: if is_q04 {
                            fader.map(|semantics| {
                                i32::from(level).clamp(semantics.min, semantics.max)
                            })
                        } else if supports("fader") {
                            Some(i32::from(level))
                        } else {
                            None
                        },
                        pan: if is_q04 || supports("pan") {
                            mixer.and_then(|mixer| mixer.pan_value_from_raw(pan))
                        } else {
                            None
                        },
                        send: None,
                        muted: if is_q04 || supports("mute") {
                            Some(PanState::state_code_is_muted(state_code))
                        } else {
                            None
                        },
                        soloed: if is_q04 || supports("solo") {
                            Some(PanState::state_code_is_soloed(state_code))
                        } else {
                            None
                        },
                        linked: None,
                        meter: supports("meter").then_some(level),
                        parameters: Vec::new(),
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(DynamicMixerSurface {
                surface,
                name: String::new(),
                master: None,
                strips,
            })
        };

        if is_q04 {
            let surface = layout.surface?;
            if self
                .profile
                .mixers
                .iter()
                .all(|mixer| mixer.mix_index != surface)
            {
                return None;
            }
            return Some(crate::driver::DynamicStatePatch::Mixer(decode_surface(
                surface,
                (0..layout.record_count).collect(),
            )?));
        }

        let stride = layout.surface_stride?;
        if stride == 0 || layout.record_count > self.profile.mixers.len().checked_mul(stride)? {
            return None;
        }
        let surfaces = self
            .profile
            .mixers
            .iter()
            .map(|mixer| {
                let start = usize::from(mixer.mix_index).checked_mul(stride)?;
                let end = start.checked_add(stride)?.min(layout.record_count);
                decode_surface(mixer.mix_index, (start..end).collect())
            })
            .collect::<Option<Vec<_>>>()?;
        Some(crate::driver::DynamicStatePatch::Mixers(surfaces))
    }
}

impl DeviceDriver for ZenGoDriver {
    fn definition(&self) -> &DriverDefinition {
        &self.definition
    }
    fn startup_requests(&self) -> &[crate::query::QueryRequest] {
        &self.startup_requests
    }

    fn encode(&self, action: Action) -> Result<CommandBatch, DriverError> {
        if let Action::Query(query) = action {
            return Ok(CommandBatch {
                frames: vec![encode_query(query).to_vec()],
                refresh_requests: Vec::new(),
            });
        }
        if let Action::SetRoutingGroup {
            destination,
            changed_channel,
            sources,
        } = action
        {
            let changed_channel = changed_channel.ok_or_else(|| {
                DriverError::InvalidAction("Zen Go routing requires changed_channel".into())
            })?;
            if destination != 0 || changed_channel >= 16 || sources.len() != 16 {
                return Err(DriverError::InvalidAction(format!(
                    "Zen Go routing destination {destination} channel {changed_channel} requires exactly 16 sources"
                )));
            }
            let mut assignments = [MixerAssignment::Mute; 16];
            for (slot, source) in assignments.iter_mut().zip(sources) {
                *slot = Self::assignment(source)?;
            }
            let strip = u8::try_from(changed_channel + 1)
                .map_err(|_| DriverError::InvalidAction("Zen Go routing strip overflow".into()))?;
            let frames = crate::encoder::encode_mixer_assignment_frames_with_table(
                strip,
                assignments[usize::from(strip - 1)],
                &assignments,
            )
            .into_iter()
            .map(|frame| frame.to_vec())
            .collect();
            return Ok(CommandBatch {
                frames,
                refresh_requests: Vec::new(),
            });
        }

        let command = match action {
            Action::SetGlobal {
                control: GlobalControl::SampleRate,
                value,
            } => Command::SetSampleRate(SampleRate::from_code(Self::byte(
                Self::int_value(value, "sample rate")?,
                "sample rate",
            )?)),
            Action::SetGlobal {
                control: GlobalControl::ClockSource,
                value,
            } => Command::SetClockSource(ClockSource::from_code(Self::byte(
                Self::int_value(value, "clock source")?,
                "clock source",
            )?)),
            Action::SetGlobal {
                control: GlobalControl::Surface,
                value,
            } => Command::SelectSurface(Surface::from_code(Self::byte(
                Self::int_value(value, "surface")?,
                "surface",
            )?)),
            Action::SetInput {
                address,
                control,
                value,
            } if address.space == 0 && address.index < 2 => {
                let input = address.index as u8;
                match control {
                    crate::driver::InputControl::Mode => Command::SetPreampMode {
                        input,
                        mode: PreampMode::from_raw(Self::byte(
                            Self::int_value(value, "preamp mode")?,
                            "preamp mode",
                        )?),
                    },
                    crate::driver::InputControl::Gain => Command::SetPreampGain {
                        input,
                        raw: Self::byte(Self::int_value(value, "preamp gain")?, "preamp gain")?,
                    },
                    crate::driver::InputControl::Phantom => Command::SetPreampPhantom {
                        input,
                        enabled: Self::bool_value(value, "phantom")?,
                    },
                    crate::driver::InputControl::Phase => Command::SetPreampPhase {
                        input,
                        enabled: Self::bool_value(value, "phase")?,
                    },
                    crate::driver::InputControl::Parameter(_) => {
                        return Err(DriverError::UnsupportedAction(
                            "Zen Go input parameter".into(),
                        ))
                    }
                }
            }
            Action::SetOutput {
                address,
                control,
                value,
            } if address.id < 3 => {
                let target = crate::types::OutputTarget::from_index(usize::from(address.id))
                    .ok_or_else(|| DriverError::InvalidAction("output address".into()))?;
                match control {
                    OutputControl::Level => Command::SetOutputVolume {
                        target,
                        step: Self::byte(Self::int_value(value, "output level")?, "output level")?,
                    },
                    OutputControl::Mute => Command::SetOutputMute {
                        target,
                        enabled: Self::bool_value(value, "output mute")?,
                    },
                    OutputControl::Dim => Command::SetOutputDim {
                        target,
                        enabled: Self::bool_value(value, "output dim")?,
                    },
                    OutputControl::Parameter(_) => {
                        return Err(DriverError::UnsupportedAction(
                            "Zen Go output parameter".into(),
                        ))
                    }
                }
            }
            Action::SetMixerStripState {
                address: MixerAddress { surface, strip },
                fader,
                pan,
                muted,
                soloed,
                send: None,
            } => {
                let pan_state = self
                    .profile
                    .mixer(surface)
                    .and_then(|mixer| mixer.pan_raw_from_value(pan))
                    .ok_or_else(|| {
                        DriverError::InvalidAction(format!(
                            "Zen Go mixer address/pan {surface}:{strip}/{pan}"
                        ))
                    })?;
                if !(1..=16).contains(&strip) {
                    return Err(DriverError::InvalidAction(format!(
                        "Zen Go mixer address/pan {surface}:{strip}/{pan}"
                    )));
                }
                Command::SetMixerLevel {
                    mixer: Self::mixer(surface)?,
                    channel: strip as u8,
                    level: self.validate_fader(surface, fader)?,
                    pan_state,
                    muted,
                    soloed,
                }
            }
            Action::SetMixerStripState { send: Some(_), .. } => {
                return Err(DriverError::InvalidAction(
                    "Zen Go mixer frame has no send field".into(),
                ))
            }
            Action::SetLink {
                surface,
                pair,
                enabled,
            } => {
                let mixer = Self::mixer(surface)?;
                let channel = pair
                    .checked_mul(2)
                    .and_then(|value| value.checked_add(1))
                    .and_then(|value| u8::try_from(value).ok())
                    .ok_or_else(|| DriverError::InvalidAction("link pair overflow".into()))?;
                let target = MixerLinkTarget::from_channel(mixer, channel).ok_or_else(|| {
                    DriverError::InvalidAction(format!("Zen Go link pair {pair}"))
                })?;
                Command::SetLinkState {
                    selector: target.selector,
                    enabled,
                    companion_bank: target.companion_bank(),
                }
            }
            Action::SetRouting { .. } => {
                return Err(DriverError::UnsupportedAction(
                    "Zen Go requires complete SetRoutingGroup".into(),
                ))
            }
            Action::SetMixer { .. } => {
                return Err(DriverError::UnsupportedAction(
                    "Zen Go requires complete SetMixerStripState".into(),
                ))
            }
            Action::SetGlobal { .. }
            | Action::SetInput { .. }
            | Action::SetOutput { .. }
            | Action::SetWholeState { .. } => {
                return Err(DriverError::UnsupportedAction(
                    "control is unavailable on Zen Go".into(),
                ))
            }
            Action::SetRoutingGroup { .. } | Action::Query(_) => unreachable!(),
        };
        Ok(self.encode_command_result(encode_command(command)))
    }

    fn decode(&self, bytes: &[u8]) -> Result<Option<DeviceEvent>, DriverError> {
        let frame = Frame::parse_owned_with_candidate_preamp_meters(
            bytes.to_vec(),
            self.profile.candidate_preamp_meters(),
        )?;
        Ok(Some(match frame {
            Frame::Snapshot { snapshot, raw } => DeviceEvent::Snapshot {
                state: Self::state_from_snapshot(snapshot, &self.profile, &raw),
                raw: raw.to_vec(),
            },
            Frame::QueryReply { reply, raw } => {
                let patch = self.patch_for_query_reply(reply.query_id, reply.sub_id, &reply);
                DeviceEvent::QueryReply {
                    query_id: reply.query_id,
                    sub_id: reply.sub_id,
                    body: reply.body,
                    patch,
                    raw: raw.to_vec(),
                }
            }
            Frame::Auxiliary { bytes, raw } => DeviceEvent::Auxiliary {
                bytes: bytes.to_vec(),
                raw: raw.to_vec(),
            },
            Frame::Notification { notification, raw } => DeviceEvent::Notification {
                bytes: notification.bytes.to_vec(),
                raw,
            },
        }))
    }
}
