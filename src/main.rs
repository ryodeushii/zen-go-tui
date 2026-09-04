mod cli;
mod input;
mod profile_ops;
mod runtime;
mod timing;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, CliCommand};
use crate::profile_ops::run_profile_command;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut devices = cli::open_runtime(&cli)?;

    match cli.command {
        Some(CliCommand::Profile { command }) => {
            let session = devices.take_session().ok_or_else(|| {
                anyhow::anyhow!("profile command requires one supported, unambiguous device")
            })?;
            run_profile_command(session, command)
        }
        None if cli.headless => runtime::run_headless_app(devices),
        None => runtime::run_app(devices),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use antelope_protocol::{
        control_panel_startup_queries, ClockSource, MixerAssignment, MixerSurface, OutputMode,
        OutputState, OutputTarget, PanState, PreampMode, RuntimeRoutingSourceDomain, SampleRate,
        Surface,
    };
    use zen_go_tui::app::{
        AssignmentPickerState, Controller, FocusArea, ProfileEditorMode, ProfileEditorState,
        SelectorPopupKind, SelectorPopupState,
    };
    use zen_go_tui::device::ProfileCatalog;
    use zen_go_tui::terminal::{
        AppKeyCode, AppKeyEvent, AppKeyEventKind, AppModifiers, AppMouseEvent, AppMouseEventKind,
    };
    use zen_go_tui::transport::MockTransport;
    use zen_go_tui::transport::Transport;
    use zen_go_tui::transport::TransportError;
    use zen_go_tui::ui;

    use crate::input::collect_pending_input;
    use crate::runtime::{
        activate_popup_selection, handle_key_press, handle_mouse_event, handle_runtime_error,
        refresh_after_reconnect_if_needed, KeyAction,
    };
    use crate::timing::{device_poll_interval, should_draw_frame, should_probe_reconnect};

    fn test_controller(transport: Box<dyn Transport>) -> Controller {
        let catalog = ProfileCatalog::builtin();
        let mut entry = catalog
            .entries()
            .iter()
            .find(|entry| entry.id == "zen_go_sc")
            .expect("Zen Go profile")
            .clone();
        // Assignment tests use explicit supported destination 0 to exercise legacy
        // compatibility while keeping runtime keyboard routing address-based.
        entry.profile.routing_groups[0].destination = 0;
        entry.profile.routing_groups[0].channel_count = 16;
        entry.profile.routing_groups[0].source_domains = vec![RuntimeRoutingSourceDomain {
            bank: 0,
            index_count: 16,
            status: "confirmed".into(),
            evidence: "test fixture".into(),
        }];
        Controller::new_for_entry(
            transport,
            Box::new(zen_go_tui::device::builtin_zen_go_driver().expect("Zen Go driver")),
            &entry,
        )
        .expect("Zen Go controller")
    }

    fn seed_first_mixer_strip_state(
        controller: &mut Controller,
        fader: i32,
        pan: i32,
        muted: bool,
        soloed: bool,
    ) {
        let strip = controller.state.mixer.surfaces[0]
            .strips
            .get_mut(0)
            .expect("first mixer strip");
        strip.fader = Some(fader);
        strip.pan = Some(pan);
        strip.muted = Some(muted);
        strip.soloed = Some(soloed);
    }

    fn test_key(code: AppKeyCode) -> AppKeyEvent {
        AppKeyEvent {
            code,
            modifiers: AppModifiers::default(),
            kind: AppKeyEventKind::Press,
        }
    }

    #[derive(Clone, Default)]
    struct AvailabilityTransport {
        inner: Arc<Mutex<AvailabilityTransportInner>>,
    }

    #[derive(Default)]
    struct AvailabilityTransportInner {
        available: bool,
        writes: Vec<Vec<u8>>,
    }

    impl AvailabilityTransport {
        fn set_available(&self, available: bool) {
            if let Ok(mut inner) = self.inner.lock() {
                inner.available = available;
            }
        }

        fn write_count(&self) -> usize {
            self.inner
                .lock()
                .map(|inner| inner.writes.len())
                .unwrap_or(0)
        }
    }

    impl Transport for AvailabilityTransport {
        fn write(&self, data: &[u8]) -> Result<()> {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| anyhow::anyhow!("lock poisoned"))?;
            if !inner.available {
                return Err(TransportError::DeviceUnavailable.into());
            }
            inner.writes.push(data.to_vec());
            Ok(())
        }

        fn read(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn is_available(&self) -> Result<bool> {
            Ok(self
                .inner
                .lock()
                .map(|inner| inner.available)
                .unwrap_or(false))
        }
    }

    fn seed_shared_assignments(controller: &mut Controller) {
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

        for surface in &mut controller.state.mixer.channels {
            for (channel, assignment) in surface.iter_mut().zip(assignments) {
                channel.assignment = Some(assignment);
            }
        }
    }

    #[test]
    fn opening_assignment_picker_from_keyboard_does_not_send_assignment_change() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller);
        controller.state.ui.focus = FocusArea::Mixer;
        controller.state.mixer.selected_channel = 0;
        controller.state.mixer.channels[MixerSurface::Mix1.index()][0].assignment =
            Some(MixerAssignment::Preamp(1));

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Char('a')),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("open assignment picker");

        assert_eq!(action, KeyAction::Continue);
        assert!(transport.take_writes().is_empty());
        assert_eq!(
            controller.state.popup.assignment_picker,
            Some(AssignmentPickerState { strip: 1 })
        );
    }

    #[test]
    fn opening_assignment_picker_from_routing_popup_uses_selected_routing_channel() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller);
        controller.state.popup.routing_open = true;
        controller.state.ui.focus = FocusArea::Mixer;
        controller.state.mixer.selected_channel = 5;

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Char('a')),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("open assignment picker from routing popup");

        assert_eq!(action, KeyAction::Continue);
        assert!(transport.take_writes().is_empty());
        assert_eq!(
            controller.state.popup.assignment_picker,
            Some(AssignmentPickerState { strip: 6 })
        );
    }

    #[test]
    fn opening_preamp_mode_selector_from_keyboard_does_not_send_mode_change() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        controller.state.ui.focus = FocusArea::Preamp;
        controller.state.preamp.selected_input = 1;
        controller.state.input_spaces[0].inputs[1].mode = Some(i32::from(PreampMode::Line.code()));

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Char('3')),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("open preamp mode selector");

        assert_eq!(action, KeyAction::Continue);
        assert!(transport.take_writes().is_empty());
        assert_eq!(
            controller.state.popup.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::PreampMode { input: 1 }
            })
        );
        assert_eq!(controller.state.popup.selected_index, 1);
    }

    #[test]
    fn up_key_adjusts_focused_output_level() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        controller.state.ui.focus = FocusArea::Outputs;
        controller.state.output.dynamic[0].level = Some(0x30);

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Up),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("up key");

        assert_eq!(action, KeyAction::Continue);
        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x47, 0x00, 0x2f]);
    }

    #[test]
    fn down_key_adjusts_focused_preamp_gain() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        controller.state.ui.focus = FocusArea::Preamp;
        controller.state.preamp.selected_input = 1;
        controller.state.input_spaces[0].inputs[1].mode = Some(i32::from(PreampMode::Mic.code()));
        controller.state.input_spaces[0].inputs[1].gain = Some(0x10);

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Down),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("down key");

        assert_eq!(action, KeyAction::Continue);
        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x50, 0x01, 0x0f]);
    }

    #[test]
    fn up_key_moves_popup_selection_before_adjusting_controls() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        controller.state.ui.focus = FocusArea::Outputs;
        controller.state.output.selected = 1;
        controller.state.output.states[1] =
            OutputState::new(OutputTarget::Hp1, 0x30, OutputMode::Normal);
        controller.state.popup.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::SampleRate,
        });
        controller.state.popup.selected_index = 1;

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Up),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("popup up key");

        assert_eq!(action, KeyAction::Continue);
        assert_eq!(controller.state.popup.selected_index, 0);
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn toggle_mixer_solo_sends_selected_channel_state() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        controller.state.ui.focus = FocusArea::Mixer;
        controller.state.mixer.selected_channel = 0;
        seed_first_mixer_strip_state(&mut controller, 0, 0, false, false);

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Char('o')),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("toggle solo");

        assert_eq!(action, KeyAction::Continue);
        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x00, 0xa0]
        );
    }

    #[test]
    fn mouse_assignment_picker_sends_selected_assignment_for_ordinary_strip() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller);

        controller
            .apply_intent(
                ui::Intent::OpenAssignmentPicker(5),
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open picker");
        assert_eq!(
            controller.state.popup.assignment_picker,
            Some(AssignmentPickerState { strip: 5 })
        );

        controller
            .apply_intent(
                ui::Intent::PickAssignment {
                    strip: 5,
                    assignment: MixerAssignment::Oscillator(1),
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("pick assignment");

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 5);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x03]);
        assert_eq!(&writes[0][0x10 + 0x0b..0x10 + 0x0d], &[0x09, 0x00]);
    }

    #[test]
    fn opening_assignment_picker_preselects_current_assignment() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport));
        controller.state.mixer.surface = Surface::MonitorHp1;
        controller.state.mixer.channels[MixerSurface::Mix1.index()][4].assignment =
            Some(MixerAssignment::Oscillator(1));

        controller
            .apply_intent(
                ui::Intent::OpenAssignmentPicker(5),
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open picker");

        assert_eq!(controller.state.popup.selected_index, 13);
    }

    #[test]
    fn mouse_output_mute_uses_selected_output_target() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        controller.state.output.states[1] =
            OutputState::new(OutputTarget::Hp1, 0x30, OutputMode::Normal);

        controller
            .apply_intent(
                ui::Intent::ToggleOutputMute(1),
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("toggle output mute");

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x48, 0x01, 0x01]);
    }

    #[test]
    fn mouse_output_level_action_sends_exact_step() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));

        controller
            .apply_intent(
                ui::Intent::SetOutputLevel {
                    index: 1,
                    step: 0x12,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("set output level");

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x47, 0x01, 0x12]);
    }

    #[test]
    fn mouse_preamp_gain_action_sends_exact_raw_gain() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));

        controller
            .apply_intent(
                ui::Intent::SetPreampGain {
                    input: 1,
                    raw: 0x11,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("set preamp gain");

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x50, 0x01, 0x11]);
    }

    #[test]
    fn mouse_mixer_level_action_sends_exact_level() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        seed_first_mixer_strip_state(&mut controller, 0, 0, false, false);

        controller
            .apply_intent(
                ui::Intent::SetMixerLevel {
                    index: 0,
                    level: 0x15,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("set mixer level");

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x15, 0x20]
        );
    }

    #[test]
    fn mouse_mixer_pan_action_sends_exact_pan() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        seed_first_mixer_strip_state(&mut controller, 0, 0, false, false);

        controller
            .apply_intent(
                ui::Intent::SetMixerPan {
                    index: 0,
                    pan: PanState::from_raw(0x12),
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("set mixer pan");

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x00, 0x12]
        );
    }

    #[test]
    fn mouse_adjust_mixer_level_action_sends_single_step_change() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        seed_first_mixer_strip_state(&mut controller, 0x20, 0, false, false);

        controller
            .apply_intent(
                ui::Intent::AdjustMixerLevel {
                    index: 0,
                    increase: true,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("adjust mixer level");

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x1f, 0x20]
        );
    }

    #[test]
    fn mouse_adjust_mixer_pan_action_sends_single_step_change() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        seed_first_mixer_strip_state(&mut controller, 0, 0, false, false);

        controller
            .apply_intent(
                ui::Intent::AdjustMixerPan {
                    index: 0,
                    right: true,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("adjust mixer pan");

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x00, 0x21]
        );
    }

    #[test]
    fn handle_mouse_event_scroll_up_on_output_slider_sends_adjustment() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        controller.state.output.dynamic[0].level = Some(0x30);
        let area = ratatui::layout::Rect::new(0, 0, 120, 50);
        let chunks = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Length(3),
                ratatui::layout::Constraint::Min(17),
            ])
            .split(area);
        let page = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Min(14),
                ratatui::layout::Constraint::Length(8),
            ])
            .split(chunks[1]);
        let inner = ratatui::layout::Rect::new(
            page[1].x + 2,
            page[1].y + 2,
            page[1].width.saturating_sub(4),
            page[1].height.saturating_sub(4),
        );
        let card = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([
                ratatui::layout::Constraint::Percentage(34),
                ratatui::layout::Constraint::Percentage(33),
                ratatui::layout::Constraint::Percentage(33),
            ])
            .split(ratatui::layout::Rect::new(inner.x, inner.y, inner.width, 3))[0];
        let slider_row = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Length(1),
            ])
            .split(card)[1];
        let slider_area = ratatui::layout::Rect::new(
            slider_row.x,
            slider_row.y,
            slider_row.width.min(40),
            slider_row.height,
        );
        let label_width = 12.min(slider_area.width.saturating_sub(1)).max(1);
        let track = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([
                ratatui::layout::Constraint::Length(label_width),
                ratatui::layout::Constraint::Min(1),
            ])
            .split(slider_area)[1];

        handle_mouse_event(
            area,
            &mut controller,
            AppMouseEvent {
                kind: AppMouseEventKind::ScrollUp,
                column: track.x,
                row: track.y,
                modifiers: Default::default(),
            },
        )
        .expect("wheel output slider");

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x47, 0x00, 0x2f]);
    }

    #[test]
    fn page_mixer_strips_right_moves_to_second_bank() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport));
        // Area width 155 gives inner_width=151, card_width=18, stride=19, capacity=8
        let area = ratatui::layout::Rect::new(0, 0, 155, 50);

        controller
            .apply_intent(ui::Intent::PageMixerStripsRight, area)
            .expect("page strips right");

        assert_eq!(controller.state.mixer.strip_scroll, 8);
    }

    #[test]
    fn handle_mouse_event_scroll_in_strip_panel_does_not_scroll_viewport() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport));
        controller.state.mixer.strip_scroll = 8;
        let area = ratatui::layout::Rect::new(0, 0, 120, 50);

        handle_mouse_event(
            area,
            &mut controller,
            AppMouseEvent {
                kind: AppMouseEventKind::ScrollDown,
                column: 60,
                row: 18,
                modifiers: Default::default(),
            },
        )
        .expect("scroll strip panel");

        assert_eq!(controller.state.mixer.strip_scroll, 8);
    }

    #[test]
    fn mouse_hotkeys_toggle_flips_popup_state() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport));

        controller
            .apply_intent(
                ui::Intent::ToggleHotkeysPopup,
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open hotkeys");
        assert!(controller.state.popup.hotkeys_open);

        controller
            .apply_intent(
                ui::Intent::ToggleHotkeysPopup,
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("close hotkeys");
        assert!(!controller.state.popup.hotkeys_open);
    }

    #[test]
    fn mouse_sample_rate_selector_opens_and_pick_sends_exact_rate() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        controller.state.device.status.clock_source = Some(ClockSource::Internal);

        controller
            .apply_intent(
                ui::Intent::OpenSampleRateSelector,
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open sample rate selector");
        assert_eq!(
            controller.state.popup.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::SampleRate,
            })
        );

        controller
            .apply_intent(
                ui::Intent::PickSampleRate(SampleRate::Hz48000),
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("pick sample rate");
        assert_eq!(controller.state.popup.selector_popup, None);

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x12], &[0x03, 0x02]);
    }

    #[test]
    fn sample_rate_controls_are_disabled_when_clock_source_is_not_internal() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        controller.state.device.status.clock_source = Some(ClockSource::Usb);
        controller.state.device.status.sample_rate = Some(SampleRate::Hz192000);

        controller
            .apply_intent(
                ui::Intent::OpenSampleRateSelector,
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open sample rate selector");
        assert_eq!(controller.state.popup.selector_popup, None);

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Char('s')),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("cycle sample rate should no-op");

        assert_eq!(action, KeyAction::Continue);
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn mouse_preamp_mode_selector_pick_sends_exact_mode() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));

        controller
            .apply_intent(
                ui::Intent::OpenPreampModeSelector(1),
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open preamp mode selector");
        assert_eq!(
            controller.state.popup.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::PreampMode { input: 1 },
            })
        );

        controller
            .apply_intent(
                ui::Intent::PickPreampMode {
                    input: 1,
                    mode: PreampMode::HiZ,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("pick preamp mode");
        assert_eq!(controller.state.popup.selector_popup, None);

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x4f, 0x01, 0x02]);
    }

    #[test]
    fn popup_selection_wraps_with_keyboard_navigation() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport));
        controller.state.popup.assignment_picker = Some(AssignmentPickerState { strip: 1 });

        handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Up),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("popup up");
        assert_eq!(
            controller.state.popup.selected_index,
            MixerAssignment::grounded_choices().len() - 1
        );

        handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Down),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("popup down");
        assert_eq!(controller.state.popup.selected_index, 0);
    }

    #[test]
    fn profile_popup_selection_uses_saved_profile_list() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport));
        controller.state.popup.profiles_open = true;
        controller.state.popup.profile_names = vec!["tracking".to_string(), "mixdown".to_string()];

        handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Up),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("popup up");
        assert_eq!(controller.state.popup.selected_index, 1);

        handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Down),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("popup down");
        assert_eq!(controller.state.popup.selected_index, 0);
    }

    #[test]
    fn profile_editor_accepts_characters_and_backspace() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport));
        controller.state.popup.profile_editor = Some(ProfileEditorState {
            mode: ProfileEditorMode::Save,
            original_name: None,
            value: "mix".to_string(),
        });

        handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Char('1')),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("append profile name char");
        handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Backspace),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("backspace profile name char");

        assert_eq!(
            controller
                .state
                .popup
                .profile_editor
                .as_ref()
                .map(|editor| &editor.value),
            Some(&"mix".to_string())
        );
    }

    #[test]
    fn activating_popup_selection_submits_highlighted_assignment() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller);
        controller.state.popup.assignment_picker = Some(AssignmentPickerState { strip: 5 });
        controller.state.popup.selected_index = 13;

        activate_popup_selection(&mut controller).expect("activate popup selection");

        controller.flush_commands().expect("flush");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 5);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x03]);
        assert_eq!(&writes[0][0x10 + 0x0b..0x10 + 0x0d], &[0x09, 0x00]);
        assert_eq!(controller.state.popup.assignment_picker, None);
    }

    #[test]
    fn wait_for_transport_retries_until_device_appears() {
        let mut attempts = 0;
        let mut retries = 0;

        let _transport = cli::wait_for_transport(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(TransportError::DeviceUnavailable.into())
                } else {
                    Ok(Box::new(MockTransport::default()) as Box<dyn Transport>)
                }
            },
            |count, _| {
                retries = count;
                Ok(())
            },
        )
        .expect("transport should eventually open");

        assert_eq!(attempts, 3);
        assert_eq!(retries, 2);
    }

    #[test]
    fn cli_accepts_headless_flag() {
        let cli = Cli::try_parse_from(["zen-go-tui", "--headless"]).expect("parse cli");

        assert!(cli.headless);
        assert!(!cli.mock);
        assert!(cli.command.is_none());
    }

    #[test]
    fn cli_accepts_runtime_device_and_profile_pack_flags() {
        let cli = Cli::try_parse_from([
            "zen-go-tui",
            "--device",
            "23e5:a015",
            "--profile-pack",
            "/tmp/profiles.json",
        ])
        .expect("parse runtime selection flags");

        assert_eq!(cli.device.as_deref(), Some("23e5:a015"));
        assert_eq!(
            cli.profile_pack.as_deref(),
            Some(std::path::Path::new("/tmp/profiles.json"))
        );
    }

    #[test]
    fn handle_runtime_error_marks_controller_disconnected_for_device_errors() {
        let transport = MockTransport::default();
        let mut controller = test_controller(Box::new(transport));
        controller.state.device.connection.connected = true;

        handle_runtime_error(&mut controller, TransportError::DeviceDisconnected.into())
            .expect("device errors should be swallowed");

        assert!(!controller.state.device.connection.connected);
        assert_eq!(
            controller.state.ui.last_message,
            "Waiting for Zen Go device..."
        );
    }

    #[test]
    fn refresh_after_reconnect_runs_startup_query_sweep_once_device_returns() {
        let transport = AvailabilityTransport::default();
        let mut controller = test_controller(Box::new(transport.clone()));
        let mut pending = true;

        refresh_after_reconnect_if_needed(&mut controller, &mut pending)
            .expect("unavailable transport should not fail");
        assert!(pending);
        assert_eq!(transport.write_count(), 0);

        transport.set_available(true);
        refresh_after_reconnect_if_needed(&mut controller, &mut pending)
            .expect("available transport should refresh");

        assert!(!pending);
        assert_eq!(
            transport.write_count(),
            control_panel_startup_queries().len()
        );
    }

    #[test]
    fn draw_scheduler_throttles_dirty_redraws_but_refreshes_idle_ui() {
        let now = Instant::now();
        let fps = 30u8;

        assert!(should_draw_frame(None, false, now, fps));
        assert!(!should_draw_frame(
            Some(now - Duration::from_millis(10)),
            true,
            now,
            fps,
        ));
        assert!(!should_draw_frame(
            Some(now - Duration::from_millis(30)),
            true,
            now,
            fps,
        ));
        assert!(should_draw_frame(
            Some(now - Duration::from_millis(35)),
            true,
            now,
            fps,
        ));
        assert!(should_draw_frame(
            Some(now - Duration::from_millis(1200)),
            false,
            now,
            fps,
        ));
    }

    #[test]
    fn reconnect_probe_scheduler_backs_off_between_attempts() {
        let now = Instant::now();

        assert!(should_probe_reconnect(None, 0, now));
        assert!(!should_probe_reconnect(
            Some(now - Duration::from_millis(300)),
            0,
            now,
        ));
        assert!(should_probe_reconnect(
            Some(now - Duration::from_millis(600)),
            0,
            now,
        ));
        assert!(!should_probe_reconnect(
            Some(now - Duration::from_millis(1500)),
            1,
            now,
        ));
        assert!(should_probe_reconnect(
            Some(now - Duration::from_millis(2500)),
            1,
            now,
        ));
    }

    #[test]
    fn device_retry_interval_backs_off_after_first_wait() {
        assert_eq!(timing::device_retry_interval(1), Duration::from_millis(500));
        assert_eq!(timing::device_retry_interval(2), Duration::from_secs(2));
        assert_eq!(timing::device_retry_interval(8), Duration::from_secs(2));
    }

    #[test]
    fn device_poll_interval_stays_fast_after_recent_activity() {
        let now = Instant::now();

        assert_eq!(
            device_poll_interval(Some(now - Duration::from_millis(700)), true, now),
            Duration::from_millis(50)
        );
        assert_eq!(
            device_poll_interval(Some(now - Duration::from_millis(700)), false, now),
            Duration::from_millis(50)
        );
    }

    #[test]
    fn device_poll_interval_backs_off_when_idle() {
        let now = Instant::now();

        assert_eq!(
            device_poll_interval(Some(now - Duration::from_millis(1500)), true, now),
            Duration::from_millis(100)
        );
        assert_eq!(
            device_poll_interval(Some(now - Duration::from_millis(1500)), false, now),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn collect_pending_input_drains_channel_in_order() {
        use crate::input::InputThreadMessage;

        let (sender, receiver) = std::sync::mpsc::channel();
        sender
            .send(InputThreadMessage::Event(
                zen_go_tui::terminal::AppInputEvent::FocusGained,
            ))
            .expect("send focus gained");
        sender
            .send(InputThreadMessage::Event(
                zen_go_tui::terminal::AppInputEvent::FocusLost,
            ))
            .expect("send focus lost");

        let events = collect_pending_input(&receiver).expect("collect input");

        assert_eq!(
            events,
            vec![
                zen_go_tui::terminal::AppInputEvent::FocusGained,
                zen_go_tui::terminal::AppInputEvent::FocusLost
            ]
        );
    }

    #[test]
    fn collect_pending_input_surfaces_reader_error() {
        use crate::input::InputThreadMessage;

        let (sender, receiver) = std::sync::mpsc::channel();
        sender
            .send(InputThreadMessage::Error("broken input".to_string()))
            .expect("send error");

        let error = collect_pending_input(&receiver).expect_err("reader error should bubble up");

        assert!(error.to_string().contains("broken input"));
    }
}
