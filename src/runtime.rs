use std::io;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use antelope_protocol::{
    Action, ClockSource, MixerAssignment, MixerSurface, PanState, PreampMode, SampleRate, Surface,
    ZenGoDriver,
};
use zen_go_tui::app::{
    Controller, FocusArea, Intent, PeakHoldDuration, RefreshRate, SelectorPopupKind,
    SelectorPopupState,
};
use zen_go_tui::settings;
use zen_go_tui::terminal::{
    AppKeyCode, AppKeyEvent, AppKeyEventKind, AppMouseButton, AppMouseEvent, AppMouseEventKind,
};
use zen_go_tui::transport::{is_device_error, Transport};
use zen_go_tui::ui;

use crate::input::{collect_pending_input, spawn_input_reader, InputThreadMessage};
use crate::profile_ops::{append_profile_editor_text, load_selected_profile};
use crate::timing::{device_poll_interval, should_draw_frame, should_probe_reconnect};

pub fn run_app(transport: Box<dyn Transport>) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    let input_rx = spawn_input_reader();
    let mut controller = Controller::new(transport, Box::new(ZenGoDriver::new()))?;
    if let Ok(saved) = settings::load_settings() {
        controller.state.ui.settings = saved;
    }
    controller.bootstrap()?;
    let result = app_loop(&mut terminal, &mut controller, &input_rx);
    let _ = settings::save_settings(&controller.state.ui.settings);
    disable_raw_mode()?;
    terminal.show_cursor()?;
    io::stdout().execute(DisableMouseCapture)?;
    io::stdout().execute(LeaveAlternateScreen)?;
    result
}

pub fn run_headless_app(transport: Box<dyn Transport>) -> Result<()> {
    eprintln!("Headless mode active. Press Ctrl+C to stop.");
    let mut controller = Controller::new(transport, Box::new(ZenGoDriver::new()))?;
    controller.bootstrap()?;
    headless_loop(&mut controller)
}

