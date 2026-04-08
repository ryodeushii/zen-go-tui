use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::protocol::{
    control_panel_startup_queries, encode_command, encode_link_companion,
    encode_mixer_assignment_frames_with_table, encode_query, ClockSource, Command, DeviceMetadata,
    DeviceSnapshot, Frame, MixerAssignment, MixerChannelState, MixerLinkTarget, MixerSurface,
    OutputMode, OutputState, OutputTarget, PanState, PreampMode, PreampState, QueryReply75,
    SampleRate, Snapshot73, Surface,
};
use crate::transport::Transport;

#[derive(Debug, Clone)]
pub struct DeviceStatus {
    pub sample_rate: Option<SampleRate>,
    pub clock_source: Option<ClockSource>,
    pub lock_known: bool,
    pub locked: Option<bool>,
    pub metadata: Option<DeviceMetadata>,
    pub startup_query_summaries: [Option<String>; 3],
    pub last_refresh_summary: String,
}

impl Default for DeviceStatus {
    fn default() -> Self {
        Self {
            sample_rate: None,
            clock_source: None,
            lock_known: false,
            locked: None,
            metadata: None,
            startup_query_summaries: [None, None, None],
            last_refresh_summary: "waiting for device snapshot".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConnectionState {
    pub connected: bool,
    pub last_snapshot_at: Option<Instant>,
    pub last_frame_type: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusArea {
    Status,
    Outputs,
    Mixer,
    Preamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPacketTab {
    Query74,
    State73,
    Auxiliary83,
    Query75,
    Notification81,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignmentPickerState {
    pub strip: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryReplyLogEntry {
    pub summary: String,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub device: DeviceStatus,
    pub outputs: [OutputState; 3],
    pub preamp: PreampState,
    pub surface: Surface,
    pub mixer_channels: [Vec<MixerChannelState>; 2],
    pub connection: ConnectionState,
    pub focus: FocusArea,
    pub selected_output: usize,
    pub selected_channel: usize,
    pub selected_preamp_input: usize,
    pub raw_view_open: bool,
    pub selected_raw_packet: RawPacketTab,
    pub last_message: String,
    pub last_auxiliary_len: Option<usize>,
    pub dsp_cluster: [u8; 4],
    pub latest_raw_73: Option<Vec<u8>>,
    pub latest_raw_83: Option<Vec<u8>>,
    pub latest_raw_74: Option<Vec<u8>>,
    pub latest_raw_75: Option<Vec<u8>>,
    pub latest_raw_81: Option<Vec<u8>>,
    pub recent_query_request_log: Vec<String>,
    pub recent_query_reply_log: Vec<String>,
    pub recent_query_reply_entries: Vec<QueryReplyLogEntry>,
    pub selected_query_reply_entry: Option<usize>,
    pub baseline_raw_73: Option<Vec<u8>>,
    pub baseline_raw_83: Option<Vec<u8>>,
    pub baseline_raw_74: Option<Vec<u8>>,
    pub baseline_raw_75: Option<Vec<u8>>,
    pub baseline_raw_81: Option<Vec<u8>>,
    pub assignment_picker: Option<AssignmentPickerState>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            device: DeviceStatus::default(),
            outputs: [
                OutputState::new(OutputTarget::Monitor, 0, OutputMode::Normal),
                OutputState::new(OutputTarget::Hp1, 0, OutputMode::Normal),
                OutputState::new(OutputTarget::Hp2, 0, OutputMode::Normal),
            ],
            preamp: PreampState::default(),
            surface: Surface::MonitorHp1,
            mixer_channels: [
                (1..=16).map(MixerChannelState::unknown).collect(),
                (1..=16).map(MixerChannelState::unknown).collect(),
            ],
            connection: ConnectionState::default(),
            focus: FocusArea::Outputs,
            selected_output: 0,
            selected_channel: 0,
            selected_preamp_input: 0,
            raw_view_open: false,
            selected_raw_packet: RawPacketTab::State73,
            last_message:
                "Press ? for help. Device state is authoritative where decoding is confirmed."
                    .to_string(),
            last_auxiliary_len: None,
            dsp_cluster: [0; 4],
            latest_raw_73: None,
            latest_raw_83: None,
            latest_raw_74: None,
            latest_raw_75: None,
            latest_raw_81: None,
            recent_query_request_log: Vec::new(),
            recent_query_reply_log: Vec::new(),
            recent_query_reply_entries: Vec::new(),
            selected_query_reply_entry: None,
            baseline_raw_73: None,
            baseline_raw_83: None,
            baseline_raw_74: None,
            baseline_raw_75: None,
            baseline_raw_81: None,
            assignment_picker: None,
        }
    }
}

impl AppState {
    pub fn startup_query_summary(&self, query_id: u8) -> Option<&str> {
        startup_query_slot(query_id)
            .and_then(|index| self.device.startup_query_summaries[index].as_deref())
    }

    pub fn selected_query_reply_entry(&self) -> Option<&QueryReplyLogEntry> {
        self.selected_query_reply_entry
            .and_then(|index| self.recent_query_reply_entries.get(index))
    }

    pub fn active_mixer_surface(&self) -> MixerSurface {
        MixerSurface::from_surface(self.surface)
    }

    pub fn active_mixer_channels(&self) -> &[MixerChannelState] {
        &self.mixer_channels[self.active_mixer_surface().index()]
    }

    pub fn apply_snapshot(&mut self, snapshot: Snapshot73) {
        self.device.sample_rate = Some(snapshot.sample_rate);
        self.device.clock_source = Some(snapshot.clock_source);
        self.device.last_refresh_summary = format!(
            "snapshot {} / {} / surface {}",
            snapshot.sample_rate.label(),
            snapshot.clock_source.label(),
            snapshot.surface.label()
        );
        self.outputs = snapshot.outputs;
        self.dsp_cluster = snapshot.dsp_cluster;
        self.preamp = PreampState::from_cluster(snapshot.dsp_cluster);
        self.preamp.input1.observed_meter = snapshot.mixer_decode.observed_preamp1_meter;
        self.preamp.input2.observed_meter = snapshot.mixer_decode.observed_preamp2_meter;
        self.surface = snapshot.surface;
        self.apply_passive_mixer_decode(&snapshot);
    }

    fn apply_passive_mixer_decode(&mut self, snapshot: &Snapshot73) {
        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            for channel in 1..=16 {
                let Some(decoded) = snapshot.mixer_decode.strip(mixer, channel) else {
                    continue;
                };
                let Some(slot) = self.state_slot_mut(mixer, channel) else {
                    continue;
                };

                if let Some(meter) = decoded.meter {
                    slot.meter = Some(meter);
                }
                if let Some(muted) = decoded.muted {
                    slot.muted = Some(muted);
                }
                if let Some(linked) = decoded.linked {
                    slot.linked = Some(linked);
                }
            }
        }
    }

    fn state_slot_mut(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
    ) -> Option<&mut MixerChannelState> {
        self.mixer_channels[mixer.index()].get_mut(channel.checked_sub(1)? as usize)
    }

    fn refresh_preamp_from_cluster_preserving_observed_meter(&mut self) {
        let observed_meter_input1 = self.preamp.input1.observed_meter;
        let observed_meter_input2 = self.preamp.input2.observed_meter;
        self.preamp = PreampState::from_cluster(self.dsp_cluster);
        self.preamp.input1.observed_meter = observed_meter_input1;
        self.preamp.input2.observed_meter = observed_meter_input2;
    }

    pub fn observe_frame(&mut self, frame: DeviceSnapshot, raw: Vec<u8>) {
        self.connection.connected = true;
        self.connection.last_snapshot_at = Some(Instant::now());
        match frame {
            DeviceSnapshot::Snapshot(snapshot) => {
                self.connection.last_frame_type = Some("0x73 snapshot");
                self.latest_raw_73 = Some(raw);
                self.apply_snapshot(snapshot);
            }
            DeviceSnapshot::Auxiliary83(bytes) => {
                self.connection.last_frame_type = Some("0x83 auxiliary");
                self.last_auxiliary_len = Some(bytes.len());
                self.latest_raw_83 = Some(raw);
            }
            DeviceSnapshot::QueryReply(reply) => {
                self.connection.last_frame_type = Some("0x75 query reply");
                self.latest_raw_75 = Some(raw.clone());
                self.store_startup_query_summary(&reply);
                self.apply_query_reply_readback(&reply);
                self.push_query_reply_log(&reply, raw);
                if let Some(metadata) = reply.metadata() {
                    self.last_message = format!(
                        "Connected to {} (hw {}, serial {})",
                        metadata.product_name, metadata.hardware_version, metadata.serial
                    );
                    self.device.metadata = Some(metadata);
                }
            }
            DeviceSnapshot::Notification(_) => {
                self.connection.last_frame_type = Some("0x81 notification");
                self.latest_raw_81 = Some(raw);
            }
        }
    }

    fn store_startup_query_summary(&mut self, reply: &QueryReply75) {
        if let Some(index) = startup_query_slot(reply.query_id) {
            self.device.startup_query_summaries[index] = Some(reply.summary_label());
        }
    }

    fn push_query_reply_log(&mut self, reply: &QueryReply75, raw: Vec<u8>) {
        let detail = if reply.selector_bitmap().is_some() || reply.selector_pair_bank().is_some() {
            reply.summary_label()
        } else {
            let preview = reply
                .body
                .iter()
                .take(8)
                .map(|byte| format!("{:02x}", byte))
                .collect::<Vec<_>>()
                .join(" ");
            format!("[{} bytes] {}", reply.body.len(), preview)
        };
        let summary = format!(
            "0x75 {:02x}/{:02x} {}",
            reply.query_id, reply.sub_id, detail
        );
        self.recent_query_reply_log.push(summary.clone());
        self.recent_query_reply_entries
            .push(QueryReplyLogEntry { summary, raw });
        if self.recent_query_reply_log.len() > 16 {
            let drop_count = self.recent_query_reply_log.len() - 16;
            self.recent_query_reply_log.drain(0..drop_count);
            self.recent_query_reply_entries.drain(0..drop_count);
        }
        self.selected_query_reply_entry = Some(self.recent_query_reply_entries.len() - 1);
    }

    fn apply_query_reply_readback(&mut self, reply: &QueryReply75) {
        if let Some(assignments) = reply.assignment_readback() {
            for (index, assignment) in assignments.into_iter().enumerate() {
                let Some(assignment) = assignment else {
                    continue;
                };
                for channels in &mut self.mixer_channels {
                    channels[index].assignment = Some(assignment);
                }
            }
        }

        if let Some(startup_links) = reply.startup_link_readback_from_bitmap() {
            for (mixer, links) in startup_links {
                for (index, linked) in links.into_iter().enumerate() {
                    let Some(linked) = linked else {
                        continue;
                    };
                    let Some(slot) = self.mixer_channels[mixer.index()].get_mut(index) else {
                        continue;
                    };
                    slot.linked = Some(linked);
                }
            }
        }

        if let Some((mixer, states)) = reply.startup_pan_state_readback() {
            for (index, state) in states.into_iter().enumerate() {
                let Some(state) = state else {
                    continue;
                };
                let Some(slot) = self.mixer_channels[mixer.index()].get_mut(index) else {
                    continue;
                };
                slot.level = Some(state.level);
                slot.pan = state.pan;
                slot.muted = Some(state.muted);
                slot.soloed = Some(state.soloed);
            }
        }

        if let Some(readback) = reply.mixer_strip_readback() {
            for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
                for (index, state) in readback.surfaces[mixer.index()].into_iter().enumerate() {
                    let Some(slot) = self.mixer_channels[mixer.index()].get_mut(index) else {
                        continue;
                    };
                    slot.soloed = Some(state.soloed);
                }
            }
        }
    }

    pub fn mark_disconnected(&mut self) {
        self.connection.connected = false;
        self.connection.last_frame_type = Some("disconnected");
    }

    pub fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            FocusArea::Status => FocusArea::Outputs,
            FocusArea::Outputs => FocusArea::Mixer,
            FocusArea::Mixer => FocusArea::Preamp,
            FocusArea::Preamp => FocusArea::Status,
        };
    }

    pub fn toggle_raw_view(&mut self) {
        self.raw_view_open = !self.raw_view_open;
    }

    pub fn cycle_raw_packet(&mut self, forward: bool) {
        let tabs = [
            RawPacketTab::Query74,
            RawPacketTab::State73,
            RawPacketTab::Auxiliary83,
            RawPacketTab::Query75,
            RawPacketTab::Notification81,
        ];
        let index = tabs
            .iter()
            .position(|tab| *tab == self.selected_raw_packet)
            .unwrap_or(0);
        self.selected_raw_packet = if forward {
            tabs[(index + 1) % tabs.len()]
        } else {
            tabs[index.checked_sub(1).unwrap_or(tabs.len() - 1)]
        };
    }

    pub fn cycle_query_reply_entry(&mut self, forward: bool) {
        if self.recent_query_reply_entries.is_empty() {
            self.selected_query_reply_entry = None;
            return;
        }
        let current = self
            .selected_query_reply_entry
            .unwrap_or(self.recent_query_reply_entries.len() - 1);
        self.selected_query_reply_entry = Some(if forward {
            (current + 1) % self.recent_query_reply_entries.len()
        } else {
            current
                .checked_sub(1)
                .unwrap_or(self.recent_query_reply_entries.len() - 1)
        });
    }

    pub fn capture_raw_baseline(&mut self) {
        self.baseline_raw_73 = self.latest_raw_73.clone();
        self.baseline_raw_83 = self.latest_raw_83.clone();
        self.baseline_raw_74 = self.latest_raw_74.clone();
        self.baseline_raw_75 = self.latest_raw_75.clone();
        self.baseline_raw_81 = self.latest_raw_81.clone();
    }

    pub fn clear_raw_baseline(&mut self) {
        self.baseline_raw_73 = None;
        self.baseline_raw_83 = None;
        self.baseline_raw_74 = None;
        self.baseline_raw_75 = None;
        self.baseline_raw_81 = None;
    }

    pub fn observe_query_request(&mut self, raw: Vec<u8>) {
        self.latest_raw_74 = Some(raw.clone());
        let query_id = raw.get(0x08).copied().unwrap_or(0);
        let sub_id = raw.get(0x0c).copied().unwrap_or(0);
        self.recent_query_request_log
            .push(format!("0x74 {:02x}/{:02x}", query_id, sub_id));
        if self.recent_query_request_log.len() > 16 {
            let drop_count = self.recent_query_request_log.len() - 16;
            self.recent_query_request_log.drain(0..drop_count);
        }
    }
}

fn startup_query_slot(query_id: u8) -> Option<usize> {
    match query_id {
        0x01 => Some(0),
        0x00 => Some(1),
        0x11 => Some(2),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum PendingMutation {
    MixerLevel {
        mixer: MixerSurface,
        channel: u8,
        level: u8,
        pan: PanState,
        muted: bool,
    },
    MixerLinkedLevel {
        mixer: MixerSurface,
        left_channel: u8,
        right_channel: u8,
        level: u8,
        left_pan: PanState,
        right_pan: PanState,
        left_muted: bool,
        right_muted: bool,
    },
    MixerMute {
        mixer: MixerSurface,
        channel: u8,
        muted: bool,
    },
    MixerLinkedMute {
        mixer: MixerSurface,
        left_channel: u8,
        right_channel: u8,
        muted: bool,
    },
    MixerSolo {
        mixer: MixerSurface,
        channel: u8,
        soloed: bool,
    },
    MixerLinkedSolo {
        mixer: MixerSurface,
        left_channel: u8,
        right_channel: u8,
        soloed: bool,
    },
    MixerPan {
        mixer: MixerSurface,
        channel: u8,
        pan: PanState,
    },
    MixerAssignment {
        strip: u8,
        assignment: MixerAssignment,
    },
    MixerLink {
        mixer: MixerSurface,
        selector: u8,
        enabled: bool,
    },
    MixerLinkExplicit {
        mixer: MixerSurface,
        left_channel: u8,
        right_channel: u8,
        enabled: bool,
    },
    OutputVolume {
        target: OutputTarget,
        step: u8,
    },
    OutputMode {
        target: OutputTarget,
        mode: OutputMode,
    },
    PreampGain {
        input: u8,
        raw: u8,
    },
    PreampMode {
        input: u8,
        mode: PreampMode,
    },
    PreampPhantom {
        input: u8,
        enabled: bool,
    },
    PreampPhase {
        input: u8,
        enabled: bool,
    },
}

pub struct Controller {
    transport: Box<dyn Transport>,
    pub state: AppState,
    pending_mutation: Option<PendingMutation>,
}

const MAX_FRAMES_PER_POLL: usize = 128;

impl Controller {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            state: AppState::default(),
            pending_mutation: None,
        }
    }

    pub fn bootstrap(&mut self) -> Result<()> {
        self.refresh_queried_state()
    }

    pub fn transport_available(&self) -> Result<bool> {
        self.transport.is_available()
    }

    pub fn refresh_queried_state(&mut self) -> Result<()> {
        for query in control_panel_startup_queries() {
            let frame = encode_query(*query);
            self.state.observe_query_request(frame.clone());
            self.transport.write(&frame)?;
        }
        Ok(())
    }

    fn shared_assignment_table(&self) -> Result<[MixerAssignment; 16]> {
        let mut assignments = [MixerAssignment::Mute; 16];
        for (index, slot) in assignments.iter_mut().enumerate() {
            *slot = self.state.mixer_channels[0][index]
                .assignment
                .or(self.state.mixer_channels[1][index].assignment)
                .ok_or_else(|| {
                    anyhow::anyhow!("assignment table is incomplete for CH {:02}", index + 1)
                })?;
        }
        Ok(assignments)
    }

    pub fn send(&mut self, command: Command) -> Result<()> {
        let pending_mutation = pending_from_command(command);
        if let Command::SetMixerAssignment { strip, assignment } = command {
            let assignments = self.shared_assignment_table()?;
            for frame in encode_mixer_assignment_frames_with_table(strip, assignment, &assignments)
            {
                self.transport.write(&frame)?;
            }
            self.pending_mutation = pending_mutation;
            self.state.last_message = format!("Sent {:?}", command);
            return Ok(());
        }
        if let Command::SetLinkState {
            enabled,
            companion_bank: Some(bank),
            ..
        } = command
        {
            self.transport
                .write(&encode_link_companion(bank, enabled))?;
        }
        self.transport.write(&encode_command(command))?;
        if matches!(command, Command::SelectSurface(_)) {
            self.refresh_queried_state()?;
        }
        self.pending_mutation = pending_mutation;
        self.state.last_message = format!("Sent {:?}", command);
        Ok(())
    }

    pub fn send_mixer_level_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        level: u8,
    ) -> Result<()> {
        let index = channel.saturating_sub(1) as usize;
        let Some(active) = self.state.mixer_channels[mixer.index()].get(index).copied() else {
            bail!("invalid mixer channel {channel}");
        };

        if active.linked == Some(true) {
            let (left_channel, right_channel) = if channel % 2 == 1 {
                (channel, channel.saturating_add(1))
            } else {
                (channel.saturating_sub(1), channel)
            };
            let left_index = left_channel.saturating_sub(1) as usize;
            let right_index = right_channel.saturating_sub(1) as usize;
            let Some(left) = self.state.mixer_channels[mixer.index()]
                .get(left_index)
                .copied()
            else {
                bail!("invalid linked left channel {left_channel}");
            };
            let Some(right) = self.state.mixer_channels[mixer.index()]
                .get(right_index)
                .copied()
            else {
                bail!("invalid linked right channel {right_channel}");
            };

            let pending_mutation = Some(PendingMutation::MixerLinkedLevel {
                mixer,
                left_channel,
                right_channel,
                level,
                left_pan: left.pan,
                right_pan: right.pan,
                left_muted: left.muted.unwrap_or(false),
                right_muted: right.muted.unwrap_or(false),
            });
            self.transport
                .write(&encode_command(Command::SetMixerLevel {
                    mixer,
                    channel: left_channel,
                    level,
                    pan_state: left.pan,
                    muted: left.muted.unwrap_or(false),
                    soloed: left.soloed.unwrap_or(false),
                }))?;
            self.transport
                .write(&encode_command(Command::SetMixerLevel {
                    mixer,
                    channel: right_channel,
                    level,
                    pan_state: right.pan,
                    muted: right.muted.unwrap_or(false),
                    soloed: right.soloed.unwrap_or(false),
                }))?;
            self.pending_mutation = pending_mutation;
            self.state.last_message = format!(
                "Sent linked mixer level {:?} ch {}-{}",
                mixer, left_channel, right_channel
            );
            return Ok(());
        }

        self.send(Command::SetMixerLevel {
            mixer,
            channel,
            level,
            pan_state: active.pan,
            muted: active.muted.unwrap_or(false),
            soloed: active.soloed.unwrap_or(false),
        })
    }

