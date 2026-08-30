use anyhow::Result;
use ratatui::layout::Rect;
use slint::{ComponentHandle, Timer, TimerMode};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;
use zen_go_tui::app::Controller;
use zen_go_tui::transport::Transport;

use crate::cli::Cli;
use crate::commands::GuiCommand;
use crate::mapper::snapshot_from_app_state;
use crate::models::GuiPage;
use crate::models::GuiSnapshot;
use crate::ui_bridge::{apply_snapshot, AppWindow};

pub fn run(cli: Cli) -> Result<()> {
    let transport = crate::transport::open_transport(cli.mock)?;
    let app = AppWindow::new()?;
    app.window()
        .set_size(slint::LogicalSize::new(1280.0, 820.0));
    let initial_snapshot = GuiSnapshot::disconnected(GuiPage::Mixer);
    apply_snapshot(&app, &initial_snapshot);

    let worker = Rc::new(WorkerHandle::start(
        transport,
        GuiPage::Mixer,
        cli.no_bootstrap,
        cli.mock,
    )?);
    wire_callbacks(&app, Rc::clone(&worker));
    start_poll_timer(&app, worker);

    app.run()?;
    Ok(())
}

fn wire_callbacks(app: &AppWindow, worker: Rc<WorkerHandle>) {
    let page_worker = Rc::clone(&worker);
    app.on_page_requested(move |index| {
        if let Some(command) = GuiCommand::set_page_from_index(index) {
            page_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_output_level_adjusted(move |index, current, delta| {
        if let Some(command) =
            GuiCommand::adjust_output_level(index as usize, current as u8, delta as i16)
        {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_output_mute_requested(move |index| {
        if let Some(command) = GuiCommand::toggle_output_mute(index as usize) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_output_dim_requested(move |index| {
        if let Some(command) = GuiCommand::toggle_output_dim(index as usize) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_mixer_level_set(move |channel, level| {
        if let Some(command) = GuiCommand::set_mixer_level_by_channel(channel, level) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_mixer_pan_set(move |channel, raw| {
        if let Some(command) = GuiCommand::set_mixer_pan_by_channel(channel, raw) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_mixer_mute_requested(move |channel| {
        if let Some(command) = GuiCommand::toggle_mixer_mute(channel as u8) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_mixer_solo_requested(move |channel| {
        if let Some(command) = GuiCommand::toggle_mixer_solo(channel as u8) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_preamp_gain_adjusted(move |input, current, delta| {
        if let Some(command) =
            GuiCommand::adjust_preamp_gain(input as u8, current as u8, delta as i16)
        {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_preamp_mode_requested(move |input, mode_index| {
        if let Some(command) = GuiCommand::pick_preamp_mode_from_index(input as u8, mode_index) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_preamp_phase_requested(move |input| {
        if let Some(command) = GuiCommand::toggle_preamp_phase(input as u8) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_preamp_phantom_requested(move |input| {
        if let Some(command) = GuiCommand::toggle_preamp_phantom(input as u8) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_sample_rate_requested(move |index| {
        if let Some(command) = GuiCommand::pick_sample_rate_from_index(index as usize) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_clock_source_requested(move |index| {
        if let Some(command) = GuiCommand::pick_clock_source_from_index(index as usize) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_mixer_link_requested(move |channel| {
        if let Some(command) = GuiCommand::toggle_mixer_link_by_channel(channel) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_assignment_requested(move |channel, choice_index| {
        if let Some(command) = GuiCommand::pick_assignment_from_indices(channel, choice_index) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_profile_selected(move |index| {
        command_worker.send_command(GuiCommand::SelectProfile(index as usize));
    });

    let command_worker = Rc::clone(&worker);
    app.on_profile_load_requested(move || {
        command_worker.send_command(GuiCommand::LoadSelectedProfile);
    });

    let command_worker = Rc::clone(&worker);
    app.on_profile_save_requested(move || {
        command_worker.send_command(GuiCommand::SaveProfile(String::new()));
    });

    let command_worker = Rc::clone(&worker);
    app.on_profile_rename_requested(move || {
        command_worker.send_command(GuiCommand::RenameProfile(String::new()));
    });

    let command_worker = Rc::clone(&worker);
    app.on_profile_delete_requested(move || {
        command_worker.send_command(GuiCommand::DeleteSelectedProfile);
    });

    let command_worker = Rc::clone(&worker);
    app.on_raw_capture_baseline_requested(move || {
        command_worker.send_command(GuiCommand::CaptureRawBaseline);
    });

    let command_worker = Rc::clone(&worker);
    app.on_raw_clear_baseline_requested(move || {
        command_worker.send_command(GuiCommand::ClearRawBaseline);
    });

    let command_worker = Rc::clone(&worker);
    app.on_refresh_requested(move || {
        command_worker.send_command(GuiCommand::RefreshQueriedState);
    });

    let command_worker = Rc::clone(&worker);
    app.on_settings_refresh_rate_requested(move |index| {
        if let Some(command) = GuiCommand::set_refresh_rate_from_index(index as usize) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_settings_peak_threshold_adjusted(move |increase| {
        command_worker.send_command(GuiCommand::CyclePeakThreshold(increase));
    });

    let command_worker = Rc::clone(&worker);
    app.on_settings_peak_enabled_toggled(move || {
        command_worker.send_command(GuiCommand::TogglePeakEnabled);
    });

    let command_worker = Rc::clone(&worker);
    app.on_settings_peak_hold_requested(move |index| {
        if let Some(command) = GuiCommand::set_peak_hold_from_index(index as usize) {
            command_worker.send_command(command);
        }
    });

    let command_worker = Rc::clone(&worker);
    app.on_auto_save_toggled(move || {
        command_worker.send_command(GuiCommand::ToggleAutoSave);
    });
}

fn start_poll_timer(app: &AppWindow, worker: Rc<WorkerHandle>) {
    let app_weak = app.as_weak();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        if let Some(app) = app_weak.upgrade() {
            for event in worker.drain_events() {
                match event {
                    WorkerEvent::Snapshot(snapshot) => apply_snapshot(&app, &snapshot),
                    WorkerEvent::Error(error) => app.set_footer_notice(error.into()),
                }
            }
        }
    });
    std::mem::forget(timer);
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkerEvent {
    Snapshot(GuiSnapshot),
    Error(String),
}

pub struct WorkerHandle {
    command_tx: Sender<GuiCommand>,
    event_rx: Receiver<WorkerEvent>,
}

impl WorkerHandle {
    pub fn start(
        transport: Box<dyn Transport>,
        active_page: GuiPage,
        no_bootstrap: bool,
        mock_mode: bool,
    ) -> Result<Self> {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        thread::spawn(move || {
            run_worker_loop(
                transport,
                active_page,
                no_bootstrap,
                mock_mode,
                command_rx,
                event_tx,
            );
        });

        Ok(Self {
            command_tx,
            event_rx,
        })
    }

    pub fn send_command(&self, command: GuiCommand) -> bool {
        self.command_tx.send(command).is_ok()
    }

    pub fn drain_events(&self) -> Vec<WorkerEvent> {
        self.event_rx.try_iter().collect()
    }
}

fn run_worker_loop(
    transport: Box<dyn Transport>,
    active_page: GuiPage,
    no_bootstrap: bool,
    mock_mode: bool,
    command_rx: Receiver<GuiCommand>,
    event_tx: Sender<WorkerEvent>,
) {
    let mut worker = match WorkerCore::new(transport, active_page, no_bootstrap, mock_mode) {
        Ok(worker) => worker,
        Err(error) => {
            let _ = event_tx.send(WorkerEvent::Error(format!("Startup failed: {error}")));
            return;
        }
    };

    let _ = event_tx.send(WorkerEvent::Snapshot(worker.snapshot()));

    loop {
        match command_rx.recv_timeout(worker.poll_interval()) {
            Ok(GuiCommand::Shutdown) => break,
            Ok(command) => match worker.apply_command(command) {
                Ok(snapshot) => {
                    let _ = event_tx.send(WorkerEvent::Snapshot(snapshot));
                }
                Err(error) => {
                    let _ = event_tx.send(WorkerEvent::Error(format!("Command failed: {error}")));
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => match worker.poll_once(Duration::ZERO) {
                Ok(Some(snapshot)) => {
                    let _ = event_tx.send(WorkerEvent::Snapshot(snapshot));
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = event_tx.send(WorkerEvent::Error(format!("Poll failed: {error}")));
                }
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

pub struct WorkerCore {
    controller: Controller,
    active_page: GuiPage,
}

impl WorkerCore {
    pub fn new(
        transport: Box<dyn Transport>,
        active_page: GuiPage,
        no_bootstrap: bool,
        mock_mode: bool,
    ) -> Result<Self> {
        let mut controller = Controller::new(transport);
        if !no_bootstrap {
            controller.bootstrap()?;
        }
        if mock_mode {
            // MockTransport has no device frame to establish a connection, but its
            // in-memory writes make control interactions safe to exercise.
            controller.state.device.connection.connected = true;
        }
        Ok(Self {
            controller,
            active_page,
        })
    }

    pub fn apply_command(&mut self, command: GuiCommand) -> Result<GuiSnapshot> {
        if let GuiCommand::SetPage(page) = command {
            self.active_page = page;
            return Ok(self.snapshot());
        }

        if let Some(intent) = command.to_intent() {
            self.controller.apply_intent(intent, Rect::default())?;
        }

        Ok(self.snapshot())
    }

    pub fn poll_once(&mut self, timeout: Duration) -> Result<Option<GuiSnapshot>> {
        let dirty = self.controller.poll_device(timeout)?;
        Ok(dirty.then(|| self.snapshot()))
    }

    fn poll_interval(&self) -> Duration {
        let fps = u64::from(self.controller.state.ui.settings.refresh_rate.fps());
        Duration::from_millis(1_000 / fps)
    }

    pub fn snapshot(&self) -> GuiSnapshot {
        snapshot_from_app_state(&self.controller.state, self.active_page)
    }

    pub fn selected_mixer_channel(&self) -> u8 {
        (self.controller.state.mixer.selected_channel + 1).clamp(1, 16) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::WorkerCore;
    use crate::commands::GuiCommand;
    use crate::models::{GuiConnectionState, GuiPage};
    use std::thread;
    use std::time::Duration;
    use zen_go_tui::app::RefreshRate;
    use zen_go_tui::transport::MockTransport;

    #[test]
    fn empty_event_drain_does_not_block_the_ui_thread() {
        let source = include_str!("runtime.rs");
        let drain = source
            .split_once("pub fn drain_events")
            .expect("event drain function")
            .1
            .split_once("\n    }")
            .expect("event drain body")
            .0;

        assert!(drain.contains("self.event_rx.try_iter().collect()"));
        assert!(!drain.contains("recv_timeout"));
    }

    #[test]
    fn worker_does_not_emit_snapshots_for_clean_polls() {
        let transport = MockTransport::default();
        let handle =
            super::WorkerHandle::start(Box::new(transport), GuiPage::Mixer, true, false).unwrap();

        let mut initial_events = Vec::new();
        for _ in 0..20 {
            initial_events.extend(handle.drain_events());
            if initial_events
                .iter()
                .any(|event| matches!(event, super::WorkerEvent::Snapshot(_)))
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(initial_events
            .iter()
            .any(|event| matches!(event, super::WorkerEvent::Snapshot(_))));

        thread::sleep(Duration::from_millis(150));
        let events = handle.drain_events();
        assert!(events
            .iter()
            .all(|event| !matches!(event, super::WorkerEvent::Snapshot(_))));
    }

    #[test]
    fn mock_worker_starts_with_connected_controls() {
        let transport = MockTransport::default();
        let worker = WorkerCore::new(Box::new(transport), GuiPage::Mixer, true, true).unwrap();

        assert_eq!(
            worker.snapshot().header.connection,
            GuiConnectionState::Connected
        );
    }

    #[test]
    fn source_picker_uses_a_top_layer_popup_window() {
        let picker = include_str!("../ui/primitives/zen-source-picker.slint");
        let strip = include_str!("../ui/domain/mixer-strip.slint");

        assert!(picker.contains("popup := PopupWindow"));
        assert!(picker.contains("x: 0px;"));
        assert!(picker.contains("y: root.height + 2px;"));
        assert!(picker.contains("popup.show();"));
        assert!(picker.contains("popup.close();"));
        assert!(picker.contains("close-policy: close-on-click-outside;"));
        assert!(!picker.contains("popup.is-open()"));
        assert!(!strip.contains("source-picker.open"));
    }

    #[test]
    fn status_chip_dots_and_labels_are_centered_in_their_pills() {
        let source = include_str!("../ui/primitives/zen-status-chip.slint");

        assert!(source.contains("dot-slot := Rectangle"));
        assert!(source.contains("height: parent.height;"));
        assert!(source.contains("y: (parent.height - 7px) / 2;"));
        assert!(source.contains("vertical-alignment: center;"));
    }

    #[test]
    fn mixer_strips_have_no_active_selector_and_dim_when_muted() {
        let strip = include_str!("../ui/domain/mixer-strip.slint");
        let page = include_str!("../ui/pages/mixer.slint");

        assert!(strip.contains("opacity: root.strip.muted ? 0.58 : 1.0;"));
        assert!(!strip.contains("root.strip.selected"));
        assert!(!strip.contains("callback selected"));
        assert!(!page.contains("selected(channel)"));
    }

    #[test]
    fn toggle_labels_stay_fixed_when_active() {
        let mixer = include_str!("../ui/domain/mixer-strip.slint");
        let outputs = include_str!("../ui/domain/output-card.slint");
        let preamps = include_str!("../ui/domain/preamp-card.slint");

        assert!(mixer.contains("label: \"S\";"));
        assert!(mixer.contains("label: \"M\";"));
        assert!(!mixer.contains("root.strip.soloed ? \"SOLO\""));
        assert!(!mixer.contains("root.strip.muted ? \"MUTE\""));
        assert!(outputs.contains("label: \"MUTE\";"));
        assert!(outputs.contains("label: \"DIM\";"));
        assert!(preamps.contains("label: \"48V\";"));
        assert!(preamps.contains("label: \"PHASE\";"));
    }

    #[test]
    fn mixer_fader_and_pan_double_click_to_neutral_values() {
        let fader = include_str!("../ui/primitives/zen-level-fader.slint");
        let pan = include_str!("../ui/primitives/zen-pan-slider.slint");

        assert!(fader.contains("double-clicked =>"));
        assert!(fader.contains("root.value-released(0);"));
        assert!(pan.contains("double-clicked =>"));
        assert!(pan.contains("root.value-released(32);"));
    }

    #[test]
    fn settings_are_compact_and_interactive() {
        let settings = include_str!("../ui/pages/settings.slint");

        assert!(settings.contains("width: min(parent.width - 48px, 760px);"));
        assert!(settings.contains("refresh-rate-requested"));
        assert!(settings.contains("peak-threshold-adjusted"));
        assert!(settings.contains("peak-enabled-toggled"));
        assert!(settings.contains("peak-hold-requested"));
        assert!(settings.contains("label: \"PEAK\";"));
        assert!(settings.contains("label: \"AUTO-SAVE\";"));
    }

    #[test]
    fn header_content_is_vertically_centered() {
        let source = include_str!("../ui/main.slint");
        let header = source
            .split_once("height: 64px;")
            .expect("header rectangle")
            .1
            .split_once("ZenTabBar")
            .expect("header content")
            .0;

        assert!(header.contains("y: (parent.height - ZenTheme.control-height) / 2;"));
        assert!(header.contains("height: ZenTheme.control-height;"));
        assert!(header.contains("vertical-alignment: center;"));
    }

    #[test]
    fn mixer_footer_centers_only_visible_controls() {
        let source = include_str!("../ui/domain/mixer-strip.slint");
        let footer = source
            .rsplit_once("HorizontalLayout {")
            .expect("mixer footer layout")
            .1;

        assert!(footer.contains("alignment: center;"));
        assert!(footer.contains("if root.strip.linkable"));
        assert!(!footer.contains("horizontal-stretch: 1;"));
    }

    #[test]
    fn worker_poll_interval_follows_refresh_rate_setting() {
        let transport = MockTransport::default();
        let mut worker =
            WorkerCore::new(Box::new(transport), GuiPage::Settings, true, true).unwrap();

        assert_eq!(worker.poll_interval(), Duration::from_millis(33));
        worker
            .apply_command(GuiCommand::SetRefreshRate(RefreshRate::Fps15))
            .unwrap();
        assert_eq!(worker.poll_interval(), Duration::from_millis(66));
        worker
            .apply_command(GuiCommand::SetRefreshRate(RefreshRate::Fps60))
            .unwrap();
        assert_eq!(worker.poll_interval(), Duration::from_millis(16));
    }

    #[test]
    fn ui_event_drain_services_the_sixty_fps_ceiling() {
        let source = include_str!("runtime.rs");
        let timer = source
            .split_once("fn start_poll_timer")
            .expect("poll timer")
            .1
            .split_once("#[derive(Debug, Clone, PartialEq)]")
            .expect("poll timer body")
            .0;

        assert!(timer.contains("Duration::from_millis(16)"));
    }

    #[test]
    fn worker_applies_command_and_returns_updated_snapshot() {
        let transport = MockTransport::default();
        let mut worker =
            WorkerCore::new(Box::new(transport), GuiPage::Mixer, false, false).unwrap();

        let snapshot = worker
            .apply_command(GuiCommand::SetOutputLevel { index: 1, step: 24 })
            .unwrap();

        assert_eq!(snapshot.outputs[1].level_step, 24);
        assert_eq!(snapshot.outputs[1].level_db, -24);
    }

    #[test]
    fn worker_keeps_page_commands_in_gui_layer() {
        let transport = MockTransport::default();
        let mut worker = WorkerCore::new(Box::new(transport), GuiPage::Mixer, true, false).unwrap();

        let snapshot = worker
            .apply_command(GuiCommand::SetPage(GuiPage::Raw))
            .unwrap();

        assert_eq!(snapshot.header.active_page, GuiPage::Raw);
    }

    #[test]
    fn worker_reports_selected_mixer_channel_as_one_based() {
        let transport = MockTransport::default();
        let mut worker = WorkerCore::new(Box::new(transport), GuiPage::Mixer, true, false).unwrap();

        worker
            .apply_command(GuiCommand::SelectMixerChannel(3))
            .unwrap();

        assert_eq!(worker.selected_mixer_channel(), 4);
    }

    #[test]
    fn worker_handle_queues_commands_and_drains_snapshots() {
        let transport = MockTransport::default();
        let handle =
            super::WorkerHandle::start(Box::new(transport), GuiPage::Mixer, true, false).unwrap();

        assert!(handle.send_command(GuiCommand::SetPage(GuiPage::Raw)));

        let mut events = Vec::new();
        for _ in 0..10 {
            events.extend(handle.drain_events());
            if events.iter().any(|event| {
                matches!(event, super::WorkerEvent::Snapshot(snapshot) if snapshot.header.active_page == GuiPage::Raw)
            }) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(events
            .iter()
            .any(|event| matches!(event, super::WorkerEvent::Snapshot(snapshot) if snapshot.header.active_page == GuiPage::Raw)));
    }
}
