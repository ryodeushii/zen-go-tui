use std::sync::{Arc, Mutex};

use anyhow::Result;

use antelope_protocol::{
    control_panel_startup_queries, encode_command, encode_link_companion,
    encode_mixer_assignment_frames_with_table, encode_query, Command as DeviceCommand,
    DeviceSnapshot, EncodeResult, Frame, MixerAssignment, MixerSurface, OutputTarget,
};
use zen_go_tui::app::AppState;
use zen_go_tui::app::Intent;
use zen_go_tui::command_queue::CommandQueue;
use zen_go_tui::profile::DeviceProfile;
use zen_go_tui::transport::Transport;

#[derive(Debug, Clone, Default)]
pub struct GuiPopupState {
    pub profiles_open: bool,
    pub routing_open: bool,
    pub options_open: bool,
    pub hotkeys_open: bool,
    pub profile_names: Vec<String>,
    pub selected_profile_index: usize,
}

pub struct GuiApp {
    pub state: AppState,
    pub popup: GuiPopupState,
    pub error: Option<String>,
    command_queue: CommandQueue,
}

impl GuiApp {
    pub fn new() -> Self {
        Self {
            state: AppState::default(),
            popup: GuiPopupState::default(),
            error: None,
            command_queue: CommandQueue::new(),
        }
    }