    pub fn send_mixer_mute_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        muted: bool,
    ) -> Result<()> {
        let index = channel.saturating_sub(1) as usize;
        let Some(active) = self.state.mixer_channels[mixer.index()].get(index).copied() else {
            bail!("invalid mixer channel {channel}");
        };

        if active.linked == Some(true) {
            let (left_channel, right_channel) = if channel % 2 == 1 {
                (channel, channel.saturating_add(1))
            } else {
                (channel.saturating_sub(1), channel)
            };
            let left_index = left_channel.saturating_sub(1) as usize;
            let right_index = right_channel.saturating_sub(1) as usize;
            let Some(left) = self.state.mixer_channels[mixer.index()]
                .get(left_index)
                .copied()
            else {
                bail!("invalid linked left channel {left_channel}");
            };
            let Some(right) = self.state.mixer_channels[mixer.index()]
                .get(right_index)
                .copied()
            else {
                bail!("invalid linked right channel {right_channel}");
            };

            let pending_mutation = Some(PendingMutation::MixerLinkedMute {
                mixer,
                left_channel,
                right_channel,
                muted,
            });
            self.transport
                .write(&encode_command(Command::SetMixerMute {
                    mixer,
                    channel: left_channel,
                    muted,
                    pan_state: left.pan,
                    soloed: left.soloed.unwrap_or(false),
                }))?;
            self.transport
                .write(&encode_command(Command::SetMixerMute {
                    mixer,
                    channel: right_channel,
                    muted,
                    pan_state: right.pan,
                    soloed: right.soloed.unwrap_or(false),
                }))?;
            self.pending_mutation = pending_mutation;
            self.state.last_message = format!(
                "Sent linked mixer mute {:?} ch {}-{}",
                mixer, left_channel, right_channel
            );
            return Ok(());
        }

        self.send(Command::SetMixerMute {
            mixer,
            channel,
            muted,
            pan_state: active.pan,
            soloed: active.soloed.unwrap_or(false),
        })
    }

    pub fn send_mixer_solo_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        soloed: bool,
    ) -> Result<()> {
        let index = channel.saturating_sub(1) as usize;
        let Some(active) = self.state.mixer_channels[mixer.index()].get(index).copied() else {
            bail!("invalid mixer channel {channel}");
        };

        if active.linked == Some(true) {
            let (left_channel, right_channel) = if channel % 2 == 1 {
                (channel, channel.saturating_add(1))
            } else {
                (channel.saturating_sub(1), channel)
            };
            let left_index = left_channel.saturating_sub(1) as usize;
            let right_index = right_channel.saturating_sub(1) as usize;
            let Some(left) = self.state.mixer_channels[mixer.index()]
                .get(left_index)
                .copied()
            else {
                bail!("invalid linked left channel {left_channel}");
            };
            let Some(right) = self.state.mixer_channels[mixer.index()]
                .get(right_index)
                .copied()
            else {
                bail!("invalid linked right channel {right_channel}");
            };

            let pending_mutation = Some(PendingMutation::MixerLinkedSolo {
                mixer,
                left_channel,
                right_channel,
                soloed,
            });
            self.transport
                .write(&encode_command(Command::SetMixerSolo {
                    mixer,
                    channel: left_channel,
                    soloed,
                    muted: left.muted.unwrap_or(false),
                    pan_state: left.pan,
                }))?;
            self.transport
                .write(&encode_command(Command::SetMixerSolo {
                    mixer,
                    channel: right_channel,
                    soloed,
                    muted: right.muted.unwrap_or(false),
                    pan_state: right.pan,
                }))?;
            self.pending_mutation = pending_mutation;
            self.state.last_message = format!(
                "Sent linked mixer solo {:?} ch {}-{}",
                mixer, left_channel, right_channel
            );
            return Ok(());
        }

        self.send(Command::SetMixerSolo {
            mixer,
            channel,
            soloed,
            muted: active.muted.unwrap_or(false),
            pan_state: active.pan,
        })
    }

    pub fn send_mixer_link_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        enabled: bool,
    ) -> Result<()> {
        let Some(target) = MixerLinkTarget::from_channel(mixer, channel) else {
            bail!("invalid mixer link channel {channel}");
        };
        let pending_mutation = Some(PendingMutation::MixerLinkExplicit {
            mixer,
            left_channel: target.left_channel,
            right_channel: target.right_channel,
            enabled,
        });
        if let Some(bank) = target.companion_bank() {
            self.transport
                .write(&encode_link_companion(bank, enabled))?;
        }
        self.transport
            .write(&encode_command(Command::SetLinkState {
                selector: target.selector,
                enabled,
                companion_bank: None,
            }))?;
        self.pending_mutation = pending_mutation;
        self.state.last_message = format!(
            "Sent mixer link {:?} ch {}-{}",
            mixer, target.left_channel, target.right_channel
        );
        Ok(())
    }

    pub fn poll_device(&mut self, timeout: Duration) -> Result<()> {
        let mut next_timeout = timeout;

        for _ in 0..MAX_FRAMES_PER_POLL {
            let Some(bytes) = self.transport.read(next_timeout)? else {
                break;
            };

            next_timeout = Duration::ZERO;

            if let Ok(frame) = Frame::parse(&bytes) {
                let raw = frame.raw_bytes().to_vec();
                let snapshot = DeviceSnapshot::from(frame);
                if let DeviceSnapshot::Snapshot(snapshot73) = &snapshot {
                    self.confirm_pending_write(snapshot73.clone());
                }
                self.state.observe_frame(snapshot, raw);
            }
        }

        Ok(())
    }

    pub fn confirm_pending_write(&mut self, _snapshot: Snapshot73) {
        match self.pending_mutation.take() {
            Some(PendingMutation::MixerLevel {
                mixer,
                channel,
                level,
                pan,
                muted,
            }) => {
                if let Some(slot) = self.state.mixer_channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.level = Some(level);
                    slot.muted = Some(muted);
                    slot.pan = pan;
                }
            }
            Some(PendingMutation::MixerLinkedLevel {
                mixer,
                left_channel,
                right_channel,
                level,
                left_pan,
                right_pan,
                left_muted,
                right_muted,
            }) => {
                for (channel, pan, muted) in [
                    (left_channel, left_pan, left_muted),
                    (right_channel, right_pan, right_muted),
                ] {
                    if let Some(slot) = self.state.mixer_channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.level = Some(level);
                        slot.muted = Some(muted);
                        slot.pan = pan;
                    }
                }
            }
            Some(PendingMutation::MixerMute {
                mixer,
                channel,
                muted,
            }) => {
                if let Some(slot) = self.state.mixer_channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.muted = Some(muted);
                }
            }
            Some(PendingMutation::MixerSolo {
                mixer,
                channel,
                soloed,
            }) => {
                if let Some(slot) = self.state.mixer_channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.soloed = Some(soloed);
                }
            }
            Some(PendingMutation::MixerLinkedMute {
                mixer,
                left_channel,
                right_channel,
                muted,
            }) => {
                for channel in [left_channel, right_channel] {
                    if let Some(slot) = self.state.mixer_channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.muted = Some(muted);
                    }
                }
            }
            Some(PendingMutation::MixerLinkedSolo {
                mixer,
                left_channel,
                right_channel,
                soloed,
            }) => {
                for channel in [left_channel, right_channel] {
                    if let Some(slot) = self.state.mixer_channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.soloed = Some(soloed);
                    }
                }
            }
            Some(PendingMutation::MixerPan {
                mixer,
                channel,
                pan,
            }) => {
                if let Some(slot) = self.state.mixer_channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.pan = pan;
                }
            }
            Some(PendingMutation::MixerAssignment { strip, assignment }) => {
                let index = strip.saturating_sub(1) as usize;
                for channels in &mut self.state.mixer_channels {
                    if let Some(slot) = channels.get_mut(index) {
                        slot.assignment = Some(assignment);
                    }
                }
            }
            Some(PendingMutation::MixerLink {
                mixer,
                selector,
                enabled,
            }) => {
                if let Some((left, right)) = link_pair_from_selector(mixer, selector) {
                    for channel in [left, right] {
                        if let Some(slot) = self.state.mixer_channels[mixer.index()]
                            .get_mut(channel.saturating_sub(1) as usize)
                        {
                            slot.linked = Some(enabled);
                        }
                    }
                }
            }
            Some(PendingMutation::MixerLinkExplicit {
                mixer,
                left_channel,
                right_channel,
                enabled,
            }) => {
                for channel in [left_channel, right_channel] {
                    if let Some(slot) = self.state.mixer_channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.linked = Some(enabled);
                    }
                }
            }
            Some(PendingMutation::OutputVolume { target, step }) => {
                self.state.outputs[target.index() as usize].volume = step;
            }
            Some(PendingMutation::OutputMode { target, mode }) => {
                self.state.outputs[target.index() as usize].mode = mode;
            }
            Some(PendingMutation::PreampGain { input, raw }) => {
                self.state.dsp_cluster[input.min(1) as usize] = raw;
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
            }
            Some(PendingMutation::PreampMode { input, mode }) => {
                let offset = 2 + input.min(1) as usize;
                let preserved_bits = self.state.dsp_cluster[offset] & 0xf0;
                self.state.dsp_cluster[offset] = preserved_bits | mode.code();
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
            }
            Some(PendingMutation::PreampPhantom { input, enabled }) => {
                let offset = 2 + input.min(1) as usize;
                let low = self.state.dsp_cluster[offset] & 0x0f;
                self.state.dsp_cluster[offset] = low | if enabled { 0x10 } else { 0x00 };
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
            }
            Some(PendingMutation::PreampPhase { input, enabled }) => {
                let offset = 2 + input.min(1) as usize;
                let low = self.state.dsp_cluster[offset] & 0x1f;
                self.state.dsp_cluster[offset] = low | if enabled { 0x40 } else { 0x00 };
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
            }
            None => {}
        }
    }
}

