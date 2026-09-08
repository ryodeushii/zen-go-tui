use std::io;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

#[cfg(test)]
use antelope_protocol::AuraVerbParameter;
use antelope_protocol::{
    Action, ControlValue, GlobalControl, InputControl, MixerAddress, MixerAssignment, PreampMode,
    SampleRate,
};
use zen_go_tui::app::{
    is_auraverb_write_unavailable, is_surround_write_unavailable, AuraVerbControlFocus, Controller,
    FocusArea, Intent, PeakHoldDuration, RefreshRate, SelectorPopupKind, SelectorPopupState,
    SurroundControlFocus, UiPage,
};
use zen_go_tui::device::{DeviceCandidate, DevicePickerState, RuntimeDeviceState};
use zen_go_tui::settings;
use zen_go_tui::terminal::{
    AppKeyCode, AppKeyEvent, AppKeyEventKind, AppMouseButton, AppMouseEvent, AppMouseEventKind,
    CrosstermTerminalControl, TerminalSession,
};
use zen_go_tui::transport::is_device_error;
use zen_go_tui::ui;

use crate::input::{collect_pending_input, spawn_input_reader, InputThreadMessage};
use crate::profile_ops::{append_profile_editor_text, load_selected_profile};
use crate::timing::{device_poll_interval, should_draw_frame};

pub fn run_app(mut devices: RuntimeDeviceState) -> Result<()> {
    let mut terminal_session = TerminalSession::setup(CrosstermTerminalControl)?;
    let keyboard_release_events_enabled = terminal_session.keyboard_release_events_enabled();
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    let input_rx = spawn_input_reader();

    let result = (|| -> Result<()> {
        loop {
            if devices.session().is_none() {
                if !device_picker_loop(&mut terminal, &mut devices, &input_rx)? {
                    return Ok(());
                }
                continue;
            }
            let mut session = devices.take_session().expect("checked session");
            let active_candidate = session.candidate().cloned();
            let exit = {
                let controller = session.controller_mut();
                if let Ok(saved) = settings::load_settings() {
                    controller.state.ui.settings = saved;
                }
                controller.state.ui.keyboard_release_events_enabled =
                    keyboard_release_events_enabled;
                controller.bootstrap()?;
                let result = app_loop_with_devices(
                    &mut terminal,
                    controller,
                    &mut devices,
                    active_candidate.as_ref(),
                    &input_rx,
                );
                let _ = settings::save_settings(&controller.state.ui.settings);
                result
            };
            match exit {
                Ok(AppLoopExit::Quit) => return Ok(()),
                Ok(AppLoopExit::Disconnected) => {
                    drop(session);
                    devices.disconnect_and_rediscover_from(active_candidate)?;
                }
                Ok(AppLoopExit::Switch(candidate)) => {
                    drop(session);
                    if let Err(error) = devices.switch_to(candidate) {
                        let notice = format!("Device selection failed: {error}");
                        let _ = devices.rediscover();
                        devices.set_picker_notice(notice);
                    }
                }
                Err(error) if is_device_error(&error) => {
                    drop(session);
                    devices.disconnect_and_rediscover_from(active_candidate)?;
                }
                Err(error) => return Err(error),
            }
        }
    })();

    terminal.show_cursor()?;
    terminal_session.cleanup()?;
    result
}

