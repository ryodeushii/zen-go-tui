use std::time::{Duration, Instant};

use anyhow::Result;

use crate::protocol::{
    encode_command, encode_query, ClockSource, Command, DeviceMetadata, DeviceSnapshot, Frame,
    MixerChannelState, OutputMode, OutputState, OutputTarget, SampleRate, Snapshot73, Surface,
};
use crate::transport::Transport;

#[derive(Debug, Clone)]
pub struct DeviceStatus {
    pub sample_rate: Option<SampleRate>,
    pub clock_source: Option<ClockSource>,
    pub lock_known: bool,
    pub locked: Option<bool>,
    pub metadata: Option<DeviceMetadata>,
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

#[derive(Debug, Clone)]
pub struct AppState {
    pub device: DeviceStatus,
    pub outputs: [OutputState; 3],
    pub surface: Surface,
    pub mixer_channels: Vec<MixerChannelState>,
    pub connection: ConnectionState,
    pub focus: FocusArea,
    pub selected_output: usize,
    pub selected_channel: usize,
    pub last_message: String,
    pub last_auxiliary_len: Option<usize>,
    pub dsp_cluster: [u8; 4],
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
            surface: Surface::MonitorHp1,
            mixer_channels: (1..=15).map(MixerChannelState::unknown).collect(),
            connection: ConnectionState::default(),
            focus: FocusArea::Outputs,
            selected_output: 0,
            selected_channel: 0,
            last_message:
                "Press ? for help. Device state is authoritative where decoding is confirmed."
                    .to_string(),
            last_auxiliary_len: None,
            dsp_cluster: [0; 4],
        }
    }
}

impl AppState {
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
        self.surface = snapshot.surface;
        self.dsp_cluster = snapshot.dsp_cluster;
    }

    pub fn observe_frame(&mut self, frame: DeviceSnapshot) {
        self.connection.connected = true;
        self.connection.last_snapshot_at = Some(Instant::now());
        match frame {
            DeviceSnapshot::Snapshot(snapshot) => {
                self.connection.last_frame_type = Some("0x73 snapshot");
                self.apply_snapshot(snapshot);
            }
            DeviceSnapshot::Auxiliary83(bytes) => {
                self.connection.last_frame_type = Some("0x83 auxiliary");
                self.last_auxiliary_len = Some(bytes.len());
            }
            DeviceSnapshot::QueryReply(reply) => {
                self.connection.last_frame_type = Some("0x75 query reply");
                if let Some(metadata) = reply.metadata() {
                    self.last_message = format!(
                        "Connected to {} ({})",
                        metadata.product_name, metadata.version
                    );
                    self.device.metadata = Some(metadata);
                }
            }
            DeviceSnapshot::Notification(_) => {
                self.connection.last_frame_type = Some("0x81 notification");
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
}

#[derive(Debug, Clone, Copy)]
enum PendingMutation {
    MixerLevel {
        channel: u8,
        level: u8,
    },
    MixerMute {
        channel: u8,
        muted: bool,
    },
    OutputVolume {
        target: OutputTarget,
        step: u8,
    },
    OutputMode {
        target: OutputTarget,
        mode: OutputMode,
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
        for query in [0x01_u8, 0x00, 0x11] {
            self.transport.write(&encode_query(query))?;
        }
        Ok(())
    }

    pub fn send(&mut self, command: Command) -> Result<()> {
        self.pending_mutation = pending_from_command(command);
        self.transport.write(&encode_command(command))?;
        self.state.last_message = format!("Sent {:?}", command);
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
                let snapshot = DeviceSnapshot::from(frame);
                if let DeviceSnapshot::Snapshot(snapshot73) = &snapshot {
                    self.confirm_pending_write(snapshot73.clone());
                }
                self.state.observe_frame(snapshot);
            }
        }

        Ok(())
    }

    pub fn confirm_pending_write(&mut self, _snapshot: Snapshot73) {
        match self.pending_mutation.take() {
            Some(PendingMutation::MixerLevel { channel, level }) => {
                if let Some(slot) = self
                    .state
                    .mixer_channels
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.level = Some(level);
                    slot.muted = Some(false);
                }
            }
            Some(PendingMutation::MixerMute { channel, muted }) => {
                if let Some(slot) = self
                    .state
                    .mixer_channels
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.muted = Some(muted);
                }
            }
            Some(PendingMutation::OutputVolume { target, step }) => {
                self.state.outputs[target.index() as usize].volume = step;
            }
            Some(PendingMutation::OutputMode { target, mode }) => {
                self.state.outputs[target.index() as usize].mode = mode;
            }
            None => {}
        }
    }
}

fn pending_from_command(command: Command) -> Option<PendingMutation> {
    match command {
        Command::SetMixerLevel { channel, level, .. } => {
            Some(PendingMutation::MixerLevel { channel, level })
        }
        Command::SetMixerMute { channel, muted, .. } => {
            Some(PendingMutation::MixerMute { channel, muted })
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
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::{
        ClockSource, Command, DeviceSnapshot, MixerChannelState, OutputMode, OutputState,
        OutputTarget, SampleRate, Snapshot73, Surface,
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
            surface: Surface::MonitorHp1,
            late_shadow: [0; 12],
        }
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
    fn bootstrap_sends_queries_and_mutations_use_transport() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller.bootstrap().expect("bootstrap");
        controller
            .send(Command::SetClockSource(ClockSource::Usb))
            .expect("write command");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 4);
        assert_eq!(&writes[0][0x08..0x10], &[0x01, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[1][0x08..0x10], &[0x00, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[2][0x08..0x10], &[0x11, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[3][0x10..0x12], &[0x04, 0x02]);
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
                pan_state: crate::protocol::PanState::Left,
            })
            .expect("send mixer");

        assert!(controller.state.mixer_channels[2].level.is_none());

        controller.confirm_pending_write(snapshot());

        assert_eq!(
            controller.state.mixer_channels[2],
            MixerChannelState::known(3, Some(0x2c), Some(false))
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
                pan_state: crate::protocol::PanState::Center,
            })
            .expect("send mute");

        controller.confirm_pending_write(snapshot());

        assert_eq!(controller.state.mixer_channels[6].level, None);
        assert_eq!(controller.state.mixer_channels[6].muted, Some(true));

        controller
            .send(Command::SetMixerMute {
                mixer: crate::protocol::MixerSurface::Mix1,
                channel: 7,
                muted: false,
                pan_state: crate::protocol::PanState::Center,
            })
            .expect("send unmute");

        controller.confirm_pending_write(snapshot());

        assert_eq!(controller.state.mixer_channels[6].level, None);
        assert_eq!(controller.state.mixer_channels[6].muted, Some(false));
    }

    #[test]
    fn connection_status_changes_when_frames_arrive() {
        let mut state = AppState::default();
        state.mark_disconnected();
        assert!(!state.connection.connected);

        state.observe_frame(DeviceSnapshot::Snapshot(snapshot()));

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
}
