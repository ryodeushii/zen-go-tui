use std::io;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use antelope_protocol::{
    Action, ControlValue, GlobalControl, InputControl, MixerAddress, MixerAssignment, PreampMode,
    SampleRate,
};
use zen_go_tui::app::{
    Controller, FocusArea, Intent, PeakHoldDuration, RefreshRate, SelectorPopupKind,
    SelectorPopupState,
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
        AppKeyCode::Up
            if controller.state.popup.routing_open
                && controller.state.popup.routing_editor.is_some() =>
        {
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
            let destination =
                controller.state.routing_capabilities[index.saturating_sub(1)].destination;
            controller.apply_intent(Intent::SelectRoutingDestination { destination }, area)?;
            Ok(())
        }
        AppKeyCode::Down
            if controller.state.popup.routing_open
                && controller.state.popup.routing_editor.is_some() =>
        {
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
            let next = index.saturating_add(1).min(
                controller
                    .state
                    .routing_capabilities
                    .len()
                    .saturating_sub(1),
            );
            let destination = controller.state.routing_capabilities[next].destination;
            controller.apply_intent(Intent::SelectRoutingDestination { destination }, area)?;
            Ok(())
        }
        AppKeyCode::Left
            if controller.state.popup.routing_open
                && controller.state.popup.routing_editor.is_some() =>
        {
            let editor = controller
                .state
                .popup
                .routing_editor
                .expect("guarded above");
            controller.apply_intent(
                Intent::SelectRoutingChannel {
                    destination: editor.destination,
                    channel: editor.channel.saturating_sub(1),
                },
                area,
            )?;
            Ok(())
        }
        AppKeyCode::Right
            if controller.state.popup.routing_open
                && controller.state.popup.routing_editor.is_some() =>
        {
            let editor = controller
                .state
                .popup
                .routing_editor
                .expect("guarded above");
            let last = controller
                .state
                .routing_capabilities
                .iter()
                .find(|group| group.destination == editor.destination)
                .map_or(0, |group| group.channel_count.saturating_sub(1));
            controller.apply_intent(
                Intent::SelectRoutingChannel {
                    destination: editor.destination,
                    channel: editor.channel.saturating_add(1).min(last),
                },
                area,
            )?;
            Ok(())
        }
        AppKeyCode::Enter
            if controller.state.popup.routing_open
                && controller.state.popup.routing_editor.is_some() =>
        {
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
                )?;
            } else {
                controller.state.ui.last_message =
                    "Routing source is unavailable until complete readback arrives".to_string();
            }
            Ok(())
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
        AppKeyCode::Char('a') => {
            if controller.state.ui.focus == FocusArea::Mixer {
                if let Some(address) = controller
                    .state
                    .active_mixer_surface()
                    .and_then(|index| controller.state.mixers().get(index))
                    .and_then(|surface| surface.strips.get(controller.state.mixer.selected_channel))
                    .map(|strip| MixerAddress {
                        surface: controller
                            .state
                            .active_mixer_surface()
                            .and_then(|index| controller.state.mixers().get(index))
                            .map_or(0, |surface| surface.surface),
                        strip: strip.strip,
                    })
                {
                    if controller
                        .state
                        .ui_profile
                        .supports_assignment(address.surface, address.strip)
                    {
                        controller
                            .apply_intent(Intent::OpenAssignmentPickerAt { address }, area)?;
                    } else {
                        controller.state.ui.last_message =
                            "Routing assignment is unsupported for the selected strip.".into();
                    }
                }
            }
            Ok(())
        }
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
            controller.state.popup.routing_editor = None;
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
    !state.popup.raw_view_open
        && matches!(mouse.kind, AppMouseEventKind::Down(AppMouseButton::Left))
        && ui::device_header_name_hit(area, state, mouse.column, mouse.row)
}

fn open_device_selector_for_runtime(
    runtime: &mut RuntimeDeviceState,
    controller: &mut Controller,
    active_candidate: Option<&DeviceCandidate>,
) {
    if let Err(error) = runtime.open_selector_for(active_candidate.cloned()) {
        controller.state.ui.last_message = format!("Device selector unavailable: {error}");
    }
}

fn handle_device_selector_hotkey(
    runtime: &mut RuntimeDeviceState,
    controller: &mut Controller,
    active_candidate: Option<&DeviceCandidate>,
    key: AppKeyCode,
) -> bool {
    if key != AppKeyCode::F(2) {
        return false;
    }
    open_device_selector_for_runtime(runtime, controller, active_candidate);
    true
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

                            if handle_device_selector_hotkey(
                                runtime,
                                controller,
                                active_candidate,
                                key.code,
                            ) {
                                controller.release_talkback_if_held()?;
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
            if let Some(action) = ui::mouse_action(area, &controller.state, mouse.column, mouse.row)
            {
                controller.apply_intent(action, area)?;
            }
        }
        AppMouseEventKind::Up(AppMouseButton::Left) => {
            controller.release_talkback_if_held()?;
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
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use antelope_protocol::{
        CommandBatch, DeviceDriver, DeviceEvent, DriverDefinition, DriverError, GlobalControl,
        InputAddress, OutputTrimAddress, ProfileDriver, QueryRequest,
    };
    use ratatui::{backend::TestBackend, Terminal};
    use zen_go_tui::terminal::AppModifiers;
    use zen_go_tui::transport::{MockTransport, Transport};

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
    fn runtime_selector_activation_ignores_raw_view_header_click_but_keeps_visible_title_and_f2() {
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

        assert!(handle_device_selector_hotkey(
            &mut devices,
            &mut controller,
            None,
            AppKeyCode::F(2),
        ));
        assert!(devices.selector().is_some());
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
