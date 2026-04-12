mod cli;
mod input;
mod profile_ops;
mod timing;

use std::io;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use antelope_protocol::{
    ClockSource, Command, MixerAssignment, MixerSurface, PanState, PreampMode, SampleRate, Surface,
};
use zen_go_tui::app::{
    Controller, FocusArea, Intent, MainPage, PeakHoldDuration, RefreshRate, SelectorPopupKind,
    SelectorPopupState,
};
use zen_go_tui::settings;
use zen_go_tui::terminal::{
    AppKeyCode, AppKeyEvent, AppKeyEventKind, AppMouseButton, AppMouseEvent, AppMouseEventKind,
};
use zen_go_tui::transport::{is_device_error, Transport};
use zen_go_tui::ui;

use crate::cli::{Cli, CliCommand};
use crate::input::{collect_pending_input, spawn_input_reader, InputThreadMessage};
use crate::profile_ops::{append_profile_editor_text, load_selected_profile, run_profile_command};
use crate::timing::{device_poll_interval, should_draw_frame, should_probe_reconnect};

fn main() -> Result<()> {
    let cli = Cli::parse();
    let transport = cli::open_transport(cli.mock)?;

    match cli.command {
        Some(CliCommand::Profile { command }) => run_profile_command(transport, command),
        None if cli.headless => run_headless_app(transport),
        None => run_app(transport),
    }
}

fn handle_runtime_error(controller: &mut Controller, error: anyhow::Error) -> Result<()> {
    if is_device_error(&error) {
        controller.state.mark_disconnected();
        controller.state.last_message = "Waiting for Zen Go device...".to_string();
        return Ok(());
    }

    Err(error)
}