fn pending_from_command(command: Command) -> Option<PendingMutation> {
    match command {
        Command::SetMixerLevel {
            mixer,
            channel,
            level,
            pan_state,
            muted,
            soloed: _,
        } => Some(PendingMutation::MixerLevel {
            mixer,
            channel,
            level,
            pan: pan_state,
            muted,
        }),
        Command::SetMixerMute {
            mixer,
            channel,
            muted,
            ..
        } => Some(PendingMutation::MixerMute {
            mixer,
            channel,
            muted,
        }),
        Command::SetMixerSolo {
            mixer,
            channel,
            soloed,
            ..
        } => Some(PendingMutation::MixerSolo {
            mixer,
            channel,
            soloed,
        }),
        Command::SetMixerPan {
            mixer,
            channel,
            pan,
            muted: _,
            soloed: _,
        } => Some(PendingMutation::MixerPan {
            mixer,
            channel,
            pan,
        }),
        Command::SetMixerAssignment { strip, assignment } => {
            Some(PendingMutation::MixerAssignment { strip, assignment })
        }
        Command::SetLinkState {
            selector,
            enabled,
            companion_bank: _,
        } => {
            let mixer = if MixerLinkTarget::from_selector(MixerSurface::Mix1, selector).is_some() {
                MixerSurface::Mix1
            } else if MixerLinkTarget::from_selector(MixerSurface::Mix2, selector).is_some() {
                MixerSurface::Mix2
            } else {
                return None;
            };

            Some(PendingMutation::MixerLink {
                mixer,
                selector,
                enabled,
            })
        }
        Command::SetOutputVolume { target, step } => {
            Some(PendingMutation::OutputVolume { target, step })
        }
        Command::SetOutputMute { target, enabled } => Some(PendingMutation::OutputMode {
            target,
            mode: if enabled {
                OutputMode::Mute
            } else {
                OutputMode::Normal
            },
        }),
        Command::SetOutputDim { target, enabled } => Some(PendingMutation::OutputMode {
            target,
            mode: if enabled {
                OutputMode::Dim
            } else {
                OutputMode::Normal
            },
        }),
        Command::SetPreampGain { input, raw } => Some(PendingMutation::PreampGain { input, raw }),
        Command::SetPreampMode { input, mode } => Some(PendingMutation::PreampMode { input, mode }),
        Command::SetPreampPhantom { input, enabled } => {
            Some(PendingMutation::PreampPhantom { input, enabled })
        }
        Command::SetPreampPhase { input, enabled } => {
            Some(PendingMutation::PreampPhase { input, enabled })
        }
        _ => None,
    }
}