pub fn run_headless_app(mut devices: RuntimeDeviceState) -> Result<()> {
    eprintln!("Headless mode active. Press Ctrl+C to stop.");
    loop {
        let result = {
            let session = devices.session_mut().ok_or_else(|| {
                anyhow::anyhow!("headless mode requires one supported, unambiguous device")
            })?;
            session.controller_mut().bootstrap()?;
            headless_loop(session.controller_mut())
        };
        match result {
            Err(error) if is_device_error(&error) => {
                devices.disconnect_and_rediscover()?;
                if devices.session().is_none() {
                    return Err(anyhow::anyhow!(
                        "headless reconnect found no supported, unambiguous device"
                    ));
                }
            }
            result => return result,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppLoopExit {
    Quit,
    Disconnected,
    Switch(DeviceCandidate),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeviceSelectorAction {
    Continue,
    Cancel,
    Switch(DeviceCandidate),
}

fn handle_device_selector_key(
    picker: &mut DevicePickerState,
    key: AppKeyCode,
) -> DeviceSelectorAction {
    match key {
        AppKeyCode::Up => picker.select_previous(),
        AppKeyCode::Down => picker.select_next(),
        AppKeyCode::Esc => return DeviceSelectorAction::Cancel,
        AppKeyCode::Enter => {
            let Some(candidate) = picker.activate_selected().cloned() else {
                return DeviceSelectorAction::Continue;
            };
            if picker.is_active(&candidate) {
                return DeviceSelectorAction::Cancel;
            }
            return DeviceSelectorAction::Switch(candidate);
        }
        _ => return DeviceSelectorAction::Continue,
    }
    DeviceSelectorAction::Continue
}

fn device_picker_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    devices: &mut RuntimeDeviceState,
    input_rx: &std::sync::mpsc::Receiver<InputThreadMessage>,
) -> Result<bool> {
    loop {
        for event in collect_pending_input(input_rx)? {
            match event {
                zen_go_tui::terminal::AppInputEvent::Key(key)
                    if key.kind == AppKeyEventKind::Press =>
                {
                    match key.code {
                        AppKeyCode::Char('q') | AppKeyCode::Esc => return Ok(false),
                        AppKeyCode::Up => devices.picker_mut().select_previous(),
                        AppKeyCode::Down => devices.picker_mut().select_next(),
                        AppKeyCode::Enter => {
                            if devices.open_selected()? {
                                return Ok(true);
                            }
                        }
                        _ => {}
                    }
                }
                zen_go_tui::terminal::AppInputEvent::Mouse(mouse)
                    if matches!(mouse.kind, AppMouseEventKind::Down(AppMouseButton::Left)) =>
                {
                    let size = terminal.size()?;
                    let area = ratatui::layout::Rect::new(0, 0, size.width, size.height);
                    if let Some(row) = ui::device_picker_activation_row(
                        area,
                        devices.picker(),
                        mouse.column,
                        mouse.row,
                    ) {
                        devices.picker_mut().select_row(row);
                        if devices.open_selected()? {
                            return Ok(true);
                        }
                    }
                }
                _ => {}
            }
        }

        let should_retry =
            devices.picker().last_discovery_at.elapsed() >= devices.picker().retry_after;
        if should_retry {
            devices.rediscover()?;
            if devices.session().is_some() {
                return Ok(true);
            }
        }
        terminal.draw(|frame| ui::draw_device_picker(frame, devices.picker()))?;
        std::thread::sleep(crate::timing::MIN_LOOP_SLEEP);
    }
}

pub fn headless_loop(controller: &mut Controller) -> Result<()> {
    let mut last_runtime_activity_at = Some(std::time::Instant::now());

    loop {
        let now = std::time::Instant::now();
        match controller.poll_device(device_poll_interval(last_runtime_activity_at, false, now)) {
            Ok(observed_frame) => {
                if observed_frame {
                    last_runtime_activity_at = Some(std::time::Instant::now());
                }
            }
            Err(error) => return Err(error),
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

const AURAVERB_INTERACTION_UNAVAILABLE_MESSAGE: &str =
    "AuraVerb controls are read-only until a fresh device session provides authoritative Mix-1 readback";
const SURROUND_INTERACTION_UNAVAILABLE_MESSAGE: &str =
    "Surround controls are read-only until a fresh device session provides authoritative readback";

fn apply_interaction_intent(
    controller: &mut Controller,
    intent: Intent,
    area: ratatui::layout::Rect,
) -> Result<()> {
    match controller.apply_intent(intent, area) {
        Err(error) if is_auraverb_write_unavailable(&error) => {
            controller.state.ui.auraverb_drag = None;
            controller.state.ui.last_message = AURAVERB_INTERACTION_UNAVAILABLE_MESSAGE.to_string();
            Ok(())
        }
        Err(error) if is_surround_write_unavailable(&error) => {
            controller.state.ui.surround_drag = None;
            controller.state.ui.last_message = SURROUND_INTERACTION_UNAVAILABLE_MESSAGE.to_string();
            Ok(())
        }
        Err(error) => {
            controller.state.ui.auraverb_drag = None;
            controller.state.ui.surround_drag = None;
            Err(error)
        }
        Ok(()) => Ok(()),
    }
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
                .filter(|c| zen_go_tui::profile::is_profile_name_character(*c))
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

fn handle_routing_source_picker(
    controller: &mut Controller,
    key_code: AppKeyCode,
    ctrl: bool,
    area: ratatui::layout::Rect,
) -> Result<KeyAction> {
    let result = match key_code {
        AppKeyCode::Char('q') => return Ok(KeyAction::Quit),
        AppKeyCode::Char('c') if ctrl => return Ok(KeyAction::Quit),
        AppKeyCode::Up => controller.apply_intent(Intent::MovePopupSelection(false), area),
        AppKeyCode::Down => controller.apply_intent(Intent::MovePopupSelection(true), area),
        AppKeyCode::Enter => activate_popup_selection(controller),
        AppKeyCode::Esc => controller.apply_intent(Intent::CloseRoutingSourcePicker, area),
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

fn handle_selector_popup(
    controller: &mut Controller,
    key: AppKeyEvent,
    area: ratatui::layout::Rect,
) -> Result<KeyAction> {
    let key_code = key.code;
    let ctrl = key.modifiers.ctrl;
    let talkback_button = controller
        .state
        .popup
        .selector_popup
        .is_some_and(|popup| popup.kind == SelectorPopupKind::TalkbackButton);
    if key.kind == AppKeyEventKind::Release {
        if talkback_button && matches!(key_code, AppKeyCode::Enter | AppKeyCode::Char(' ')) {
            controller.apply_intent(Intent::SetTalkbackButton(false), area)?;
        }
        return Ok(KeyAction::Continue);
    }
    if key.kind != AppKeyEventKind::Press {
        return Ok(KeyAction::Continue);
    }
    let result = match key_code {
        AppKeyCode::Char('q') => {
            controller.release_talkback_if_held()?;
            return Ok(KeyAction::Quit);
        }
        AppKeyCode::Char('c') if ctrl => {
            controller.release_talkback_if_held()?;
            return Ok(KeyAction::Quit);
        }
        AppKeyCode::Up => controller.apply_intent(Intent::MovePopupSelection(false), area),
        AppKeyCode::Down => controller.apply_intent(Intent::MovePopupSelection(true), area),
        AppKeyCode::Enter => activate_popup_selection(controller),
        AppKeyCode::Esc => controller.apply_intent(Intent::CloseSelectorPopup, area),
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

fn handle_assignment_picker(
    controller: &mut Controller,
    key_code: AppKeyCode,
    ctrl: bool,
    area: ratatui::layout::Rect,
) -> Result<KeyAction> {
    let result = match key_code {
        AppKeyCode::Char('q') => return Ok(KeyAction::Quit),
        AppKeyCode::Char('c') if ctrl => return Ok(KeyAction::Quit),
        AppKeyCode::Up => controller.apply_intent(Intent::MovePopupSelection(false), area),
        AppKeyCode::Down => controller.apply_intent(Intent::MovePopupSelection(true), area),
        AppKeyCode::Enter => activate_popup_selection(controller),
        AppKeyCode::Esc => controller.apply_intent(Intent::CloseAssignmentPicker, area),
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

fn open_selected_assignment_picker(
    controller: &mut Controller,
    area: ratatui::layout::Rect,
) -> Result<()> {
    if controller.state.ui.focus != FocusArea::Mixer {
        return Ok(());
    }
    let Some(address) = controller
        .state
        .active_mixer_surface()
        .and_then(|index| controller.state.mixers().get(index))
        .and_then(|surface| {
            surface
                .strips
                .get(controller.state.mixer.selected_channel)
                .map(|strip| MixerAddress {
                    surface: surface.surface,
                    strip: strip.strip,
                })
        })
    else {
        return Ok(());
    };
    if controller
        .state
        .ui_profile
        .supports_assignment(address.surface, address.strip)
    {
        controller.apply_intent(Intent::OpenAssignmentPickerAt { address }, area)
    } else {
        controller.state.ui.last_message =
            "Routing assignment is unsupported for the selected strip.".into();
        Ok(())
    }
}

fn handle_routing_popup(
    controller: &mut Controller,
    key_code: AppKeyCode,
    ctrl: bool,
    area: ratatui::layout::Rect,
) -> Result<KeyAction> {
    let result = match key_code {
        AppKeyCode::Char('q') => return Ok(KeyAction::Quit),
        AppKeyCode::Char('c') if ctrl => return Ok(KeyAction::Quit),
        AppKeyCode::Char('r') => controller.apply_intent(Intent::ToggleRoutingPopup, area),
        AppKeyCode::Char('a') => open_selected_assignment_picker(controller, area),
        AppKeyCode::Esc => controller.apply_intent(Intent::CloseRoutingPopup, area),
        AppKeyCode::Up | AppKeyCode::Down if controller.state.popup.routing_editor.is_some() => {
            let editor = controller
                .state
                .popup
                .routing_editor
                .expect("guarded above");
            let index = controller
                .state
                .routing_capabilities
                .iter()
                .position(|group| group.destination == editor.destination)
                .unwrap_or(0);
            let index = if key_code == AppKeyCode::Up {
                index.saturating_sub(1)
            } else {
                index.saturating_add(1).min(
                    controller
                        .state
                        .routing_capabilities
                        .len()
                        .saturating_sub(1),
                )
            };
            let destination = controller.state.routing_capabilities[index].destination;
            controller.apply_intent(Intent::SelectRoutingDestination { destination }, area)
        }
        AppKeyCode::Left | AppKeyCode::Right if controller.state.popup.routing_editor.is_some() => {
            let editor = controller
                .state
                .popup
                .routing_editor
                .expect("guarded above");
            let channel = if key_code == AppKeyCode::Left {
                editor.channel.saturating_sub(1)
            } else {
                let last = controller
                    .state
                    .routing_capabilities
                    .iter()
                    .find(|group| group.destination == editor.destination)
                    .map_or(0, |group| group.channel_count.saturating_sub(1));
                editor.channel.saturating_add(1).min(last)
            };
            controller.apply_intent(
                Intent::SelectRoutingChannel {
                    destination: editor.destination,
                    channel,
                },
                area,
            )
        }
        AppKeyCode::Enter if controller.state.popup.routing_editor.is_some() => {
            let editor = controller
                .state
                .popup
                .routing_editor
                .expect("guarded above");
            if controller
                .state
                .general_routing_channel_available(editor.destination, editor.channel)
            {
                controller.apply_intent(
                    Intent::OpenRoutingSourcePicker {
                        destination: editor.destination,
                        channel: editor.channel,
                    },
                    area,
                )
            } else {
                controller.state.ui.last_message =
                    "Routing source is unavailable until complete readback arrives".to_string();
                Ok(())
            }
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

fn handle_raw_view(
    controller: &mut Controller,
    key_code: AppKeyCode,
    ctrl: bool,
    area: ratatui::layout::Rect,
) -> Result<KeyAction> {
    match key_code {
        AppKeyCode::Char('q') => return Ok(KeyAction::Quit),
        AppKeyCode::Char('c') if ctrl => return Ok(KeyAction::Quit),
        AppKeyCode::Char('d') if ctrl => {
            controller.apply_intent(Intent::ToggleRawView, area)?;
        }
        AppKeyCode::Char('[') => {
            controller.apply_intent(Intent::CycleRawMapScope { forward: false }, area)?;
        }
        AppKeyCode::Char(']') => {
            controller.apply_intent(Intent::CycleRawMapScope { forward: true }, area)?;
        }
        AppKeyCode::PageUp => controller.apply_intent(
            Intent::ScrollRawDump {
                increase: false,
                page: true,
            },
            area,
        )?,
        AppKeyCode::PageDown => controller.apply_intent(
            Intent::ScrollRawDump {
                increase: true,
                page: true,
            },
            area,
        )?,
        AppKeyCode::Left => {
            if controller.state.raw_view.selected_tab == zen_go_tui::app::RawPacketTab::Query75 {
                controller.apply_intent(Intent::ScrollQueryReplyList { increase: false }, area)?;
            } else {
                controller.state.cycle_raw_packet(false);
            }
        }
        AppKeyCode::Right => {
            if controller.state.raw_view.selected_tab == zen_go_tui::app::RawPacketTab::Query75 {
                controller.apply_intent(Intent::ScrollQueryReplyList { increase: true }, area)?;
            } else {
                controller.state.cycle_raw_packet(true);
            }
        }
        AppKeyCode::Char('b') => controller.apply_intent(Intent::CaptureRawBaseline, area)?,
        AppKeyCode::Char('x') => controller.apply_intent(Intent::ClearRawBaseline, area)?,
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

    if controller.state.popup.routing_source_picker.is_some() {
        return handle_routing_source_picker(controller, key_code, ctrl, area);
    }

    if controller.state.popup.selector_popup.is_some() {
        return handle_selector_popup(controller, key, area);
    }

    if ctrl && key_code == AppKeyCode::Char('c') {
        return Ok(KeyAction::Quit);
    }

    if controller.state.popup.assignment_picker.is_some() {
        return handle_assignment_picker(controller, key_code, ctrl, area);
    }

    if controller.state.popup.routing_open {
        return handle_routing_popup(controller, key_code, ctrl, area);
    }

    if controller.state.popup.raw_view_open {
        return handle_raw_view(controller, key_code, ctrl, area);
    }

    if ctrl {
        match key_code {
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
        AppKeyCode::F(1) if page_navigation_available(&controller.state) => {
            controller.apply_intent(Intent::SelectUiPage(UiPage::Mixer), area)?;
            Ok(())
        }
        AppKeyCode::F(2)
            if page_navigation_available(&controller.state)
                && controller.state.auraverb_page_available() =>
        {
            controller.apply_intent(Intent::SelectUiPage(UiPage::AuraVerb), area)?;
            Ok(())
        }
        AppKeyCode::F(3)
            if page_navigation_available(&controller.state)
                && controller.state.surround_page_available() =>
        {
            controller.apply_intent(Intent::SelectUiPage(UiPage::Surround), area)?;
            Ok(())
        }
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
        AppKeyCode::Tab if auraverb_page_input_active(&controller.state) => {
            controller.apply_intent(Intent::CycleAuraVerbFocus { forward: true }, area)?;
            Ok(())
        }
        AppKeyCode::BackTab if auraverb_page_input_active(&controller.state) => {
            controller.apply_intent(Intent::CycleAuraVerbFocus { forward: false }, area)?;
            Ok(())
        }
        AppKeyCode::Tab if surround_page_input_active(&controller.state) => {
            controller.apply_intent(Intent::CycleSurroundFocus { forward: true }, area)?;
            Ok(())
        }
        AppKeyCode::BackTab if surround_page_input_active(&controller.state) => {
            controller.apply_intent(Intent::CycleSurroundFocus { forward: false }, area)?;
            Ok(())
        }
        AppKeyCode::Tab => {
            controller.apply_intent(Intent::CycleFocus, area)?;
            Ok(())
        }
        AppKeyCode::BackTab => Ok(()),
        AppKeyCode::Char('?') => {
            controller.state.ui.auraverb_drag = None;
            controller.state.ui.surround_drag = None;
            controller.state.toggle_hotkeys_popup();
            Ok(())
        }
        AppKeyCode::Left | AppKeyCode::Down if auraverb_page_input_active(&controller.state) => {
            if let Some(intent) = auraverb_adjust_intent(&controller.state, -1) {
                apply_interaction_intent(controller, intent, area)?;
            }
            Ok(())
        }
        AppKeyCode::Right | AppKeyCode::Up if auraverb_page_input_active(&controller.state) => {
            if let Some(intent) = auraverb_adjust_intent(&controller.state, 1) {
                apply_interaction_intent(controller, intent, area)?;
            }
            Ok(())
        }
        AppKeyCode::PageUp if auraverb_page_input_active(&controller.state) => {
            if let Some(intent) = auraverb_adjust_intent(&controller.state, 10) {
                apply_interaction_intent(controller, intent, area)?;
            }
            Ok(())
        }
        AppKeyCode::PageDown if auraverb_page_input_active(&controller.state) => {
            if let Some(intent) = auraverb_adjust_intent(&controller.state, -10) {
                apply_interaction_intent(controller, intent, area)?;
            }
            Ok(())
        }
        AppKeyCode::Home if auraverb_page_input_active(&controller.state) => {
            if let Some(intent) = auraverb_bound_intent(&controller.state, false) {
                apply_interaction_intent(controller, intent, area)?;
            }
            Ok(())
        }
        AppKeyCode::End if auraverb_page_input_active(&controller.state) => {
            if let Some(intent) = auraverb_bound_intent(&controller.state, true) {
                apply_interaction_intent(controller, intent, area)?;
            }
            Ok(())
        }
        AppKeyCode::Enter | AppKeyCode::Char(' ')
            if auraverb_page_input_active(&controller.state) =>
        {
            if let Some(intent) = auraverb_toggle_intent(&controller.state) {
                apply_interaction_intent(controller, intent, area)?;
            }
            Ok(())
        }
        AppKeyCode::Left | AppKeyCode::Down if surround_page_input_active(&controller.state) => {
            if let Some(intent) = surround_adjust_intent(&controller.state, false) {
                apply_interaction_intent(controller, intent, area)?;
            }
            Ok(())
        }
        AppKeyCode::Right | AppKeyCode::Up if surround_page_input_active(&controller.state) => {
            if let Some(intent) = surround_adjust_intent(&controller.state, true) {
                apply_interaction_intent(controller, intent, area)?;
            }
            Ok(())
        }
        AppKeyCode::Char('m' | 'd' | 'o' | 'a' | 'l' | '[' | ']' | '3')
            if matches!(
                controller.state.active_ui_page(),
                UiPage::AuraVerb | UiPage::Surround
            ) =>
        {
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
                if let Some((surface, strip)) = controller
                    .state
                    .active_mixer_surface()
                    .and_then(|index| controller.state.mixers().get(index))
                    .and_then(|surface| {
                        surface
                            .strips
                            .get(controller.state.mixer.selected_channel)
                            .map(|strip| (surface.surface, strip.strip))
                    })
                {
                    controller.apply_intent(
                        Intent::ToggleMixerSoloAt {
                            address: MixerAddress { surface, strip },
                        },
                        area,
                    )?;
                }
            }
            Ok(())
        }
        AppKeyCode::Char('a') => open_selected_assignment_picker(controller, area),
        AppKeyCode::Char('l') => {
            if controller.state.ui.focus == FocusArea::Mixer {
                if let Some((surface, strip)) = controller
                    .state
                    .active_mixer_surface()
                    .and_then(|index| controller.state.mixers().get(index))
                    .and_then(|surface| {
                        surface
                            .strips
                            .get(controller.state.mixer.selected_channel)
                            .map(|strip| (surface.surface, strip.strip))
                    })
                {
                    controller.apply_intent(
                        Intent::ToggleMixerLinkAt {
                            address: MixerAddress { surface, strip },
                        },
                        area,
                    )?;
                }
            }
            Ok(())
        }
        AppKeyCode::Char('[') | AppKeyCode::Char(']') => {
            if controller.state.ui.focus == FocusArea::Mixer {
                let Some(surface) = controller
                    .state
                    .active_mixer_surface()
                    .and_then(|index| controller.state.mixers().get(index))
                else {
                    return Ok(KeyAction::Continue);
                };
                let Some(strip) = surface.strips.get(controller.state.mixer.selected_channel)
                else {
                    return Ok(KeyAction::Continue);
                };
                controller.apply_intent(
                    Intent::AdjustMixerPanAt {
                        address: MixerAddress {
                            surface: surface.surface,
                            strip: strip.strip,
                        },
                        right: key_code == AppKeyCode::Char(']'),
                    },
                    area,
                )?;
            }
            Ok(())
        }
        AppKeyCode::Char('3') => {
            if controller.state.ui.focus == FocusArea::Preamp {
                let Some(input) = controller
                    .state
                    .inputs_for_space("physical_inputs")
                    .get(controller.state.preamp.selected_input)
                    .cloned()
                else {
                    return Ok(KeyAction::Continue);
                };
                if !controller
                    .state
                    .ui_profile
                    .supports_input(input.address, InputControl::Mode)
                {
                    return Ok(KeyAction::Continue);
                }
                let Some(input_index) = u8::try_from(controller.state.preamp.selected_input).ok()
                else {
                    return Ok(KeyAction::Continue);
                };
                let current = PreampMode::from_raw(input.mode.unwrap_or_default() as u8);
                controller.state.popup.selected_index =
                    [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                        .iter()
                        .position(|mode| *mode == current)
                        .unwrap_or(0);
                controller.state.popup.selector_popup = Some(SelectorPopupState {
                    kind: SelectorPopupKind::PreampMode { input: input_index },
                });
                controller.state.ui.focus = FocusArea::Preamp;
            } else if let Some(surface) = controller
                .state
                .mixers()
                .get(2)
                .map(|surface| surface.surface)
            {
                controller.apply_intent(Intent::SelectMixerSurface { surface }, area)?;
            }
            Ok(())
        }
        AppKeyCode::Char('s') => {
            if controller
                .state
                .ui_profile
                .clock_source_is_internal(controller.state.device.status.clock_source)
            {
                let current = controller
                    .state
                    .device
                    .status
                    .sample_rate
                    .unwrap_or(SampleRate::Hz48000);
                let all = SampleRate::all_confirmed();
                let position = all.iter().position(|rate| *rate == current).unwrap_or(2);
                let next = all[(position + 1) % all.len()];
                controller.send(
                    Action::SetGlobal {
                        control: GlobalControl::SampleRate,
                        value: ControlValue::Enum(i32::from(next.code())),
                    },
                    None,
                )?;
            }
            Ok(())
        }
        AppKeyCode::Char('c') => {
            let choices = controller.state.ui_profile.clock_source_choices();
            if choices.is_empty()
                || !controller
                    .state
                    .ui_profile
                    .supports_global(GlobalControl::ClockSource)
            {
                return Ok(KeyAction::Continue);
            }
            let position = controller
                .state
                .device
                .status
                .clock_source
                .and_then(|current| choices.iter().position(|choice| choice.value == current));
            let next = choices[(position.map_or(0, |index| index + 1)) % choices.len()].value;
            controller.send(
                Action::SetGlobal {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(next),
                },
                None,
            )?;
            Ok(())
        }
        AppKeyCode::Char('1' | '2' | '4') => {
            let index = match key_code {
                AppKeyCode::Char('1') => 0,
                AppKeyCode::Char('2') => 1,
                AppKeyCode::Char('4') => 3,
                _ => unreachable!(),
            };
            if let Some(surface) = controller
                .state
                .mixers()
                .get(index)
                .map(|surface| surface.surface)
            {
                controller.apply_intent(Intent::SelectMixerSurface { surface }, area)?;
            }
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
) -> Result<AppLoopExit> {
    app_loop_inner(terminal, controller, None, None, input_rx)
}

fn app_loop_with_devices(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    controller: &mut Controller,
    devices: &mut RuntimeDeviceState,
    active_candidate: Option<&DeviceCandidate>,
    input_rx: &std::sync::mpsc::Receiver<InputThreadMessage>,
) -> Result<AppLoopExit> {
    app_loop_inner(
        terminal,
        controller,
        Some(devices),
        active_candidate,
        input_rx,
    )
}

fn poll_controller_for_runtime(
    controller: &mut Controller,
    selector_open: bool,
    timeout: std::time::Duration,
) -> Result<bool> {
    if selector_open {
        controller.poll_device_without_writes(timeout)
    } else {
        controller.poll_device(timeout)
    }
}

fn handle_profile_editor_paste(controller: &mut Controller, selector_open: bool, text: &str) {
    if !selector_open && controller.state.popup.profile_editor.is_some() {
        append_profile_editor_text(controller, text);
    }
}

fn device_header_name_mouse_hit(
    area: ratatui::layout::Rect,
    state: &zen_go_tui::app::AppState,
    mouse: AppMouseEvent,
) -> bool {
    page_navigation_available(state)
        && matches!(mouse.kind, AppMouseEventKind::Down(AppMouseButton::Left))
        && ui::device_header_name_hit(area, state, mouse.column, mouse.row)
}

fn open_device_selector_for_runtime(
    runtime: &mut RuntimeDeviceState,
    controller: &mut Controller,
    active_candidate: Option<&DeviceCandidate>,
) {
    controller.state.ui.auraverb_drag = None;
    controller.state.ui.surround_drag = None;
    if let Err(error) = runtime.open_selector_for(active_candidate.cloned()) {
        controller.state.ui.last_message = format!("Device selector unavailable: {error}");
    }
}

fn handle_device_selector_header_mouse(
    runtime: &mut RuntimeDeviceState,
    controller: &mut Controller,
    active_candidate: Option<&DeviceCandidate>,
    area: ratatui::layout::Rect,
    mouse: AppMouseEvent,
) -> bool {
    if !device_header_name_mouse_hit(area, &controller.state, mouse) {
        return false;
    }
    open_device_selector_for_runtime(runtime, controller, active_candidate);
    true
}

fn app_loop_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    controller: &mut Controller,
    mut devices: Option<&mut RuntimeDeviceState>,
    active_candidate: Option<&DeviceCandidate>,
    input_rx: &std::sync::mpsc::Receiver<InputThreadMessage>,
) -> Result<AppLoopExit> {
    let mut last_draw_at = None;
    let mut needs_redraw = true;
    let mut last_runtime_activity_at = Some(std::time::Instant::now());

    'app: loop {
        let now = std::time::Instant::now();
        let input_events = collect_pending_input(input_rx)?;
        if !input_events.is_empty() {
            last_runtime_activity_at = Some(std::time::Instant::now());

            for event in input_events {
                match event {
                    zen_go_tui::terminal::AppInputEvent::Key(key) => {
                        if key.kind != AppKeyEventKind::Press
                            && controller
                                .state
                                .popup
                                .selector_popup
                                .is_none_or(|popup| popup.kind != SelectorPopupKind::TalkbackButton)
                        {
                            continue;
                        }

                        if let Some(runtime) = devices.as_deref_mut() {
                            if runtime.selector().is_some() {
                                let action = runtime
                                    .selector_mut()
                                    .map(|picker| handle_device_selector_key(picker, key.code))
                                    .expect("selector exists");
                                match action {
                                    DeviceSelectorAction::Cancel => runtime.close_selector(),
                                    DeviceSelectorAction::Switch(candidate) => {
                                        return Ok(AppLoopExit::Switch(candidate));
                                    }
                                    DeviceSelectorAction::Continue => {}
                                }
                                needs_redraw = true;
                                continue;
                            }
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
                            return Ok(AppLoopExit::Disconnected);
                        }

                        needs_redraw = true;
                    }
                    zen_go_tui::terminal::AppInputEvent::Mouse(mouse) => {
                        let size = terminal.size()?;
                        let area = ratatui::layout::Rect::new(0, 0, size.width, size.height);

                        if let Some(runtime) = devices.as_deref_mut() {
                            if runtime.selector().is_some() {
                                if matches!(
                                    mouse.kind,
                                    AppMouseEventKind::Down(AppMouseButton::Left)
                                ) {
                                    let row = runtime.selector().and_then(|picker| {
                                        ui::device_picker_activation_row(
                                            area,
                                            picker,
                                            mouse.column,
                                            mouse.row,
                                        )
                                    });
                                    if let Some(row) = row {
                                        if let Some(picker) = runtime.selector_mut() {
                                            picker.select_row(row);
                                        }
                                        if let Some(candidate) = runtime.selector_selected() {
                                            if runtime.selector_selected_is_active() {
                                                runtime.close_selector();
                                            } else {
                                                return Ok(AppLoopExit::Switch(candidate));
                                            }
                                        }
                                    }
                                }
                                needs_redraw = true;
                                continue;
                            }

                            if handle_device_selector_header_mouse(
                                runtime,
                                controller,
                                active_candidate,
                                area,
                                mouse,
                            ) {
                                controller.release_talkback_if_held()?;
                                needs_redraw = true;
                                continue;
                            }
                        }

                        if let Err(error) = handle_mouse_event(area, controller, mouse) {
                            if is_device_error(&error) {
                                return Ok(AppLoopExit::Disconnected);
                            }
                            return Err(error);
                        }
                        if controller.state.ui.quit_requested {
                            break 'app;
                        }
                        needs_redraw = true;
                    }
                    zen_go_tui::terminal::AppInputEvent::Paste(text) => {
                        let selector_open = devices
                            .as_deref()
                            .is_some_and(|runtime| runtime.selector().is_some());
                        handle_profile_editor_paste(controller, selector_open, &text);
                        needs_redraw = true;
                    }
                    zen_go_tui::terminal::AppInputEvent::FocusLost => {
                        controller.state.ui.auraverb_drag = None;
                        controller.state.ui.surround_drag = None;
                        if let Err(error) = controller.release_talkback_if_held() {
                            if is_device_error(&error) {
                                return Ok(AppLoopExit::Disconnected);
                            }
                            return Err(error);
                        }
                        needs_redraw = true;
                    }
                    zen_go_tui::terminal::AppInputEvent::Resize { .. }
                    | zen_go_tui::terminal::AppInputEvent::FocusGained => needs_redraw = true,
                }
            }
        }

        let selector_open = devices
            .as_deref()
            .is_some_and(|runtime| runtime.selector().is_some());
        match poll_controller_for_runtime(
            controller,
            selector_open,
            device_poll_interval(last_runtime_activity_at, true, now),
        ) {
            Ok(observed_frame) => {
                needs_redraw |= observed_frame;
                if observed_frame {
                    last_runtime_activity_at = Some(std::time::Instant::now());
                }
            }
            Err(error) if is_device_error(&error) => return Ok(AppLoopExit::Disconnected),
            Err(error) => return Err(error),
        }

        if let Some(runtime) = devices.as_deref_mut() {
            let should_refresh = runtime
                .selector()
                .is_some_and(|picker| picker.last_discovery_at.elapsed() >= picker.retry_after);
            if should_refresh {
                if let Err(error) = runtime.refresh_selector() {
                    controller.state.ui.last_message =
                        format!("Device selector refresh failed: {error}");
                }
                needs_redraw = true;
            }
        }

        let now = std::time::Instant::now();
        controller.state.prune_expired_peaks();
        let fps = controller.state.ui.settings.refresh_rate.fps();
        if should_draw_frame(last_draw_at, needs_redraw, now, fps) {
            let selector = devices.as_deref().and_then(|runtime| runtime.selector());
            terminal.draw(|frame| {
                ui::draw(frame, &controller.state);
                if let Some(picker) = selector {
                    ui::draw_device_picker(frame, picker);
                }
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

    Ok(AppLoopExit::Quit)
}

fn page_navigation_available(state: &zen_go_tui::app::AppState) -> bool {
    !state.popup.raw_view_open
        && !state.popup.hotkeys_open
        && !state.popup.profiles_open
        && state.popup.profile_editor.is_none()
        && state.popup.selector_popup.is_none()
        && state.popup.assignment_picker.is_none()
        && state.popup.routing_source_picker.is_none()
        && !state.popup.routing_open
        && !state.popup.options_open
}

fn auraverb_page_input_active(state: &zen_go_tui::app::AppState) -> bool {
    page_navigation_available(state) && state.active_ui_page() == UiPage::AuraVerb
}

fn auraverb_adjust_intent(state: &zen_go_tui::app::AppState, delta: i16) -> Option<Intent> {
    if !state.auraverb_controls_enabled() {
        return None;
    }
    let AuraVerbControlFocus::Parameter(parameter) = state.ui.auraverb_focus else {
        return None;
    };
    let current = i16::from(state.displayed_auraverb_state()?.value(parameter));
    Some(Intent::SetAuraVerbParameter {
        parameter,
        value: u8::try_from((current + delta).clamp(0, 100)).ok()?,
    })
}

fn auraverb_bound_intent(state: &zen_go_tui::app::AppState, maximum: bool) -> Option<Intent> {
    if !state.auraverb_controls_enabled() {
        return None;
    }
    let AuraVerbControlFocus::Parameter(parameter) = state.ui.auraverb_focus else {
        return None;
    };
    Some(Intent::SetAuraVerbParameter {
        parameter,
        value: if maximum { 100 } else { 0 },
    })
}

fn auraverb_toggle_intent(state: &zen_go_tui::app::AppState) -> Option<Intent> {
    if !state.auraverb_controls_enabled()
        || state.ui.auraverb_focus != AuraVerbControlFocus::Enabled
    {
        return None;
    }
    Some(Intent::SetAuraVerbEnabled(
        !state.displayed_auraverb_state()?.enabled,
    ))
}

fn surround_page_input_active(state: &zen_go_tui::app::AppState) -> bool {
    page_navigation_available(state) && state.active_ui_page() == UiPage::Surround
}

fn surround_adjust_intent(state: &zen_go_tui::app::AppState, increase: bool) -> Option<Intent> {
    if !state.surround_controls_enabled() {
        return None;
    }
    let display = state.displayed_surround_state()?;
    let (level_range, delay_range) = state.surround_control_ranges()?;
    Some(match state.ui.surround_focus {
        SurroundControlFocus::Level => Intent::SetSurroundGlobalLevel(
            if increase {
                display.level_raw.saturating_add(1)
            } else {
                display.level_raw.saturating_sub(1)
            }
            .clamp(level_range.0, level_range.1),
        ),
        SurroundControlFocus::Delay => Intent::SetSurroundGlobalDelay(
            if increase {
                display.delay_tenths_ms.saturating_add(1)
            } else {
                display.delay_tenths_ms.saturating_sub(1)
            }
            .clamp(delay_range.0, delay_range.1),
        ),
    })
}

pub fn move_selection(controller: &mut Controller, right: bool, area: ratatui::layout::Rect) {
    match controller.state.ui.focus {
        FocusArea::Outputs => {
            let outputs_len = controller.state.outputs().len();
            if outputs_len == 0 {
                return;
            }
            controller.state.output.selected = if right {
                (controller.state.output.selected + 1) % outputs_len
            } else {
                controller
                    .state
                    .output
                    .selected
                    .checked_sub(1)
                    .unwrap_or(outputs_len - 1)
            };
        }
        FocusArea::Mixer => {
            let channels_len = controller
                .state
                .active_mixer_surface()
                .and_then(|index| controller.state.mixers().get(index))
                .map_or(0, |surface| surface.strips.len());
            if channels_len == 0 {
                return;
            }
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
            let inputs_len = controller.state.inputs_for_space("physical_inputs").len();
            if inputs_len == 0 {
                return;
            }
            controller.state.preamp.selected_input = if right {
                (controller.state.preamp.selected_input + 1) % inputs_len
            } else {
                controller
                    .state
                    .preamp
                    .selected_input
                    .checked_sub(1)
                    .unwrap_or(inputs_len - 1)
            };
        }
        _ => {}
    }
}

pub fn activate_popup_selection(controller: &mut Controller) -> Result<()> {
    if let Some(picker) = controller.state.popup.routing_source_picker {
        let choices = controller
            .state
            .routing_source_choices_for_destination(picker.destination);
        if let Some(choice) = choices.get(controller.state.popup.selected_index) {
            return controller.apply_intent(
                ui::Intent::PickRoutingSource {
                    destination: picker.destination,
                    channel: picker.channel,
                    source: choice.source,
                },
                ratatui::layout::Rect::new(0, 0, 160, 50),
            );
        }
    }

    if let Some(picker) = controller.state.popup.assignment_picker {
        if let Some(address) = controller.state.popup.assignment_picker_address {
            let choices = controller.state.routing_source_choices(address.surface);
            if let Some(choice) = choices.get(controller.state.popup.selected_index) {
                return controller.apply_intent(
                    ui::Intent::PickRoutingSourceAt {
                        address,
                        source: choice.source,
                    },
                    ratatui::layout::Rect::new(0, 0, 160, 50),
                );
            }
        }
        if let Some(assignment) = MixerAssignment::grounded_choices()
            .get(controller.state.popup.selected_index)
            .copied()
        {
            let intent = controller.state.popup.assignment_picker_address.map_or(
                ui::Intent::PickAssignment {
                    strip: picker.strip,
                    assignment,
                },
                |address| ui::Intent::PickAssignmentAt {
                    address,
                    assignment,
                },
            );
            return controller.apply_intent(intent, ratatui::layout::Rect::new(0, 0, 160, 50));
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
            SelectorPopupKind::ClockSource => controller
                .state
                .ui_profile
                .clock_source_choices()
                .get(controller.state.popup.selected_index)
                .map(|choice| ui::Intent::PickClockSource(choice.value)),
            SelectorPopupKind::PreampMode { input } => {
                [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                    .get(controller.state.popup.selected_index)
                    .copied()
                    .map(|mode| ui::Intent::PickPreampMode { input, mode })
            }
            SelectorPopupKind::Settings => controller
                .state
                .ui_profile
                .setting_rows()
                .get(controller.state.popup.selected_index)
                .and_then(|control| match control {
                    antelope_protocol::GlobalControl::Brightness => {
                        Some(ui::Intent::OpenBrightnessSelector)
                    }
                    antelope_protocol::GlobalControl::OutputTrim(address) => {
                        Some(ui::Intent::OpenOutputTrimSelector(*address))
                    }
                    antelope_protocol::GlobalControl::TalkbackButton => {
                        Some(ui::Intent::OpenTalkbackButton)
                    }
                    antelope_protocol::GlobalControl::TalkbackSource => {
                        Some(ui::Intent::OpenTalkbackSourceSelector)
                    }
                    antelope_protocol::GlobalControl::TalkbackGain => {
                        Some(ui::Intent::OpenTalkbackGainSelector)
                    }
                    _ => None,
                }),
            SelectorPopupKind::Brightness => (controller.state.popup.selected_index <= 100)
                .then_some(ui::Intent::PickBrightness(
                    controller.state.popup.selected_index as i32,
                )),
            SelectorPopupKind::OutputTrim { target } => controller
                .state
                .ui_profile
                .output_trim_value_labels()
                .get(controller.state.popup.selected_index)
                .map(|(value, _)| ui::Intent::PickOutputTrim {
                    address: antelope_protocol::OutputTrimAddress { target },
                    value: *value,
                }),
            SelectorPopupKind::TalkbackButton => match controller.state.popup.selected_index {
                0 if controller.state.ui.keyboard_release_events_enabled => {
                    Some(ui::Intent::SetTalkbackButton(true))
                }
                1 => Some(ui::Intent::SetTalkbackButton(false)),
                _ => None,
            },
            SelectorPopupKind::TalkbackSource => controller
                .state
                .ui_profile
                .talkback_source_choices()
                .get(controller.state.popup.selected_index)
                .map(|(source, _)| ui::Intent::PickTalkbackSource(*source)),
            SelectorPopupKind::TalkbackGain => (controller.state.popup.selected_index <= 96)
                .then_some(ui::Intent::PickTalkbackGain(
                    controller.state.popup.selected_index as i32,
                )),
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
            let aura_drag =
                ui::auraverb_drag_target(area, &controller.state, mouse.column, mouse.row);
            controller.state.ui.auraverb_drag = aura_drag;
            if let Some(focus) = aura_drag {
                controller.state.ui.auraverb_focus = focus;
            }
            let surround_drag =
                ui::surround_drag_target(area, &controller.state, mouse.column, mouse.row);
            controller.state.ui.surround_drag = surround_drag;
            if let Some(focus) = surround_drag {
                controller.state.ui.surround_focus = focus;
            }
            if let Some(action) = ui::mouse_action(area, &controller.state, mouse.column, mouse.row)
            {
                apply_interaction_intent(controller, action, area)?;
            }
        }
        AppMouseEventKind::Up(AppMouseButton::Left) => {
            controller.state.ui.auraverb_drag = None;
            controller.state.ui.surround_drag = None;
            controller.release_talkback_if_held()?;
        }
        AppMouseEventKind::Drag(AppMouseButton::Left) => {
            if let Some(action) =
                ui::slider_mouse_action(area, &controller.state, mouse.column, mouse.row)
            {
                apply_interaction_intent(controller, action, area)?;
            } else {
                controller.state.ui.auraverb_drag = None;
                controller.state.ui.surround_drag = None;
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
                apply_interaction_intent(controller, action, area)?;
                return Ok(());
            }
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::time::Duration;

    use super::*;
    use antelope_protocol::{
        CommandBatch, DeviceDriver, DeviceEvent, DriverDefinition, DriverError, GlobalControl,
        InputAddress, OutputTrimAddress, ProfileDriver, QueryRequest,
    };
    use ratatui::{backend::TestBackend, Terminal};
    use zen_go_tui::terminal::AppModifiers;
    use zen_go_tui::transport::{MockTransport, Transport, TransportError};

    struct RecordingDriver {
        definition: DriverDefinition,
        actions: Arc<Mutex<Vec<Action>>>,
    }

    impl RecordingDriver {
        fn new() -> (Self, Arc<Mutex<Vec<Action>>>) {
            let actions = Arc::new(Mutex::new(Vec::new()));
            let definition = zen_go_tui::device::builtin_zen_go_driver()
                .expect("Zen Go driver")
                .definition()
                .clone();
            (
                Self {
                    definition,
                    actions: actions.clone(),
                },
                actions,
            )
        }
    }

    impl DeviceDriver for RecordingDriver {
        fn definition(&self) -> &DriverDefinition {
            &self.definition
        }

        fn startup_requests(&self) -> &[QueryRequest] {
            &[]
        }

        fn encode(&self, action: Action) -> std::result::Result<CommandBatch, DriverError> {
            self.actions
                .lock()
                .expect("recording driver actions")
                .push(action);
            Ok(CommandBatch {
                frames: vec![vec![0; 64]],
                refresh_requests: Vec::new(),
            })
        }

        fn decode(&self, _bytes: &[u8]) -> std::result::Result<Option<DeviceEvent>, DriverError> {
            Ok(None)
        }
    }

    fn synthetic_entry() -> antelope_protocol::RuntimeEntry {
        let mut entry = zen_go_tui::device::ProfileCatalog::builtin()
            .entries()
            .iter()
            .find(|entry| entry.id == "zen_go_sc")
            .expect("Zen Go profile")
            .clone();
        let mut input = entry.profile.inputs[1].clone();
        input.index = 2;
        input.id = "physical_input_3".into();
        input.name = "Input 3".into();
        entry.profile.inputs.push(input);
        let mut output = entry.profile.outputs[2].clone();
        output.id = 3;
        output.name = "Output 4".into();
        entry.profile.outputs.push(output);
        entry.profile.address_spaces[0].count = Some(3);
        entry.profile.mixers.truncate(1);
        entry.profile.mixers[0].strip_count = 7;
        entry
    }

    fn key(code: AppKeyCode) -> AppKeyEvent {
        AppKeyEvent {
            code,
            modifiers: AppModifiers::default(),
            kind: AppKeyEventKind::Press,
        }
    }

    #[test]
    fn clock_selector_is_keyboard_modal_and_only_activates_selected_choice() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 50);
        let transport = MockTransport::default();
        let (driver, actions) = RecordingDriver::new();
        let entry = synthetic_entry();
        let mut controller =
            Controller::new_for_entry(Box::new(transport.clone()), Box::new(driver), &entry)
                .expect("controller");
        controller.state.device.status.clock_source = Some(0);
        controller.state.device.status.sample_rate = Some(SampleRate::Hz48000);
        controller.state.ui.focus = FocusArea::Outputs;
        controller.state.output.selected = 3;
        controller
            .apply_intent(Intent::OpenClockSourceSelector, area)
            .expect("open clock selector");

        for code in [
            AppKeyCode::Char('c'),
            AppKeyCode::Char('s'),
            AppKeyCode::Char('r'),
            AppKeyCode::Char('x'),
            AppKeyCode::Left,
            AppKeyCode::Tab,
        ] {
            assert_eq!(
                handle_key_press(&mut controller, key(code), area).expect("consume modal key"),
                KeyAction::Continue
            );
        }

        assert!(actions.lock().expect("recorded actions").is_empty());
        assert!(transport.take_writes().is_empty());
        assert_eq!(controller.state.device.status.clock_source, Some(0));
        assert_eq!(controller.state.output.selected, 3);
        assert_eq!(
            controller.state.device.status.sample_rate,
            Some(SampleRate::Hz48000)
        );
        assert!(!controller.state.popup.routing_open);
        assert_eq!(controller.state.popup.selected_index, 0);

        handle_key_press(&mut controller, key(AppKeyCode::Down), area).expect("selector down");
        assert_eq!(controller.state.popup.selected_index, 1);
        handle_key_press(&mut controller, key(AppKeyCode::Up), area).expect("selector up");
        assert_eq!(controller.state.popup.selected_index, 0);
        handle_key_press(&mut controller, key(AppKeyCode::Up), area).expect("selector wrap up");
        assert_eq!(controller.state.popup.selected_index, 2);
        handle_key_press(&mut controller, key(AppKeyCode::Enter), area)
            .expect("activate clock choice");

        assert!(controller.state.popup.selector_popup.is_none());
        assert_eq!(controller.state.device.status.clock_source, Some(2));
        assert!(matches!(
            actions.lock().expect("recorded actions").as_slice(),
            [Action::SetGlobal {
                control: GlobalControl::ClockSource,
                value: ControlValue::Enum(2),
            }]
        ));

        controller
            .apply_intent(Intent::OpenClockSourceSelector, area)
            .expect("reopen clock selector");
        handle_key_press(&mut controller, key(AppKeyCode::Esc), area).expect("cancel selector");
        assert!(controller.state.popup.selector_popup.is_none());
    }

    #[test]
    fn runtime_header_activation_hits_the_rendered_device_name() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 50);
        let state = zen_go_tui::app::AppState::default();
        let mut terminal =
            Terminal::new(TestBackend::new(area.width, area.height)).expect("test terminal");
        terminal
            .draw(|frame| ui::draw(frame, &state))
            .expect("draw titlebar");

        let device_area = ui::device_header_area(area);
        let name_x = device_area.x.saturating_add(2);
        let name_y = device_area.y.saturating_add(1);
        assert!(!terminal.backend().buffer()[(name_x, name_y)]
            .symbol()
            .trim()
            .is_empty());
        assert!(device_header_name_mouse_hit(
            area,
            &state,
            AppMouseEvent {
                kind: AppMouseEventKind::Down(AppMouseButton::Left),
                column: name_x,
                row: name_y,
                modifiers: AppModifiers::default(),
            }
        ));
    }

    #[test]
    fn runtime_selector_activation_ignores_raw_view_and_modal_header_clicks() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 50);
        let mut devices = RuntimeDeviceState::mock(zen_go_tui::device::ProfileCatalog::builtin())
            .expect("mock runtime devices");
        let mut controller = Controller::new(
            Box::new(MockTransport::default()),
            Box::new(zen_go_tui::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("controller");
        let mut terminal =
            Terminal::new(TestBackend::new(area.width, area.height)).expect("test terminal");
        terminal
            .draw(|frame| ui::draw(frame, &controller.state))
            .expect("draw visible titlebar");

        let device_area = ui::device_header_area(area);
        let name_click = AppMouseEvent {
            kind: AppMouseEventKind::Down(AppMouseButton::Left),
            column: device_area.x.saturating_add(2),
            row: device_area.y.saturating_add(1),
            modifiers: AppModifiers::default(),
        };

        assert!(handle_device_selector_header_mouse(
            &mut devices,
            &mut controller,
            None,
            area,
            name_click,
        ));
        assert!(devices.selector().is_some());
        devices.close_selector();

        controller.state.popup.raw_view_open = true;
        assert!(!handle_device_selector_header_mouse(
            &mut devices,
            &mut controller,
            None,
            area,
            name_click,
        ));
        assert!(devices.selector().is_none());

        controller.state.popup.raw_view_open = false;
        controller.state.popup.options_open = true;
        assert!(!handle_device_selector_header_mouse(
            &mut devices,
            &mut controller,
            None,
            area,
            name_click,
        ));
        assert!(devices.selector().is_none());

        assert!(devices.selector().is_none());
    }

    #[test]
    fn device_selector_keys_navigate_activate_and_cancel_without_mutating_connection() {
        let first = DeviceCandidate::new(
            "selector-first",
            0x23e5,
            0xa015,
            Some("ZEN-FIRST".into()),
            Some("Zen Go".into()),
            0,
            0,
            3,
        );
        let second = DeviceCandidate::new(
            "selector-second",
            0x23e5,
            0xa015,
            Some("ZEN-SECOND".into()),
            Some("Zen Go".into()),
            0,
            0,
            3,
        );
        let mut picker = DevicePickerState::new(
            vec![first.clone(), second.clone()],
            &zen_go_tui::device::ProfileCatalog::builtin(),
        );
        picker.set_active_candidate(Some(first));

        assert_eq!(
            handle_device_selector_key(&mut picker, AppKeyCode::Down),
            DeviceSelectorAction::Continue
        );
        assert_eq!(
            handle_device_selector_key(&mut picker, AppKeyCode::Enter),
            DeviceSelectorAction::Switch(second)
        );

        let selected = picker.selected_index();
        assert_eq!(
            handle_device_selector_key(&mut picker, AppKeyCode::Esc),
            DeviceSelectorAction::Cancel
        );
        assert_eq!(picker.selected_index(), selected);

        picker.select_row(0);
        assert_eq!(
            handle_device_selector_key(&mut picker, AppKeyCode::Enter),
            DeviceSelectorAction::Cancel
        );
    }

    fn orion_settings_controller(transport: Box<dyn Transport>) -> Controller {
        let entry = zen_go_tui::device::ProfileCatalog::builtin()
            .entries()
            .iter()
            .find(|entry| entry.id == "orion_studio_3")
            .expect("Orion entry")
            .clone();
        let driver = ProfileDriver::new(entry.clone()).expect("Orion profile driver");
        Controller::new_for_entry(transport, Box::new(driver), &entry).expect("controller")
    }

    fn auraverb_readback_fixture() -> Vec<u8> {
        include_str!(
            "../antelope-protocol/tests/fixtures/orion/auraverb/readback_mix1_poweroff_on2.hex"
        )
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("AuraVerb fixture byte"))
        .collect()
    }

    fn authoritative_auraverb_controller() -> (Controller, MockTransport) {
        let transport = MockTransport::default();
        let mut controller = orion_settings_controller(Box::new(transport.clone()));
        transport.push_read(auraverb_readback_fixture());
        assert!(controller
            .poll_device(Duration::ZERO)
            .expect("AuraVerb readback"));
        transport.take_writes();
        (controller, transport)
    }

    fn surround_readback_fixture() -> Vec<u8> {
        include_str!("../antelope-protocol/tests/fixtures/orion/surround_global_20_readback.hex")
            .split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).expect("Surround fixture byte"))
            .collect()
    }

    fn authoritative_surround_controller() -> (Controller, MockTransport) {
        let transport = MockTransport::default();
        let mut controller = orion_settings_controller(Box::new(transport.clone()));
        transport.push_read(surround_readback_fixture());
        assert!(controller
            .poll_device(Duration::ZERO)
            .expect("Surround readback"));
        transport.take_writes();
        (controller, transport)
    }

    struct DisconnectOnWriteTransport {
        read: Mutex<Option<Vec<u8>>>,
        write_attempts: Arc<AtomicUsize>,
    }

    impl Transport for DisconnectOnWriteTransport {
        fn write(&self, _data: &[u8]) -> Result<()> {
            self.write_attempts.fetch_add(1, Ordering::SeqCst);
            Err(anyhow::anyhow!(TransportError::DeviceDisconnected))
        }

        fn read(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
            Ok(self.read.lock().expect("disconnect read lock").take())
        }
    }

    #[test]
    fn auraverb_keyboard_covers_all_fields_toggle_bounds_focus_scroll_and_fkeys() {
        let area = ratatui::layout::Rect::new(0, 0, 60, 30);
        let (mut controller, transport) = authoritative_auraverb_controller();
        let initial = controller
            .state
            .displayed_auraverb_state()
            .expect("authoritative AuraVerb")
            .clone();

        handle_key_press(&mut controller, key(AppKeyCode::F(2)), area).expect("F2 AuraVerb");
        assert_eq!(controller.state.ui.page, UiPage::AuraVerb);
        assert_eq!(
            controller.state.ui.auraverb_focus,
            AuraVerbControlFocus::ALL[0]
        );
        for expected in AuraVerbControlFocus::ALL.into_iter().skip(1) {
            handle_key_press(&mut controller, key(AppKeyCode::Tab), area).expect("AuraVerb Tab");
            assert_eq!(controller.state.ui.auraverb_focus, expected);
        }
        assert!(controller.state.ui.auraverb_scroll > 0);
        handle_key_press(&mut controller, key(AppKeyCode::BackTab), area)
            .expect("AuraVerb BackTab");
        assert_eq!(
            controller.state.ui.auraverb_focus,
            AuraVerbControlFocus::Parameter(AuraVerbParameter::ReverbLevel)
        );

        for (code, expected) in [
            (
                AppKeyCode::Up,
                initial.reverb_level.saturating_add(1).min(100),
            ),
            (
                AppKeyCode::PageUp,
                initial.reverb_level.saturating_add(11).min(100),
            ),
            (AppKeyCode::Home, 0),
            (AppKeyCode::End, 100),
        ] {
            handle_key_press(&mut controller, key(code), area).expect("AuraVerb adjustment");
            assert_eq!(
                controller
                    .state
                    .displayed_auraverb_state()
                    .expect("pending AuraVerb")
                    .reverb_level,
                expected
            );
        }
        assert!(!transport.take_writes().is_empty());

        controller.state.ui.auraverb_focus = AuraVerbControlFocus::Enabled;
        let before_toggle = controller
            .state
            .displayed_auraverb_state()
            .expect("pending before toggle")
            .clone();
        handle_key_press(&mut controller, key(AppKeyCode::Enter), area)
            .expect("toggle AuraVerb enabled");
        let after_toggle = controller
            .state
            .displayed_auraverb_state()
            .expect("pending after toggle");
        assert_eq!(after_toggle.enabled, !before_toggle.enabled);
        for parameter in AuraVerbParameter::ALL {
            assert_eq!(
                after_toggle.value(parameter),
                before_toggle.value(parameter)
            );
        }
        assert!(!transport.take_writes().is_empty());

        handle_key_press(&mut controller, key(AppKeyCode::F(1)), area).expect("F1 Mixer");
        assert_eq!(controller.state.ui.page, UiPage::Mixer);
        handle_key_press(&mut controller, key(AppKeyCode::F(3)), area).expect("F3 Surround");
        assert_eq!(controller.state.ui.page, UiPage::Surround);

        let transport = MockTransport::default();
        let mut zen = Controller::new(
            Box::new(transport),
            Box::new(zen_go_tui::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("Zen Go controller");
        handle_key_press(&mut zen, key(AppKeyCode::F(2)), area).expect("unavailable F2");
        assert_eq!(zen.state.ui.page, UiPage::Mixer);
    }

    #[test]
    fn auraverb_ui_writes_preserve_peers_for_each_typed_field() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);
        for parameter in AuraVerbParameter::ALL {
            let (mut controller, transport) = authoritative_auraverb_controller();
            controller
                .apply_intent(Intent::SelectUiPage(UiPage::AuraVerb), area)
                .unwrap();
            let before = controller
                .state
                .displayed_auraverb_state()
                .expect("authoritative AuraVerb")
                .clone();
            controller.state.ui.auraverb_focus = AuraVerbControlFocus::Parameter(parameter);
            handle_key_press(&mut controller, key(AppKeyCode::End), area)
                .expect("typed field write");
            let after = controller
                .state
                .displayed_auraverb_state()
                .expect("pending AuraVerb");
            for peer in AuraVerbParameter::ALL {
                assert_eq!(
                    after.value(peer),
                    if peer == parameter {
                        100
                    } else {
                        before.value(peer)
                    },
                    "peer mismatch after {parameter:?}"
                );
            }
            assert_eq!(after.enabled, before.enabled);
            assert_eq!(transport.take_writes().len(), 2);
        }
    }

    #[test]
    fn auraverb_freshness_race_is_consumed_but_range_and_io_errors_propagate() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);
        let (mut stale, transport) = authoritative_auraverb_controller();
        stale
            .apply_intent(Intent::SelectUiPage(UiPage::AuraVerb), area)
            .unwrap();
        let intent = auraverb_adjust_intent(&stale.state, 1).expect("eligible intent");
        stale.state.auraverb.as_mut().unwrap().freshness =
            zen_go_tui::app::AuraVerbFreshness::Stale;
        stale.state.ui.auraverb_drag =
            Some(AuraVerbControlFocus::Parameter(AuraVerbParameter::Color));
        apply_interaction_intent(&mut stale, intent, area).expect("typed stale rejection consumed");
        assert!(transport.take_writes().is_empty());
        assert!(stale.state.ui.auraverb_drag.is_none());
        assert_eq!(
            stale.state.ui.last_message,
            AURAVERB_INTERACTION_UNAVAILABLE_MESSAGE
        );

        let (mut invalid, transport) = authoritative_auraverb_controller();
        let error = apply_interaction_intent(
            &mut invalid,
            Intent::SetAuraVerbParameter {
                parameter: AuraVerbParameter::Color,
                value: 101,
            },
            area,
        )
        .expect_err("range error propagates");
        assert!(!is_auraverb_write_unavailable(&error));
        assert!(!is_device_error(&error));
        assert!(transport.take_writes().is_empty());

        let write_attempts = Arc::new(AtomicUsize::new(0));
        let transport = DisconnectOnWriteTransport {
            read: Mutex::new(Some(auraverb_readback_fixture())),
            write_attempts: write_attempts.clone(),
        };
        let mut disconnected = orion_settings_controller(Box::new(transport));
        disconnected.poll_device(Duration::ZERO).unwrap();
        disconnected
            .apply_intent(Intent::SelectUiPage(UiPage::AuraVerb), area)
            .unwrap();
        let error = handle_key_press(&mut disconnected, key(AppKeyCode::Up), area)
            .expect_err("device error propagates");
        assert!(is_device_error(&error));
        assert!(!is_auraverb_write_unavailable(&error));
        assert_eq!(write_attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn auraverb_modal_keyboard_and_mouse_events_do_not_leak() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);
        for overlay in ["raw", "routing", "assignment", "options"] {
            let (mut controller, transport) = authoritative_auraverb_controller();
            controller
                .apply_intent(Intent::SelectUiPage(UiPage::AuraVerb), area)
                .unwrap();
            let point = (0..area.height)
                .find_map(|y| {
                    (0..area.width).find_map(|x| {
                        ui::auraverb_drag_target(area, &controller.state, x, y)
                            .is_some()
                            .then_some((x, y))
                    })
                })
                .expect("visible AuraVerb track");
            match overlay {
                "raw" => controller
                    .apply_intent(Intent::ToggleRawView, area)
                    .unwrap(),
                "routing" => controller
                    .apply_intent(Intent::OpenRoutingPopup, area)
                    .unwrap(),
                "assignment" => {
                    controller.state.popup.assignment_picker =
                        Some(zen_go_tui::app::AssignmentPickerState { strip: 1 });
                    controller.state.ui.auraverb_drag = None;
                }
                "options" => controller
                    .apply_intent(Intent::OpenOptionsPopup, area)
                    .unwrap(),
                _ => unreachable!(),
            }
            let focus = controller.state.ui.auraverb_focus;
            handle_key_press(&mut controller, key(AppKeyCode::Tab), area).unwrap();
            handle_mouse_event(
                area,
                &mut controller,
                AppMouseEvent {
                    kind: AppMouseEventKind::ScrollUp,
                    column: point.0,
                    row: point.1,
                    modifiers: AppModifiers::default(),
                },
            )
            .unwrap();
            assert_eq!(controller.state.ui.auraverb_focus, focus, "{overlay}");
            assert!(controller.state.ui.auraverb_drag.is_none(), "{overlay}");
            assert!(transport.take_writes().is_empty(), "{overlay}");
        }
    }

    #[test]
    fn auraverb_drag_lifetime_ends_on_release_page_modal_and_disconnect() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);
        let (mut controller, transport) = authoritative_auraverb_controller();
        controller
            .apply_intent(Intent::SelectUiPage(UiPage::AuraVerb), area)
            .unwrap();
        let geometry = ui::auraverb_drag_target;
        let (left, right, y) = (0..area.height)
            .find_map(|y| {
                let xs = (0..area.width)
                    .filter(|x| geometry(area, &controller.state, *x, y).is_some())
                    .collect::<Vec<_>>();
                (!xs.is_empty()).then(|| (xs[0], *xs.last().unwrap(), y))
            })
            .expect("visible AuraVerb track");
        let down = AppMouseEvent {
            kind: AppMouseEventKind::Down(AppMouseButton::Left),
            column: left,
            row: y,
            modifiers: AppModifiers::default(),
        };
        handle_mouse_event(area, &mut controller, down).expect("AuraVerb pointer down");
        assert_eq!(
            controller.state.ui.auraverb_drag,
            Some(AuraVerbControlFocus::Parameter(AuraVerbParameter::Color))
        );
        handle_mouse_event(
            area,
            &mut controller,
            AppMouseEvent {
                kind: AppMouseEventKind::Drag(AppMouseButton::Left),
                column: right,
                ..down
            },
        )
        .expect("AuraVerb drag");
        assert_eq!(
            controller
                .state
                .displayed_auraverb_state()
                .expect("pending AuraVerb")
                .color,
            100
        );
        assert!(!transport.take_writes().is_empty());
        handle_mouse_event(
            area,
            &mut controller,
            AppMouseEvent {
                kind: AppMouseEventKind::Up(AppMouseButton::Left),
                ..down
            },
        )
        .unwrap();
        assert!(controller.state.ui.auraverb_drag.is_none());

        controller.state.ui.auraverb_drag =
            Some(AuraVerbControlFocus::Parameter(AuraVerbParameter::Color));
        controller
            .apply_intent(Intent::OpenOptionsPopup, area)
            .unwrap();
        assert!(controller.state.ui.auraverb_drag.is_none());
        controller.state.popup.options_open = false;

        controller.state.ui.auraverb_drag =
            Some(AuraVerbControlFocus::Parameter(AuraVerbParameter::Color));
        controller
            .apply_intent(Intent::SelectUiPage(UiPage::Mixer), area)
            .unwrap();
        assert!(controller.state.ui.auraverb_drag.is_none());

        controller.state.ui.auraverb_drag =
            Some(AuraVerbControlFocus::Parameter(AuraVerbParameter::Color));
        controller.state.mark_disconnected();
        assert!(controller.state.ui.auraverb_drag.is_none());
    }

    #[test]
    fn surround_keyboard_navigation_adjustment_and_modal_isolation_are_page_local() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);
        let (mut controller, transport) = authoritative_surround_controller();
        assert_eq!(controller.state.ui.page, UiPage::Mixer);
        handle_key_press(&mut controller, key(AppKeyCode::F(3)), area).expect("F3 Surround");
        assert_eq!(controller.state.ui.page, UiPage::Surround);
        assert_eq!(
            controller.state.ui.surround_focus,
            SurroundControlFocus::Level
        );

        let keyboard = surround_adjust_intent(&controller.state, true);
        let wheel = (0..area.height).find_map(|y| {
            (0..area.width)
                .find_map(|x| ui::slider_wheel_action(area, &controller.state, x, y, true))
        });
        assert_eq!(keyboard, Some(Intent::SetSurroundGlobalLevel(601)));
        assert_eq!(wheel, keyboard);

        handle_key_press(&mut controller, key(AppKeyCode::Tab), area).expect("forward focus");
        assert_eq!(
            controller.state.ui.surround_focus,
            SurroundControlFocus::Delay
        );
        handle_key_press(&mut controller, key(AppKeyCode::BackTab), area).expect("reverse focus");
        assert_eq!(
            controller.state.ui.surround_focus,
            SurroundControlFocus::Level
        );

        handle_key_press(&mut controller, key(AppKeyCode::Up), area).expect("level up");
        assert_eq!(
            controller
                .state
                .surround_global
                .as_ref()
                .and_then(|cache| cache.pending_expected.as_ref())
                .map(|state| state.level_raw),
            Some(601)
        );
        assert!(!transport.take_writes().is_empty());

        controller.state.popup.options_open = true;
        let before = controller
            .state
            .surround_global
            .as_ref()
            .and_then(|cache| cache.pending_expected.as_ref())
            .map(|state| state.level_raw);
        handle_key_press(&mut controller, key(AppKeyCode::Up), area).expect("modal key");
        assert_eq!(
            controller
                .state
                .surround_global
                .as_ref()
                .and_then(|cache| cache.pending_expected.as_ref())
                .map(|state| state.level_raw),
            before
        );
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn raw_routing_and_assignment_keyboard_handlers_are_exhaustive() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);
        let dangerous = [
            AppKeyCode::Char('m'),
            AppKeyCode::F(1),
            AppKeyCode::Tab,
            AppKeyCode::Char('2'),
            AppKeyCode::Char('s'),
            AppKeyCode::Char('c'),
        ];

        let (mut raw, raw_transport) = authoritative_surround_controller();
        raw.apply_intent(Intent::SelectUiPage(UiPage::Surround), area)
            .expect("select Surround");
        raw.apply_intent(Intent::ToggleRawView, area)
            .expect("open raw view");
        let raw_focus = raw.state.ui.focus;
        let surround_focus = raw.state.ui.surround_focus;
        for code in dangerous {
            assert_eq!(
                handle_key_press(&mut raw, key(code), area).expect("raw consumes key"),
                KeyAction::Continue
            );
        }
        assert_eq!(raw.state.ui.page, UiPage::Surround);
        assert_eq!(raw.state.ui.focus, raw_focus);
        assert_eq!(raw.state.ui.surround_focus, surround_focus);
        assert!(raw_transport.take_writes().is_empty());
        handle_key_press(&mut raw, key(AppKeyCode::PageDown), area)
            .expect("raw page-down remains active");
        assert_eq!(raw.state.raw_view.raw_dump_scroll, 10);

        let (mut routing, routing_transport) = authoritative_surround_controller();
        routing
            .apply_intent(Intent::SelectUiPage(UiPage::Surround), area)
            .expect("select Surround");
        routing
            .apply_intent(Intent::OpenRoutingPopup, area)
            .expect("open routing");
        let routing_focus = routing.state.ui.focus;
        let surround_focus = routing.state.ui.surround_focus;
        for code in dangerous {
            assert_eq!(
                handle_key_press(&mut routing, key(code), area).expect("routing consumes key"),
                KeyAction::Continue
            );
        }
        assert_eq!(routing.state.ui.page, UiPage::Surround);
        assert_eq!(routing.state.ui.focus, routing_focus);
        assert_eq!(routing.state.ui.surround_focus, surround_focus);
        assert!(routing_transport.take_writes().is_empty());
        let channel = routing
            .state
            .popup
            .routing_editor
            .expect("routing editor")
            .channel;
        handle_key_press(&mut routing, key(AppKeyCode::Right), area)
            .expect("routing right remains active");
        assert_eq!(
            routing
                .state
                .popup
                .routing_editor
                .expect("routing editor")
                .channel,
            channel.saturating_add(1)
        );
        handle_key_press(&mut routing, key(AppKeyCode::Char('r')), area)
            .expect("routing shortcut closes popup");
        assert!(!routing.state.popup.routing_open);

        let (mut assignment, assignment_transport) = authoritative_surround_controller();
        assignment
            .apply_intent(Intent::SelectUiPage(UiPage::Surround), area)
            .expect("select Surround");
        assignment.state.popup.assignment_picker =
            Some(zen_go_tui::app::AssignmentPickerState { strip: 1 });
        assignment.state.popup.selected_index = 0;
        let assignment_focus = assignment.state.ui.focus;
        let surround_focus = assignment.state.ui.surround_focus;
        for code in dangerous {
            assert_eq!(
                handle_key_press(&mut assignment, key(code), area)
                    .expect("assignment consumes key"),
                KeyAction::Continue
            );
        }
        assert_eq!(assignment.state.ui.page, UiPage::Surround);
        assert_eq!(assignment.state.ui.focus, assignment_focus);
        assert_eq!(assignment.state.ui.surround_focus, surround_focus);
        assert!(assignment_transport.take_writes().is_empty());
        handle_key_press(&mut assignment, key(AppKeyCode::Down), area)
            .expect("assignment down remains active");
        assert_eq!(assignment.state.popup.selected_index, 1);
        handle_key_press(&mut assignment, key(AppKeyCode::Esc), area)
            .expect("assignment escape remains active");
        assert!(assignment.state.popup.assignment_picker.is_none());
    }

    #[test]
    fn stale_surround_interactions_are_consumed_without_writes() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);

        for origin in ["keyboard", "drag", "wheel"] {
            let (mut controller, transport) = authoritative_surround_controller();
            controller
                .apply_intent(Intent::SelectUiPage(UiPage::Surround), area)
                .expect("select Surround");
            let intent = match origin {
                "keyboard" => surround_adjust_intent(&controller.state, true),
                "drag" => {
                    controller.state.ui.surround_drag = Some(SurroundControlFocus::Level);
                    (0..area.height).find_map(|y| {
                        (0..area.width)
                            .find_map(|x| ui::slider_mouse_action(area, &controller.state, x, y))
                    })
                }
                "wheel" => (0..area.height).find_map(|y| {
                    (0..area.width)
                        .find_map(|x| ui::slider_wheel_action(area, &controller.state, x, y, true))
                }),
                _ => unreachable!(),
            }
            .expect("interaction is eligible before freshness changes");
            controller
                .state
                .surround_global
                .as_mut()
                .expect("Surround cache")
                .freshness = zen_go_tui::app::SurroundFreshness::Stale;
            controller.state.ui.surround_drag = Some(SurroundControlFocus::Level);

            apply_interaction_intent(&mut controller, intent, area)
                .expect("stale interaction is consumed");

            assert!(transport.take_writes().is_empty(), "{origin}");
            assert_eq!(
                controller
                    .state
                    .surround_global
                    .as_ref()
                    .expect("Surround cache")
                    .freshness,
                zen_go_tui::app::SurroundFreshness::Stale,
                "{origin}"
            );
            assert!(controller.state.ui.surround_drag.is_none(), "{origin}");
            assert_eq!(
                controller.state.ui.last_message, SURROUND_INTERACTION_UNAVAILABLE_MESSAGE,
                "{origin}"
            );
        }
    }

    #[test]
    fn surround_interaction_does_not_swallow_range_or_device_errors() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);
        let (mut invalid, transport) = authoritative_surround_controller();
        let error =
            apply_interaction_intent(&mut invalid, Intent::SetSurroundGlobalLevel(u16::MAX), area)
                .expect_err("invalid driver range must propagate");
        assert!(!is_surround_write_unavailable(&error));
        assert!(!is_device_error(&error));
        assert!(transport.take_writes().is_empty());

        let write_attempts = Arc::new(AtomicUsize::new(0));
        let transport = DisconnectOnWriteTransport {
            read: Mutex::new(Some(surround_readback_fixture())),
            write_attempts: write_attempts.clone(),
        };
        let mut disconnected = orion_settings_controller(Box::new(transport));
        disconnected
            .poll_device(Duration::ZERO)
            .expect("authoritative Surround readback");
        disconnected
            .apply_intent(Intent::SelectUiPage(UiPage::Surround), area)
            .expect("select Surround");

        let error = handle_key_press(&mut disconnected, key(AppKeyCode::Up), area)
            .expect_err("device failure must propagate to the existing runtime disconnect path");
        assert!(is_device_error(&error));
        assert!(!is_surround_write_unavailable(&error));
        assert_eq!(write_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(
            disconnected
                .state
                .surround_global
                .as_ref()
                .expect("Surround cache")
                .freshness,
            zen_go_tui::app::SurroundFreshness::Stale
        );
    }

    #[test]
    fn surround_drag_lifetime_ends_on_release_page_modal_and_disconnect() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);
        let (mut controller, _) = authoritative_surround_controller();
        controller
            .apply_intent(Intent::SelectUiPage(UiPage::Surround), area)
            .expect("select Surround");
        let (x, y) = (0..area.height)
            .find_map(|y| {
                (0..area.width).find_map(|x| {
                    matches!(
                        ui::mouse_action(area, &controller.state, x, y),
                        Some(Intent::SetSurroundGlobalLevel(_))
                    )
                    .then_some((x, y))
                })
            })
            .expect("visible level track");
        let down = AppMouseEvent {
            kind: AppMouseEventKind::Down(AppMouseButton::Left),
            column: x,
            row: y,
            modifiers: AppModifiers::default(),
        };
        handle_mouse_event(area, &mut controller, down).expect("start drag");
        assert_eq!(
            controller.state.ui.surround_drag,
            Some(SurroundControlFocus::Level)
        );
        handle_mouse_event(
            area,
            &mut controller,
            AppMouseEvent {
                kind: AppMouseEventKind::Up(AppMouseButton::Left),
                ..down
            },
        )
        .expect("release drag");
        assert!(controller.state.ui.surround_drag.is_none());

        controller.state.ui.surround_drag = Some(SurroundControlFocus::Delay);
        controller
            .apply_intent(Intent::OpenOptionsPopup, area)
            .expect("open modal");
        assert!(controller.state.ui.surround_drag.is_none());
        controller.state.popup.options_open = false;

        controller.state.ui.surround_drag = Some(SurroundControlFocus::Level);
        controller
            .apply_intent(Intent::SelectUiPage(UiPage::Mixer), area)
            .expect("select Mixer");
        assert!(controller.state.ui.surround_drag.is_none());

        controller.state.ui.surround_drag = Some(SurroundControlFocus::Level);
        controller.state.mark_disconnected();
        assert!(controller.state.ui.surround_drag.is_none());
    }

    #[test]
    fn raw_and_modal_mouse_events_cannot_reach_hidden_surround_controls() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);

        for overlay in ["raw", "routing", "assignment", "options"] {
            for kind in [
                AppMouseEventKind::Down(AppMouseButton::Left),
                AppMouseEventKind::Drag(AppMouseButton::Left),
                AppMouseEventKind::ScrollUp,
            ] {
                let (mut controller, transport) = authoritative_surround_controller();
                controller
                    .apply_intent(Intent::SelectUiPage(UiPage::Surround), area)
                    .expect("select Surround");
                let (x, y) = (0..area.height)
                    .find_map(|y| {
                        (0..area.width).find_map(|x| {
                            ui::surround_drag_target(area, &controller.state, x, y)
                                .is_some()
                                .then_some((x, y))
                        })
                    })
                    .expect("visible Surround track");
                controller.state.ui.surround_drag = Some(SurroundControlFocus::Delay);
                match overlay {
                    "raw" => controller
                        .apply_intent(Intent::ToggleRawView, area)
                        .expect("open raw view"),
                    "routing" => controller
                        .apply_intent(Intent::OpenRoutingPopup, area)
                        .expect("open routing"),
                    "assignment" => {
                        controller.state.popup.assignment_picker =
                            Some(zen_go_tui::app::AssignmentPickerState { strip: 1 });
                        controller.state.ui.surround_drag = None;
                    }
                    "options" => controller
                        .apply_intent(Intent::OpenOptionsPopup, area)
                        .expect("open options"),
                    _ => unreachable!(),
                }
                assert!(controller.state.ui.surround_drag.is_none(), "{overlay}");
                assert_eq!(
                    ui::surround_drag_target(area, &controller.state, x, y),
                    None,
                    "{overlay}"
                );

                handle_mouse_event(
                    area,
                    &mut controller,
                    AppMouseEvent {
                        kind,
                        column: x,
                        row: y,
                        modifiers: AppModifiers::default(),
                    },
                )
                .expect("overlay consumes hidden control event");

                assert!(controller.state.ui.surround_drag.is_none(), "{overlay}");
                assert!(transport.take_writes().is_empty(), "{overlay}: {kind:?}");
            }
        }

        let header_area = ratatui::layout::Rect::new(0, 0, 160, 50);
        let transport = MockTransport::default();
        let mut raw = Controller::new(
            Box::new(transport.clone()),
            Box::new(zen_go_tui::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("Zen Go controller");
        raw.state.device.status.clock_source =
            (-16..=16).find(|value| raw.state.ui_profile.clock_source_is_internal(Some(*value)));
        assert!(raw.state.device.status.clock_source.is_some());
        let (sample_x, sample_y) = (0..header_area.height)
            .find_map(|y| {
                (0..header_area.width).find_map(|x| {
                    matches!(
                        ui::mouse_action(header_area, &raw.state, x, y),
                        Some(Intent::OpenSampleRateSelector)
                    )
                    .then_some((x, y))
                })
            })
            .expect("visible sample-rate control");
        raw.apply_intent(Intent::ToggleRawView, header_area)
            .expect("open raw view");
        handle_mouse_event(
            header_area,
            &mut raw,
            AppMouseEvent {
                kind: AppMouseEventKind::Down(AppMouseButton::Left),
                column: sample_x,
                row: sample_y,
                modifiers: AppModifiers::default(),
            },
        )
        .expect("raw view consumes hidden sample-rate click");
        assert!(raw.state.popup.selector_popup.is_none());
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn zen_go_f3_is_absent_and_mixer_surface_shortcuts_remain_unchanged() {
        let area = ratatui::layout::Rect::new(0, 0, 120, 30);
        let transport = MockTransport::default();
        let (driver, actions) = RecordingDriver::new();
        let entry = zen_go_tui::device::ProfileCatalog::builtin()
            .entries()
            .iter()
            .find(|entry| entry.id == "zen_go_sc")
            .expect("Zen Go entry")
            .clone();
        let mut controller =
            Controller::new_for_entry(Box::new(transport), Box::new(driver), &entry)
                .expect("controller");

        handle_key_press(&mut controller, key(AppKeyCode::F(3)), area).expect("ignored F3");
        assert_eq!(controller.state.ui.page, UiPage::Mixer);
        handle_key_press(&mut controller, key(AppKeyCode::Char('2')), area)
            .expect("Mixer 2 shortcut");
        assert_eq!(controller.state.mixer.surface_index, 1);
        handle_key_press(&mut controller, key(AppKeyCode::Char('1')), area)
            .expect("Mixer 1 shortcut");
        assert_eq!(controller.state.mixer.surface_index, 0);
        assert!(actions.lock().expect("recorded actions").is_empty());
    }

    #[derive(Default)]
    struct FailingSettingsTransport;

    impl Transport for FailingSettingsTransport {
        fn write(&self, _data: &[u8]) -> Result<()> {
            anyhow::bail!("synthetic settings write failure")
        }

        fn read(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn is_available(&self) -> Result<bool> {
            Ok(true)
        }
    }

    #[test]
    fn settings_selectors_are_modal_and_escape_restores_parent_selection() {
        let mut controller = orion_settings_controller(Box::new(MockTransport::default()));
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        controller
            .apply_intent(Intent::OpenSettingsSelector, area)
            .expect("open settings");
        controller.state.popup.selected_index = 3;
        activate_popup_selection(&mut controller).expect("open trim target 2");
        assert_eq!(
            controller.state.popup.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::OutputTrim { target: 2 }
            })
        );
        handle_key_press(&mut controller, key(AppKeyCode::Esc), area).expect("back to settings");
        assert_eq!(
            controller.state.popup.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::Settings
            })
        );
        assert_eq!(controller.state.popup.selected_index, 3);
        handle_key_press(&mut controller, key(AppKeyCode::Esc), area).expect("close settings");
        assert!(controller.state.popup.selector_popup.is_none());
    }

    #[test]
    fn settings_values_become_authoritative_only_from_snapshot_readback() {
        let transport = MockTransport::default();
        let mut controller = orion_settings_controller(Box::new(transport.clone()));
        let mut raw = include_str!("../antelope-protocol/tests/fixtures/orion/state_report_73.hex")
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .flat_map(|line| line.split_whitespace())
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect::<Vec<_>>();
        raw[24] = 0x60;
        raw[25] = 0x98;
        raw[26] = 75;
        raw[73] = 3;
        transport.push_read(raw);

        assert!(controller
            .poll_device(Duration::ZERO)
            .expect("snapshot poll"));
        assert_eq!(
            controller.state.global_value(GlobalControl::Brightness),
            Some(75)
        );
        assert_eq!(
            controller
                .state
                .global_value(GlobalControl::OutputTrim(OutputTrimAddress { target: 0 })),
            Some(6)
        );
        assert_eq!(
            controller
                .state
                .global_value(GlobalControl::OutputTrim(OutputTrimAddress { target: 1 })),
            Some(6)
        );
        assert_eq!(
            controller
                .state
                .global_value(GlobalControl::OutputTrim(OutputTrimAddress { target: 2 })),
            Some(4)
        );
        assert_eq!(
            controller
                .state
                .global_bool_value(GlobalControl::TalkbackButton),
            Some(false)
        );
        assert_eq!(
            controller
                .state
                .global_value(GlobalControl::TalkbackSourceResidue),
            Some(3)
        );
        assert_eq!(
            controller.state.global_value(GlobalControl::TalkbackSource),
            None
        );
        controller.state.popup.selected_index = 9;
        controller
            .apply_intent(
                Intent::OpenTalkbackSourceSelector,
                ratatui::layout::Rect::default(),
            )
            .expect("open partial source selector");
        assert_eq!(
            controller.state.popup.selected_index, 0,
            "modulo residue must never become one of the 13 selected source indices"
        );
        assert_eq!(
            controller.state.global_value(GlobalControl::TalkbackGain),
            Some(0)
        );
    }

    #[test]
    fn talkback_source_and_gain_write_only_within_confirmed_bounds() {
        let transport = MockTransport::default();
        let mut controller = orion_settings_controller(Box::new(transport.clone()));
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        controller
            .apply_intent(Intent::PickTalkbackSource(12), area)
            .expect("queue source");
        controller
            .apply_intent(Intent::PickTalkbackGain(96), area)
            .expect("queue gain");
        controller.flush_commands().expect("talkback writes");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(&writes[0][16..18], &[0x27, 12]);
        assert_eq!(&writes[1][16..18], &[0x20, 96]);
        assert!(writes
            .iter()
            .all(|frame| { frame.len() == 320 && frame[0] == 0x70 && frame[4] == 0x12 }));
        for intent in [Intent::PickTalkbackSource(13), Intent::PickTalkbackGain(97)] {
            controller.apply_intent(intent, area).expect("reject bound");
        }
        controller.flush_commands().expect("nothing outside bounds");
        assert!(transport.take_writes().is_empty());
        assert_eq!(
            controller.state.global_value(GlobalControl::TalkbackSource),
            None
        );
        assert_eq!(
            controller.state.global_value(GlobalControl::TalkbackGain),
            None
        );
    }

    #[test]
    fn zen_go_talkback_intents_produce_no_writes() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(
            Box::new(transport.clone()),
            Box::new(zen_go_tui::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("Zen Go controller");
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        for intent in [
            Intent::SetTalkbackButton(false),
            Intent::PickTalkbackSource(0),
            Intent::PickTalkbackGain(0),
        ] {
            controller
                .apply_intent(intent, area)
                .expect("ignored intent");
        }
        controller.flush_commands().expect("empty queue");
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn talkback_button_is_hold_to_talk_and_best_effort_releases_on_key_up_cancel_and_quit() {
        let transport = MockTransport::default();
        let mut controller = orion_settings_controller(Box::new(transport.clone()));
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        controller
            .apply_intent(Intent::OpenSettingsSelector, area)
            .expect("open settings");
        controller.state.popup.selected_index = 4;
        activate_popup_selection(&mut controller).expect("open talkback button");
        assert_eq!(
            controller.state.popup.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::TalkbackButton
            })
        );

        handle_key_press(&mut controller, key(AppKeyCode::Enter), area)
            .expect("unsupported keyboard hold is ignored");
        assert!(!controller.state.popup.talkback_button_held);
        assert!(transport.take_writes().is_empty());

        controller.state.ui.keyboard_release_events_enabled = true;
        handle_key_press(&mut controller, key(AppKeyCode::Enter), area).expect("press");
        assert!(controller.state.popup.talkback_button_held);
        handle_key_press(
            &mut controller,
            AppKeyEvent {
                code: AppKeyCode::Enter,
                modifiers: AppModifiers::default(),
                kind: AppKeyEventKind::Release,
            },
            area,
        )
        .expect("key-up release");
        assert!(!controller.state.popup.talkback_button_held);

        handle_key_press(&mut controller, key(AppKeyCode::Enter), area).expect("press again");
        handle_key_press(&mut controller, key(AppKeyCode::Esc), area).expect("cancel release");
        assert!(!controller.state.popup.talkback_button_held);
        assert_eq!(
            controller.state.popup.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::Settings
            })
        );

        controller.state.popup.selected_index = 4;
        activate_popup_selection(&mut controller).expect("reopen talkback button");
        handle_key_press(&mut controller, key(AppKeyCode::Enter), area).expect("press for quit");
        assert_eq!(
            handle_key_press(&mut controller, key(AppKeyCode::Char('q')), area)
                .expect("quit release"),
            KeyAction::Quit
        );
        assert!(!controller.state.popup.talkback_button_held);

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 6);
        assert_eq!(
            writes.iter().map(|frame| frame[17]).collect::<Vec<_>>(),
            vec![1, 0, 1, 0, 1, 0]
        );
        assert!(writes.iter().all(|frame| {
            frame.len() == 320 && frame[0] == 0x70 && frame[4] == 0x12 && frame[16] == 0x1f
        }));
    }

    #[test]
    fn talkback_mouse_hold_remains_available_without_keyboard_release_events() {
        let transport = MockTransport::default();
        let mut controller = orion_settings_controller(Box::new(transport.clone()));
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        controller
            .apply_intent(Intent::OpenTalkbackButton, area)
            .expect("open talkback button");
        assert!(!controller.state.ui.keyboard_release_events_enabled);

        handle_mouse_event(
            area,
            &mut controller,
            AppMouseEvent {
                kind: AppMouseEventKind::Down(AppMouseButton::Left),
                column: 20,
                row: 2,
                modifiers: AppModifiers::default(),
            },
        )
        .expect("mouse press");
        assert!(controller.state.popup.talkback_button_held);
        handle_mouse_event(
            area,
            &mut controller,
            AppMouseEvent {
                kind: AppMouseEventKind::Up(AppMouseButton::Left),
                column: 20,
                row: 2,
                modifiers: AppModifiers::default(),
            },
        )
        .expect("mouse release");
        assert!(!controller.state.popup.talkback_button_held);
        assert_eq!(
            transport
                .take_writes()
                .iter()
                .map(|frame| frame[17])
                .collect::<Vec<_>>(),
            vec![1, 0]
        );
    }

    #[test]
    fn explicit_talkback_release_does_not_depend_on_observed_button_state() {
        let transport = MockTransport::default();
        let mut controller = orion_settings_controller(Box::new(transport.clone()));
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        controller
            .apply_intent(Intent::PickTalkbackSource(1), area)
            .expect("queue unrelated setting");
        controller
            .apply_intent(Intent::OpenTalkbackButton, area)
            .expect("open button without readback");
        controller.state.popup.selected_index = 1;
        activate_popup_selection(&mut controller).expect("explicit release");
        let writes = transport.take_writes();
        assert_eq!(
            writes.len(),
            1,
            "release bypasses the bounded settings queue"
        );
        assert_eq!(&writes[0][16..18], &[0x1f, 0]);
        controller.flush_commands().expect("later source write");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][16..18], &[0x27, 1]);
    }

    #[test]
    fn settings_writes_never_become_authoritative_without_readback() {
        let mut controller = orion_settings_controller(Box::new(FailingSettingsTransport));
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        assert_eq!(
            controller.state.global_value(GlobalControl::Brightness),
            None
        );
        controller
            .apply_intent(Intent::PickBrightness(75), area)
            .expect("queue brightness");
        assert_eq!(
            controller.state.global_value(GlobalControl::Brightness),
            None
        );
        assert!(controller.flush_commands().is_err());
        assert_eq!(
            controller.state.global_value(GlobalControl::Brightness),
            None
        );

        let mut controller = orion_settings_controller(Box::new(FailingSettingsTransport));
        let trim = GlobalControl::OutputTrim(OutputTrimAddress { target: 1 });
        controller
            .apply_intent(
                Intent::PickOutputTrim {
                    address: OutputTrimAddress { target: 1 },
                    value: 4,
                },
                area,
            )
            .expect("queue trim");
        assert_eq!(controller.state.global_value(trim), None);
        assert!(controller.flush_commands().is_err());
        assert_eq!(controller.state.global_value(trim), None);

        let transport = MockTransport::default();
        let mut controller = orion_settings_controller(Box::new(transport.clone()));
        controller
            .apply_intent(Intent::PickBrightness(75), area)
            .expect("queue valid brightness");
        controller
            .apply_intent(
                Intent::PickOutputTrim {
                    address: OutputTrimAddress { target: 1 },
                    value: 4,
                },
                area,
            )
            .expect("queue valid trim");
        controller.flush_commands().expect("settings write");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0][4], 0x12);
        assert_eq!(&writes[0][16..18], &[0x0e, 75]);
        assert_eq!(writes[1][4], 0x13);
        assert_eq!(&writes[1][16..19], &[0x4b, 1, 4]);
        assert_eq!(
            controller.state.global_value(GlobalControl::Brightness),
            None
        );
        assert_eq!(
            controller
                .state
                .global_value(GlobalControl::OutputTrim(OutputTrimAddress { target: 1 })),
            None
        );

        controller
            .apply_intent(Intent::PickBrightness(101), area)
            .expect("invalid brightness is ignored");
        controller
            .apply_intent(
                Intent::PickOutputTrim {
                    address: OutputTrimAddress { target: 3 },
                    value: 0,
                },
                area,
            )
            .expect("invalid trim target is ignored");
        controller.flush_commands().expect("no queued settings");
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn selector_modal_poll_drains_frames_without_flushing_pending_commands() {
        let transport = MockTransport::default();
        let (driver, _actions) = RecordingDriver::new();
        let mut controller =
            Controller::new(Box::new(transport.clone()), Box::new(driver)).expect("controller");
        controller
            .send(
                Action::SetGlobal {
                    control: GlobalControl::ClockSource,
                    value: ControlValue::Enum(0),
                },
                None,
            )
            .expect("queue command");
        transport.push_read(vec![0x75]);

        poll_controller_for_runtime(&mut controller, true, std::time::Duration::ZERO)
            .expect("selector poll");

        assert!(transport.take_writes().is_empty());
        assert!(transport
            .read(std::time::Duration::ZERO)
            .expect("remaining transport read")
            .is_none());
    }

    #[test]
    fn selector_modal_paste_does_not_mutate_hidden_profile_editor() {
        let mut controller = Controller::new(
            Box::new(MockTransport::default()),
            Box::new(zen_go_tui::device::builtin_zen_go_driver().expect("Zen Go driver")),
        )
        .expect("controller");
        controller.state.popup.profile_editor = Some(zen_go_tui::app::ProfileEditorState {
            mode: zen_go_tui::app::ProfileEditorMode::Save,
            original_name: None,
            value: "existing".to_string(),
        });

        handle_profile_editor_paste(&mut controller, true, "pasted");

        assert_eq!(
            controller
                .state
                .popup
                .profile_editor
                .as_ref()
                .expect("profile editor")
                .value,
            "existing"
        );
    }

    #[test]
    fn empty_mixer_pan_keys_are_stable_no_ops() {
        for code in [AppKeyCode::Char('['), AppKeyCode::Char(']')] {
            let transport = MockTransport::default();
            let mut controller = Controller::new(
                Box::new(transport.clone()),
                Box::new(zen_go_tui::device::builtin_zen_go_driver().expect("Zen Go driver")),
            )
            .expect("controller");
            controller.state.ui.focus = FocusArea::Mixer;
            controller.state.mixer.surfaces.clear();
            controller.state.mixer.channels.clear();
            controller.state.mixer.surface_index = usize::MAX;
            controller.state.mixer.selected_channel = usize::MAX;
            controller.state.mixer.strip_scroll = usize::MAX;
            let selected = controller.state.mixer.selected_channel;
            let scroll = controller.state.mixer.strip_scroll;
            let message = controller.state.ui.last_message.clone();

            assert!(handle_key_press(
                &mut controller,
                key(code),
                ratatui::layout::Rect::new(0, 0, 120, 50),
            )
            .is_ok());
            assert_eq!(controller.state.mixer.selected_channel, selected);
            assert_eq!(controller.state.mixer.strip_scroll, scroll);
            assert_eq!(controller.state.ui.last_message, message);
            assert!(transport.take_writes().is_empty());
        }
    }

    #[test]
    fn synthetic_profile_keyboard_reaches_third_input_and_fourth_output() {
        let entry = synthetic_entry();
        let transport = MockTransport::default();
        let (driver, actions) = RecordingDriver::new();
        let mut controller =
            Controller::new_for_entry(Box::new(transport.clone()), Box::new(driver), &entry)
                .expect("synthetic controller");
        let area = ratatui::layout::Rect::new(0, 0, 120, 50);

        controller.state.ui.focus = FocusArea::Outputs;
        assert_eq!(controller.state.output.selected, 0);
        for _ in 0..3 {
            handle_key_press(&mut controller, key(AppKeyCode::Right), area)
                .expect("navigate output selection");
        }
        assert_eq!(controller.state.output.selected, 3);
        assert_eq!(controller.state.outputs()[3].name, "Output 4");

        controller.state.ui.focus = FocusArea::Preamp;
        assert_eq!(controller.state.preamp.selected_input, 0);
        for _ in 0..2 {
            handle_key_press(&mut controller, key(AppKeyCode::Right), area)
                .expect("navigate preamp selection");
        }
        assert_eq!(controller.state.preamp.selected_input, 2);
        let third_input_address = controller
            .state
            .inputs_for_space("physical_inputs")
            .get(2)
            .expect("third physical input")
            .address;

        handle_key_press(&mut controller, key(AppKeyCode::Char('3')), area)
            .expect("open third-input mode selector");
        assert_eq!(
            controller.state.popup.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::PreampMode { input: 2 }
            })
        );
        let third_input = controller
            .state
            .inputs_for_space("physical_inputs")
            .iter()
            .find(|input| input.address == third_input_address)
            .expect("addressed third physical input");
        assert_eq!(third_input.address, third_input_address);
        assert_eq!(third_input_address.space, 0);
        assert_eq!(third_input_address.index, 2);
        assert!(transport.take_writes().is_empty());

        handle_key_press(&mut controller, key(AppKeyCode::Enter), area)
            .expect("activate third-input mode selector");
        assert!(transport.take_writes().is_empty());
        let action = actions
            .lock()
            .expect("recording driver actions")
            .last()
            .cloned()
            .expect("third-input mode action");
        assert_eq!(
            action,
            Action::SetInput {
                address: InputAddress { space: 0, index: 2 },
                control: InputControl::Mode,
                value: ControlValue::Enum(0),
            }
        );
    }
}