fn refresh_after_reconnect_if_needed(
    controller: &mut Controller,
    reconnect_refresh_pending: &mut bool,
) -> Result<()> {
    if !*reconnect_refresh_pending {
        return Ok(());
    }

    if !controller.transport_available()? {
        return Ok(());
    }

    match controller.refresh_queried_state() {
        Ok(()) => {
            controller.state.last_message = "Zen Go reconnected, refreshing state...".to_string();
            *reconnect_refresh_pending = false;
            Ok(())
        }
        Err(error) if is_device_error(&error) => {
            handle_runtime_error(controller, error)?;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn run_app(transport: Box<dyn Transport>) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    let input_rx = spawn_input_reader();
    let mut controller = Controller::new(transport);
    if let Ok(saved) = settings::load_settings() {
        controller.state.settings = saved;
    }
    controller.bootstrap()?;
    let result = app_loop(&mut terminal, &mut controller, &input_rx);
    let _ = settings::save_settings(&controller.state.settings);
    disable_raw_mode()?;
    terminal.show_cursor()?;
    io::stdout().execute(DisableMouseCapture)?;
    io::stdout().execute(LeaveAlternateScreen)?;
    result
}

fn run_headless_app(transport: Box<dyn Transport>) -> Result<()> {
    eprintln!("Headless mode active. Press Ctrl+C to stop.");
    let mut controller = Controller::new(transport);
    controller.bootstrap()?;
    headless_loop(&mut controller)
}

fn headless_loop(controller: &mut Controller) -> Result<()> {
    let mut reconnect_refresh_pending = false;
    let mut last_reconnect_probe_at = None;
    let mut reconnect_probe_attempts = 0;
    let mut last_runtime_activity_at = Some(std::time::Instant::now());

    loop {
        let now = std::time::Instant::now();
        if reconnect_refresh_pending {
            if should_probe_reconnect(last_reconnect_probe_at, reconnect_probe_attempts, now) {
                refresh_after_reconnect_if_needed(controller, &mut reconnect_refresh_pending)?;
                last_reconnect_probe_at = Some(now);
                reconnect_probe_attempts += 1;
                if !reconnect_refresh_pending {
                    last_reconnect_probe_at = None;
                    reconnect_probe_attempts = 0;
                }
            } else if let Some(last_probe_at) = last_reconnect_probe_at {
                let wait =
                    timing::device_retry_interval(reconnect_probe_attempts.saturating_add(1))
                        .saturating_sub(now.duration_since(last_probe_at));
                std::thread::sleep(wait);
                continue;
            }
        }

        match controller.poll_device(device_poll_interval(last_runtime_activity_at, false, now)) {
            Ok(observed_frame) => {
                if observed_frame {
                    last_runtime_activity_at = Some(std::time::Instant::now());
                }
            }
            Err(error) => {
                if is_device_error(&error) {
                    reconnect_refresh_pending = true;
                    last_reconnect_probe_at = None;
                    reconnect_probe_attempts = 0;
                }
                handle_runtime_error(controller, error)?;
            }
        }

        std::thread::sleep(timing::MIN_LOOP_SLEEP);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    Continue,
    ReconnectPending,
    Quit,
}

fn handle_key_press(
    controller: &mut Controller,
    key: AppKeyEvent,
    area: ratatui::layout::Rect,
) -> Result<KeyAction> {
    let key_code = key.code;
    let ctrl = key.modifiers.ctrl;

    if ctrl {
        match key_code {
            AppKeyCode::Char('c') => return Ok(KeyAction::Quit),
            AppKeyCode::Char('d') => {
                controller.apply_intent(Intent::ToggleRawView, area)?;
                return Ok(KeyAction::Continue);
            }
            AppKeyCode::Char('o') => {
                controller.apply_intent(Intent::ToggleOptionsPopup, area)?;
                return Ok(KeyAction::Continue);
            }
            _ => {}
        }
    }

    if controller.state.hotkeys_popup_open {
        match key_code {
            AppKeyCode::Char('q') => return Ok(KeyAction::Quit),
            AppKeyCode::Char('?') | AppKeyCode::Esc => {
                controller.apply_intent(Intent::ToggleHotkeysPopup, area)?;
            }
            _ => {}
        }
        return Ok(KeyAction::Continue);
    }

    if controller.state.options_popup_open {
        match key_code {
            AppKeyCode::Char('q') => return Ok(KeyAction::Quit),
            AppKeyCode::Esc => {
                controller.apply_intent(Intent::CloseOptionsPopup, area)?;
            }
            AppKeyCode::Char('1') => {
                controller.apply_intent(Intent::SetRefreshRate(RefreshRate::Fps15), area)?;
            }
            AppKeyCode::Char('2') => {
                controller.apply_intent(Intent::SetRefreshRate(RefreshRate::Fps30), area)?;
            }
            AppKeyCode::Char('3') => {
                controller.apply_intent(Intent::SetRefreshRate(RefreshRate::Fps60), area)?;
            }
            AppKeyCode::Up => {
                controller.apply_intent(Intent::CyclePeakThreshold(true), area)?;
            }
            AppKeyCode::Down => {
                controller.apply_intent(Intent::CyclePeakThreshold(false), area)?;
            }
            AppKeyCode::Char('p') => {
                controller.apply_intent(Intent::TogglePeakEnabled, area)?;
            }
            AppKeyCode::Char('h') | AppKeyCode::Char('H') => {
                let all = PeakHoldDuration::all();
                let current = controller.state.settings.peak_hold_duration;
                let pos = all.iter().position(|&v| v == current).unwrap_or(1);
                let next = all[(pos + 1) % all.len()];
                controller.apply_intent(Intent::CyclePeakHoldDuration(next), area)?;
                if controller.state.settings.auto_save {
                    let _ = settings::save_settings(&controller.state.settings);
                }
            }
            AppKeyCode::Char('l') | AppKeyCode::Char('L') => {
                let all = PeakHoldDuration::all();
                let current = controller.state.settings.peak_hold_duration;
                let pos = all.iter().position(|&v| v == current).unwrap_or(1);
                let next = all[pos.checked_sub(1).unwrap_or(all.len() - 1)];
                controller.apply_intent(Intent::CyclePeakHoldDuration(next), area)?;
                if controller.state.settings.auto_save {
                    let _ = settings::save_settings(&controller.state.settings);
                }
            }
            AppKeyCode::Char('a') => {
                controller.apply_intent(Intent::ToggleAutoSave, area)?;
            }
            _ => {}
        }
        return Ok(KeyAction::Continue);
    }

    if controller.state.profile_editor.is_some() {
        match key_code {
            AppKeyCode::Char(ch) => {
                let valid: String = ch
                    .to_string()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                if !valid.is_empty() {
                    controller.apply_intent(Intent::ProfileEditorChar(valid), area)?;
                }
            }
            AppKeyCode::Backspace => {
                controller.apply_intent(Intent::ProfileEditorBackspace, area)?;
            }
            AppKeyCode::Enter => {
                controller.apply_intent(Intent::ProfileEditorCommit, area)?;
            }
            AppKeyCode::Esc => {
                controller.apply_intent(Intent::ProfileEditorCancel, area)?;
            }
            _ => {}
        }
        return Ok(KeyAction::Continue);
    }

    if controller.state.profiles_popup_open {
        match key_code {
            AppKeyCode::Up => {
                controller.apply_intent(Intent::MovePopupSelection(false), area)?;
            }
            AppKeyCode::Down => {
                controller.apply_intent(Intent::MovePopupSelection(true), area)?;
            }
            AppKeyCode::Enter => {
                controller.apply_intent(Intent::LoadSelectedProfile, area)?;
            }
            AppKeyCode::Char('s') => {
                controller.apply_intent(Intent::StartSaveProfile, area)?;
            }
            AppKeyCode::Char('r') => {
                if controller.state.selected_profile_name().is_some() {
                    controller.apply_intent(Intent::StartRenameProfile, area)?;
                } else {
                    controller.state.last_message = "No profile selected to rename.".to_string();
                }
            }
            AppKeyCode::Char('d') => {
                controller.apply_intent(Intent::DeleteSelectedProfile, area)?;
            }
            AppKeyCode::Esc => {
                controller.apply_intent(Intent::CloseProfilesPopup, area)?;
            }
            _ => {}
        }
        return Ok(KeyAction::Continue);
    }

    let result = match key_code {
        AppKeyCode::Char('q') => return Ok(KeyAction::Quit),
        AppKeyCode::Char('r') => {
            controller.apply_intent(Intent::ToggleRoutingPopup, area)?;
            Ok(())
        }
        AppKeyCode::Char('p') => {
            if controller.state.profiles_popup_open {
                controller.apply_intent(Intent::CloseProfilesPopup, area)?;
            } else {
                controller.apply_intent(Intent::OpenProfilesPopup, area)?;
            }
            Ok(())
        }
        AppKeyCode::Char('O') => {
            controller.apply_intent(Intent::ToggleOptionsPopup, area)?;
            Ok(())
        }
        AppKeyCode::Char('R') => {
            controller.apply_intent(Intent::RefreshQueriedState, area)?;
            Ok(())
        }
        AppKeyCode::Tab => {
            controller.apply_intent(Intent::CycleFocus, area)?;
            Ok(())
        }
        AppKeyCode::BackTab => Ok(()),
        AppKeyCode::Char('?') => {
            controller.state.toggle_hotkeys_popup();
            Ok(())
        }
        AppKeyCode::Up
            if controller.state.assignment_picker.is_some()
                || controller.state.selector_popup.is_some() =>
        {
            controller.apply_intent(Intent::MovePopupSelection(false), area)?;
            Ok(())
        }
        AppKeyCode::Down
            if controller.state.assignment_picker.is_some()
                || controller.state.selector_popup.is_some() =>
        {
            controller.apply_intent(Intent::MovePopupSelection(true), area)?;
            Ok(())
        }
        AppKeyCode::Enter
            if controller.state.assignment_picker.is_some()
                || controller.state.selector_popup.is_some() =>
        {
            activate_popup_selection(controller)
        }
        AppKeyCode::Left if controller.state.raw_view_open => {
            if controller.state.selected_raw_packet == zen_go_tui::app::RawPacketTab::Query75 {
                controller.apply_intent(Intent::ScrollQueryReplyList { increase: false }, area)?;
            } else {
                controller.state.cycle_raw_packet(false);
            }
            Ok(())
        }
        AppKeyCode::Right if controller.state.raw_view_open => {
            if controller.state.selected_raw_packet == zen_go_tui::app::RawPacketTab::Query75 {
                controller.apply_intent(Intent::ScrollQueryReplyList { increase: true }, area)?;
            } else {
                controller.state.cycle_raw_packet(true);
            }
            Ok(())
        }
        AppKeyCode::Left => {
            move_selection(controller, false, area);
            Ok(())
        }
        AppKeyCode::Right => {
            move_selection(controller, true, area);
            Ok(())
        }
        AppKeyCode::Up => {
            controller.apply_intent(Intent::AdjustFocused(true), area)?;
            Ok(())
        }
        AppKeyCode::Down => {
            controller.apply_intent(Intent::AdjustFocused(false), area)?;
            Ok(())
        }
        AppKeyCode::Char('m') => {
            controller.apply_intent(Intent::ToggleFocusedMute, area)?;
            Ok(())
        }
        AppKeyCode::Char('d') => {
            controller.apply_intent(Intent::ToggleFocusedDim, area)?;
            Ok(())
        }
        AppKeyCode::Char('o') => {
            if controller.state.page == MainPage::Mixer
                && controller.state.focus == FocusArea::Mixer
            {
                let active_channel =
                    controller.state.active_mixer_channels()[controller.state.selected_channel];
                let mixer = MixerSurface::from_surface(controller.state.surface);
                controller.send_mixer_solo_change(
                    mixer,
                    active_channel.channel,
                    !active_channel.soloed.unwrap_or(false),
                )?;
            }
            Ok(())
        }
        AppKeyCode::Char('a') => {
            if controller.state.page == MainPage::Mixer
                && controller.state.focus == FocusArea::Mixer
            {
                let active_channel =
                    controller.state.active_mixer_channels()[controller.state.selected_channel];
                if !antelope_protocol::MixerStrip::assignment_write_is_grounded(
                    active_channel.channel,
                ) {
                    controller.state.last_message =
                        "Assignment picking is not grounded for the selected strip.".to_string();
                } else {
                    controller
                        .apply_intent(Intent::OpenAssignmentPicker(active_channel.channel), area)?;
                }
            }
            Ok(())
        }
        AppKeyCode::Char('l') => {
            if controller.state.page == MainPage::Mixer
                && controller.state.focus == FocusArea::Mixer
            {
                let active_channel =
                    controller.state.active_mixer_channels()[controller.state.selected_channel];
                let mixer = MixerSurface::from_surface(controller.state.surface);
                controller.send_mixer_link_change(
                    mixer,
                    active_channel.channel,
                    !active_channel.linked.unwrap_or(false),
                )?;
            }
            Ok(())
        }
        AppKeyCode::Char('[') => {
            if controller.state.page == MainPage::Mixer
                && controller.state.focus == FocusArea::Mixer
            {
                let active_channel =
                    controller.state.active_mixer_channels()[controller.state.selected_channel];
                let next = active_channel
                    .pan
                    .raw()
                    .saturating_sub(1)
                    .max(PanState::MIN);
                controller.send(Command::SetMixerPan {
                    mixer: MixerSurface::from_surface(controller.state.surface),
                    channel: active_channel.channel,
                    pan: PanState::from_raw(next),
                    muted: active_channel.muted.unwrap_or(false),
                    soloed: active_channel.soloed.unwrap_or(false),
                })?;
            }
            Ok(())
        }
        AppKeyCode::Char(']') => {
            if controller.state.page == MainPage::Mixer
                && controller.state.focus == FocusArea::Mixer
            {
                let active_channel =
                    controller.state.active_mixer_channels()[controller.state.selected_channel];
                let next = active_channel
                    .pan
                    .raw()
                    .saturating_add(1)
                    .min(PanState::MAX);
                controller.send(Command::SetMixerPan {
                    mixer: MixerSurface::from_surface(controller.state.surface),
                    channel: active_channel.channel,
                    pan: PanState::from_raw(next),
                    muted: active_channel.muted.unwrap_or(false),
                    soloed: active_channel.soloed.unwrap_or(false),
                })?;
            }
            Ok(())
        }
        AppKeyCode::Char('3') => {
            if controller.state.page == MainPage::Mixer
                && controller.state.focus == FocusArea::Preamp
            {
                let input = controller.state.selected_preamp_input as u8;
                let current = if input == 0 {
                    controller.state.preamp.input1.mode
                } else {
                    controller.state.preamp.input2.mode
                };
                controller.state.popup_selected_index =
                    [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                        .iter()
                        .position(|mode| *mode == current)
                        .unwrap_or(0);
                controller.state.selector_popup = Some(SelectorPopupState {
                    kind: SelectorPopupKind::PreampMode { input },
                });
                controller.state.focus = FocusArea::Preamp;
                controller.state.selected_preamp_input = input.min(1) as usize;
            }
            Ok(())
        }
        AppKeyCode::Char('s') => {
            if controller.state.device.clock_source == Some(ClockSource::Internal) {
                let current = controller
                    .state
                    .device
                    .sample_rate
                    .unwrap_or(SampleRate::Hz48000);
                let all = SampleRate::all_confirmed();
                let position = all.iter().position(|rate| *rate == current).unwrap_or(2);
                let next = all[(position + 1) % all.len()];
                controller.send(Command::SetSampleRate(next))?;
            }
            Ok(())
        }
        AppKeyCode::Char('c') => {
            let current = controller
                .state
                .device
                .clock_source
                .unwrap_or(ClockSource::Internal);
            let all = ClockSource::all_confirmed();
            let position = all
                .iter()
                .position(|source| *source == current)
                .unwrap_or(0);
            let next = all[(position + 1) % all.len()];
            controller.send(Command::SetClockSource(next))?;
            Ok(())
        }
        AppKeyCode::Char('1') => controller.send(Command::SelectSurface(Surface::MonitorHp1)),
        AppKeyCode::Char('2') => controller.send(Command::SelectSurface(Surface::Hp2)),
        AppKeyCode::Char('b') if controller.state.raw_view_open => {
            controller.apply_intent(Intent::CaptureRawBaseline, area)?;
            Ok(())
        }
        AppKeyCode::Char('x') if controller.state.raw_view_open => {
            controller.apply_intent(Intent::ClearRawBaseline, area)?;
            Ok(())
        }
        AppKeyCode::Esc
            if controller.state.assignment_picker.is_some()
                || controller.state.selector_popup.is_some()
                || controller.state.routing_popup_open
                || controller.state.hotkeys_popup_open
                || controller.state.options_popup_open =>
        {
            controller.state.assignment_picker = None;
            controller.state.selector_popup = None;
            controller.state.routing_popup_open = false;
            controller.state.popup_selected_index = 0;
            controller.state.hotkeys_popup_open = false;
            controller.state.options_popup_open = false;
            controller.state.last_message = "Closed popup".to_string();
            Ok(())
        }
        _ => Ok(()),
    };

    match result {
        Ok(()) => Ok(KeyAction::Continue),
        Err(error) if is_device_error(&error) => {
            handle_runtime_error(controller, error)?;
            Ok(KeyAction::ReconnectPending)
        }
        Err(error) => Err(error),
    }
}

fn app_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    controller: &mut Controller,
    input_rx: &std::sync::mpsc::Receiver<InputThreadMessage>,
) -> Result<()> {
    let mut reconnect_refresh_pending = false;
    let mut last_reconnect_probe_at = None;
    let mut reconnect_probe_attempts = 0;
    let mut last_draw_at = None;
    let mut needs_redraw = true;
    let mut last_runtime_activity_at = Some(std::time::Instant::now());

    'app: loop {
        let now = std::time::Instant::now();
        if reconnect_refresh_pending
            && should_probe_reconnect(last_reconnect_probe_at, reconnect_probe_attempts, now)
        {
            refresh_after_reconnect_if_needed(controller, &mut reconnect_refresh_pending)?;
            last_reconnect_probe_at = Some(now);
            reconnect_probe_attempts += 1;
            if !reconnect_refresh_pending {
                last_reconnect_probe_at = None;
                reconnect_probe_attempts = 0;
            }
        }

        let input_events = collect_pending_input(input_rx)?;
        if !input_events.is_empty() {
            last_runtime_activity_at = Some(std::time::Instant::now());

            for event in input_events {
                match event {
                    zen_go_tui::terminal::AppInputEvent::Key(key) => {
                        if key.kind != AppKeyEventKind::Press {
                            continue;
                        }
                        let size = terminal.size()?;
                        let action = handle_key_press(
                            controller,
                            key,
                            ratatui::layout::Rect::new(0, 0, size.width, size.height),
                        )?;

                        if action == KeyAction::Quit {
                            break 'app;
                        }

                        if action == KeyAction::ReconnectPending {
                            reconnect_refresh_pending = true;
                        }

                        needs_redraw = true;
                    }
                    zen_go_tui::terminal::AppInputEvent::Mouse(mouse) => {
                        let size = terminal.size()?;
                        if let Err(error) = handle_mouse_event(
                            ratatui::layout::Rect::new(0, 0, size.width, size.height),
                            controller,
                            mouse,
                        ) {
                            if is_device_error(&error) {
                                reconnect_refresh_pending = true;
                            }
                            handle_runtime_error(controller, error)?;
                        }
                        if controller.state.quit_requested {
                            break 'app;
                        }
                        needs_redraw = true;
                    }
                    zen_go_tui::terminal::AppInputEvent::Paste(text) => {
                        if controller.state.profile_editor.is_some() {
                            append_profile_editor_text(controller, &text);
                        }
                        needs_redraw = true;
                    }
                    zen_go_tui::terminal::AppInputEvent::Resize { .. }
                    | zen_go_tui::terminal::AppInputEvent::FocusGained
                    | zen_go_tui::terminal::AppInputEvent::FocusLost => needs_redraw = true,
                }
            }
        }

        if !reconnect_refresh_pending {
            match controller.poll_device(device_poll_interval(last_runtime_activity_at, true, now))
            {
                Ok(observed_frame) => {
                    needs_redraw |= observed_frame;
                    if observed_frame {
                        last_runtime_activity_at = Some(std::time::Instant::now());
                    }
                }
                Err(error) => {
                    if is_device_error(&error) {
                        reconnect_refresh_pending = true;
                        last_reconnect_probe_at = None;
                        reconnect_probe_attempts = 0;
                    }
                    handle_runtime_error(controller, error)?;
                    needs_redraw = true;
                }
            }
        }

        let now = std::time::Instant::now();
        controller.state.prune_expired_peaks();
        let fps = controller.state.settings.refresh_rate.fps();
        if should_draw_frame(last_draw_at, needs_redraw, now, fps) {
            terminal.draw(|frame| {
                ui::draw(frame, &controller.state);
                if let Some((x, y)) = ui::profile_editor_cursor(frame.area(), &controller.state) {
                    frame.set_cursor_position((x, y));
                }
            })?;
            if controller.state.profile_editor.is_some() {
                terminal.show_cursor()?;
            } else {
                terminal.hide_cursor()?;
            }
            last_draw_at = Some(now);
            needs_redraw = false;
        }

        std::thread::sleep(timing::loop_sleep_for_fps(fps));
    }

    Ok(())
}

fn move_selection(controller: &mut Controller, right: bool, area: ratatui::layout::Rect) {
    if controller.state.page != MainPage::Mixer {
        return;
    }

    match controller.state.focus {
        FocusArea::Outputs => {
            controller.state.selected_output = if right {
                (controller.state.selected_output + 1) % controller.state.outputs.len()
            } else {
                controller
                    .state
                    .selected_output
                    .checked_sub(1)
                    .unwrap_or(controller.state.outputs.len() - 1)
            };
        }
        FocusArea::Mixer => {
            let channels_len = controller.state.active_mixer_channels().len();
            controller.state.selected_channel = if right {
                (controller.state.selected_channel + 1) % channels_len
            } else {
                controller
                    .state
                    .selected_channel
                    .checked_sub(1)
                    .unwrap_or(channels_len - 1)
            };
            let visible = ui::mixer_strip_viewport_capacity(area, &controller.state);
            controller
                .state
                .ensure_selected_mixer_channel_visible(visible);
        }
        FocusArea::Preamp => {
            controller.state.selected_preamp_input = if right { 1 } else { 0 };
        }
        _ => {}
    }
}

fn activate_popup_selection(controller: &mut Controller) -> Result<()> {
    if let Some(picker) = controller.state.assignment_picker {
        if let Some(assignment) = MixerAssignment::grounded_choices()
            .get(controller.state.popup_selected_index)
            .copied()
        {
            return controller.apply_intent(
                ui::Intent::PickAssignment {
                    strip: picker.strip,
                    assignment,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            );
        }
    }

    if controller.state.profiles_popup_open {
        return load_selected_profile(controller);
    }

    if let Some(popup) = controller.state.selector_popup {
        let action = match popup.kind {
            SelectorPopupKind::SampleRate => SampleRate::all_confirmed()
                .get(controller.state.popup_selected_index)
                .copied()
                .map(ui::Intent::PickSampleRate),
            SelectorPopupKind::ClockSource => ClockSource::all_confirmed()
                .get(controller.state.popup_selected_index)
                .copied()
                .map(ui::Intent::PickClockSource),
            SelectorPopupKind::PreampMode { input } => {
                [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                    .get(controller.state.popup_selected_index)
                    .copied()
                    .map(|mode| ui::Intent::PickPreampMode { input, mode })
            }
        };

        if let Some(action) = action {
            return controller.apply_intent(action, ratatui::layout::Rect::new(0, 0, 160, 50));
        }
    }

    Ok(())
}

fn handle_mouse_event(
    area: ratatui::layout::Rect,
    controller: &mut Controller,
    mouse: AppMouseEvent,
) -> Result<()> {
    match mouse.kind {
        AppMouseEventKind::Down(AppMouseButton::Left) => {
            if let Some(action) = ui::mouse_action(area, &controller.state, mouse.column, mouse.row)
            {
                controller.apply_intent(action, area)?;
            }
        }
        AppMouseEventKind::Drag(AppMouseButton::Left) => {
            if let Some(action) =
                ui::slider_mouse_action(area, &controller.state, mouse.column, mouse.row)
            {
                controller.apply_intent(action, area)?;
            }
        }
        AppMouseEventKind::ScrollLeft
        | AppMouseEventKind::ScrollRight
        | AppMouseEventKind::ScrollUp
        | AppMouseEventKind::ScrollDown => {
            let increase = matches!(
                mouse.kind,
                AppMouseEventKind::ScrollUp | AppMouseEventKind::ScrollRight
            );
            if let Some(action) =
                ui::slider_wheel_action(area, &controller.state, mouse.column, mouse.row, increase)
            {
                controller.apply_intent(action, area)?;
                return Ok(());
            }
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use antelope_protocol::{
        control_panel_startup_queries, MixerAssignment, MixerSurface, OutputMode, OutputState,
        OutputTarget,
    };
    use zen_go_tui::app::{AssignmentPickerState, ProfileEditorMode, ProfileEditorState};
    use zen_go_tui::terminal::AppModifiers;
    use zen_go_tui::transport::MockTransport;
    use zen_go_tui::transport::TransportError;

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

        for surface in &mut controller.state.mixer_channels {
            for (channel, assignment) in surface.iter_mut().zip(assignments) {
                channel.assignment = Some(assignment);
            }
        }
    }

    #[test]
    fn opening_assignment_picker_from_keyboard_does_not_send_assignment_change() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller);
        controller.state.focus = FocusArea::Mixer;
        controller.state.selected_channel = 0;
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].assignment =
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
            controller.state.assignment_picker,
            Some(AssignmentPickerState { strip: 1 })
        );
    }

    #[test]
    fn opening_assignment_picker_from_routing_popup_uses_selected_routing_channel() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller);
        controller.state.routing_popup_open = true;
        controller.state.focus = FocusArea::Mixer;
        controller.state.selected_channel = 5;

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Char('a')),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("open assignment picker from routing popup");

        assert_eq!(action, KeyAction::Continue);
        assert!(transport.take_writes().is_empty());
        assert_eq!(
            controller.state.assignment_picker,
            Some(AssignmentPickerState { strip: 6 })
        );
    }

    #[test]
    fn opening_preamp_mode_selector_from_keyboard_does_not_send_mode_change() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.focus = FocusArea::Preamp;
        controller.state.selected_preamp_input = 1;
        controller.state.preamp.input2.mode = PreampMode::Line;

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Char('3')),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("open preamp mode selector");

        assert_eq!(action, KeyAction::Continue);
        assert!(transport.take_writes().is_empty());
        assert_eq!(
            controller.state.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::PreampMode { input: 1 }
            })
        );
        assert_eq!(controller.state.popup_selected_index, 1);
    }

    #[test]
    fn up_key_adjusts_focused_output_level() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.focus = FocusArea::Outputs;
        controller.state.outputs[0] =
            OutputState::new(OutputTarget::Monitor, 0x30, OutputMode::Normal);

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Up),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("up key");

        assert_eq!(action, KeyAction::Continue);
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x47, 0x00, 0x2f]);
    }

    #[test]
    fn down_key_adjusts_focused_preamp_gain() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.focus = FocusArea::Preamp;
        controller.state.selected_preamp_input = 1;
        controller.state.preamp.input2.mode = PreampMode::Mic;
        controller.state.preamp.input2.gain_raw = 0x10;

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Down),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("down key");

        assert_eq!(action, KeyAction::Continue);
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x50, 0x01, 0x0f]);
    }

    #[test]
    fn up_key_moves_popup_selection_before_adjusting_controls() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.focus = FocusArea::Outputs;
        controller.state.selected_output = 1;
        controller.state.outputs[1] = OutputState::new(OutputTarget::Hp1, 0x30, OutputMode::Normal);
        controller.state.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::SampleRate,
        });
        controller.state.popup_selected_index = 1;

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Up),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("popup up key");

        assert_eq!(action, KeyAction::Continue);
        assert_eq!(controller.state.popup_selected_index, 0);
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn toggle_mixer_solo_sends_selected_channel_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.focus = FocusArea::Mixer;
        controller.state.selected_channel = 0;
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].pan = PanState::center();
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].muted = Some(false);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].soloed = Some(false);

        let action = handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Char('o')),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("toggle solo");

        assert_eq!(action, KeyAction::Continue);
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
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller);

        controller
            .apply_intent(
                ui::Intent::OpenAssignmentPicker(5),
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open picker");
        assert_eq!(
            controller.state.assignment_picker,
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

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 5);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x03]);
        assert_eq!(&writes[0][0x10 + 0x0b..0x10 + 0x0d], &[0x09, 0x00]);
    }

    #[test]
    fn opening_assignment_picker_preselects_current_assignment() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.surface = Surface::MonitorHp1;
        controller.state.mixer_channels[MixerSurface::Mix1.index()][4].assignment =
            Some(MixerAssignment::Oscillator(1));

        controller
            .apply_intent(
                ui::Intent::OpenAssignmentPicker(5),
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open picker");

        assert_eq!(controller.state.popup_selected_index, 13);
    }

    #[test]
    fn mouse_output_mute_uses_selected_output_target() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.outputs[1] = OutputState::new(OutputTarget::Hp1, 0x30, OutputMode::Normal);

        controller
            .apply_intent(
                ui::Intent::ToggleOutputMute(1),
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("toggle output mute");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x48, 0x01, 0x01]);
    }

    #[test]
    fn mouse_output_level_action_sends_exact_step() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .apply_intent(
                ui::Intent::SetOutputLevel {
                    index: 1,
                    step: 0x12,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("set output level");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x47, 0x01, 0x12]);
    }

    #[test]
    fn mouse_preamp_gain_action_sends_exact_raw_gain() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .apply_intent(
                ui::Intent::SetPreampGain {
                    input: 1,
                    raw: 0x11,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("set preamp gain");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x50, 0x01, 0x11]);
    }

    #[test]
    fn mouse_mixer_level_action_sends_exact_level() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].pan = PanState::center();
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].muted = Some(false);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].soloed = Some(false);

        controller
            .apply_intent(
                ui::Intent::SetMixerLevel {
                    index: 0,
                    level: 0x15,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("set mixer level");

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
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].muted = Some(false);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].soloed = Some(false);

        controller
            .apply_intent(
                ui::Intent::SetMixerPan {
                    index: 0,
                    pan: PanState::from_raw(0x12),
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("set mixer pan");

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
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].level = Some(0x20);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].pan = PanState::center();
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].muted = Some(false);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].soloed = Some(false);

        controller
            .apply_intent(
                ui::Intent::AdjustMixerLevel {
                    index: 0,
                    increase: true,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("adjust mixer level");

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
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].pan = PanState::center();
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].muted = Some(false);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].soloed = Some(false);

        controller
            .apply_intent(
                ui::Intent::AdjustMixerPan {
                    index: 0,
                    right: true,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("adjust mixer pan");

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
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.outputs[0] =
            OutputState::new(OutputTarget::Monitor, 0x30, OutputMode::Normal);
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

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x47, 0x00, 0x2f]);
    }

    #[test]
    fn page_mixer_strips_right_moves_to_second_bank() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        // Area width 155 gives inner_width=151, card_width=18, stride=19, capacity=8
        let area = ratatui::layout::Rect::new(0, 0, 155, 50);

        controller
            .apply_intent(ui::Intent::PageMixerStripsRight, area)
            .expect("page strips right");

        assert_eq!(controller.state.mixer_strip_scroll, 8);
    }

    #[test]
    fn handle_mouse_event_scroll_in_strip_panel_does_not_scroll_viewport() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.mixer_strip_scroll = 8;
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

        assert_eq!(controller.state.mixer_strip_scroll, 8);
    }

    #[test]
    fn mouse_hotkeys_toggle_flips_popup_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller
            .apply_intent(
                ui::Intent::ToggleHotkeysPopup,
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open hotkeys");
        assert!(controller.state.hotkeys_popup_open);

        controller
            .apply_intent(
                ui::Intent::ToggleHotkeysPopup,
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("close hotkeys");
        assert!(!controller.state.hotkeys_popup_open);
    }

    #[test]
    fn mouse_sample_rate_selector_opens_and_pick_sends_exact_rate() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.device.clock_source = Some(ClockSource::Internal);

        controller
            .apply_intent(
                ui::Intent::OpenSampleRateSelector,
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open sample rate selector");
        assert_eq!(
            controller.state.selector_popup,
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
        assert_eq!(controller.state.selector_popup, None);

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x12], &[0x03, 0x02]);
    }

    #[test]
    fn sample_rate_controls_are_disabled_when_clock_source_is_not_internal() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.device.clock_source = Some(ClockSource::Usb);
        controller.state.device.sample_rate = Some(SampleRate::Hz192000);

        controller
            .apply_intent(
                ui::Intent::OpenSampleRateSelector,
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open sample rate selector");
        assert_eq!(controller.state.selector_popup, None);

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
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .apply_intent(
                ui::Intent::OpenPreampModeSelector(1),
                ratatui::layout::Rect::new(0, 0, 160, 50),
            )
            .expect("open preamp mode selector");
        assert_eq!(
            controller.state.selector_popup,
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
        assert_eq!(controller.state.selector_popup, None);

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x4f, 0x01, 0x02]);
    }

    #[test]
    fn popup_selection_wraps_with_keyboard_navigation() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.assignment_picker = Some(AssignmentPickerState { strip: 1 });

        handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Up),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("popup up");
        assert_eq!(
            controller.state.popup_selected_index,
            MixerAssignment::grounded_choices().len() - 1
        );

        handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Down),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("popup down");
        assert_eq!(controller.state.popup_selected_index, 0);
    }

    #[test]
    fn profile_popup_selection_uses_saved_profile_list() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.profiles_popup_open = true;
        controller.state.profile_names = vec!["tracking".to_string(), "mixdown".to_string()];

        handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Up),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("popup up");
        assert_eq!(controller.state.popup_selected_index, 1);

        handle_key_press(
            &mut controller,
            test_key(AppKeyCode::Down),
            ratatui::layout::Rect::new(0, 0, 120, 50),
        )
        .expect("popup down");
        assert_eq!(controller.state.popup_selected_index, 0);
    }

    #[test]
    fn profile_editor_accepts_characters_and_backspace() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.profile_editor = Some(ProfileEditorState {
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
                .profile_editor
                .as_ref()
                .map(|editor| &editor.value),
            Some(&"mix".to_string())
        );
    }

    #[test]
    fn activating_popup_selection_submits_highlighted_assignment() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller);
        controller.state.assignment_picker = Some(AssignmentPickerState { strip: 5 });
        controller.state.popup_selected_index = 13;

        activate_popup_selection(&mut controller).expect("activate popup selection");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 5);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x03]);
        assert_eq!(&writes[0][0x10 + 0x0b..0x10 + 0x0d], &[0x09, 0x00]);
        assert_eq!(controller.state.assignment_picker, None);
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
    fn handle_runtime_error_marks_controller_disconnected_for_device_errors() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.connection.connected = true;

        handle_runtime_error(&mut controller, TransportError::DeviceDisconnected.into())
            .expect("device errors should be swallowed");

        assert!(!controller.state.connection.connected);
        assert_eq!(
            controller.state.last_message,
            "Waiting for Zen Go device..."
        );
    }

    #[test]
    fn refresh_after_reconnect_runs_startup_query_sweep_once_device_returns() {
        let transport = AvailabilityTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
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