fn link_pair_from_selector(mixer: MixerSurface, selector: u8) -> Option<(u8, u8)> {
    MixerLinkTarget::from_selector(mixer, selector)
        .map(|target| (target.left_channel, target.right_channel))
}

#[cfg(test)]
mod tests {
    use crate::protocol::{
        ClockSource, Command, DeviceSnapshot, MixerAssignment, MixerChannelState, MixerStrip,
        MixerSurface, OutputMode, OutputState, OutputTarget, PanState, PreampMode, PreampState,
        SampleRate, Snapshot73, Surface,
    };
    use crate::transport::MockTransport;

    use super::*;

    fn snapshot() -> Snapshot73 {
        Snapshot73 {
            sample_rate: SampleRate::Hz48000,
            clock_source: ClockSource::Internal,
            sample_rate_hz: 48_000,
            status_flags: [0x08, 0x00],
            front_panel_bytes: [0, 0, 0],
            outputs: [
                OutputState::new(OutputTarget::Monitor, 0x50, OutputMode::Normal),
                OutputState::new(OutputTarget::Hp1, 0x40, OutputMode::Mute),
                OutputState::new(OutputTarget::Hp2, 0x30, OutputMode::Dim),
            ],
            dsp_cluster: [0x2f, 0x34, 0x50, 0x10],
            preamp: PreampState::from_cluster([0x2f, 0x34, 0x50, 0x10]),
            surface: Surface::MonitorHp1,
            mixer_decode: Default::default(),
            late_shadow: [0; 12],
        }
    }

    fn seed_shared_assignments(state: &mut AppState) {
        let assignments = [
            MixerAssignment::Preamp(1),
            MixerAssignment::Preamp(2),
            MixerAssignment::ComputerPlay(1),
            MixerAssignment::ComputerPlay(2),
            MixerAssignment::ComputerPlay(3),
            MixerAssignment::ComputerPlay(4),
            MixerAssignment::ComputerPlay(5),
            MixerAssignment::ComputerPlay(6),
            MixerAssignment::ComputerPlay(7),
            MixerAssignment::ComputerPlay(8),
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
        ];

        for surface in &mut state.mixer_channels {
            for (channel, assignment) in surface.iter_mut().zip(assignments) {
                channel.assignment = Some(assignment);
            }
        }
    }

    fn assignment_pairs(frame: &[u8], count: usize) -> Vec<[u8; 2]> {
        let payload = &frame[0x10 + 0x03..];
        payload
            .chunks_exact(2)
            .take(count)
            .map(|chunk| [chunk[0], chunk[1]])
            .collect()
    }

