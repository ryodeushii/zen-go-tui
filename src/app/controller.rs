use std::time::Duration;

use anyhow::{bail, Result};
use ratatui::layout::Rect;

use crate::command_queue::CommandQueue;
use crate::profile::{preamp_mode_raw, DeviceProfile};
use crate::transport::Transport;
use antelope_protocol::{
    control_panel_startup_queries, encode_command, encode_link_companion,
    encode_mixer_assignment_frames_with_table, encode_query, ClockSource, Command, DeviceSnapshot,
    EncodeResult, Frame, MixerAssignment, MixerChannelState, MixerLinkTarget, MixerSurface,
    OutputMode, OutputTarget, PanState, PreampMode, SampleRate, Surface,
};

use super::picker::{AssignmentPickerState, SelectorPopupKind, SelectorPopupState};
use super::profile_editor::{ProfileEditorMode, ProfileEditorState};
use super::types::{FocusArea, Intent, PeakHoldDuration, PendingMutation, RawPacketTab, RefreshRate};
use super::AppState;

pub(crate) const MAX_FRAMES_PER_POLL: usize = 32;

pub struct Controller {
    transport: Box<dyn Transport>,
    pub state: AppState,
    pub(crate) pending_mutation: Option<PendingMutation>,
    command_queue: CommandQueue,
}

