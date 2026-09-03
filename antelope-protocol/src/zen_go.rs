//! Zen Go implementation of the driver-neutral protocol interface.

use crate::driver::{
    Action, CommandBatch, ControlValue, DeviceDriver, DeviceEvent, DriverDefinition, DriverError,
    DynamicDeviceState, DynamicGlobalState, DynamicInputState, DynamicMixerStrip,
    DynamicMixerSurface, DynamicOutputState, GlobalControl, InputAddress, MixerAddress,
    OutputAddress, OutputControl, RoutingSource, ZenGoCompatibilityState,
};
use crate::encoder::{encode_command, encode_query, Command, EncodeResult};
use crate::frame::Frame;
use crate::mixer::{MixerAssignment, MixerChannelState, MixerLinkTarget, MixerSurface};
use crate::profile::RuntimeProfile;
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
            || profile.outputs.len() != 3
            || !valid_mixers
        {
            return Err(DriverError::InvalidAction(
                "Zen Go driver requires two physical inputs, three outputs, and two 16-strip mixer surfaces".into(),
            ));
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

    fn state_from_snapshot(
        snapshot: DeviceStateSnapshot,
        profile: &RuntimeProfile,
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
                                    .and_then(|state| state.pan.map(|pan| i32::from(pan.raw()))),
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
                DynamicOutputState {
                    address: OutputAddress { id: declared.id },
                    name: declared.name.clone(),
                    level: output.map(|value| i32::from(value.volume)),
                    muted: output.map(|value| matches!(value.mode, crate::types::OutputMode::Mute)),
                    dimmed: output.map(|value| matches!(value.mode, crate::types::OutputMode::Dim)),
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
        let (surface, records) = if is_q04 {
            let surface = layout.surface?;
            if self
                .profile
                .mixers
                .iter()
                .all(|mixer| mixer.mix_index != surface)
            {
                return None;
            }
            (surface, 0..layout.record_count)
        } else {
            // DynamicStatePatch carries one surface. q18's 32-record block is
            // retained raw; expose first surface's declared fields here.
            let stride = layout.surface_stride?;
            if stride == 0 {
                return None;
            }
            (0, 0..stride.min(layout.record_count))
        };
        let fader = self.profile.mixer_fader(surface);
        let strips = records
            .map(|index| {
                let (level, state_code) = reply.readback_record(layout, index)?;
                let pan = PanState::from_state_code(state_code);
                Some(DynamicMixerStrip {
                    strip: (index % layout.surface_stride.unwrap_or(layout.record_count) + 1)
                        as u16,
                    name: String::new(),
                    fader: if is_q04 {
                        fader.map(|semantics| i32::from(level).clamp(semantics.min, semantics.max))
                    } else if layout.supported_fields.iter().any(|field| field == "fader") {
                        Some(i32::from(level))
                    } else {
                        None
                    },
                    pan: if is_q04 || layout.supported_fields.iter().any(|field| field == "pan") {
                        Some(i32::from(pan.raw()))
                    } else {
                        None
                    },
                    send: None,
                    muted: if is_q04 || layout.supported_fields.iter().any(|field| field == "mute")
                    {
                        Some(PanState::state_code_is_muted(state_code))
                    } else {
                        None
                    },
                    soloed: if is_q04 || layout.supported_fields.iter().any(|field| field == "solo")
                    {
                        Some(PanState::state_code_is_soloed(state_code))
                    } else {
                        None
                    },
                    linked: None,
                    meter: if layout.supported_fields.iter().any(|field| field == "meter") {
                        Some(level)
                    } else {
                        None
                    },
                    parameters: Vec::new(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(crate::driver::DynamicStatePatch::Mixer(
            DynamicMixerSurface {
                surface,
                name: String::new(),
                master: None,
                strips,
            },
        ))
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
                if !(1..=16).contains(&strip)
                    || !(i32::from(PanState::MIN)..=i32::from(PanState::MAX)).contains(&pan)
                {
                    return Err(DriverError::InvalidAction(format!(
                        "Zen Go mixer address/pan {surface}:{strip}/{pan}"
                    )));
                }
                Command::SetMixerLevel {
                    mixer: Self::mixer(surface)?,
                    channel: strip as u8,
                    level: self.validate_fader(surface, fader)?,
                    pan_state: PanState::from_raw(pan as u8),
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
        let frame = Frame::parse_owned(bytes.to_vec())?;
        Ok(Some(match frame {
            Frame::Snapshot { snapshot, raw } => DeviceEvent::Snapshot {
                state: Self::state_from_snapshot(snapshot, &self.profile),
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