    pub fn error_message(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_error(&mut self, message: &str) {
        self.error = Some(message.to_string());
    }

    pub fn set_disconnected(&mut self) {
        self.state.mark_disconnected();
        self.error = Some("Device disconnected. Reconnecting...".to_string());
    }

    pub fn popup_open(&self) -> bool {
        self.popup.profiles_open
            || self.popup.routing_open
            || self.popup.options_open
            || self.popup.hotkeys_open
    }

    pub fn process_frame(&mut self, raw: [u8; 320]) {
        self.error = None;
        if let Ok(frame) = Frame::parse_owned(raw.to_vec()) {
            let (snapshot, raw_array) = frame.into_snapshot_and_raw();
            self.state.observe_frame(snapshot, raw_array);
        } else {
            self.error = Some("Failed to decode frame".to_string());
        }
    }

    pub fn handle_intent(
        &mut self,
        intent: &Intent,
        transport: Arc<Mutex<Box<dyn Transport>>>,
    ) -> Result<()> {
        match intent {
            Intent::ToggleRawView => {
                self.state.toggle_raw_view();
            }
            Intent::ToggleHotkeysPopup => {
                self.popup.hotkeys_open = !self.popup.hotkeys_open;
            }
            Intent::OpenProfilesPopup => {
                self.popup.profiles_open = true;
                self.popup.profile_names =
                    zen_go_tui::profile::list_profile_names().unwrap_or_default();
                self.popup.selected_profile_index = 0;
            }
            Intent::CloseProfilesPopup => {
                self.popup.profiles_open = false;
            }
            Intent::OpenRoutingPopup => {
                self.popup.routing_open = true;
            }
            Intent::CloseRoutingPopup => {
                self.popup.routing_open = false;
            }
            Intent::OpenOptionsPopup => {
                self.popup.options_open = true;
            }
            Intent::CloseOptionsPopup => {
                self.popup.options_open = false;
            }
            Intent::SelectProfile(index) => {
                self.popup.selected_profile_index = *index;
            }
            Intent::LoadSelectedProfile => {
                if let Some(name) = self
                    .popup
                    .profile_names
                    .get(self.popup.selected_profile_index)
                {
                    let profile = DeviceProfile::read_named(name)?;
                    self.apply_profile(&profile, transport.clone())?;
                    self.popup.profiles_open = false;
                }
            }
            Intent::SelectSurface(surface) => {
                self.send_command(DeviceCommand::SelectSurface(*surface), transport.clone())?;
                self.refresh_queried_state(transport.clone())?;
            }
            Intent::SetOutputLevel { index, step } => {
                let target = match index {
                    0 => OutputTarget::Monitor,
                    1 => OutputTarget::Hp1,
                    2 => OutputTarget::Hp2,
                    _ => return Ok(()),
                };
                self.state.output.states[target.index() as usize].volume = *step;
                self.send_command(
                    DeviceCommand::SetOutputVolume {
                        target,
                        step: *step,
                    },
                    transport.clone(),
                )?;
            }
            Intent::ToggleOutputMute(index) => {
                let target = match index {
                    0 => OutputTarget::Monitor,
                    1 => OutputTarget::Hp1,
                    2 => OutputTarget::Hp2,
                    _ => return Ok(()),
                };
                let current = self.state.output.states[target.index() as usize].mode;
                let enabled = current != antelope_protocol::OutputMode::Mute;
                self.state.output.states[target.index() as usize].mode = if enabled {
                    antelope_protocol::OutputMode::Mute
                } else {
                    antelope_protocol::OutputMode::Normal
                };
                self.send_command(
                    DeviceCommand::SetOutputMute { target, enabled },
                    transport.clone(),
                )?;
            }
            Intent::ToggleOutputDim(index) => {
                let target = match index {
                    0 => OutputTarget::Monitor,
                    1 => OutputTarget::Hp1,
                    2 => OutputTarget::Hp2,
                    _ => return Ok(()),
                };
                let current = self.state.output.states[target.index() as usize].mode;
                let enabled = current != antelope_protocol::OutputMode::Dim;
                self.state.output.states[target.index() as usize].mode = if enabled {
                    antelope_protocol::OutputMode::Dim
                } else {
                    antelope_protocol::OutputMode::Normal
                };
                self.send_command(
                    DeviceCommand::SetOutputDim { target, enabled },
                    transport.clone(),
                )?;
            }
            Intent::SetMixerLevel { index, level } => {
                let mixer = self.state.active_mixer_surface();
                let channel = (*index + 1) as u8;
                let channels = &self.state.mixer.channels[mixer.index()];
                if let Some(ch) = channels.get(*index) {
                    let pan = ch.pan;
                    let muted = ch.muted.unwrap_or(false);
                    let soloed = ch.soloed.unwrap_or(false);
                    self.send_command(
                        DeviceCommand::SetMixerLevel {
                            mixer,
                            channel,
                            level: *level,
                            pan_state: pan,
                            muted,
                            soloed,
                        },
                        transport.clone(),
                    )?;
                }
            }
            Intent::ToggleMixerMute(channel) => {
                let mixer = self.state.active_mixer_surface();
                let idx = channel.saturating_sub(1) as usize;
                if let Some(ch) = self.state.mixer.channels[mixer.index()].get(idx) {
                    let muted = !ch.muted.unwrap_or(false);
                    let pan = ch.pan;
                    let soloed = ch.soloed.unwrap_or(false);
                    self.send_command(
                        DeviceCommand::SetMixerMute {
                            mixer,
                            channel: *channel,
                            muted,
                            pan_state: pan,
                            soloed,
                        },
                        transport.clone(),
                    )?;
                }
            }
            Intent::ToggleMixerSolo(channel) => {
                let mixer = self.state.active_mixer_surface();
                let idx = channel.saturating_sub(1) as usize;
                if let Some(ch) = self.state.mixer.channels[mixer.index()].get(idx) {
                    let soloed = !ch.soloed.unwrap_or(false);
                    let muted = ch.muted.unwrap_or(false);
                    let pan = ch.pan;
                    self.send_command(
                        DeviceCommand::SetMixerSolo {
                            mixer,
                            channel: *channel,
                            soloed,
                            muted,
                            pan_state: pan,
                        },
                        transport.clone(),
                    )?;
                }
            }
            Intent::ToggleMixerLink(channel) => {
                let mixer = self.state.active_mixer_surface();
                let idx = channel.saturating_sub(1) as usize;
                if let Some(ch) = self.state.mixer.channels[mixer.index()].get(idx) {
                    let enabled = !ch.linked.unwrap_or(false);
                    if let Some(target) =
                        antelope_protocol::MixerLinkTarget::from_channel(mixer, *channel)
                    {
                        let t = transport.clone();
                        let mut guard = t
                            .lock()
                            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
                        if let Some(bank) = target.companion_bank() {
                            guard.write(&encode_link_companion(bank, enabled))?;
                        }
                        guard.write(
                            &encode_command(DeviceCommand::SetLinkState {
                                selector: target.selector,
                                enabled,
                                companion_bank: None,
                            })
                            .unwrap_single(),
                        )?;
                    }
                }
            }
            Intent::SetMixerPan { index, pan } => {
                let mixer = self.state.active_mixer_surface();
                let channel = (*index + 1) as u8;
                let channels = &self.state.mixer.channels[mixer.index()];
                if let Some(ch) = channels.get(*index) {
                    let muted = ch.muted.unwrap_or(false);
                    let soloed = ch.soloed.unwrap_or(false);
                    self.send_command(
                        DeviceCommand::SetMixerPan {
                            mixer,
                            channel,
                            pan: *pan,
                            muted,
                            soloed,
                        },
                        transport.clone(),
                    )?;
                }
            }
            Intent::SetPreampGain { input, raw } => {
                self.state.device.dsp_cluster[*input as usize] = *raw;
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                self.send_command(
                    DeviceCommand::SetPreampGain {
                        input: *input,
                        raw: *raw,
                    },
                    transport.clone(),
                )?;
            }
            Intent::PickPreampMode { input, mode } => {
                self.send_command(
                    DeviceCommand::SetPreampMode {
                        input: *input,
                        mode: *mode,
                    },
                    transport.clone(),
                )?;
            }
            Intent::TogglePreampPhantom(input) => {
                let enabled = if *input == 0 {
                    !self.state.preamp.state.input1.phantom_on
                } else {
                    !self.state.preamp.state.input2.phantom_on
                };
                self.send_command(
                    DeviceCommand::SetPreampPhantom {
                        input: *input,
                        enabled,
                    },
                    transport.clone(),
                )?;
            }
            Intent::TogglePreampPhase(input) => {
                let mode_raw = if *input == 0 {
                    self.state.preamp.state.input1.mode_raw
                } else {
                    self.state.preamp.state.input2.mode_raw
                };
                let enabled = mode_raw & 0x40 == 0;
                self.send_command(
                    DeviceCommand::SetPreampPhase {
                        input: *input,
                        enabled,
                    },
                    transport.clone(),
                )?;
            }
            Intent::PickSampleRate(rate) => {
                self.send_command(DeviceCommand::SetSampleRate(*rate), transport.clone())?;
            }
            Intent::PickClockSource(source) => {
                self.send_command(DeviceCommand::SetClockSource(*source), transport.clone())?;
            }
            Intent::PickAssignment { strip, assignment } => {
                let assignments = self.shared_assignment_table()?;
                let frames =
                    encode_mixer_assignment_frames_with_table(*strip, *assignment, &assignments);
                let t = transport.clone();
                let mut guard = t
                    .lock()
                    .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
                for frame in frames {
                    guard.write(&frame)?;
                }
            }
            Intent::CycleFocus => {
                self.state.cycle_focus();
            }
            _ => {}
        }
        Ok(())
    }

    pub fn refresh_state(&mut self, transport: Arc<Mutex<Box<dyn Transport>>>) -> Result<()> {
        if self.state.device.connection.connected {
            return Ok(());
        }
        self.refresh_queried_state(transport)
    }

    fn send_command(
        &mut self,
        command: DeviceCommand,
        transport: Arc<Mutex<Box<dyn Transport>>>,
    ) -> Result<()> {
        let result = encode_command(command);
        match result {
            EncodeResult::Single(frame) => {
                let mut guard = transport
                    .lock()
                    .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
                guard.write(&frame)?;
            }
            EncodeResult::Multi(frames) => {
                let mut guard = transport
                    .lock()
                    .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
                for frame in frames {
                    guard.write(&frame)?;
                }
            }
            EncodeResult::WithCompanion { companion, main } => {
                let mut guard = transport
                    .lock()
                    .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
                guard.write(&companion)?;
                guard.write(&main)?;
            }
            EncodeResult::WithRefresh(frame) => {
                let mut guard = transport
                    .lock()
                    .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
                guard.write(&frame)?;
            }
            EncodeResult::MixerAssignment { strip, assignment } => {
                let assignments = self.shared_assignment_table()?;
                let frames =
                    encode_mixer_assignment_frames_with_table(strip, assignment, &assignments);
                let mut guard = transport
                    .lock()
                    .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
                for frame in frames {
                    guard.write(&frame)?;
                }
            }
        }
        Ok(())
    }

    fn refresh_queried_state(&mut self, transport: Arc<Mutex<Box<dyn Transport>>>) -> Result<()> {
        let mut guard = transport
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        for query in control_panel_startup_queries() {
            let frame = encode_query(*query);
            guard.write(&frame)?;
        }
        Ok(())
    }

    fn apply_profile(
        &mut self,
        profile: &DeviceProfile,
        transport: Arc<Mutex<Box<dyn Transport>>>,
    ) -> Result<()> {
        profile.validate()?;
        for (target, output) in [
            (OutputTarget::Monitor, &profile.outputs.monitor),
            (OutputTarget::Hp1, &profile.outputs.hp1),
            (OutputTarget::Hp2, &profile.outputs.hp2),
        ] {
            self.send_command(
                DeviceCommand::SetOutputVolume {
                    target,
                    step: output.volume_step,
                },
                transport.clone(),
            )?;
            self.send_command(
                DeviceCommand::SetOutputDim {
                    target,
                    enabled: false,
                },
                transport.clone(),
            )?;
            self.send_command(
                DeviceCommand::SetOutputMute {
                    target,
                    enabled: false,
                },
                transport.clone(),
            )?;
            match output.mode.into_device() {
                antelope_protocol::OutputMode::Mute => {
                    self.send_command(
                        DeviceCommand::SetOutputMute {
                            target,
                            enabled: true,
                        },
                        transport.clone(),
                    )?;
                }
                antelope_protocol::OutputMode::Dim => {
                    self.send_command(
                        DeviceCommand::SetOutputDim {
                            target,
                            enabled: true,
                        },
                        transport.clone(),
                    )?;
                }
                _ => {}
            }
        }
        for (input, preamp) in [
            (0_u8, &profile.preamps.input1),
            (1_u8, &profile.preamps.input2),
        ] {
            self.send_command(
                DeviceCommand::SetPreampMode {
                    input,
                    mode: preamp.mode.into_device(),
                },
                transport.clone(),
            )?;
            self.send_command(
                DeviceCommand::SetPreampGain {
                    input,
                    raw: preamp.gain_raw,
                },
                transport.clone(),
            )?;
            self.send_command(
                DeviceCommand::SetPreampPhantom {
                    input,
                    enabled: preamp.phantom_on,
                },
                transport.clone(),
            )?;
            self.send_command(
                DeviceCommand::SetPreampPhase {
                    input,
                    enabled: preamp.phase_inverted,
                },
                transport.clone(),
            )?;
        }
        profile.apply_to_state(&mut self.state);
        Ok(())
    }

    fn shared_assignment_table(&self) -> Result<[MixerAssignment; 16]> {
        let mut assignments = [MixerAssignment::Mute; 16];
        for (index, slot) in assignments.iter_mut().enumerate() {
            *slot = self.state.mixer.channels[0][index]
                .assignment
                .or(self.state.mixer.channels[1][index].assignment)
                .ok_or_else(|| {
                    anyhow::anyhow!("assignment table is incomplete for CH {:02}", index + 1)
                })?;
        }
        Ok(assignments)
    }
}
