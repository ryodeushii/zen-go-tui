use std::time::{Duration, Instant};

use anyhow::Result;

use crate::protocol::{
    encode_command, encode_query, ClockSource, Command, DeviceMetadata, DeviceSnapshot, Frame,
    MixerAssignment, MixerChannelState, MixerSurface, OutputMode, OutputState, OutputTarget,
    PanState, PreampMode, PreampState, SampleRate, Snapshot73, Surface,
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
    Raw,
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
    pub last_message: String,
    pub last_auxiliary_len: Option<usize>,
    pub dsp_cluster: [u8; 4],
    pub latest_raw_73: Option<Vec<u8>>,
    pub latest_raw_83: Option<Vec<u8>>,
    pub latest_raw_75: Option<Vec<u8>>,
    pub latest_raw_81: Option<Vec<u8>>,
    pub baseline_raw_73: Option<Vec<u8>>,
    pub baseline_raw_83: Option<Vec<u8>>,
    pub baseline_raw_75: Option<Vec<u8>>,
    pub baseline_raw_81: Option<Vec<u8>>,
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
            last_message:
                "Press ? for help. Device state is authoritative where decoding is confirmed."
                    .to_string(),
            last_auxiliary_len: None,
            dsp_cluster: [0; 4],
            latest_raw_73: None,
            latest_raw_83: None,
            latest_raw_75: None,
            latest_raw_81: None,
            baseline_raw_73: None,
            baseline_raw_83: None,
            baseline_raw_75: None,
            baseline_raw_81: None,
        }
    }
}

impl AppState {
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
        self.surface = snapshot.surface;
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
                self.latest_raw_75 = Some(raw);
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
                self.latest_raw_81 = Some(raw);
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
            FocusArea::Preamp => FocusArea::Raw,
            FocusArea::Raw => FocusArea::Status,
        };
    }

    pub fn capture_raw_baseline(&mut self) {
        self.baseline_raw_73 = self.latest_raw_73.clone();
        self.baseline_raw_83 = self.latest_raw_83.clone();
        self.baseline_raw_75 = self.latest_raw_75.clone();
        self.baseline_raw_81 = self.latest_raw_81.clone();
    }

    pub fn clear_raw_baseline(&mut self) {
        self.baseline_raw_73 = None;
        self.baseline_raw_83 = None;
        self.baseline_raw_75 = None;
        self.baseline_raw_81 = None;
    }
}

#[derive(Debug, Clone, Copy)]
enum PendingMutation {
    MixerLevel {
        mixer: MixerSurface,
        channel: u8,
        level: u8,
        pan: PanState,
    },
    MixerMute {
        mixer: MixerSurface,
        channel: u8,
        muted: bool,
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
            }) => {
                if let Some(slot) = self.state.mixer_channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.level = Some(level);
                    slot.muted = Some(false);
                    slot.pan = pan;
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
            Some(PendingMutation::OutputVolume { target, step }) => {
                self.state.outputs[target.index() as usize].volume = step;
            }
            Some(PendingMutation::OutputMode { target, mode }) => {
                self.state.outputs[target.index() as usize].mode = mode;
            }
            Some(PendingMutation::PreampGain { input, raw }) => {
                self.state.dsp_cluster[input.min(1) as usize] = raw;
                self.state.preamp = PreampState::from_cluster(self.state.dsp_cluster);
            }
            Some(PendingMutation::PreampMode { input, mode }) => {
                let offset = 2 + input.min(1) as usize;
                let preserved_bits = self.state.dsp_cluster[offset] & 0xf0;
                self.state.dsp_cluster[offset] = preserved_bits | mode.code();
                self.state.preamp = PreampState::from_cluster(self.state.dsp_cluster);
            }
            Some(PendingMutation::PreampPhantom { input, enabled }) => {
                let offset = 2 + input.min(1) as usize;
                let low = self.state.dsp_cluster[offset] & 0x0f;
                self.state.dsp_cluster[offset] = low | if enabled { 0x10 } else { 0x00 };
                self.state.preamp = PreampState::from_cluster(self.state.dsp_cluster);
            }
            Some(PendingMutation::PreampPhase { input, enabled }) => {
                let offset = 2 + input.min(1) as usize;
                let low = self.state.dsp_cluster[offset] & 0x1f;
                self.state.dsp_cluster[offset] = low | if enabled { 0x40 } else { 0x00 };
                self.state.preamp = PreampState::from_cluster(self.state.dsp_cluster);
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
        } => Some(PendingMutation::MixerLevel {
            mixer,
            channel,
            level,
            pan: pan_state,
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
        Command::SetMixerPan {
            mixer,
            channel,
            pan,
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
            include_companion: false,
        } => Some(PendingMutation::MixerLink {
            mixer: MixerSurface::Mix2,
            selector,
            enabled,
        }),
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
    match (mixer, selector) {
        (MixerSurface::Mix1, 0x00) => Some((1, 2)),
        (MixerSurface::Mix1, 0x03) => Some((7, 8)),
        (MixerSurface::Mix2, 0x01) => Some((1, 2)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::{
        ClockSource, Command, DeviceSnapshot, MixerAssignment, MixerChannelState, MixerSurface,
        OutputMode, OutputState, OutputTarget, PanState, PreampMode, PreampState, SampleRate,
        Snapshot73, Surface,
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
                pan_state: crate::protocol::PanState::left(),
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

        controller
            .send(Command::SetMixerAssignment {
                strip: 11,
                assignment: MixerAssignment::Oscillator(2),
            })
            .expect("send assignment");
        controller.confirm_pending_write(snapshot());

        controller
            .send(Command::SetLinkState {
                selector: 0x01,
                enabled: true,
                include_companion: false,
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
    fn mixer_pan_updates_are_tracked_per_surface() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller
            .send(Command::SetMixerPan {
                mixer: MixerSurface::Mix1,
                channel: 4,
                pan: PanState::from_raw(0x08),
            })
            .expect("mix1 pan");
        controller.confirm_pending_write(snapshot());

        controller
            .send(Command::SetMixerPan {
                mixer: MixerSurface::Mix2,
                channel: 4,
                pan: PanState::from_raw(0x36),
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
            })
            .expect("mix1 send");
        controller.confirm_pending_write(snapshot());

        controller
            .send(Command::SetMixerLevel {
                mixer: MixerSurface::Mix2,
                channel: 3,
                level: 0x10,
                pan_state: crate::protocol::PanState::center(),
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
        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x01,
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
    }

    #[test]
    fn raw_baseline_captures_latest_packets() {
        let mut state = AppState::default();
        state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), vec![0x73, 0, 0, 0]);
        state.observe_frame(
            DeviceSnapshot::Auxiliary83(vec![0x60, 0xc0, 0x60, 0x00]),
            vec![0x83, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(crate::protocol::QueryReply75 {
                query_id: 0x11,
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
        assert_eq!(state.baseline_raw_75, state.latest_raw_75);
        assert_eq!(state.baseline_raw_81, state.latest_raw_81);

        state.clear_raw_baseline();
        assert!(state.baseline_raw_73.is_none());
        assert!(state.baseline_raw_83.is_none());
        assert!(state.baseline_raw_75.is_none());
        assert!(state.baseline_raw_81.is_none());
    }
}