impl Controller {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            state: AppState::default(),
            pending_mutation: None,
            command_queue: CommandQueue::new(),
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
            self.state.observe_query_request(frame);
            self.transport.write(&frame)?;
        }
        Ok(())
    }

    pub fn apply_profile(&mut self, profile: &DeviceProfile) -> Result<()> {
        profile.validate()?;

        // Flush any pending commands before applying profile
        self.flush_commands()?;

        for (target, output) in [
            (OutputTarget::Monitor, &profile.outputs.monitor),
            (OutputTarget::Hp1, &profile.outputs.hp1),
            (OutputTarget::Hp2, &profile.outputs.hp2),
        ] {
            self.send(
                Command::SetOutputVolume {
                    target,
                    step: output.volume_step,
                },
                None,
            )?;
            self.send(
                Command::SetOutputDim {
                    target,
                    enabled: false,
                },
                None,
            )?;
            self.send(
                Command::SetOutputMute {
                    target,
                    enabled: false,
                },
                None,
            )?;
            match output.mode.into_device() {
                OutputMode::Normal => {}
                OutputMode::Mute => self.send(
                    Command::SetOutputMute {
                        target,
                        enabled: true,
                    },
                    None,
                )?,
                OutputMode::Dim => self.send(
                    Command::SetOutputDim {
                        target,
                        enabled: true,
                    },
                    None,
                )?,
                OutputMode::Unknown(_) => unreachable!(),
            }
        }
        self.flush_commands()?;

        for (input, preamp) in [
            (0_u8, &profile.preamps.input1),
            (1_u8, &profile.preamps.input2),
        ] {
            self.send(
                Command::SetPreampMode {
                    input,
                    mode: preamp.mode.into_device(),
                },
                None,
            )?;
            self.send(
                Command::SetPreampGain {
                    input,
                    raw: preamp.gain_raw,
                },
                None,
            )?;
            self.send(
                Command::SetPreampPhantom {
                    input,
                    enabled: preamp.phantom_on,
                },
                None,
            )?;
            self.send(
                Command::SetPreampPhase {
                    input,
                    enabled: preamp.phase_inverted,
                },
                None,
            )?;
        }
        self.flush_commands()?;

        let assignments = profile.assignment_table()?;
        for entry in &profile.assignments {
            for frame in encode_mixer_assignment_frames_with_table(
                entry.channel,
                entry.source.into_device(),
                &assignments,
            ) {
                self.transport.write(&frame)?;
            }
        }

        for (mixer, strips) in [
            (MixerSurface::Mix1, &profile.mixers.mix1),
            (MixerSurface::Mix2, &profile.mixers.mix2),
        ] {
            for strip in strips.iter().step_by(2) {
                self.send_mixer_link_change(mixer, strip.channel, strip.linked)?;
            }
            for strip in strips {
                self.send(
                    Command::SetMixerLevel {
                        mixer,
                        channel: strip.channel,
                        level: strip.level_raw,
                        pan_state: PanState::from_raw(strip.pan_raw),
                        muted: strip.muted,
                        soloed: strip.soloed,
                    },
                    None,
                )?;
            }
        }
        self.flush_commands()?;

        profile.apply_to_state(&mut self.state);
        self.pending_mutation = None;
        self.state.preamp.state.cluster = [
            self.state.preamp.state.input1.gain_raw,
            self.state.preamp.state.input2.gain_raw,
            preamp_mode_raw(
                profile.preamps.input1.mode,
                profile.preamps.input1.phantom_on,
                profile.preamps.input1.phase_inverted,
            ),
            preamp_mode_raw(
                profile.preamps.input2.mode,
                profile.preamps.input2.phantom_on,
                profile.preamps.input2.phase_inverted,
            ),
        ];
        self.state.ui.last_message = "Applied profile".to_string();
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

    pub fn send(&mut self, command: Command, pending: Option<PendingMutation>) -> Result<()> {
        let result = encode_command(command);
        match result {
            EncodeResult::Single(_) => {
                self.command_queue.enqueue(command);
            }
            EncodeResult::Multi(frames) => {
                self.flush_commands()?;
                for frame in &*frames {
                    self.transport.write(frame)?;
                }
            }
            EncodeResult::WithCompanion { companion, main } => {
                self.flush_commands()?;
                self.transport.write(companion.as_ref())?;
                self.transport.write(main.as_ref())?;
            }
            EncodeResult::WithRefresh(frame) => {
                self.flush_commands()?;
                self.transport.write(&frame)?;
                self.apply_command_state_update(&command);
                self.pending_mutation = pending;
                self.state.ui.last_message = format!("Sent {:?}", command);
                self.refresh_queried_state()?;
                return Ok(());
            }
            EncodeResult::MixerAssignment { strip, assignment } => {
                let assignments = self.shared_assignment_table()?;
                let frames =
                    encode_mixer_assignment_frames_with_table(strip, assignment, &assignments);
                self.flush_commands()?;
                for frame in frames {
                    self.transport.write(&frame)?;
                }
            }
        }
        self.apply_command_state_update(&command);
        self.pending_mutation = pending;
        self.state.ui.last_message = format!("Sent {:?}", command);
        Ok(())
    }

    /// Applies immediate state updates for commands that affect visible state.
    fn apply_command_state_update(&mut self, command: &Command) {
        match command {
            Command::SetClockSource(source) => {
                self.state.device.status.clock_source = Some(*source);
            }
            Command::SetSampleRate(rate) => {
                self.state.device.status.sample_rate = Some(*rate);
                self.state.device.status.sample_rate_hz = rate.hz();
            }
            _ => {}
        }
    }

    /// Flushes all pending commands from the queue to the transport.
    pub fn flush_commands(&mut self) -> Result<()> {
        self.command_queue.flush(self.transport.as_ref())?;
        Ok(())
    }

    fn resolve_linked_pair(
        &self,
        mixer: MixerSurface,
        channel: u8,
    ) -> Result<(u8, u8, MixerChannelState, MixerChannelState)> {
        let (left_channel, right_channel) = if channel % 2 == 1 {
            (channel, channel.saturating_add(1))
        } else {
            (channel.saturating_sub(1), channel)
        };
        let left_index = left_channel.saturating_sub(1) as usize;
        let right_index = right_channel.saturating_sub(1) as usize;
        let Some(left) = self.state.mixer.channels[mixer.index()]
            .get(left_index)
            .copied()
        else {
            bail!("invalid linked left channel {left_channel}");
        };
        let Some(right) = self.state.mixer.channels[mixer.index()]
            .get(right_index)
            .copied()
        else {
            bail!("invalid linked right channel {right_channel}");
        };
        Ok((left_channel, right_channel, left, right))
    }

    pub fn send_mixer_level_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        level: u8,
    ) -> Result<()> {
        let index = channel.saturating_sub(1) as usize;
        let Some(active) = self.state.mixer.channels[mixer.index()].get(index).copied() else {
            bail!("invalid mixer channel {channel}");
        };

        if active.linked == Some(true) {
            let (left_ch, right_ch, left, right) = self.resolve_linked_pair(mixer, channel)?;

            if let Some(slot) =
                self.state.mixer.channels[mixer.index()].get_mut(left_ch.saturating_sub(1) as usize)
            {
                slot.level = Some(level);
                slot.pan = left.pan;
                slot.muted = left.muted;
            }
            if let Some(slot) = self.state.mixer.channels[mixer.index()]
                .get_mut(right_ch.saturating_sub(1) as usize)
            {
                slot.level = Some(level);
                slot.pan = right.pan;
                slot.muted = right.muted;
            }

            let pending_mutation = Some(PendingMutation::MixerLinkedLevel {
                mixer,
                left_channel: left_ch,
                right_channel: right_ch,
                level,
                left_pan: left.pan,
                right_pan: right.pan,
                left_muted: left.muted.unwrap_or(false),
                right_muted: right.muted.unwrap_or(false),
            });
            self.flush_commands()?;
            self.transport.write(
                &encode_command(Command::SetMixerLevel {
                    mixer,
                    channel: left_ch,
                    level,
                    pan_state: left.pan,
                    muted: left.muted.unwrap_or(false),
                    soloed: left.soloed.unwrap_or(false),
                })
                .unwrap_single(),
            )?;
            self.transport.write(
                &encode_command(Command::SetMixerLevel {
                    mixer,
                    channel: right_ch,
                    level,
                    pan_state: right.pan,
                    muted: right.muted.unwrap_or(false),
                    soloed: right.soloed.unwrap_or(false),
                })
                .unwrap_single(),
            )?;
            self.pending_mutation = pending_mutation;
            self.state.ui.last_message = format!(
                "Sent linked mixer level {:?} ch {}-{}",
                mixer, left_ch, right_ch
            );
            return Ok(());
        }

        if let Some(slot) = self.state.mixer.channels[mixer.index()].get_mut(index) {
            slot.level = Some(level);
            slot.pan = active.pan;
            slot.muted = active.muted;
        }

        self.send(
            Command::SetMixerLevel {
                mixer,
                channel,
                level,
                pan_state: active.pan,
                muted: active.muted.unwrap_or(false),
                soloed: active.soloed.unwrap_or(false),
            },
            Some(PendingMutation::MixerLevel {
                mixer,
                channel,
                level,
                pan: active.pan,
                muted: active.muted.unwrap_or(false),
            }),
        )
    }

    pub fn send_mixer_mute_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        muted: bool,
    ) -> Result<()> {
        let index = channel.saturating_sub(1) as usize;
        let Some(active) = self.state.mixer.channels[mixer.index()].get(index).copied() else {
            bail!("invalid mixer channel {channel}");
        };

        if active.linked == Some(true) {
            let (left_ch, right_ch, left, right) = self.resolve_linked_pair(mixer, channel)?;

            if let Some(slot) =
                self.state.mixer.channels[mixer.index()].get_mut(left_ch.saturating_sub(1) as usize)
            {
                slot.muted = Some(muted);
            }
            if let Some(slot) = self.state.mixer.channels[mixer.index()]
                .get_mut(right_ch.saturating_sub(1) as usize)
            {
                slot.muted = Some(muted);
            }

            let pending_mutation = Some(PendingMutation::MixerLinkedMute {
                mixer,
                left_channel: left_ch,
                right_channel: right_ch,
                muted,
            });
            self.flush_commands()?;
            self.transport.write(
                &encode_command(Command::SetMixerMute {
                    mixer,
                    channel: left_ch,
                    muted,
                    pan_state: left.pan,
                    soloed: left.soloed.unwrap_or(false),
                })
                .unwrap_single(),
            )?;
            self.transport.write(
                &encode_command(Command::SetMixerMute {
                    mixer,
                    channel: right_ch,
                    muted,
                    pan_state: right.pan,
                    soloed: right.soloed.unwrap_or(false),
                })
                .unwrap_single(),
            )?;
            self.pending_mutation = pending_mutation;
            self.state.ui.last_message = format!(
                "Sent linked mixer mute {:?} ch {}-{}",
                mixer, left_ch, right_ch
            );
            return Ok(());
        }

        if let Some(slot) = self.state.mixer.channels[mixer.index()].get_mut(index) {
            slot.muted = Some(muted);
        }

        self.send(
            Command::SetMixerMute {
                mixer,
                channel,
                muted,
                pan_state: active.pan,
                soloed: active.soloed.unwrap_or(false),
            },
            Some(PendingMutation::MixerMute {
                mixer,
                channel,
                muted,
            }),
        )
    }

    pub fn send_mixer_solo_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        soloed: bool,
    ) -> Result<()> {
        let index = channel.saturating_sub(1) as usize;
        let Some(active) = self.state.mixer.channels[mixer.index()].get(index).copied() else {
            bail!("invalid mixer channel {channel}");
        };

        if active.linked == Some(true) {
            let (left_ch, right_ch, left, right) = self.resolve_linked_pair(mixer, channel)?;

            if let Some(slot) =
                self.state.mixer.channels[mixer.index()].get_mut(left_ch.saturating_sub(1) as usize)
            {
                slot.soloed = Some(soloed);
            }
            if let Some(slot) = self.state.mixer.channels[mixer.index()]
                .get_mut(right_ch.saturating_sub(1) as usize)
            {
                slot.soloed = Some(soloed);
            }

            let pending_mutation = Some(PendingMutation::MixerLinkedSolo {
                mixer,
                left_channel: left_ch,
                right_channel: right_ch,
                soloed,
            });
            self.flush_commands()?;
            self.transport.write(
                &encode_command(Command::SetMixerSolo {
                    mixer,
                    channel: left_ch,
                    soloed,
                    muted: left.muted.unwrap_or(false),
                    pan_state: left.pan,
                })
                .unwrap_single(),
            )?;
            self.transport.write(
                &encode_command(Command::SetMixerSolo {
                    mixer,
                    channel: right_ch,
                    soloed,
                    muted: right.muted.unwrap_or(false),
                    pan_state: right.pan,
                })
                .unwrap_single(),
            )?;
            self.pending_mutation = pending_mutation;
            self.state.ui.last_message = format!(
                "Sent linked mixer solo {:?} ch {}-{}",
                mixer, left_ch, right_ch
            );
            return Ok(());
        }

        if let Some(slot) = self.state.mixer.channels[mixer.index()].get_mut(index) {
            slot.soloed = Some(soloed);
        }

        self.send(
            Command::SetMixerSolo {
                mixer,
                channel,
                soloed,
                muted: active.muted.unwrap_or(false),
                pan_state: active.pan,
            },
            Some(PendingMutation::MixerSolo {
                mixer,
                channel,
                soloed,
            }),
        )
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
        self.flush_commands()?;
        if let Some(bank) = target.companion_bank() {
            self.transport
                .write(&encode_link_companion(bank, enabled))?;
        }
        self.transport.write(
            &encode_command(Command::SetLinkState {
                selector: target.selector,
                enabled,
                companion_bank: None,
            })
            .unwrap_single(),
        )?;
        for channel in [target.left_channel, target.right_channel] {
            if let Some(slot) = self.state.mixer.channels[mixer.index()]
                .get_mut(usize::from(channel - 1))
            {
                slot.linked = Some(enabled);
            }
        }
        self.pending_mutation = pending_mutation;
        self.state.ui.last_message = format!(
            "Sent mixer link {:?} ch {}-{}",
            mixer, target.left_channel, target.right_channel
        );
        Ok(())
    }

    pub fn apply_intent(&mut self, intent: Intent, area: Rect) -> Result<()> {
        let pending = intent.pending_mutation(&self.state);
        match intent {
            Intent::Quit => {
                self.state.ui.quit_requested = true;
            }
            Intent::ToggleRawView => self.state.toggle_raw_view(),
            Intent::ToggleHotkeysPopup => self.state.toggle_hotkeys_popup(),
            Intent::OpenProfilesPopup => self.handle_open_profiles_popup(),
            Intent::CloseProfilesPopup => self.handle_close_profiles_popup(),
            Intent::OpenRoutingPopup => self.handle_open_routing_popup(),
            Intent::CloseRoutingPopup => self.handle_close_routing_popup(),
            Intent::OpenOptionsPopup => self.handle_open_options_popup(),
            Intent::CloseOptionsPopup => self.handle_close_options_popup(),
            Intent::SetRefreshRate(rate) => self.handle_set_refresh_rate(rate),
            Intent::CyclePeakThreshold(increase) => self.handle_cycle_peak_threshold(increase),
            Intent::TogglePeakEnabled => self.handle_toggle_peak_enabled(),
            Intent::CyclePeakHoldDuration(duration) => self.handle_cycle_peak_hold_duration(duration),
            Intent::ToggleAutoSave => self.handle_toggle_auto_save(),
            Intent::SelectProfile(index) => self.handle_select_profile(index),
            Intent::LoadSelectedProfile => self.handle_load_selected_profile(),
            Intent::StartSaveProfile => self.handle_start_save_profile(),
            Intent::StartRenameProfile => self.handle_start_rename_profile(),
            Intent::DeleteSelectedProfile => self.handle_delete_selected_profile(),
            Intent::PageMixerStripsLeft => self.handle_page_mixer_strips(area, false),
            Intent::PageMixerStripsRight => self.handle_page_mixer_strips(area, true),
            Intent::OpenSampleRateSelector => self.handle_open_sample_rate_selector(),
            Intent::OpenClockSourceSelector => self.handle_open_clock_source_selector(),
            Intent::SelectRawPacketTab(tab) => self.handle_select_raw_packet_tab(tab),
            Intent::SelectOutput(index) => self.handle_output_select(index),
            Intent::AdjustOutputLevel { index, increase } => {
                self.handle_output_adjust(index, increase, pending)?
            }
            Intent::SetOutputLevel { index, step } => {
                self.handle_output_set_level(index, step, pending)?
            }
            Intent::ToggleOutputDim(index) => self.handle_output_toggle_dim(index, pending)?,
            Intent::ToggleOutputMute(index) => self.handle_output_toggle_mute(index, pending)?,
            Intent::SelectQueryReplyEntry(index) => self.handle_select_query_reply_entry(index),
            Intent::ScrollQueryReplyList { increase } => {
                self.handle_scroll_query_reply_list(increase)
            }
            Intent::SelectSurface(surface) => self.handle_select_surface(surface, pending)?,
            Intent::SelectMixerChannel(index) => self.handle_select_mixer_channel(index),
            Intent::AdjustMixerLevel { index, increase } => {
                self.handle_adjust_mixer_level(index, increase, pending)?
            }
            Intent::SetMixerLevel { index, level } => {
                self.handle_set_mixer_level(index, level, pending)?
            }
            Intent::AdjustMixerPan { index, right } => {
                self.handle_adjust_mixer_pan(index, right, pending)?
            }
            Intent::SetMixerPan { index, pan } => self.handle_set_mixer_pan(index, pan, pending)?,
            Intent::ToggleMixerMute(channel) => {
                self.handle_toggle_mixer_mute(channel, pending)?
            }
            Intent::ToggleMixerSolo(channel) => {
                self.handle_toggle_mixer_solo(channel, pending)?
            }
            Intent::ToggleMixerLink(channel) => {
                self.handle_toggle_mixer_link(channel, pending)?
            }
            Intent::OpenAssignmentPicker(strip) => self.handle_open_assignment_picker(strip),
            Intent::PickAssignment { strip, assignment } => {
                self.handle_pick_assignment(strip, assignment, pending)?
            }
            Intent::CloseAssignmentPicker => self.handle_close_assignment_picker(),
            Intent::CloseSelectorPopup => self.handle_close_selector_popup(),
            Intent::SelectPreampInput(input) => self.handle_select_preamp_input(input),
            Intent::AdjustPreampGain { input, increase } => {
                self.handle_adjust_preamp_gain(input, increase, pending)?
            }
            Intent::SetPreampGain { input, raw } => {
                self.handle_set_preamp_gain(input, raw, pending)?
            }
            Intent::OpenPreampModeSelector(input) => {
                self.handle_open_preamp_mode_selector(input)
            }
            Intent::CyclePreampMode(input) => {
                self.handle_cycle_preamp_mode(input, pending)?
            }
            Intent::PickSampleRate(rate) => self.handle_pick_sample_rate(rate, pending)?,
            Intent::PickClockSource(source) => {
                self.handle_pick_clock_source(source, pending)?
            }
            Intent::PickPreampMode { input, mode } => {
                self.handle_pick_preamp_mode(input, mode, pending)?
            }
            Intent::TogglePreampPhase(input) => {
                self.handle_toggle_preamp_phase(input, pending)?
            }
            Intent::TogglePreampPhantom(input) => {
                self.handle_toggle_preamp_phantom(input, pending)?
            }
            Intent::AdjustFocused(increase) => {
                self.handle_adjust_focused(increase, pending)?
            }
            Intent::ToggleFocusedMute => self.handle_toggle_focused_mute(pending)?,
            Intent::ToggleFocusedDim => self.handle_toggle_focused_dim(pending)?,
            Intent::ToggleRoutingPopup => self.handle_toggle_routing_popup(),
            Intent::RefreshQueriedState => self.handle_refresh_queried_state()?,
            Intent::CycleFocus => self.handle_cycle_focus(),
            Intent::MovePopupSelection(down) => self.handle_move_popup_selection(down),
            Intent::ProfileEditorChar(ch) => self.handle_profile_editor_char(ch),
            Intent::ProfileEditorBackspace => self.handle_profile_editor_backspace(),
            Intent::ProfileEditorCommit => self.handle_profile_editor_commit(),
            Intent::ProfileEditorCancel => self.handle_profile_editor_cancel(),
            Intent::CaptureRawBaseline => self.handle_capture_raw_baseline(),
            Intent::ClearRawBaseline => self.handle_clear_raw_baseline(),
            Intent::ToggleOptionsPopup => self.handle_toggle_options_popup(),
        }
        Ok(())
    }

    pub fn poll_device(&mut self, timeout: Duration) -> Result<bool> {
        // Flush pending commands before reading so device sees latest state
        self.flush_commands()?;

        let mut next_timeout = timeout;
        let mut state_dirty = false;

        for _ in 0..MAX_FRAMES_PER_POLL {
            let Some(bytes) = self.transport.read(next_timeout)? else {
                break;
            };

            next_timeout = Duration::ZERO;

            if let Ok(frame) = Frame::parse_owned(bytes) {
                let (snapshot, raw) = frame.into_snapshot_and_raw();
                if matches!(&snapshot, DeviceSnapshot::Snapshot(_)) {
                    state_dirty |= self.confirm_pending_write();
                }
                state_dirty |= self.state.observe_frame(snapshot, raw);
            }
        }

        Ok(state_dirty)
    }

    pub fn confirm_pending_write(&mut self) -> bool {
        let Some(pending) = self.pending_mutation.take() else {
            return false;
        };
        match pending {
            PendingMutation::MixerLevel {
                mixer,
                channel,
                level,
                pan,
                muted,
            } => {
                if let Some(slot) = self.state.mixer.channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.level = Some(level);
                    slot.muted = Some(muted);
                    slot.pan = pan;
                }
                true
            }
            PendingMutation::MixerLinkedLevel {
                mixer,
                left_channel,
                right_channel,
                level,
                left_pan,
                right_pan,
                left_muted,
                right_muted,
            } => {
                for (channel, pan, muted) in [
                    (left_channel, left_pan, left_muted),
                    (right_channel, right_pan, right_muted),
                ] {
                    if let Some(slot) = self.state.mixer.channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.level = Some(level);
                        slot.muted = Some(muted);
                        slot.pan = pan;
                    }
                }
                true
            }
            PendingMutation::MixerMute {
                mixer,
                channel,
                muted,
            } => {
                if let Some(slot) = self.state.mixer.channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.muted = Some(muted);
                }
                true
            }
            PendingMutation::MixerSolo {
                mixer,
                channel,
                soloed,
            } => {
                if let Some(slot) = self.state.mixer.channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.soloed = Some(soloed);
                }
                true
            }
            PendingMutation::MixerLinkedMute {
                mixer,
                left_channel,
                right_channel,
                muted,
            } => {
                for channel in [left_channel, right_channel] {
                    if let Some(slot) = self.state.mixer.channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.muted = Some(muted);
                    }
                }
                true
            }
            PendingMutation::MixerLinkedSolo {
                mixer,
                left_channel,
                right_channel,
                soloed,
            } => {
                for channel in [left_channel, right_channel] {
                    if let Some(slot) = self.state.mixer.channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.soloed = Some(soloed);
                    }
                }
                true
            }
            PendingMutation::MixerPan {
                mixer,
                channel,
                pan,
            } => {
                if let Some(slot) = self.state.mixer.channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.pan = pan;
                }
                true
            }
            PendingMutation::MixerAssignment { strip, assignment } => {
                let index = strip.saturating_sub(1) as usize;
                for channels in &mut self.state.mixer.channels {
                    if let Some(slot) = channels.get_mut(index) {
                        slot.assignment = Some(assignment);
                    }
                }
                true
            }
            PendingMutation::MixerLink {
                mixer,
                selector,
                enabled,
            } => {
                if let Some((left, right)) = link_pair_from_selector(mixer, selector) {
                    for channel in [left, right] {
                        if let Some(slot) = self.state.mixer.channels[mixer.index()]
                            .get_mut(channel.saturating_sub(1) as usize)
                        {
                            slot.linked = Some(enabled);
                        }
                    }
                }
                true
            }
            PendingMutation::MixerLinkExplicit {
                mixer,
                left_channel,
                right_channel,
                enabled,
            } => {
                for channel in [left_channel, right_channel] {
                    if let Some(slot) = self.state.mixer.channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.linked = Some(enabled);
                    }
                }
                true
            }
            PendingMutation::OutputVolume { target, step } => {
                self.state.output.states[target.index() as usize].volume = step;
                true
            }
            PendingMutation::OutputMode { target, mode } => {
                self.state.output.states[target.index() as usize].mode = mode;
                true
            }
            PendingMutation::PreampGain { input, raw } => {
                self.state.device.dsp_cluster[input.min(1) as usize] = raw;
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                true
            }
            PendingMutation::PreampMode { input, mode } => {
                let offset = 2 + input.min(1) as usize;
                let preserved_bits = self.state.device.dsp_cluster[offset] & 0xf0;
                self.state.device.dsp_cluster[offset] = preserved_bits | mode.code();
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                true
            }
            PendingMutation::PreampPhantom { input, enabled } => {
                let offset = 2 + input.min(1) as usize;
                let low = self.state.device.dsp_cluster[offset] & 0x0f;
                self.state.device.dsp_cluster[offset] = low | if enabled { 0x10 } else { 0x00 };
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                true
            }
            PendingMutation::PreampPhase { input, enabled } => {
                let offset = 2 + input.min(1) as usize;
                let low = self.state.device.dsp_cluster[offset] & 0x1f;
                self.state.device.dsp_cluster[offset] = low | if enabled { 0x40 } else { 0x00 };
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                true
            }
        }
    }

    fn handle_output_select(&mut self, index: usize) {
        self.state.ui.focus = FocusArea::Outputs;
        self.state.output.selected = index.min(self.state.output.states.len() - 1);
    }

    fn handle_output_adjust(&mut self, index: usize, increase: bool, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Outputs;
        self.state.output.selected = index.min(self.state.output.states.len() - 1);
        let output = self.state.output.states[self.state.output.selected];
        let next = if increase {
            output.volume.saturating_sub(1)
        } else {
            output.volume.saturating_add(1).min(0x60)
        };
        self.state.output.states[self.state.output.selected].volume = next;
        self.send(
            Command::SetOutputVolume {
                target: output.target,
                step: next,
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_output_set_level(&mut self, index: usize, step: u8, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Outputs;
        self.state.output.selected = index.min(self.state.output.states.len() - 1);
        let output = self.state.output.states[self.state.output.selected];
        self.state.output.states[self.state.output.selected].volume = step.min(0x60);
        self.send(
            Command::SetOutputVolume {
                target: output.target,
                step: step.min(0x60),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_output_toggle_dim(&mut self, index: usize, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Outputs;
        self.state.output.selected = index.min(self.state.output.states.len() - 1);
        let output = self.state.output.states[self.state.output.selected];
        let new_mode = if output.mode != OutputMode::Dim {
            OutputMode::Dim
        } else {
            OutputMode::Normal
        };
        self.state.output.states[self.state.output.selected].mode = new_mode;
        self.send(
            Command::SetOutputDim {
                target: output.target,
                enabled: output.mode != OutputMode::Dim,
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_output_toggle_mute(&mut self, index: usize, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Outputs;
        self.state.output.selected = index.min(self.state.output.states.len() - 1);
        let output = self.state.output.states[self.state.output.selected];
        let new_mode = if output.mode != OutputMode::Mute {
            OutputMode::Mute
        } else {
            OutputMode::Normal
        };
        self.state.output.states[self.state.output.selected].mode = new_mode;
        self.send(
            Command::SetOutputMute {
                target: output.target,
                enabled: output.mode != OutputMode::Mute,
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_open_sample_rate_selector(&mut self) {
        if self.state.device.status.clock_source == Some(ClockSource::Internal) {
            self.state.popup.selected_index = self
                .state
                .device
                .status
                .sample_rate
                .and_then(|current| {
                    SampleRate::all_confirmed()
                        .iter()
                        .position(|rate| *rate == current)
                })
                .unwrap_or(0);
            self.state.popup.selector_popup = Some(SelectorPopupState {
                kind: SelectorPopupKind::SampleRate,
            });
        }
    }

    fn handle_open_clock_source_selector(&mut self) {
        self.state.popup.selected_index = self
            .state
            .device
            .status
            .clock_source
            .and_then(|current| {
                ClockSource::all_confirmed()
                    .iter()
                    .position(|source| *source == current)
            })
            .unwrap_or(0);
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::ClockSource,
        });
    }

    fn handle_pick_sample_rate(&mut self, rate: SampleRate, pending: Option<PendingMutation>) -> Result<()> {
        self.state.popup.selector_popup = None;
        self.state.popup.selected_index = 0;
        self.send(Command::SetSampleRate(rate), pending)?;
        Ok(())
    }

    fn handle_pick_clock_source(&mut self, source: ClockSource, pending: Option<PendingMutation>) -> Result<()> {
        self.state.popup.selector_popup = None;
        self.state.popup.selected_index = 0;
        self.send(Command::SetClockSource(source), pending)?;
        Ok(())
    }

    fn handle_select_raw_packet_tab(&mut self, tab: RawPacketTab) {
        self.state.raw_view.selected_tab = tab;
    }

    fn handle_select_query_reply_entry(&mut self, index: usize) {
        self.state.raw_view.selected_query_reply_entry = Some(
            index.min(
                self.state
                    .raw_view
                    .recent_query_reply_entries
                    .len()
                    .saturating_sub(1),
            ),
        );
    }

    fn handle_scroll_query_reply_list(&mut self, increase: bool) {
        self.state.cycle_query_reply_entry(increase);
    }

    fn handle_capture_raw_baseline(&mut self) {
        self.state.capture_raw_baseline();
        self.state.ui.last_message =
            "Captured raw baseline for 0x73/0x83/0x75/0x81".to_string();
    }

    fn handle_clear_raw_baseline(&mut self) {
        self.state.clear_raw_baseline();
        self.state.ui.last_message = "Cleared raw baseline".to_string();
    }

    fn handle_open_routing_popup(&mut self) {
        self.state.popup.profiles_open = false;
        self.state.popup.profile_editor = None;
        self.state.popup.routing_open = true;
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel = self.state.mixer.selected_channel.min(7);
        self.state.ui.last_message =
            "Routing popup mirrors mixer assignments for USB recording channels 1-8"
                .to_string();
    }

    fn handle_close_routing_popup(&mut self) {
        self.state.popup.routing_open = false;
        self.state.ui.last_message = "Closed routing popup".to_string();
    }

    fn handle_toggle_routing_popup(&mut self) {
        self.state.popup.routing_open = !self.state.popup.routing_open;
        self.state.ui.last_message = if self.state.popup.routing_open {
            "Routing popup mirrors mixer assignments for USB recording channels 1-8"
                .to_string()
        } else {
            "Closed routing popup".to_string()
        };
    }

    fn handle_open_options_popup(&mut self) {
        self.state.popup.profiles_open = false;
        self.state.popup.profile_editor = None;
        self.state.popup.routing_open = false;
        self.state.popup.options_open = true;
        self.state.ui.last_message = "Options popup opened".to_string();
    }

    fn handle_close_options_popup(&mut self) {
        self.state.popup.options_open = false;
        self.state.ui.last_message = "Closed options popup".to_string();
    }

    fn handle_toggle_options_popup(&mut self) {
        self.state.toggle_options_popup();
        self.state.ui.last_message = if self.state.popup.options_open {
            "Options popup opened".to_string()
        } else {
            "Closed options popup".to_string()
        };
    }

    fn handle_set_refresh_rate(&mut self, rate: RefreshRate) {
        self.state.ui.settings.refresh_rate = rate;
        self.state.ui.last_message = format!("Refresh rate set to {}", rate.label());
        if self.state.ui.settings.auto_save {
            let _ = crate::settings::save_settings(&self.state.ui.settings);
        }
    }

    fn handle_cycle_peak_threshold(&mut self, increase: bool) {
        const PEAK_THRESHOLD_CHOICES: [u8; 10] =
            [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x0a, 0x0f, 0x14];
        let current = self.state.ui.settings.peak_threshold_raw;
        let pos = PEAK_THRESHOLD_CHOICES
            .iter()
            .position(|&v| v == current)
            .unwrap_or(3);
        let next_pos = if increase {
            (pos + 1).min(PEAK_THRESHOLD_CHOICES.len() - 1)
        } else {
            pos.saturating_sub(1)
        };
        self.state.ui.settings.peak_threshold_raw = PEAK_THRESHOLD_CHOICES[next_pos];
        let db = self.state.ui.settings.peak_threshold_db();
        self.state.ui.last_message = format!("Peak threshold set to {} dB", db);
        if self.state.ui.settings.auto_save {
            let _ = crate::settings::save_settings(&self.state.ui.settings);
        }
    }

    fn handle_toggle_peak_enabled(&mut self) {
        self.state.ui.settings.peak_enabled = !self.state.ui.settings.peak_enabled;
        if self.state.ui.settings.peak_enabled {
            self.state.ui.last_message = "Peak detection enabled".to_string();
        } else {
            self.state.preamp.peaks = [None, None];
            self.state.mixer.peaks = [[None; 16]; 2];
            self.state.ui.last_message = "Peak detection disabled".to_string();
        }
        if self.state.ui.settings.auto_save {
            let _ = crate::settings::save_settings(&self.state.ui.settings);
        }
    }

    fn handle_cycle_peak_hold_duration(&mut self, duration: PeakHoldDuration) {
        self.state.ui.settings.peak_hold_duration = duration;
        self.state.ui.last_message =
            format!("Peak hold duration set to {}", duration.label());
        if self.state.ui.settings.auto_save {
            let _ = crate::settings::save_settings(&self.state.ui.settings);
        }
    }

    fn handle_toggle_auto_save(&mut self) {
        self.state.ui.settings.auto_save = !self.state.ui.settings.auto_save;
        if self.state.ui.settings.auto_save {
            self.state.ui.last_message = "Auto-save enabled".to_string();
            let _ = crate::settings::save_settings(&self.state.ui.settings);
        } else {
            self.state.ui.last_message = "Auto-save disabled".to_string();
        }
    }

    fn handle_select_preamp_input(&mut self, input: usize) {
        self.state.ui.focus = FocusArea::Preamp;
        self.state.preamp.selected_input = input.min(1);
    }

    fn handle_adjust_preamp_gain(&mut self, input: u8, increase: bool, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Preamp;
        self.state.preamp.selected_input = input.min(1) as usize;
        let current = if input == 0 {
            self.state.preamp.state.input1.gain_raw
        } else {
            self.state.preamp.state.input2.gain_raw
        };
        let next = next_preamp_gain_raw(current, increase);
        self.state.device.dsp_cluster[input.min(1) as usize] = next;
        self.state
            .refresh_preamp_from_cluster_preserving_observed_meter();
        self.send(Command::SetPreampGain { input, raw: next }, pending)?;
        Ok(())
    }

    fn handle_set_preamp_gain(&mut self, input: u8, raw: u8, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Preamp;
        self.state.preamp.selected_input = input.min(1) as usize;
        self.state.device.dsp_cluster[input.min(1) as usize] = raw;
        self.state
            .refresh_preamp_from_cluster_preserving_observed_meter();
        self.send(
            Command::SetPreampGain {
                input: input.min(1),
                raw,
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_open_preamp_mode_selector(&mut self, input: u8) {
        self.state.ui.focus = FocusArea::Preamp;
        self.state.preamp.selected_input = input.min(1) as usize;
        let current = if input == 0 {
            self.state.preamp.state.input1.mode
        } else {
            self.state.preamp.state.input2.mode
        };
        self.state.popup.selected_index =
            [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                .iter()
                .position(|mode| *mode == current)
                .unwrap_or(0);
        self.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::PreampMode { input },
        });
    }

    fn handle_cycle_preamp_mode(&mut self, input: u8, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Preamp;
        self.state.preamp.selected_input = input.min(1) as usize;
        let current = if input == 0 {
            self.state.preamp.state.input1.mode
        } else {
            self.state.preamp.state.input2.mode
        };
        let next = match current {
            PreampMode::Mic => PreampMode::Line,
            PreampMode::Line => PreampMode::HiZ,
            PreampMode::HiZ | PreampMode::Unknown(_) => PreampMode::Mic,
        };
        self.send(Command::SetPreampMode { input, mode: next }, pending)?;
        Ok(())
    }

    fn handle_pick_preamp_mode(&mut self, input: u8, mode: PreampMode, pending: Option<PendingMutation>) -> Result<()> {
        self.state.popup.selector_popup = None;
        self.state.popup.selected_index = 0;
        self.state.ui.focus = FocusArea::Preamp;
        self.state.preamp.selected_input = input.min(1) as usize;
        self.send(Command::SetPreampMode { input, mode }, pending)?;
        Ok(())
    }

    fn handle_toggle_preamp_phase(&mut self, input: u8, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Preamp;
        self.state.preamp.selected_input = input.min(1) as usize;
        let mode_raw = if input == 0 {
            self.state.preamp.state.input1.mode_raw
        } else {
            self.state.preamp.state.input2.mode_raw
        };
        self.send(
            Command::SetPreampPhase {
                input,
                enabled: mode_raw & 0x40 == 0,
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_toggle_preamp_phantom(&mut self, input: u8, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Preamp;
        self.state.preamp.selected_input = input.min(1) as usize;
        let current = if input == 0 {
            self.state.preamp.state.input1
        } else {
            self.state.preamp.state.input2
        };
        self.send(
            Command::SetPreampPhantom {
                input,
                enabled: !current.phantom_on,
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_page_mixer_strips(&mut self, area: Rect, left: bool) {
        self.state.ui.focus = FocusArea::Mixer;
        let visible = crate::ui::mixer_strip_viewport_capacity(area, &self.state);
        self.state.page_mixer_strip_viewport(left, visible);
    }

    fn handle_select_mixer_channel(&mut self, index: usize) {
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel = index;
    }

    fn handle_adjust_mixer_level(&mut self, index: usize, increase: bool, _pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel =
            index.min(self.state.active_mixer_channels().len() - 1);
        let active_channel =
            self.state.active_mixer_channels()[self.state.mixer.selected_channel];
        let current = active_channel.level.unwrap_or(0x20);
        let next = if increase {
            current.saturating_sub(1)
        } else {
            current.saturating_add(1).min(0x60)
        };
        self.send_mixer_level_change(
            MixerSurface::from_surface(self.state.mixer.surface),
            active_channel.channel,
            next,
        )?;
        Ok(())
    }

    fn handle_set_mixer_level(&mut self, index: usize, level: u8, _pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel =
            index.min(self.state.active_mixer_channels().len() - 1);
        let active_channel =
            self.state.active_mixer_channels()[self.state.mixer.selected_channel];
        self.send_mixer_level_change(
            MixerSurface::from_surface(self.state.mixer.surface),
            active_channel.channel,
            level.min(0x5a),
        )?;
        Ok(())
    }

    fn handle_adjust_mixer_pan(&mut self, index: usize, right: bool, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel =
            index.min(self.state.active_mixer_channels().len() - 1);
        let active_channel =
            self.state.active_mixer_channels()[self.state.mixer.selected_channel];
        let next = if right {
            active_channel
                .pan
                .raw()
                .saturating_add(1)
                .min(PanState::MAX)
        } else {
            active_channel
                .pan
                .raw()
                .saturating_sub(1)
                .max(PanState::MIN)
        };
        let surface = MixerSurface::from_surface(self.state.mixer.surface);
        if let Some(slot) = self.state.mixer.channels[surface.index()]
            .get_mut(active_channel.channel.saturating_sub(1) as usize)
        {
            slot.pan = PanState::from_raw(next);
        }
        self.send(
            Command::SetMixerPan {
                mixer: surface,
                channel: active_channel.channel,
                pan: PanState::from_raw(next),
                muted: active_channel.muted.unwrap_or(false),
                soloed: active_channel.soloed.unwrap_or(false),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_set_mixer_pan(&mut self, index: usize, pan: PanState, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel =
            index.min(self.state.active_mixer_channels().len() - 1);
        let active_channel =
            self.state.active_mixer_channels()[self.state.mixer.selected_channel];
        let surface = MixerSurface::from_surface(self.state.mixer.surface);
        if let Some(slot) = self.state.mixer.channels[surface.index()]
            .get_mut(active_channel.channel.saturating_sub(1) as usize)
        {
            slot.pan = pan;
        }
        self.send(
            Command::SetMixerPan {
                mixer: surface,
                channel: active_channel.channel,
                pan,
                muted: active_channel.muted.unwrap_or(false),
                soloed: active_channel.soloed.unwrap_or(false),
            },
            pending,
        )?;
        Ok(())
    }

    fn handle_toggle_mixer_mute(&mut self, channel: u8, _pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel = channel.saturating_sub(1) as usize;
        let mixer = MixerSurface::from_surface(self.state.mixer.surface);
        let active_channel = self.state.mixer.channels[mixer.index()][channel as usize - 1];
        self.send_mixer_mute_change(
            mixer,
            channel,
            !active_channel.muted.unwrap_or(false),
        )?;
        Ok(())
    }

    fn handle_toggle_mixer_solo(&mut self, channel: u8, _pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel = channel.saturating_sub(1) as usize;
        let mixer = MixerSurface::from_surface(self.state.mixer.surface);
        let active_channel = self.state.mixer.channels[mixer.index()][channel as usize - 1];
        self.send_mixer_solo_change(
            mixer,
            channel,
            !active_channel.soloed.unwrap_or(false),
        )?;
        Ok(())
    }

    fn handle_toggle_mixer_link(&mut self, channel: u8, _pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel = channel.saturating_sub(1) as usize;
        let mixer = MixerSurface::from_surface(self.state.mixer.surface);
        let active_channel = self.state.mixer.channels[mixer.index()][channel as usize - 1];
        self.send_mixer_link_change(
            mixer,
            channel,
            !active_channel.linked.unwrap_or(false),
        )?;
        Ok(())
    }

    fn handle_open_assignment_picker(&mut self, strip: u8) {
        self.state.ui.focus = FocusArea::Mixer;
        self.state.mixer.selected_channel = strip.saturating_sub(1) as usize;
        if !antelope_protocol::MixerStrip::assignment_write_is_grounded(strip) {
            self.state.ui.last_message =
                "Assignment picking is not grounded for the selected strip.".to_string();
        } else {
            self.state.popup.selected_index = self.state.mixer.channels
                [MixerSurface::from_surface(self.state.mixer.surface).index()]
                [self.state.mixer.selected_channel]
                .assignment
                .and_then(|current| {
                    MixerAssignment::grounded_choices()
                        .iter()
                        .position(|assignment| *assignment == current)
                })
                .unwrap_or(0);
            self.state.popup.assignment_picker = Some(AssignmentPickerState { strip });
            self.state.ui.last_message =
                format!("Pick source assignment for CH {strip:02}");
        }
    }

    fn handle_pick_assignment(&mut self, strip: u8, assignment: MixerAssignment, pending: Option<PendingMutation>) -> Result<()> {
        self.state.popup.assignment_picker = None;
        self.state.popup.selected_index = 0;
        self.send(Command::SetMixerAssignment { strip, assignment }, pending)?;
        Ok(())
    }

    fn handle_close_assignment_picker(&mut self) {
        self.state.popup.assignment_picker = None;
        self.state.popup.selected_index = 0;
        self.state.ui.last_message = "Closed assignment picker".to_string();
    }

    fn handle_open_profiles_popup(&mut self) {
        self.state.popup.assignment_picker = None;
        self.state.popup.selector_popup = None;
        self.state.popup.routing_open = false;
        self.state.popup.profile_editor = None;
        self.state.popup.profile_names =
            crate::profile::list_profile_names().unwrap_or_default();
        self.state.clamp_profile_selection();
        self.state.popup.profiles_open = true;
        self.state.ui.last_message = if self.state.popup.profile_names.is_empty() {
            "No saved profiles yet. Use SAVE to create one.".to_string()
        } else {
            "Select a profile to load, or use SAVE/RENAME/DELETE.".to_string()
        };
    }

    fn handle_close_profiles_popup(&mut self) {
        self.state.popup.profiles_open = false;
        self.state.popup.profile_editor = None;
        self.state.ui.last_message = "Closed profiles popup".to_string();
    }

    fn handle_select_profile(&mut self, index: usize) {
        self.state.popup.selected_index =
            index.min(self.state.popup.profile_names.len().saturating_sub(1));
    }

    fn handle_load_selected_profile(&mut self) {
        if let Some(name) = self.state.selected_profile_name().map(str::to_string) {
            let profile_result = crate::profile::DeviceProfile::read_named(&name);
            match profile_result {
                Ok(profile) => {
                    let apply_result = self.apply_profile(&profile);
                    if let Err(e) = apply_result {
                        self.state.ui.last_message = format!("Profile error: {e}");
                    } else {
                        self.state.popup.profiles_open = false;
                        self.state.popup.profile_editor = None;
                        self.state.ui.last_message = format!("Loaded profile {name}");
                    }
                }
                Err(e) => {
                    self.state.ui.last_message = format!("Profile error: {e}");
                }
            }
        } else {
            self.state.ui.last_message = "No profile selected to load.".to_string();
        }
    }

    fn handle_start_save_profile(&mut self) {
        if self.state.popup.profiles_open {
            let current_name = self.state.selected_profile_name().map(str::to_string);
            let value = current_name.clone().unwrap_or_default();
            self.state.popup.profile_editor = Some(ProfileEditorState {
                mode: ProfileEditorMode::Save,
                original_name: current_name,
                value,
            });
            self.state.ui.last_message =
                "Enter a profile name, then press Enter to save.".to_string();
        }
    }

    fn handle_start_rename_profile(&mut self) {
        if self.state.selected_profile_name().is_some() {
            let current_name = self.state.selected_profile_name().map(str::to_string);
            let value = current_name.clone().unwrap_or_default();
            self.state.popup.profile_editor = Some(ProfileEditorState {
                mode: ProfileEditorMode::Rename,
                original_name: current_name,
                value,
            });
            self.state.ui.last_message =
                "Edit the profile name, then press Enter to rename.".to_string();
        } else {
            self.state.ui.last_message = "No profile selected to rename.".to_string();
        }
    }

    fn handle_delete_selected_profile(&mut self) {
        if let Some(name) = self.state.selected_profile_name().map(str::to_string) {
            match crate::profile::delete_profile(&name) {
                Ok(()) => {
                    self.state.popup.profile_names =
                        crate::profile::list_profile_names().unwrap_or_default();
                    self.state.clamp_profile_selection();
                    self.state.ui.last_message = format!("Deleted profile {name}");
                }
                Err(e) => {
                    self.state.ui.last_message = format!("Profile error: {e}");
                }
            }
        } else {
            self.state.ui.last_message = "No profile selected to delete.".to_string();
        }
    }

    fn handle_profile_editor_char(&mut self, ch: String) {
        if let Some(editor) = self.state.popup.profile_editor.as_mut() {
            editor.value.push_str(&ch);
        }
    }

    fn handle_profile_editor_backspace(&mut self) {
        if let Some(editor) = self.state.popup.profile_editor.as_mut() {
            editor.value.pop();
        }
    }

    fn handle_profile_editor_commit(&mut self) {
        if let Some(editor) = self.state.popup.profile_editor.take() {
            let name = editor.value.trim().to_string();
            if name.is_empty() {
                self.state.ui.last_message = "Profile name cannot be empty".to_string();
                self.state.popup.profile_editor = Some(editor);
            } else {
                match editor.mode {
                    ProfileEditorMode::Save => {
                        let profile = DeviceProfile::capture(&self.state);
                        match profile {
                            Ok(profile) => match profile.write_named(&name) {
                                Ok(path) => {
                                    self.state.popup.profiles_open = false;
                                    self.state.ui.last_message =
                                        format!("Saved profile to {}", path.display());
                                }
                                Err(e) => {
                                    self.state.ui.last_message =
                                        format!("Profile error: {e}");
                                }
                            },
                            Err(e) => {
                                self.state.ui.last_message = format!("Profile error: {e}");
                            }
                        }
                    }
                    ProfileEditorMode::Rename => {
                        if let Some(original) = &editor.original_name {
                            if original != &name {
                                match crate::profile::rename_profile(original, &name) {
                                    Ok(_path) => {
                                        self.state.popup.profile_names =
                                            crate::profile::list_profile_names()
                                                .unwrap_or_default();
                                        self.state.clamp_profile_selection();
                                        self.state.ui.last_message =
                                            format!("Renamed {original} to {name}");
                                    }
                                    Err(e) => {
                                        self.state.ui.last_message =
                                            format!("Profile error: {e}");
                                    }
                                }
                            } else {
                                self.state.ui.last_message =
                                    "Profile name unchanged".to_string();
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_profile_editor_cancel(&mut self) {
        self.state.popup.profile_editor = None;
        self.state.ui.last_message = "Cancelled profile edit".to_string();
    }

    fn handle_select_surface(&mut self, surface: Surface, pending: Option<PendingMutation>) -> Result<()> {
        self.state.ui.focus = FocusArea::Mixer;
        self.send(Command::SelectSurface(surface), pending)?;
        self.flush_commands()?;
        self.refresh_queried_state()?;
        Ok(())
    }

    fn handle_cycle_focus(&mut self) {
        self.state.cycle_focus();
    }

    fn handle_move_popup_selection(&mut self, down: bool) {
        let item_count = if self.state.popup.assignment_picker.is_some() {
            antelope_protocol::MixerAssignment::grounded_choices().len()
        } else if self.state.popup.profiles_open {
            self.state.popup.profile_names.len()
        } else if let Some(popup) = self.state.popup.selector_popup {
            match popup.kind {
                SelectorPopupKind::SampleRate => SampleRate::all_confirmed().len(),
                SelectorPopupKind::ClockSource => ClockSource::all_confirmed().len(),
                SelectorPopupKind::PreampMode { .. } => 3,
            }
        } else {
            0
        };
        if item_count == 0 {
            return;
        }
        self.state.popup.selected_index = if down {
            (self.state.popup.selected_index + 1) % item_count
        } else {
            self.state
                .popup
                .selected_index
                .checked_sub(1)
                .unwrap_or(item_count - 1)
        };
    }

    fn handle_adjust_focused(&mut self, increase: bool, pending: Option<PendingMutation>) -> Result<()> {
        match self.state.ui.focus {
            FocusArea::Outputs => {
                let index = self.state.output.selected;
                let output = self.state.output.states[index];
                let next = if increase {
                    output.volume.saturating_sub(1)
                } else {
                    output.volume.saturating_add(1).min(0x60)
                };
                self.send(
                    Command::SetOutputVolume {
                        target: output.target,
                        step: next,
                    },
                    pending,
                )?;
            }
            FocusArea::Mixer => {
                let active_channel =
                    self.state.active_mixer_channels()[self.state.mixer.selected_channel];
                let channel = active_channel.channel;
                let current = active_channel.level.unwrap_or(0x20);
                let next = if increase {
                    current.saturating_sub(1)
                } else {
                    current.saturating_add(1).min(0x60)
                };
                self.send_mixer_level_change(
                    MixerSurface::from_surface(self.state.mixer.surface),
                    channel,
                    next,
                )?;
            }
            FocusArea::Preamp => {
                let input = self.state.preamp.selected_input as u8;
                let preamp_input = if input == 0 {
                    &self.state.preamp.state.input1
                } else {
                    &self.state.preamp.state.input2
                };
                let next = match preamp_input.mode {
                    PreampMode::Mic => {
                        if increase {
                            preamp_input.gain_raw.saturating_add(1).min(0x41)
                        } else {
                            preamp_input.gain_raw.saturating_sub(1)
                        }
                    }
                    PreampMode::Line => {
                        let current = i8::from_ne_bytes([preamp_input.gain_raw]);
                        let next = if increase {
                            (current + 1).min(20)
                        } else {
                            (current - 1).max(-6)
                        };
                        next as u8
                    }
                    PreampMode::HiZ => {
                        if increase {
                            preamp_input.gain_raw.saturating_add(1).min(0x2d)
                        } else {
                            preamp_input.gain_raw.saturating_sub(1)
                        }
                    }
                    PreampMode::Unknown(_) => preamp_input.gain_raw,
                };
                self.send(Command::SetPreampGain { input, raw: next }, pending)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_toggle_focused_mute(&mut self, pending: Option<PendingMutation>) -> Result<()> {
        match self.state.ui.focus {
            FocusArea::Outputs => {
                let index = self.state.output.selected;
                let output = self.state.output.states[index];
                self.send(
                    Command::SetOutputMute {
                        target: output.target,
                        enabled: output.mode != OutputMode::Mute,
                    },
                    pending,
                )?;
            }
            FocusArea::Mixer => {
                let active_channel =
                    self.state.active_mixer_channels()[self.state.mixer.selected_channel];
                let channel = active_channel.channel;
                let muted = !active_channel.muted.unwrap_or(false);
                self.send_mixer_mute_change(
                    MixerSurface::from_surface(self.state.mixer.surface),
                    channel,
                    muted,
                )?;
            }
            FocusArea::Preamp => {
                let input = self.state.preamp.selected_input as u8;
                let current = if input == 0 {
                    self.state.preamp.state.input1
                } else {
                    self.state.preamp.state.input2
                };
                self.send(
                    Command::SetPreampPhantom {
                        input,
                        enabled: !current.phantom_on,
                    },
                    pending,
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_toggle_focused_dim(&mut self, pending: Option<PendingMutation>) -> Result<()> {
        if self.state.ui.focus == FocusArea::Outputs {
            let index = self.state.output.selected;
            let output = self.state.output.states[index];
            self.send(
                Command::SetOutputDim {
                    target: output.target,
                    enabled: output.mode != OutputMode::Dim,
                },
                pending,
            )?;
        }
        Ok(())
    }

    fn handle_close_selector_popup(&mut self) {
        self.state.popup.selector_popup = None;
        self.state.popup.selected_index = 0;
        self.state.ui.last_message = "Closed selector".to_string();
    }

    fn handle_refresh_queried_state(&mut self) -> Result<()> {
        self.refresh_queried_state()?;
        self.state.ui.last_message =
            "Sent captured 0x74 startup/state refresh sweep".to_string();
        Ok(())
    }
}

fn link_pair_from_selector(mixer: MixerSurface, selector: u8) -> Option<(u8, u8)> {
    MixerLinkTarget::from_selector(mixer, selector)
        .map(|target| (target.left_channel, target.right_channel))
}

/// Computes the next preamp gain raw value for increment/decrement.
fn next_preamp_gain_raw(current: u8, up: bool) -> u8 {
    if up {
        current.saturating_add(1).min(0x41)
    } else {
        current.saturating_sub(1)
    }
}