pub fn headless_loop(controller: &mut Controller) -> Result<()> {
    let mut reconnect_refresh_pending = false;
    let mut last_reconnect_probe_at = None;
    let mut reconnect_probe_attempts = 0;
    let mut last_runtime_activity_at = Some(std::time::Instant::now());

    loop {
        let now = std::time::Instant::now();
        if reconnect_refresh_pending {
            tick_reconnect_probe(
                controller,
                &mut reconnect_refresh_pending,
                &mut last_reconnect_probe_at,
                &mut reconnect_probe_attempts,
                now,
            )?;
            if reconnect_refresh_pending {
                if let Some(last_probe_at) = last_reconnect_probe_at {
                    let wait = crate::timing::device_retry_interval(
                        reconnect_probe_attempts.saturating_add(1),
                    )
                    .saturating_sub(now.duration_since(last_probe_at));
                    std::thread::sleep(wait);
                    continue;
                }
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

        std::thread::sleep(crate::timing::MIN_LOOP_SLEEP);
    }
}

pub fn handle_runtime_error(controller: &mut Controller, error: anyhow::Error) -> Result<()> {
    if is_device_error(&error) {
        controller.state.mark_disconnected();
        controller.state.ui.last_message = "Waiting for Zen Go device...".to_string();
        return Ok(());
    }

    Err(error)
}

pub fn refresh_after_reconnect_if_needed(
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
            controller.state.ui.last_message =
                "Zen Go reconnected, refreshing state...".to_string();
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

fn tick_reconnect_probe(
    controller: &mut Controller,
    reconnect_refresh_pending: &mut bool,
    last_reconnect_probe_at: &mut Option<std::time::Instant>,
    reconnect_probe_attempts: &mut usize,
    now: std::time::Instant,
) -> Result<()> {
    if *reconnect_refresh_pending
        && should_probe_reconnect(*last_reconnect_probe_at, *reconnect_probe_attempts, now)
    {
        refresh_after_reconnect_if_needed(controller, reconnect_refresh_pending)?;
        *last_reconnect_probe_at = Some(now);
        *reconnect_probe_attempts += 1;
        if !*reconnect_refresh_pending {
            *last_reconnect_probe_at = None;
            *reconnect_probe_attempts = 0;
        }
    }
    Ok(())
}

fn cycle_peak_hold_duration(
    controller: &mut Controller,
    area: ratatui::layout::Rect,
    direction: i8,
) -> Result<()> {
    let all = PeakHoldDuration::all();
    let current = controller.state.ui.settings.peak_hold_duration;
    let pos = all.iter().position(|&v| v == current).unwrap_or(1);
    let len = all.len();
    let next = if direction > 0 {
        all[(pos + 1) % len]
    } else {
        all[pos.checked_sub(1).unwrap_or(len - 1)]
    };
    controller.apply_intent(Intent::CyclePeakHoldDuration(next), area)?;
    if controller.state.ui.settings.auto_save {
        let _ = settings::save_settings(&controller.state.ui.settings);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Continue,
    ReconnectPending,
    Quit,
}

fn handle_options_popup(
    controller: &mut Controller,
    key_code: AppKeyCode,
    area: ratatui::layout::Rect,
) -> Result<KeyAction> {
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
            cycle_peak_hold_duration(controller, area, 1)?;
        }
        AppKeyCode::Char('l') | AppKeyCode::Char('L') => {
            cycle_peak_hold_duration(controller, area, -1)?;
        }
        AppKeyCode::Char('a') => {
            controller.apply_intent(Intent::ToggleAutoSave, area)?;
        }
        _ => {}
    }
    Ok(KeyAction::Continue)
}

fn handle_profile_editor(
    controller: &mut Controller,
    key_code: AppKeyCode,
    area: ratatui::layout::Rect,
) -> Result<KeyAction> {
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
    Ok(KeyAction::Continue)
}

fn handle_profiles_popup(
    controller: &mut Controller,
    key_code: AppKeyCode,
    area: ratatui::layout::Rect,
) -> Result<KeyAction> {
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
                controller.state.ui.last_message = "No profile selected to rename.".to_string();
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
    Ok(KeyAction::Continue)
}

pub fn handle_key_press(
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

    if controller.state.popup.hotkeys_open {
        match key_code {
            AppKeyCode::Char('q') => return Ok(KeyAction::Quit),
            AppKeyCode::Char('?') | AppKeyCode::Esc => {
                controller.apply_intent(Intent::ToggleHotkeysPopup, area)?;
            }
            _ => {}
        }
        return Ok(KeyAction::Continue);
    }

    if controller.state.popup.options_open {
        return handle_options_popup(controller, key_code, area);
    }

    if controller.state.popup.profile_editor.is_some() {
        return handle_profile_editor(controller, key_code, area);
    }

    if controller.state.popup.profiles_open {
        return handle_profiles_popup(controller, key_code, area);
    }

    let result = match key_code {
        AppKeyCode::Char('q') => return Ok(KeyAction::Quit),
        AppKeyCode::Char('r') => {
            controller.apply_intent(Intent::ToggleRoutingPopup, area)?;
            Ok(())
        }
        AppKeyCode::Char('p') => {
            if controller.state.popup.profiles_open {
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
            if controller.state.popup.assignment_picker.is_some()
                || controller.state.popup.selector_popup.is_some() =>
        {
            controller.apply_intent(Intent::MovePopupSelection(false), area)?;
            Ok(())
        }
        AppKeyCode::Down
            if controller.state.popup.assignment_picker.is_some()
                || controller.state.popup.selector_popup.is_some() =>
        {
            controller.apply_intent(Intent::MovePopupSelection(true), area)?;
            Ok(())
        }
        AppKeyCode::Enter
            if controller.state.popup.assignment_picker.is_some()
                || controller.state.popup.selector_popup.is_some() =>
        {
            activate_popup_selection(controller)
        }
        AppKeyCode::Char('[') if controller.state.popup.raw_view_open => {
            controller.apply_intent(Intent::CycleRawMapScope { forward: false }, area)?;
            Ok(())
        }
        AppKeyCode::Char(']') if controller.state.popup.raw_view_open => {
            controller.apply_intent(Intent::CycleRawMapScope { forward: true }, area)?;
            Ok(())
        }
        AppKeyCode::PageUp if controller.state.popup.raw_view_open => {
            controller.apply_intent(
                Intent::ScrollRawDump {
                    increase: false,
                    page: true,
                },
                area,
            )?;
            Ok(())
        }
        AppKeyCode::PageDown if controller.state.popup.raw_view_open => {
            controller.apply_intent(
                Intent::ScrollRawDump {
                    increase: true,
                    page: true,
                },
                area,
            )?;
            Ok(())
        }
        AppKeyCode::Left if controller.state.popup.raw_view_open => {
            if controller.state.raw_view.selected_tab == zen_go_tui::app::RawPacketTab::Query75 {
                controller.apply_intent(Intent::ScrollQueryReplyList { increase: false }, area)?;
            } else {
                controller.state.cycle_raw_packet(false);
            }
            Ok(())
        }
        AppKeyCode::Right if controller.state.popup.raw_view_open => {
            if controller.state.raw_view.selected_tab == zen_go_tui::app::RawPacketTab::Query75 {
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
            if controller.state.ui.focus == FocusArea::Mixer {
                let active_channel = controller.state.active_mixer_channels()
                    [controller.state.mixer.selected_channel];
                let mixer = MixerSurface::from_surface(controller.state.mixer.surface);
                controller.send_mixer_solo_change(
                    mixer,
                    active_channel.channel,
                    !active_channel.soloed.unwrap_or(false),
                )?;
            }
            Ok(())
        }
        AppKeyCode::Char('a') => {
            if controller.state.ui.focus == FocusArea::Mixer {
                let active_channel = controller.state.active_mixer_channels()
                    [controller.state.mixer.selected_channel];
                if !antelope_protocol::MixerStrip::assignment_write_is_grounded(
                    active_channel.channel,
                ) {
                    controller.state.ui.last_message =
                        "Assignment picking is not grounded for the selected strip.".to_string();
                } else {
                    controller
                        .apply_intent(Intent::OpenAssignmentPicker(active_channel.channel), area)?;
                }
            }
            Ok(())
        }
        AppKeyCode::Char('l') => {
            if controller.state.ui.focus == FocusArea::Mixer {
                let active_channel = controller.state.active_mixer_channels()
                    [controller.state.mixer.selected_channel];
                let mixer = MixerSurface::from_surface(controller.state.mixer.surface);
                controller.send_mixer_link_change(
                    mixer,
                    active_channel.channel,
                    !active_channel.linked.unwrap_or(false),
                )?;
            }
            Ok(())
        }
        AppKeyCode::Char('[') => {
            if controller.state.ui.focus == FocusArea::Mixer {
                let active_channel = controller.state.active_mixer_channels()
                    [controller.state.mixer.selected_channel];
                let next = active_channel
                    .pan
                    .raw()
                    .saturating_sub(1)
                    .max(PanState::MIN);
                controller.send(
                    Action::SetMixerPan {
                        mixer: MixerSurface::from_surface(controller.state.mixer.surface),
                        channel: active_channel.channel,
                        pan: PanState::from_raw(next),
                        muted: active_channel.muted.unwrap_or(false),
                        soloed: active_channel.soloed.unwrap_or(false),
                    },
                    None,
                )?;
            }
            Ok(())
        }
        AppKeyCode::Char(']') => {
            if controller.state.ui.focus == FocusArea::Mixer {
                let active_channel = controller.state.active_mixer_channels()
                    [controller.state.mixer.selected_channel];
                let next = active_channel
                    .pan
                    .raw()
                    .saturating_add(1)
                    .min(PanState::MAX);
                controller.send(
                    Action::SetMixerPan {
                        mixer: MixerSurface::from_surface(controller.state.mixer.surface),
                        channel: active_channel.channel,
                        pan: PanState::from_raw(next),
                        muted: active_channel.muted.unwrap_or(false),
                        soloed: active_channel.soloed.unwrap_or(false),
                    },
                    None,
                )?;
            }
            Ok(())
        }
        AppKeyCode::Char('3') => {
            if controller.state.ui.focus == FocusArea::Preamp {
                let input = controller.state.preamp.selected_input as u8;
                let current = if input == 0 {
                    controller.state.preamp.state.input1.mode
                } else {
                    controller.state.preamp.state.input2.mode
                };
                controller.state.popup.selected_index =
                    [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                        .iter()
                        .position(|mode| *mode == current)
                        .unwrap_or(0);
                controller.state.popup.selector_popup = Some(SelectorPopupState {
                    kind: SelectorPopupKind::PreampMode { input },
                });
                controller.state.ui.focus = FocusArea::Preamp;
                controller.state.preamp.selected_input = input.min(1) as usize;
            }
            Ok(())
        }
        AppKeyCode::Char('s') => {
            if controller.state.device.status.clock_source == Some(ClockSource::Internal) {
                let current = controller
                    .state
                    .device
                    .status
                    .sample_rate
                    .unwrap_or(SampleRate::Hz48000);
                let all = SampleRate::all_confirmed();
                let position = all.iter().position(|rate| *rate == current).unwrap_or(2);
                let next = all[(position + 1) % all.len()];
                controller.send(Action::SetSampleRate(next), None)?;
            }
            Ok(())
        }
        AppKeyCode::Char('c') => {
            let current = controller
                .state
                .device
                .status
                .clock_source
                .unwrap_or(ClockSource::Internal);
            let all = ClockSource::all_confirmed();
            let position = all
                .iter()
                .position(|source| *source == current)
                .unwrap_or(0);
            let next = all[(position + 1) % all.len()];
            controller.send(Action::SetClockSource(next), None)?;
            Ok(())
        }
        AppKeyCode::Char('1') => controller.send(Action::SelectSurface(Surface::MonitorHp1), None),
        AppKeyCode::Char('2') => controller.send(Action::SelectSurface(Surface::Hp2), None),
        AppKeyCode::Char('b') if controller.state.popup.raw_view_open => {
            controller.apply_intent(Intent::CaptureRawBaseline, area)?;
            Ok(())
        }
        AppKeyCode::Char('x') if controller.state.popup.raw_view_open => {
            controller.apply_intent(Intent::ClearRawBaseline, area)?;
            Ok(())
        }
        AppKeyCode::Esc
            if controller.state.popup.assignment_picker.is_some()
                || controller.state.popup.selector_popup.is_some()
                || controller.state.popup.routing_open
                || controller.state.popup.hotkeys_open
                || controller.state.popup.options_open =>
        {
            controller.state.popup.assignment_picker = None;
            controller.state.popup.selector_popup = None;
            controller.state.popup.routing_open = false;
            controller.state.popup.selected_index = 0;
            controller.state.popup.hotkeys_open = false;
            controller.state.popup.options_open = false;
            controller.state.ui.last_message = "Closed popup".to_string();
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

pub fn app_loop(
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
        if reconnect_refresh_pending {
            tick_reconnect_probe(
                controller,
                &mut reconnect_refresh_pending,
                &mut last_reconnect_probe_at,
                &mut reconnect_probe_attempts,
                now,
            )?;
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
                        if controller.state.ui.quit_requested {
                            break 'app;
                        }
                        needs_redraw = true;
                    }
                    zen_go_tui::terminal::AppInputEvent::Paste(text) => {
                        if controller.state.popup.profile_editor.is_some() {
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
        let fps = controller.state.ui.settings.refresh_rate.fps();
        if should_draw_frame(last_draw_at, needs_redraw, now, fps) {
            terminal.draw(|frame| {
                ui::draw(frame, &controller.state);
                if let Some((x, y)) = ui::profile_editor_cursor(frame.area(), &controller.state) {
                    frame.set_cursor_position((x, y));
                }
            })?;
            if controller.state.popup.profile_editor.is_some() {
                terminal.show_cursor()?;
            } else {
                terminal.hide_cursor()?;
            }
            last_draw_at = Some(now);
            needs_redraw = false;
        }

        std::thread::sleep(crate::timing::loop_sleep_for_fps(fps));
    }

    Ok(())
}

pub fn move_selection(controller: &mut Controller, right: bool, area: ratatui::layout::Rect) {
    match controller.state.ui.focus {
        FocusArea::Outputs => {
            controller.state.output.selected = if right {
                (controller.state.output.selected + 1) % controller.state.output.states.len()
            } else {
                controller
                    .state
                    .output
                    .selected
                    .checked_sub(1)
                    .unwrap_or(controller.state.output.states.len() - 1)
            };
        }
        FocusArea::Mixer => {
            let channels_len = controller.state.active_mixer_channels().len();
            controller.state.mixer.selected_channel = if right {
                (controller.state.mixer.selected_channel + 1) % channels_len
            } else {
                controller
                    .state
                    .mixer
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
            controller.state.preamp.selected_input = if right { 1 } else { 0 };
        }
        _ => {}
    }
}

pub fn activate_popup_selection(controller: &mut Controller) -> Result<()> {
    if let Some(picker) = controller.state.popup.assignment_picker {
        if let Some(assignment) = MixerAssignment::grounded_choices()
            .get(controller.state.popup.selected_index)
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

    if controller.state.popup.profiles_open {
        return load_selected_profile(controller);
    }

    if let Some(popup) = controller.state.popup.selector_popup {
        let action = match popup.kind {
            SelectorPopupKind::SampleRate => SampleRate::all_confirmed()
                .get(controller.state.popup.selected_index)
                .copied()
                .map(ui::Intent::PickSampleRate),
            SelectorPopupKind::ClockSource => ClockSource::all_confirmed()
                .get(controller.state.popup.selected_index)
                .copied()
                .map(ui::Intent::PickClockSource),
            SelectorPopupKind::PreampMode { input } => {
                [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                    .get(controller.state.popup.selected_index)
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

pub fn handle_mouse_event(
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