    #[test]
    fn reducer_prefers_device_snapshot_state() {
        let mut state = AppState::default();
        state.outputs[0].volume = 0x10;

        state.apply_snapshot(snapshot());

        assert_eq!(state.device.sample_rate, Some(SampleRate::Hz48000));
        assert_eq!(state.outputs[0].volume, 0x50);
        assert_eq!(state.outputs[1].mode, OutputMode::Mute);
        assert_eq!(state.surface, Surface::MonitorHp1);
    }

    #[test]
    fn reducer_updates_preamp_state_from_snapshot() {
        let mut state = AppState::default();
        let mut device_snapshot = snapshot();
        device_snapshot.dsp_cluster = [0x14, 0x2a, 0x11, 0x00];

        state.apply_snapshot(device_snapshot);

        assert_eq!(state.preamp.input1.mode, PreampMode::Line);
        assert_eq!(state.preamp.input1.gain_raw, 0x14);
        assert_eq!(state.preamp.input2.mode, PreampMode::Mic);
        assert_eq!(state.preamp.input2.gain_raw, 0x2a);
        assert!(!state.preamp.input2.phantom_on);
    }

    #[test]
    fn reducer_applies_grounded_passive_mixer_decode_from_snapshot() {
        let mut state = AppState::default();
        let mut device_snapshot = snapshot();
        device_snapshot.mixer_decode.observed_preamp1_meter = Some(0x28);
        device_snapshot.mixer_decode.observed_preamp2_meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix2.index()][0].meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].muted = Some(false);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].linked = Some(true);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][1].linked = Some(true);

        state.apply_snapshot(device_snapshot);

        assert_eq!(state.preamp.input1.observed_meter, Some(0x28));
        assert_eq!(state.preamp.input2.observed_meter, Some(0x30));
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix1.index()][0].meter,
            Some(0x30)
        );
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix2.index()][0].meter,
            Some(0x30)
        );
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix1.index()][0].level,
            None
        );
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix1.index()][0].muted,
            Some(false)
        );
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix1.index()][0].pan,
            PanState::center()
        );
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix1.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix1.index()][1].linked,
            Some(true)
        );
    }

    #[test]
    fn passive_snapshot_pan_decode_does_not_override_channel_pan() {
        let mut state = AppState::default();
        state.mixer_channels[MixerSurface::Mix1.index()][0].pan = PanState::center();

        let mut device_snapshot = snapshot();
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].pan =
            Some(PanState::from_raw(0x1e));

        state.apply_snapshot(device_snapshot);

        assert_eq!(
            state.mixer_channels[MixerSurface::Mix1.index()][0].pan,
            PanState::center()
        );
    }

    #[test]
    fn query_reply_assignment_readback_updates_shared_strip_assignments() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x03,
                sub_id: 0x05,
                body: vec![0x05, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x01, 0x01],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x03,
                sub_id: 0x06,
                body: vec![
                    0x06, 0x03, 0x00, 0x03, 0x01, 0x03, 0x02, 0x03, 0x03, 0x01, 0x02, 0x01, 0x03,
                    0x01, 0x04, 0x01, 0x05, 0x01, 0x06, 0x01, 0x07, 0x08, 0x00, 0x08, 0x00, 0x08,
                    0x00, 0x08, 0x00, 0x08, 0x00, 0x08, 0x00,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            let channels = &state.mixer_channels[mixer.index()];
            assert_eq!(channels[0].assignment, Some(MixerAssignment::Preamp(1)));
            assert_eq!(channels[1].assignment, Some(MixerAssignment::Preamp(2)));
            assert_eq!(
                channels[2].assignment,
                Some(MixerAssignment::ComputerPlay(1))
            );
            assert_eq!(
                channels[3].assignment,
                Some(MixerAssignment::ComputerPlay(2))
            );
            assert_eq!(
                channels[4].assignment,
                Some(MixerAssignment::ComputerPlay(3))
            );
            assert_eq!(
                channels[9].assignment,
                Some(MixerAssignment::ComputerPlay(8))
            );
            assert!(channels[10..]
                .iter()
                .all(|slot| slot.assignment == Some(MixerAssignment::Mute)));
        }
    }

    #[test]
    fn query_reply_startup_link_readback_updates_visible_pairs_from_bitmap() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x0b,
                sub_id: 0x03,
                body: vec![
                    0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            let channels = &state.mixer_channels[mixer.index()];
            let expected_primary = if mixer == MixerSurface::Mix1 {
                Some(true)
            } else {
                Some(false)
            };
            assert_eq!(channels[0].linked, expected_primary);
            assert_eq!(channels[1].linked, expected_primary);
            for index in (2..10).step_by(2) {
                assert_eq!(channels[index].linked, Some(true));
                assert_eq!(channels[index + 1].linked, Some(true));
            }
            assert!(channels[10..].iter().all(|slot| slot.linked == Some(false)));
        }

        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x0b,
                sub_id: 0x03,
                body: vec![
                    0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        let mix1 = &state.mixer_channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer_channels[MixerSurface::Mix2.index()];
        assert!(mix1[10..].iter().all(|slot| slot.linked == Some(true)));
        assert!(mix2[10..].iter().all(|slot| slot.linked == Some(true)));
    }

    #[test]
    fn query_reply_startup_pan_state_readback_updates_mix_pan_and_mute() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x04,
                sub_id: 0x00,
                body: vec![
                    0x00, 0x20, 0x00, 0x5e, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x04,
                sub_id: 0x01,
                body: vec![
                    0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x02, 0x00, 0x3e,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        let mix1 = &state.mixer_channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer_channels[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, Some(0x00));
        assert_eq!(mix1[0].pan, PanState::from_raw(0x1e));
        assert_eq!(mix1[0].muted, Some(true));
        assert_eq!(mix1[1].level, Some(0x00));
        assert_eq!(mix1[1].pan, PanState::center());
        assert_eq!(mix1[1].muted, Some(true));
        assert_eq!(mix1[2].level, Some(0x00));
        assert_eq!(mix1[2].pan, PanState::left());
        assert_eq!(mix1[2].muted, Some(false));
        assert_eq!(mix1[3].level, Some(0x00));
        assert_eq!(mix1[3].pan, PanState::right());
        assert_eq!(mix1[3].muted, Some(false));
        assert_eq!(mix2[10].level, Some(0x00));
        assert_eq!(mix2[10].pan, PanState::left());
        assert_eq!(mix2[10].muted, Some(false));
        assert_eq!(mix2[11].level, Some(0x00));
        assert_eq!(mix2[11].pan, PanState::right());
        assert_eq!(mix2[11].muted, Some(false));
    }

    #[test]
    fn query_reply_startup_level_readback_updates_mix_levels() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x04,
                sub_id: 0x00,
                body: vec![
                    0x00, 0x20, 0x12, 0x5e, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x04,
                sub_id: 0x01,
                body: vec![
                    0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x1e, 0x02, 0x1e, 0x3e,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        let mix1 = &state.mixer_channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer_channels[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, Some(0x12));
        assert_eq!(mix2[10].level, Some(0x1e));
        assert_eq!(mix2[11].level, Some(0x1e));
    }

    #[test]
    fn query_reply_strip_readback_does_not_seed_unstable_startup_state() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x18,
                sub_id: 0x00,
                body: vec![
                    0x00, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x02, 0x60, 0x3e, 0x2e, 0x02, 0x60,
                    0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e,
                    0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60,
                    0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                    0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        let mix1 = &state.mixer_channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer_channels[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, None);
        assert_eq!(mix1[0].pan, PanState::center());
        assert_eq!(mix1[0].muted, None);
        assert!(mix1.iter().all(|slot| slot.level.is_none()));
        assert!(mix1.iter().all(|slot| slot.muted.is_none()));
        assert!(mix1.iter().all(|slot| slot.pan == PanState::center()));
        assert!(mix2.iter().all(|slot| slot.level.is_none()));
        assert!(mix2.iter().all(|slot| slot.muted.is_none()));
        assert!(mix2.iter().all(|slot| slot.pan == PanState::center()));
    }

    #[test]
    fn query_reply_strip_readback_does_not_apply_pan_or_mute_overlay() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x18,
                sub_id: 0x00,
                body: vec![
                    0x12, 0x3e, 0x60, 0x60, 0x60, 0x60, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x20, 0x60,
                    0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                    0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60,
                    0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                    0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        let mix1 = &state.mixer_channels[MixerSurface::Mix1.index()];
        assert_eq!(mix1[0].muted, None);
        assert_eq!(mix1[0].pan, PanState::center());
        assert!(mix1.iter().all(|slot| slot.muted.is_none()));
        assert!(mix1.iter().all(|slot| slot.pan == PanState::center()));
    }

    #[test]
    fn passive_meter_does_not_override_known_level_value() {
        let mut state = AppState::default();
        state.mixer_channels[MixerSurface::Mix1.index()][0].level = Some(0x00);

        let mut device_snapshot = snapshot();
        device_snapshot.mixer_decode.observed_preamp2_meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix2.index()][0].meter = Some(0x30);

        state.apply_snapshot(device_snapshot);

        assert_eq!(state.preamp.input2.observed_meter, Some(0x30));
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix1.index()][0].level,
            Some(0x00)
        );
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix1.index()][0].meter,
            Some(0x30)
        );
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix2.index()][0].meter,
            Some(0x30)
        );
    }

    #[test]
    fn preamp_pending_gain_updates_authoritative_cluster() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.dsp_cluster = [0x0a, 0x0a, 0x00, 0x00];
        controller.state.preamp = PreampState::from_cluster(controller.state.dsp_cluster);

        controller
            .send(Command::SetPreampGain {
                input: 1,
                raw: 0x2d,
            })
            .expect("send preamp gain");
        controller.confirm_pending_write(snapshot());

        assert_eq!(controller.state.preamp.input2.gain_raw, 0x2d);
        assert_eq!(controller.state.dsp_cluster[1], 0x2d);
    }

    #[test]
    fn preamp_pending_updates_preserve_observed_input_meters() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.dsp_cluster = [0x0a, 0x0a, 0x00, 0x00];
        controller.state.preamp = PreampState::from_cluster(controller.state.dsp_cluster);
        controller.state.preamp.input2.observed_meter = Some(0x30);

        controller
            .send(Command::SetPreampGain {
                input: 1,
                raw: 0x2d,
            })
            .expect("send preamp gain");
        controller.confirm_pending_write(snapshot());

        assert_eq!(controller.state.preamp.input2.gain_raw, 0x2d);
        assert_eq!(controller.state.preamp.input1.observed_meter, None);
        assert_eq!(controller.state.preamp.input2.observed_meter, Some(0x30));
    }

    #[test]
    fn preamp_pending_mode_phantom_and_phase_update_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.dsp_cluster = [0x0a, 0x0a, 0x00, 0x00];
        controller.state.preamp = PreampState::from_cluster(controller.state.dsp_cluster);

        controller
            .send(Command::SetPreampMode {
                input: 0,
                mode: PreampMode::Line,
            })
            .expect("send preamp mode");
        controller.confirm_pending_write(snapshot());
        assert_eq!(controller.state.preamp.input1.mode, PreampMode::Line);

        controller.state.dsp_cluster[3] = 0x00;
        controller.state.preamp = PreampState::from_cluster(controller.state.dsp_cluster);
        controller
            .send(Command::SetPreampPhantom {
                input: 1,
                enabled: true,
            })
            .expect("send preamp phantom");
        controller.confirm_pending_write(snapshot());
        assert!(controller.state.preamp.input2.phantom_on);

        controller.state.dsp_cluster[3] = 0x00;
        controller.state.preamp = PreampState::from_cluster(controller.state.dsp_cluster);
        controller
            .send(Command::SetPreampPhase {
                input: 1,
                enabled: true,
            })
            .expect("send preamp phase");
        controller.confirm_pending_write(snapshot());
        assert_eq!(controller.state.dsp_cluster[3], 0x40);
    }

    #[test]
    fn bootstrap_sends_queries_and_mutations_use_transport() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller.bootstrap().expect("bootstrap");
        controller
            .send(Command::SetClockSource(ClockSource::Usb))
            .expect("write command");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 48);
        assert_eq!(&writes[0][0x08..0x10], &[0x01, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[1][0x08..0x10], &[0x11, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[2][0x08..0x10], &[0x0a, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[46][0x08..0x10], &[0x12, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[47][0x10..0x12], &[0x04, 0x02]);
    }

    #[test]
    fn bootstrap_queries_include_metadata_request() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller.bootstrap().expect("bootstrap");

        let writes = transport.take_writes();
        assert!(writes
            .iter()
            .any(|frame| &frame[0x08..0x10] == [0x01, 0, 0, 0, 0, 0, 0, 0]));
    }

    #[test]
    fn surface_select_refreshes_query_readback() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .send(Command::SelectSurface(Surface::Hp2))
            .expect("select surface");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 48);
        assert_eq!(&writes[0][0x10..0x13], &[0x49, 0x00, Surface::Hp2.code()]);
        assert_eq!(&writes[1][0x08..0x10], &[0x01, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[47][0x08..0x10], &[0x12, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn mixer_overlay_is_tracked_only_after_command_round_trip() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .send(Command::SetMixerLevel {
                mixer: crate::protocol::MixerSurface::Mix1,
                channel: 3,
                level: 0x2c,
                pan_state: crate::protocol::PanState::left(),
                muted: false,
                soloed: false,
            })
            .expect("send mixer");

        assert!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][2]
                .level
                .is_none()
        );

        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][2],
            MixerChannelState::known(3, Some(0x2c), Some(false), PanState::left(), None, None)
        );
    }

    #[test]
    fn linked_mixer_level_change_writes_and_updates_both_channels() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.mixer_channels[MixerSurface::Mix1.index()][2].linked = Some(true);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][3].linked = Some(true);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][2].pan = PanState::left();
        controller.state.mixer_channels[MixerSurface::Mix1.index()][3].pan = PanState::right();

        controller
            .send_mixer_level_change(MixerSurface::Mix1, 4, 0x2c)
            .expect("send linked mixer level");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x03, 0x2c, 0x02]
        );
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x04, 0x2c, 0x3e]
        );

        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][2].level,
            Some(0x2c)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][3].level,
            Some(0x2c)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][2].pan,
            PanState::left()
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][3].pan,
            PanState::right()
        );
    }

    #[test]
    fn linked_mixer_mute_change_writes_and_updates_both_channels() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.mixer_channels[MixerSurface::Mix1.index()][2].linked = Some(true);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][3].linked = Some(true);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][2].pan = PanState::left();
        controller.state.mixer_channels[MixerSurface::Mix1.index()][3].pan = PanState::right();

        controller
            .send_mixer_mute_change(MixerSurface::Mix1, 3, true)
            .expect("send linked mixer mute");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x03, 0x00, 0x42]
        );
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x04, 0x00, 0x7e]
        );

        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][2].muted,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][3].muted,
            Some(true)
        );
    }

    #[test]
    fn linked_mixer_solo_change_writes_and_updates_both_channels() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.mixer_channels[MixerSurface::Mix1.index()][2].linked = Some(true);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][3].linked = Some(true);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][2].pan = PanState::left();
        controller.state.mixer_channels[MixerSurface::Mix1.index()][3].pan = PanState::right();
        controller.state.mixer_channels[MixerSurface::Mix1.index()][2].muted = Some(false);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][3].muted = Some(false);

        controller
            .send_mixer_solo_change(MixerSurface::Mix1, 4, true)
            .expect("send linked mixer solo");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x03, 0x00, 0x82]
        );
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x04, 0x00, 0xbe]
        );

        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][2].soloed,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][3].soloed,
            Some(true)
        );
    }

    #[test]
    fn queried_mixer_strip_readback_updates_solo_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        let mut body = [0x5a, 0x20].repeat(32);
        body[0] = 0x10;
        body[1] = 0xa0;
        body[32] = 0x10;
        body[33] = 0x20;

        controller.state.observe_frame(
            DeviceSnapshot::QueryReply(QueryReply75 {
                query_id: 0x18,
                sub_id: 0x00,
                body,
            }),
            vec![0x75, 0x18, 0x00],
        );

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][0].soloed,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][0].soloed,
            Some(false)
        );
    }

    #[test]
    fn grounded_link_target_maps_extended_pair_selectors() {
        let mix1 = MixerLinkTarget::from_channel(MixerSurface::Mix1, 11).expect("mix1 target");
        assert_eq!(
            (mix1.left_channel, mix1.right_channel, mix1.selector),
            (11, 12, 0x05)
        );
        assert_eq!(mix1.companion_bank(), None);

        let mix2 = MixerLinkTarget::from_channel(MixerSurface::Mix2, 15).expect("mix2 target");
        assert_eq!(
            (mix2.left_channel, mix2.right_channel, mix2.selector),
            (15, 16, 0x17)
        );
        assert_eq!(mix2.companion_bank(), None);
    }

    #[test]
    fn mixer_link_change_writes_selector_and_updates_pair() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .send_mixer_link_change(MixerSurface::Mix1, 11, true)
            .expect("send mix1 link");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x14], &[0xa2, 0x03, 0x05, 0x01]);
        controller.confirm_pending_write(snapshot());
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][10].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][11].linked,
            Some(true)
        );

        controller
            .send_mixer_link_change(MixerSurface::Mix2, 15, true)
            .expect("send mix2 link");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x14], &[0xa2, 0x03, 0x17, 0x01]);
        controller.confirm_pending_write(snapshot());
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][14].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][15].linked,
            Some(true)
        );
    }

    #[test]
    fn app_state_starts_with_16_strips_per_surface() {
        let state = AppState::default();

        assert_eq!(state.mixer_channels[MixerSurface::Mix1.index()].len(), 16);
        assert_eq!(state.mixer_channels[MixerSurface::Mix2.index()].len(), 16);
        assert_eq!(
            state.mixer_channels[MixerSurface::Mix1.index()][15].channel,
            16
        );
    }

    #[test]
    fn mixer_assignment_is_shared_across_surfaces_but_link_is_not() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller.pending_mutation = Some(PendingMutation::MixerAssignment {
            strip: 11,
            assignment: MixerAssignment::Oscillator(2),
        });
        controller.confirm_pending_write(snapshot());

        let target = MixerLinkTarget::from_channel(MixerSurface::Mix2, 1).expect("mix2 1-2");
        controller
            .send(Command::SetLinkState {
                selector: target.selector,
                enabled: true,
                companion_bank: target.companion_bank(),
            })
            .expect("send link");
        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][10].assignment,
            Some(MixerAssignment::Oscillator(2))
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][10].assignment,
            Some(MixerAssignment::Oscillator(2))
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][1].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][0].linked,
            None
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][1].linked,
            None
        );
    }

    #[test]
    fn mixer_assignment_overlay_updates_both_surfaces_for_strip_11() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller.pending_mutation = Some(PendingMutation::MixerAssignment {
            strip: 11,
            assignment: MixerAssignment::Mute,
        });
        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][10].assignment,
            Some(MixerAssignment::Mute)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][10].assignment,
            Some(MixerAssignment::Mute)
        );
    }

    #[test]
    fn mixer_assignment_write_sends_ordinary_strip_frames_and_updates_shared_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller.state);

        controller
            .send(Command::SetMixerAssignment {
                strip: 5,
                assignment: MixerAssignment::Oscillator(1),
            })
            .expect("assignment write should succeed");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 5);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x03]);
        assert_eq!(&writes[0][0x10 + 0x0b..0x10 + 0x0d], &[0x09, 0x00]);

        controller.confirm_pending_write(snapshot());
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][4].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][4].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
    }

    #[test]
    fn mixer_assignment_write_sends_early_strip_frames_and_updates_shared_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller.state);

        controller
            .send(Command::SetMixerAssignment {
                strip: 1,
                assignment: MixerAssignment::Oscillator(1),
            })
            .expect("assignment write should succeed");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x05]);
        assert_eq!(&writes[0][0x10 + 0x03..0x10 + 0x05], &[0x09, 0x00]);

        controller.confirm_pending_write(snapshot());
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][0].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][0].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
    }

    #[test]
    fn late_strip_assignment_write_preserves_existing_assignment_table_entries() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller.state);

        controller
            .send(Command::SetMixerAssignment {
                strip: 11,
                assignment: MixerAssignment::ComputerPlay(1),
            })
            .expect("assignment write should succeed");

        let writes = transport.take_writes();
        let bank06 = writes
            .iter()
            .find(|frame| frame[0x10..0x13] == [0xd3, 0x41, 0x06])
            .expect("bank 06 frame");

        assert_eq!(
            assignment_pairs(bank06, 16),
            vec![
                [0x03, 0x00],
                [0x03, 0x01],
                [0x03, 0x02],
                [0x03, 0x03],
                [0x01, 0x02],
                [0x01, 0x03],
                [0x01, 0x04],
                [0x01, 0x05],
                [0x01, 0x06],
                [0x01, 0x07],
                [0x01, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
            ]
        );
    }

    #[test]
    fn link_overlay_respects_full_visible_pair_mapping() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        for target in [
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 1).expect("mix1 1-2"),
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 5).expect("mix1 5-6"),
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 7).expect("mix1 7-8"),
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 1).expect("mix2 1-2"),
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 7).expect("mix2 7-8"),
        ] {
            controller
                .send(Command::SetLinkState {
                    selector: target.selector,
                    enabled: true,
                    companion_bank: target.companion_bank(),
                })
                .expect("send grounded link");
            controller.confirm_pending_write(snapshot());

            assert_eq!(
                controller.state.mixer_channels[target.mixer.index()]
                    [target.left_channel as usize - 1]
                    .linked,
                Some(true)
            );
            assert_eq!(
                controller.state.mixer_channels[target.mixer.index()]
                    [target.right_channel as usize - 1]
                    .linked,
                Some(true)
            );
        }
        assert!(MixerStrip::ordinary(4).is_none());
    }

    #[test]
    fn grounded_link_with_companion_writes_helper_before_selector_write() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        let target = MixerLinkTarget::from_channel(MixerSurface::Mix1, 1).expect("mix1 1-2");

        controller
            .send(Command::SetLinkState {
                selector: target.selector,
                enabled: true,
                companion_bank: target.companion_bank(),
            })
            .expect("send link with companion");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(&writes[0][0x10..0x14], &[0xa2, 0x04, 0x00, 0x01]);
        assert_eq!(&writes[1][0x10..0x14], &[0xa2, 0x03, 0x00, 0x01]);

        controller.confirm_pending_write(snapshot());
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][1].linked,
            Some(true)
        );
    }

    #[test]
    fn mixer_pan_updates_are_tracked_per_surface() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller
            .send(Command::SetMixerPan {
                mixer: MixerSurface::Mix1,
                channel: 4,
                pan: PanState::from_raw(0x08),
                muted: false,
                soloed: false,
            })
            .expect("mix1 pan");
        controller.confirm_pending_write(snapshot());

        controller
            .send(Command::SetMixerPan {
                mixer: MixerSurface::Mix2,
                channel: 4,
                pan: PanState::from_raw(0x36),
                muted: false,
                soloed: false,
            })
            .expect("mix2 pan");
        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][3]
                .pan
                .raw(),
            0x08
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][3]
                .pan
                .raw(),
            0x36
        );
    }

    #[test]
    fn mixer_mute_does_not_invent_zero_level_for_undecoded_channel() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller
            .send(Command::SetMixerMute {
                mixer: crate::protocol::MixerSurface::Mix1,
                channel: 7,
                muted: true,
                pan_state: crate::protocol::PanState::center(),
                soloed: false,
            })
            .expect("send mute");

        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][6].level,
            None
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][6].muted,
            Some(true)
        );

        controller
            .send(Command::SetMixerMute {
                mixer: crate::protocol::MixerSurface::Mix1,
                channel: 7,
                muted: false,
                pan_state: crate::protocol::PanState::center(),
                soloed: false,
            })
            .expect("send unmute");

        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][6].level,
            None
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][6].muted,
            Some(false)
        );
    }

    #[test]
    fn mixer_state_is_tracked_per_surface() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller
            .send(Command::SetMixerLevel {
                mixer: MixerSurface::Mix1,
                channel: 3,
                level: 0x2c,
                pan_state: crate::protocol::PanState::center(),
                muted: false,
                soloed: false,
            })
            .expect("mix1 send");
        controller.confirm_pending_write(snapshot());

        controller
            .send(Command::SetMixerLevel {
                mixer: MixerSurface::Mix2,
                channel: 3,
                level: 0x10,
                pan_state: crate::protocol::PanState::center(),
                muted: false,
                soloed: false,
            })
            .expect("mix2 send");
        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix1.index()][2].level,
            Some(0x2c)
        );
        assert_eq!(
            controller.state.mixer_channels[MixerSurface::Mix2.index()][2].level,
            Some(0x10)
        );
    }

    #[test]
    fn mixer_first_adjustment_starts_from_safe_midpoint_not_minimum() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.focus = FocusArea::Mixer;
        controller.state.selected_channel = 0;

        let channel = controller.state.active_mixer_channels()[0].channel;
        controller
            .send(Command::SetMixerLevel {
                mixer: MixerSurface::from_surface(controller.state.surface),
                channel,
                level: 0x1f,
                pan_state: crate::protocol::PanState::center(),
                muted: false,
                soloed: false,
            })
            .expect("send first adjustment");

        let writes = transport.take_writes();
        let mixer_write = writes.last().expect("mixer write");
        assert_eq!(
            &mixer_write[0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x1f, 0x20]
        );
    }

    #[test]
    fn connection_status_changes_when_frames_arrive() {
        let mut state = AppState::default();
        state.mark_disconnected();
        assert!(!state.connection.connected);

        state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), vec![0x73, 0, 0, 0]);

        assert!(state.connection.connected);
        assert!(state.connection.last_snapshot_at.is_some());
    }

    #[test]
    fn poll_device_drains_backlog_to_latest_snapshot() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        let mut first = vec![0_u8; 320];
        first[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        first[4..8].copy_from_slice(&0x20_u32.to_le_bytes());
        let first_payload = &mut first[0x10..];
        first_payload[0x02] = SampleRate::Hz44100.code();
        first_payload[0x03] = ClockSource::Internal.code();
        first_payload[0x04..0x08].copy_from_slice(&44_100_u32.to_be_bytes());

        let mut second = vec![0_u8; 320];
        second[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        second[4..8].copy_from_slice(&0x20_u32.to_le_bytes());
        let second_payload = &mut second[0x10..];
        second_payload[0x02] = SampleRate::Hz48000.code();
        second_payload[0x03] = ClockSource::Usb.code();
        second_payload[0x04..0x08].copy_from_slice(&48_000_u32.to_be_bytes());

        transport.push_read(first);
        transport.push_read(second);

        controller.poll_device(Duration::ZERO).expect("poll");

        assert_eq!(
            controller.state.device.sample_rate,
            Some(SampleRate::Hz48000)
        );
        assert_eq!(controller.state.device.clock_source, Some(ClockSource::Usb));
    }

    #[test]
    fn raw_state_tracks_latest_snapshot_and_auxiliary_frames() {
        let mut state = AppState::default();
        let mut raw73 = vec![0_u8; 320];
        raw73[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        raw73[0x10 + 0xcf] = 0x4c;
        state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), raw73.clone());

        let mut raw83 = vec![0_u8; 0x14];
        raw83[0..4].copy_from_slice(&0x83_u32.to_le_bytes());
        raw83[0x10..0x14].copy_from_slice(&[0x60, 0xc0, 0x60, 0x00]);
        state.observe_frame(
            DeviceSnapshot::Auxiliary83(vec![0x60, 0xc0, 0x60, 0x00]),
            raw83.clone(),
        );

        assert!(state.latest_raw_73.is_some());
        assert!(state.latest_raw_83.is_some());
        assert_eq!(
            state.latest_raw_73.as_ref().expect("0x73")[0x10 + 0xcf],
            0x4c
        );
        assert_eq!(
            &state.latest_raw_83.as_ref().expect("0x83")[0..4],
            &raw83[0..4]
        );

        let raw75 = vec![
            0x75, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0x01, 0, 0, 0, 0, 0, 0, 0, b'Z',
        ];
        let raw74 = vec![0x74, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0x11, 0, 0, 0, 0x03];
        state.observe_query_request(raw74.clone());
        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x01,
                sub_id: 0x00,
                body: vec![b'Z'],
            }),
            raw75.clone(),
        );

        let raw81 = vec![0x81, 0x10, 0x20, 0x30, 0x40, 0x50];
        state.observe_frame(
            DeviceSnapshot::Notification(crate::protocol::Notification81 {
                bytes: [0x81, 0x10, 0x20, 0x30, 0x40, 0x50],
            }),
            raw81.clone(),
        );

        assert_eq!(state.latest_raw_75, Some(raw75));
        assert_eq!(state.latest_raw_81, Some(raw81));
        assert_eq!(state.latest_raw_74, Some(raw74));
        assert_eq!(state.recent_query_request_log.len(), 1);
        assert!(state.recent_query_request_log[0].contains("0x74 11/03"));
        assert_eq!(state.recent_query_reply_log.len(), 1);
        assert!(state.recent_query_reply_log[0].contains("0x75 01/00"));
        assert_eq!(
            state.startup_query_summary(0x01),
            Some("Metadata: undecoded")
        );
    }

    #[test]
    fn raw_baseline_captures_latest_packets() {
        let mut state = AppState::default();
        state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), vec![0x73, 0, 0, 0]);
        state.observe_frame(
            DeviceSnapshot::Auxiliary83(vec![0x60, 0xc0, 0x60, 0x00]),
            vec![0x83, 0, 0, 0],
        );
        state.observe_query_request(vec![0x74, 0, 0, 0]);
        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x11,
                sub_id: 0x00,
                body: vec![0xaa, 0xbb],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::Notification(crate::protocol::Notification81 {
                bytes: [1, 2, 3, 4, 5, 6],
            }),
            vec![1, 2, 3, 4, 5, 6],
        );

        state.capture_raw_baseline();
        assert_eq!(state.baseline_raw_73, state.latest_raw_73);
        assert_eq!(state.baseline_raw_83, state.latest_raw_83);
        assert_eq!(state.baseline_raw_74, state.latest_raw_74);
        assert_eq!(state.baseline_raw_75, state.latest_raw_75);
        assert_eq!(state.baseline_raw_81, state.latest_raw_81);

        state.clear_raw_baseline();
        assert!(state.baseline_raw_73.is_none());
        assert!(state.baseline_raw_83.is_none());
        assert!(state.baseline_raw_74.is_none());
        assert!(state.baseline_raw_75.is_none());
        assert!(state.baseline_raw_81.is_none());
    }

    #[test]
    fn stores_grounded_startup_query_summaries_for_all_bootstrap_replies() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x00,
                sub_id: 0x00,
                body: vec![0xaa, 0xbb, 0xcc],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x11,
                sub_id: 0x00,
                body: vec![0x12],
            }),
            vec![0x75, 0, 0, 0],
        );

        assert_eq!(
            state.startup_query_summary(0x00),
            Some("Capability/default block: 3 bytes [aa bb cc]")
        );
        assert_eq!(
            state.startup_query_summary(0x11),
            Some("Status/capability value: 1 bytes [12]")
        );
    }

    #[test]
    fn metadata_reply_updates_serial_and_hardware_version() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x01,
                sub_id: 0x00,
                body: [
                    b"Zen Go Synergy Core".as_slice(),
                    b"\0".as_slice(),
                    b"4502721001300".as_slice(),
                    b"\0".as_slice(),
                    b"6.6".as_slice(),
                    b"\0".as_slice(),
                ]
                .concat(),
            }),
            vec![0x75, 0, 0, 0],
        );

        let metadata = state.device.metadata.expect("metadata");
        assert_eq!(metadata.product_name, "Zen Go Synergy Core");
        assert_eq!(metadata.serial, "4502721001300");
        assert_eq!(metadata.hardware_version, "6.6");
        assert_eq!(
            state.last_message,
            "Connected to Zen Go Synergy Core (hw 6.6, serial 4502721001300)"
        );
    }

    #[test]
    fn query_reply_log_keeps_recent_entries() {
        let mut state = AppState::default();

        for sub_id in 0..20_u8 {
            state.observe_frame(
                DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                    query_id: 0x03,
                    sub_id,
                    body: vec![sub_id, 0xaa],
                }),
                vec![0x75, 0, 0, 0],
            );
        }

        assert_eq!(state.recent_query_reply_log.len(), 16);
        assert!(state
            .recent_query_reply_log
            .first()
            .unwrap()
            .contains("0x75 03/04"));
        assert!(state
            .recent_query_reply_log
            .last()
            .unwrap()
            .contains("0x75 03/13"));
        assert_eq!(state.selected_query_reply_entry, Some(15));
    }

    #[test]
    fn query_reply_log_surfaces_selector_family_summaries() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x0b,
                sub_id: 0x03,
                body: vec![
                    0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x04,
                sub_id: 0x01,
                body: vec![
                    0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00,
                    0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                    0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00,
                    0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                    0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        assert!(state.recent_query_reply_log[0].contains("Selector bitmap"));
        assert!(state.recent_query_reply_log[1].contains("Startup Mix2 pan categories"));
    }

    #[test]
    fn selected_query_reply_entry_tracks_latest_reply_and_cycles() {
        let mut state = AppState::default();
        for sub_id in 0..3_u8 {
            state.observe_frame(
                DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                    query_id: 0x03,
                    sub_id,
                    body: vec![sub_id],
                }),
                vec![0x75, sub_id],
            );
        }

        assert_eq!(state.selected_query_reply_entry, Some(2));
        assert_eq!(
            state
                .selected_query_reply_entry()
                .map(|entry| entry.raw.clone()),
            Some(vec![0x75, 0x02])
        );

        state.cycle_query_reply_entry(false);
        assert_eq!(state.selected_query_reply_entry, Some(1));
        state.cycle_query_reply_entry(true);
        assert_eq!(state.selected_query_reply_entry, Some(2));
    }

    #[test]
    fn query_request_log_keeps_recent_entries() {
        let mut state = AppState::default();

        for sub_id in 0..20_u8 {
            state.observe_query_request(vec![0x74, 0, 0, 0, 0, 0, 0, 0, 0x03, 0, 0, 0, sub_id]);
        }

        assert_eq!(state.recent_query_request_log.len(), 16);
        assert!(state
            .recent_query_request_log
            .first()
            .unwrap()
            .contains("0x74 03/04"));
        assert!(state
            .recent_query_request_log
            .last()
            .unwrap()
            .contains("0x74 03/13"));
    }

    #[test]
    fn focus_cycle_skips_raw_view_state() {
        let mut state = AppState::default();
        state.focus = FocusArea::Status;

        state.cycle_focus();
        assert_eq!(state.focus, FocusArea::Outputs);
        state.cycle_focus();
        assert_eq!(state.focus, FocusArea::Mixer);
        state.cycle_focus();
        assert_eq!(state.focus, FocusArea::Preamp);
        state.cycle_focus();
        assert_eq!(state.focus, FocusArea::Status);
    }

    #[test]
    fn raw_view_toggle_and_packet_tab_cycle_are_independent_of_focus() {
        let mut state = AppState::default();

        state.toggle_raw_view();
        assert!(state.raw_view_open);
        assert_eq!(state.selected_raw_packet, RawPacketTab::State73);

        state.cycle_raw_packet(true);
        assert_eq!(state.selected_raw_packet, RawPacketTab::Auxiliary83);
        state.cycle_raw_packet(false);
        assert_eq!(state.selected_raw_packet, RawPacketTab::State73);

        state.toggle_raw_view();
        assert!(!state.raw_view_open);
    }
}
