//! Zen Go implementation of the driver-neutral protocol interface.

use crate::driver::{
    Action, CommandBatch, DeviceDefinition, DeviceDriver, DeviceEvent, DriverError,
    DynamicDeviceState, DynamicMixerSurface,
};
use crate::encoder::{encode_command, encode_query, Command, EncodeResult};
use crate::frame::Frame;
use crate::mixer::{MixerChannelState, MixerSurface};
use crate::query::control_panel_startup_queries;
use crate::types::DeviceStateSnapshot;

const ZEN_GO_DEFINITION: DeviceDefinition = DeviceDefinition {
    id: "zen-go-synergy-core",
    name: "Antelope Zen Go Synergy Core",
    vid: 0x23e5,
    pid: 0xa015,
    supported: true,
};

/// Driver for Antelope Zen Go Synergy Core protocol reports.
#[derive(Debug, Clone, Copy)]
pub struct ZenGoDriver {
    definition: DeviceDefinition,
}

impl Default for ZenGoDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl ZenGoDriver {
    /// Construct a Zen Go driver with its canonical identity.
    pub const fn new() -> Self {
        Self {
            definition: ZEN_GO_DEFINITION,
        }
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
            EncodeResult::MixerAssignment { .. } => {
                unreachable!("mixer assignment must be encoded with its current assignment table")
            }
        }
    }

    fn state_from_snapshot(snapshot: DeviceStateSnapshot) -> DynamicDeviceState {
        let mixer_surfaces = snapshot
            .mixer_decode
            .surfaces
            .into_iter()
            .enumerate()
            .map(|(surface_index, strips)| DynamicMixerSurface {
                mixer: match surface_index {
                    0 => MixerSurface::Mix1,
                    1 => MixerSurface::Mix2,
                    _ => unreachable!("Zen Go has two mixer surfaces"),
                },
                strips: strips
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
                    .collect(),
            })
            .collect();

        DynamicDeviceState {
            sample_rate: snapshot.sample_rate,
            clock_source: snapshot.clock_source,
            sample_rate_hz: snapshot.sample_rate_hz,
            status_flags: snapshot.status_flags.to_vec(),
            front_panel_bytes: snapshot.front_panel_bytes.to_vec(),
            outputs: snapshot.outputs.to_vec(),
            preamps: vec![snapshot.preamp.input1, snapshot.preamp.input2],
            dsp_cluster: snapshot.dsp_cluster.to_vec(),
            surface: snapshot.surface,
            mixer_surfaces,
            late_shadow: snapshot.late_shadow.to_vec(),
        }
    }
}

impl DeviceDriver for ZenGoDriver {
    fn definition(&self) -> &DeviceDefinition {
        &self.definition
    }

    fn startup_requests(&self) -> &[crate::query::QueryRequest] {
        control_panel_startup_queries()
    }

    fn encode(&self, action: Action) -> Result<CommandBatch, DriverError> {
        if let Action::Query(query) = action {
            return Ok(CommandBatch {
                frames: vec![encode_query(query).to_vec()],
                refresh_requests: Vec::new(),
            });
        }

        if let Action::SetMixerAssignment {
            strip,
            assignment,
            assignments,
        } = action
        {
            return Ok(CommandBatch {
                frames: crate::encoder::encode_mixer_assignment_frames_with_table(
                    strip,
                    assignment,
                    &assignments,
                )
                .into_iter()
                .map(|frame| frame.to_vec())
                .collect(),
                refresh_requests: Vec::new(),
            });
        }

        let command = match action {
            Action::SetSampleRate(rate) => Command::SetSampleRate(rate),
            Action::SetClockSource(source) => Command::SetClockSource(source),
            Action::SelectSurface(surface) => Command::SelectSurface(surface),
            Action::SetPreampMode { input, mode } => Command::SetPreampMode { input, mode },
            Action::SetPreampGain { input, raw } => Command::SetPreampGain { input, raw },
            Action::SetPreampPhantom { input, enabled } => {
                Command::SetPreampPhantom { input, enabled }
            }
            Action::SetPreampPhase { input, enabled } => Command::SetPreampPhase { input, enabled },
            Action::SetOutputVolume { target, step } => Command::SetOutputVolume { target, step },
            Action::SetOutputMute { target, enabled } => Command::SetOutputMute { target, enabled },
            Action::SetOutputDim { target, enabled } => Command::SetOutputDim { target, enabled },
            Action::SetMixerLevel {
                mixer,
                channel,
                level,
                pan_state,
                muted,
                soloed,
            } => Command::SetMixerLevel {
                mixer,
                channel,
                level,
                pan_state,
                muted,
                soloed,
            },
            Action::SetMixerMute {
                mixer,
                channel,
                muted,
                pan_state,
                soloed,
            } => Command::SetMixerMute {
                mixer,
                channel,
                muted,
                pan_state,
                soloed,
            },
            Action::SetMixerSolo {
                mixer,
                channel,
                soloed,
                muted,
                pan_state,
            } => Command::SetMixerSolo {
                mixer,
                channel,
                soloed,
                muted,
                pan_state,
            },
            Action::SetMixerPan {
                mixer,
                channel,
                pan,
                muted,
                soloed,
            } => Command::SetMixerPan {
                mixer,
                channel,
                pan,
                muted,
                soloed,
            },
            Action::SetLinkState {
                selector,
                enabled,
                companion_bank,
            } => Command::SetLinkState {
                selector,
                enabled,
                companion_bank,
            },
            Action::Query(_) | Action::SetMixerAssignment { .. } => unreachable!(),
        };

        Ok(self.encode_command_result(encode_command(command)))
    }

    fn decode(&self, bytes: &[u8]) -> Result<Option<DeviceEvent>, DriverError> {
        let frame = Frame::parse_owned(bytes.to_vec())?;
        let event = match frame {
            Frame::Snapshot { snapshot, raw } => DeviceEvent::Snapshot {
                state: Self::state_from_snapshot(snapshot),
                raw: raw.to_vec(),
            },
            Frame::QueryReply { reply, raw } => DeviceEvent::QueryReply {
                query_id: reply.query_id,
                sub_id: reply.sub_id,
                body: reply.body,
                raw: raw.to_vec(),
            },
            Frame::Auxiliary { bytes, raw } => DeviceEvent::Auxiliary {
                bytes: bytes.to_vec(),
                raw: raw.to_vec(),
            },
            Frame::Notification { notification, raw } => DeviceEvent::Notification {
                bytes: notification.bytes.to_vec(),
                raw,
            },
        };

        Ok(Some(event))
    }
}
