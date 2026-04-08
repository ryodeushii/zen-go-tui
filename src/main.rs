use std::io;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use zen_go_tui::app::{Controller, FocusArea, MainPage, SelectorPopupKind, SelectorPopupState};
use zen_go_tui::protocol::{
    ClockSource, Command, MixerAssignment, MixerSurface, OutputMode, OutputTarget, PanState,
    PreampMode, SampleRate, Surface,
};
use zen_go_tui::terminal::{
    self, AppInputEvent, AppKeyCode, AppKeyEventKind, AppMouseButton, AppMouseEvent,
    AppMouseEventKind,
};
use zen_go_tui::transport::{is_device_error, HidTransport, MockTransport, Transport};
use zen_go_tui::ui;

#[derive(Parser, Debug)]
#[command(author, version, about = "Zen Go Synergy Core terminal control panel")]
struct Cli {
    #[arg(long)]
    mock: bool,
}

const ZEN_GO_VID: u16 = 0x23e5;
const ZEN_GO_PID: u16 = 0xa015;
const DEVICE_RETRY_INTERVAL: Duration = Duration::from_millis(500);

fn main() -> Result<()> {
    let cli = Cli::parse();
    let transport: Box<dyn Transport> = if cli.mock {
        Box::new(MockTransport::default())
    } else {
        wait_for_transport(
            || Ok(Box::new(HidTransport::open(ZEN_GO_VID, ZEN_GO_PID)?) as Box<dyn Transport>),
            |attempt, _| {
                if attempt == 1 {
                    eprintln!("Waiting for Zen Go device...");
                }
                thread::sleep(DEVICE_RETRY_INTERVAL);
                Ok(())
            },
        )?
    };

    run_app(transport)
}

fn wait_for_transport<T, F, R>(mut open: F, mut on_retry: R) -> Result<T>
where
    F: FnMut() -> Result<T>,
    R: FnMut(usize, &anyhow::Error) -> Result<()>,
{
    let mut retries = 0;

    loop {
        match open() {
            Ok(transport) => return Ok(transport),
            Err(error) if is_device_error(&error) => {
                retries += 1;
                on_retry(retries, &error)?;
            }
            Err(error) => return Err(error),
        }
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
    let mut controller = Controller::new(transport);
    controller.bootstrap()?;
    let result = app_loop(&mut terminal, &mut controller);
    disable_raw_mode()?;
    io::stdout().execute(DisableMouseCapture)?;
    io::stdout().execute(LeaveAlternateScreen)?;
    result
}

fn app_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    controller: &mut Controller,
) -> Result<()> {
    let mut reconnect_refresh_pending = false;

    loop {
        refresh_after_reconnect_if_needed(controller, &mut reconnect_refresh_pending)?;

        if let Err(error) = controller.poll_device(Duration::from_millis(5)) {
            if is_device_error(&error) {
                reconnect_refresh_pending = true;
            }
            handle_runtime_error(controller, error)?;
        }
        terminal.draw(|frame| ui::draw(frame, &controller.state))?;

        if !terminal::poll_input(Duration::from_millis(10))? {
            continue;
        }

        match terminal::read_input_event()? {
            Some(AppInputEvent::Key(key)) => {
                if key.kind != AppKeyEventKind::Press {
                    continue;
                }

                if controller.state.hotkeys_popup_open {
                    match key.code {
                        AppKeyCode::Char('q') => break,
                        AppKeyCode::Char('?') | AppKeyCode::Esc => {
                            controller.state.toggle_hotkeys_popup();
                        }
                        _ => {}
                    }
                    continue;
                }

                let result = match key.code {
                    AppKeyCode::Char('q') => break,
                    AppKeyCode::Char('r') => {
                        controller.state.toggle_raw_view();
                        Ok(())
                    }
                    AppKeyCode::Char('R') => match controller.refresh_queried_state() {
                        Ok(()) => {
                            controller.state.last_message =
                                "Sent captured 0x74 startup/state refresh sweep".to_string();
                            Ok(())
                        }
                        Err(error) => Err(error),
                    },
                    AppKeyCode::Tab => {
                        if controller.state.page == MainPage::Mixer {
                            controller.state.cycle_focus();
                        }
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
                        move_popup_selection(controller, false);
                        Ok(())
                    }
                    AppKeyCode::Down
                        if controller.state.assignment_picker.is_some()
                            || controller.state.selector_popup.is_some() =>
                    {
                        move_popup_selection(controller, true);
                        Ok(())
                    }
                    AppKeyCode::Enter
                        if controller.state.assignment_picker.is_some()
                            || controller.state.selector_popup.is_some() =>
                    {
                        activate_popup_selection(controller)
                    }
                    AppKeyCode::Left if controller.state.raw_view_open => {
                        if controller.state.selected_raw_packet
                            == zen_go_tui::app::RawPacketTab::Query75
                        {
                            controller.state.cycle_query_reply_entry(false)
                        } else {
                            controller.state.cycle_raw_packet(false)
                        }
                        Ok(())
                    }
                    AppKeyCode::Right if controller.state.raw_view_open => {
                        if controller.state.selected_raw_packet
                            == zen_go_tui::app::RawPacketTab::Query75
                        {
                            controller.state.cycle_query_reply_entry(true)
                        } else {
                            controller.state.cycle_raw_packet(true)
                        }
                        Ok(())
                    }
                    AppKeyCode::Left => {
                        let size = terminal.size()?;
                        move_selection(
                            controller,
                            false,
                            ratatui::layout::Rect::new(0, 0, size.width, size.height),
                        );
                        Ok(())
                    }
                    AppKeyCode::Right => {
                        let size = terminal.size()?;
                        move_selection(
                            controller,
                            true,
                            ratatui::layout::Rect::new(0, 0, size.width, size.height),
                        );
                        Ok(())
                    }
                    AppKeyCode::Char('+') | AppKeyCode::Char('=') => {
                        adjust_focused(controller, true)
                    }
                    AppKeyCode::Char('-') => adjust_focused(controller, false),
                    AppKeyCode::Char('m') => toggle_mute(controller),
                    AppKeyCode::Char('o') => toggle_mixer_solo(controller),
                    AppKeyCode::Char('d') => toggle_dim(controller),
                    AppKeyCode::Char('a') => open_mixer_assignment_picker(controller),
                    AppKeyCode::Char('l') => toggle_mixer_link(controller),
                    AppKeyCode::Char('[') => adjust_mixer_pan(controller, false),
                    AppKeyCode::Char(']') => adjust_mixer_pan(controller, true),
                    AppKeyCode::Char('p') => toggle_preamp_phase(controller),
                    AppKeyCode::Char('3') => open_preamp_mode_selector(controller),
                    AppKeyCode::Char('s') => cycle_sample_rate(controller),
                    AppKeyCode::Char('c') => cycle_clock_source(controller),
                    AppKeyCode::Char('1') => {
                        controller.send(Command::SelectSurface(Surface::MonitorHp1))
                    }
                    AppKeyCode::Char('2') => controller.send(Command::SelectSurface(Surface::Hp2)),
                    AppKeyCode::Char('b') if controller.state.raw_view_open => {
                        controller.state.capture_raw_baseline();
                        controller.state.last_message =
                            "Captured raw baseline for 0x73/0x83/0x75/0x81".to_string();
                        Ok(())
                    }
                    AppKeyCode::Char('x') if controller.state.raw_view_open => {
                        controller.state.clear_raw_baseline();
                        controller.state.last_message = "Cleared raw baseline".to_string();
                        Ok(())
                    }
                    AppKeyCode::Esc
                        if controller.state.assignment_picker.is_some()
                            || controller.state.selector_popup.is_some()
                            || controller.state.routing_popup_open
                            || controller.state.hotkeys_popup_open =>
                    {
                        controller.state.assignment_picker = None;
                        controller.state.selector_popup = None;
                        controller.state.routing_popup_open = false;
                        controller.state.popup_selected_index = 0;
                        controller.state.hotkeys_popup_open = false;
                        controller.state.last_message = "Closed popup".to_string();
                        Ok(())
                    }
                    _ => Ok(()),
                };

                if let Err(error) = result {
                    if is_device_error(&error) {
                        reconnect_refresh_pending = true;
                    }
                    handle_runtime_error(controller, error)?;
                }
            }
            Some(AppInputEvent::Mouse(mouse)) => {
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
            }
            Some(AppInputEvent::Resize { .. })
            | Some(AppInputEvent::FocusGained)
            | Some(AppInputEvent::FocusLost)
            | Some(AppInputEvent::Paste(_))
            | None => {}
        }
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

fn popup_item_count(controller: &Controller) -> usize {
    if controller.state.assignment_picker.is_some() {
        MixerAssignment::grounded_choices().len()
    } else if let Some(popup) = controller.state.selector_popup {
        match popup.kind {
            SelectorPopupKind::SampleRate => SampleRate::all_confirmed().len(),
            SelectorPopupKind::ClockSource => ClockSource::all_confirmed().len(),
            SelectorPopupKind::PreampMode { .. } => 3,
        }
    } else {
        0
    }
}

fn move_popup_selection(controller: &mut Controller, down: bool) {
    let item_count = popup_item_count(controller);
    if item_count == 0 {
        return;
    }

    controller.state.popup_selected_index = if down {
        (controller.state.popup_selected_index + 1) % item_count
    } else {
        controller
            .state
            .popup_selected_index
            .checked_sub(1)
            .unwrap_or(item_count - 1)
    };
}

fn activate_popup_selection(controller: &mut Controller) -> Result<()> {
    if let Some(picker) = controller.state.assignment_picker {
        if let Some(assignment) = MixerAssignment::grounded_choices()
            .get(controller.state.popup_selected_index)
            .copied()
        {
            return apply_mouse_action(
                controller,
                ui::MouseAction::PickAssignment {
                    strip: picker.strip,
                    assignment,
                },
            );
        }
    }

    if let Some(popup) = controller.state.selector_popup {
        let action = match popup.kind {
            SelectorPopupKind::SampleRate => SampleRate::all_confirmed()
                .get(controller.state.popup_selected_index)
                .copied()
                .map(ui::MouseAction::PickSampleRate),
            SelectorPopupKind::ClockSource => ClockSource::all_confirmed()
                .get(controller.state.popup_selected_index)
                .copied()
                .map(ui::MouseAction::PickClockSource),
            SelectorPopupKind::PreampMode { input } => {
                [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                    .get(controller.state.popup_selected_index)
                    .copied()
                    .map(|mode| ui::MouseAction::PickPreampMode { input, mode })
            }
        };

        if let Some(action) = action {
            return apply_mouse_action(controller, action);
        }
    }

    Ok(())
}

fn adjust_focused(controller: &mut Controller, up: bool) -> Result<()> {
    if controller.state.page != MainPage::Mixer {
        return Ok(());
    }

    match controller.state.focus {
        FocusArea::Outputs => {
            let index = controller.state.selected_output;
            let output = controller.state.outputs[index];
            let next = if up {
                output.volume.saturating_sub(1)
            } else {
                output.volume.saturating_add(1).min(0x60)
            };
            controller.send(Command::SetOutputVolume {
                target: output.target,
                step: next,
            })?;
        }
        FocusArea::Mixer => {
            let active_channel =
                controller.state.active_mixer_channels()[controller.state.selected_channel];
            let channel = active_channel.channel;
            let current = active_channel.level.unwrap_or(0x20);
            let next = if up {
                current.saturating_sub(1)
            } else {
                current.saturating_add(1).min(0x60)
            };
            controller.send_mixer_level_change(
                MixerSurface::from_surface(controller.state.surface),
                channel,
                next,
            )?;
        }
        FocusArea::Preamp => {
            let input = controller.state.selected_preamp_input as u8;
            let current = if input == 0 {
                controller.state.preamp.input1
            } else {
                controller.state.preamp.input2
            };
            let next = next_preamp_gain_raw(current, up);
            controller.send(Command::SetPreampGain { input, raw: next })?;
        }
        _ => {}
    }
    Ok(())
}

fn toggle_mute(controller: &mut Controller) -> Result<()> {
    if controller.state.page != MainPage::Mixer {
        return Ok(());
    }

    match controller.state.focus {
        FocusArea::Outputs => {
            let index = controller.state.selected_output;
            let output = controller.state.outputs[index];
            controller.send(Command::SetOutputMute {
                target: output.target,
                enabled: output.mode != OutputMode::Mute,
            })?;
        }
        FocusArea::Mixer => {
            let active_channel =
                controller.state.active_mixer_channels()[controller.state.selected_channel];
            let channel = active_channel.channel;
            let muted = !active_channel.muted.unwrap_or(false);
            controller.send_mixer_mute_change(
                MixerSurface::from_surface(controller.state.surface),
                channel,
                muted,
            )?;
        }
        FocusArea::Preamp => {
            let input = controller.state.selected_preamp_input as u8;
            let current = if input == 0 {
                controller.state.preamp.input1
            } else {
                controller.state.preamp.input2
            };
            controller.send(Command::SetPreampPhantom {
                input,
                enabled: !current.phantom_on,
            })?;
        }
        _ => {}
    }
    Ok(())
}

fn toggle_dim(controller: &mut Controller) -> Result<()> {
    if controller.state.page != MainPage::Mixer {
        return Ok(());
    }

    if controller.state.focus != FocusArea::Outputs {
        return Ok(());
    }
    let index = controller.state.selected_output;
    let output = controller.state.outputs[index];
    controller.send(Command::SetOutputDim {
        target: output.target,
        enabled: output.mode != OutputMode::Dim,
    })?;
    Ok(())
}

fn adjust_mixer_pan(controller: &mut Controller, right: bool) -> Result<()> {
    if controller.state.page != MainPage::Mixer {
        return Ok(());
    }

    if controller.state.focus != FocusArea::Mixer {
        return Ok(());
    }

    let active_channel =
        controller.state.active_mixer_channels()[controller.state.selected_channel];
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

    controller.send(Command::SetMixerPan {
        mixer: MixerSurface::from_surface(controller.state.surface),
        channel: active_channel.channel,
        pan: PanState::from_raw(next),
        muted: active_channel.muted.unwrap_or(false),
        soloed: active_channel.soloed.unwrap_or(false),
    })?;
    Ok(())
}

fn toggle_mixer_solo(controller: &mut Controller) -> Result<()> {
    if controller.state.page != MainPage::Mixer {
        return Ok(());
    }

    if controller.state.focus != FocusArea::Mixer {
        return Ok(());
    }

    let active_channel =
        controller.state.active_mixer_channels()[controller.state.selected_channel];
    let mixer = MixerSurface::from_surface(controller.state.surface);
    controller.send_mixer_solo_change(
        mixer,
        active_channel.channel,
        !active_channel.soloed.unwrap_or(false),
    )?;

    Ok(())
}

fn open_mixer_assignment_picker(controller: &mut Controller) -> Result<()> {
    if controller.state.page != MainPage::Mixer {
        return Ok(());
    }

    if controller.state.focus != FocusArea::Mixer {
        return Ok(());
    }

    let active_channel =
        controller.state.active_mixer_channels()[controller.state.selected_channel];
    if !zen_go_tui::protocol::MixerStrip::assignment_write_is_grounded(active_channel.channel) {
        controller.state.last_message =
            "Assignment picking is not grounded for the selected strip.".to_string();
        return Ok(());
    }

    apply_mouse_action(
        controller,
        ui::MouseAction::OpenAssignmentPicker(active_channel.channel),
    )
}

fn toggle_mixer_link(controller: &mut Controller) -> Result<()> {
    if controller.state.page != MainPage::Mixer {
        return Ok(());
    }

    if controller.state.focus != FocusArea::Mixer {
        return Ok(());
    }

    let active_channel =
        controller.state.active_mixer_channels()[controller.state.selected_channel];
    let mixer = MixerSurface::from_surface(controller.state.surface);
    controller.send_mixer_link_change(
        mixer,
        active_channel.channel,
        !active_channel.linked.unwrap_or(false),
    )?;

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
                apply_mouse_action(controller, action)?;
            }
        }
        AppMouseEventKind::Drag(AppMouseButton::Left) => {
            if let Some(action) =
                ui::slider_mouse_action(area, &controller.state, mouse.column, mouse.row)
            {
                apply_mouse_action(controller, action)?;
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
                apply_mouse_action(controller, action)?;
                return Ok(());
            }
        }
        _ => {}
    }

    Ok(())
}

fn apply_mouse_action(controller: &mut Controller, action: ui::MouseAction) -> Result<()> {
    match action {
        ui::MouseAction::ToggleRawView => controller.state.toggle_raw_view(),
        ui::MouseAction::ToggleHotkeysPopup => controller.state.toggle_hotkeys_popup(),
        ui::MouseAction::OpenRoutingPopup => {
            controller.state.routing_popup_open = true;
            controller.state.focus = FocusArea::Mixer;
            controller.state.selected_channel = controller.state.selected_channel.min(7);
            controller.state.last_message =
                "Routing popup mirrors mixer assignments for USB recording channels 1-8"
                    .to_string();
        }
        ui::MouseAction::CloseRoutingPopup => {
            controller.state.routing_popup_open = false;
            controller.state.last_message = "Closed routing popup".to_string();
        }
        ui::MouseAction::PageMixerStripsLeft => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.page_mixer_strip_viewport(false);
        }
        ui::MouseAction::PageMixerStripsRight => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.page_mixer_strip_viewport(true);
        }
        ui::MouseAction::OpenSampleRateSelector => {
            if controller.state.device.clock_source == Some(ClockSource::Internal) {
                controller.state.popup_selected_index = controller
                    .state
                    .device
                    .sample_rate
                    .and_then(|current| {
                        SampleRate::all_confirmed()
                            .iter()
                            .position(|rate| *rate == current)
                    })
                    .unwrap_or(0);
                controller.state.selector_popup = Some(SelectorPopupState {
                    kind: SelectorPopupKind::SampleRate,
                });
            }
        }
        ui::MouseAction::OpenClockSourceSelector => {
            controller.state.popup_selected_index = controller
                .state
                .device
                .clock_source
                .and_then(|current| {
                    ClockSource::all_confirmed()
                        .iter()
                        .position(|source| *source == current)
                })
                .unwrap_or(0);
            controller.state.selector_popup = Some(SelectorPopupState {
                kind: SelectorPopupKind::ClockSource,
            });
        }
        ui::MouseAction::SelectPage(page) => controller.state.page = page,
        ui::MouseAction::SelectOutput(index) => {
            controller.state.focus = FocusArea::Outputs;
            controller.state.selected_output = index.min(controller.state.outputs.len() - 1);
        }
        ui::MouseAction::AdjustOutputLevel { index, increase } => {
            controller.state.focus = FocusArea::Outputs;
            controller.state.selected_output = index.min(controller.state.outputs.len() - 1);
            let output = controller.state.outputs[controller.state.selected_output];
            let next = if increase {
                output.volume.saturating_sub(1)
            } else {
                output.volume.saturating_add(1).min(0x60)
            };
            controller.send(Command::SetOutputVolume {
                target: output.target,
                step: next,
            })?;
        }
        ui::MouseAction::SetOutputLevel { index, step } => {
            controller.state.focus = FocusArea::Outputs;
            controller.state.selected_output = index.min(controller.state.outputs.len() - 1);
            let output = controller.state.outputs[controller.state.selected_output];
            controller.send(Command::SetOutputVolume {
                target: output.target,
                step: step.min(0x60),
            })?;
        }
        ui::MouseAction::ToggleOutputDim(index) => {
            controller.state.focus = FocusArea::Outputs;
            controller.state.selected_output = index.min(controller.state.outputs.len() - 1);
            let output = controller.state.outputs[controller.state.selected_output];
            controller.send(Command::SetOutputDim {
                target: output.target,
                enabled: output.mode != OutputMode::Dim,
            })?;
        }
        ui::MouseAction::ToggleOutputMute(index) => {
            controller.state.focus = FocusArea::Outputs;
            controller.state.selected_output = index.min(controller.state.outputs.len() - 1);
            let output = controller.state.outputs[controller.state.selected_output];
            controller.send(Command::SetOutputMute {
                target: output.target,
                enabled: output.mode != OutputMode::Mute,
            })?;
        }
        ui::MouseAction::SelectRawPacketTab(tab) => controller.state.selected_raw_packet = tab,
        ui::MouseAction::SelectQueryReplyEntry(index) => {
            controller.state.selected_query_reply_entry = Some(index)
        }
        ui::MouseAction::SelectSurface(surface) => {
            controller.state.focus = FocusArea::Mixer;
            controller.send(Command::SelectSurface(surface))?;
        }
        ui::MouseAction::SelectMixerChannel(index) => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.selected_channel = index;
        }
        ui::MouseAction::AdjustMixerLevel { index, increase } => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.selected_channel =
                index.min(controller.state.active_mixer_channels().len() - 1);
            let active_channel =
                controller.state.active_mixer_channels()[controller.state.selected_channel];
            let current = active_channel.level.unwrap_or(0x20);
            let next = if increase {
                current.saturating_sub(1)
            } else {
                current.saturating_add(1).min(0x60)
            };
            controller.send_mixer_level_change(
                MixerSurface::from_surface(controller.state.surface),
                active_channel.channel,
                next,
            )?;
        }
        ui::MouseAction::SetMixerLevel { index, level } => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.selected_channel =
                index.min(controller.state.active_mixer_channels().len() - 1);
            let active_channel =
                controller.state.active_mixer_channels()[controller.state.selected_channel];
            controller.send_mixer_level_change(
                MixerSurface::from_surface(controller.state.surface),
                active_channel.channel,
                level.min(0x5a),
            )?;
        }
        ui::MouseAction::AdjustMixerPan { index, right } => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.selected_channel =
                index.min(controller.state.active_mixer_channels().len() - 1);
            let active_channel =
                controller.state.active_mixer_channels()[controller.state.selected_channel];
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
            controller.send(Command::SetMixerPan {
                mixer: MixerSurface::from_surface(controller.state.surface),
                channel: active_channel.channel,
                pan: PanState::from_raw(next),
                muted: active_channel.muted.unwrap_or(false),
                soloed: active_channel.soloed.unwrap_or(false),
            })?;
        }
        ui::MouseAction::SetMixerPan { index, pan } => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.selected_channel =
                index.min(controller.state.active_mixer_channels().len() - 1);
            let active_channel =
                controller.state.active_mixer_channels()[controller.state.selected_channel];
            controller.send(Command::SetMixerPan {
                mixer: MixerSurface::from_surface(controller.state.surface),
                channel: active_channel.channel,
                pan,
                muted: active_channel.muted.unwrap_or(false),
                soloed: active_channel.soloed.unwrap_or(false),
            })?;
        }
        ui::MouseAction::ToggleMixerMute(channel) => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.selected_channel = channel.saturating_sub(1) as usize;
            let mixer = MixerSurface::from_surface(controller.state.surface);
            let active_channel =
                controller.state.mixer_channels[mixer.index()][channel as usize - 1];
            controller.send_mixer_mute_change(
                mixer,
                channel,
                !active_channel.muted.unwrap_or(false),
            )?;
        }
        ui::MouseAction::ToggleMixerSolo(channel) => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.selected_channel = channel.saturating_sub(1) as usize;
            let mixer = MixerSurface::from_surface(controller.state.surface);
            let active_channel =
                controller.state.mixer_channels[mixer.index()][channel as usize - 1];
            controller.send_mixer_solo_change(
                mixer,
                channel,
                !active_channel.soloed.unwrap_or(false),
            )?;
        }
        ui::MouseAction::ToggleMixerLink(channel) => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.selected_channel = channel.saturating_sub(1) as usize;
            let mixer = MixerSurface::from_surface(controller.state.surface);
            let active_channel =
                controller.state.mixer_channels[mixer.index()][channel as usize - 1];
            controller.send_mixer_link_change(
                mixer,
                channel,
                !active_channel.linked.unwrap_or(false),
            )?;
        }
        ui::MouseAction::OpenAssignmentPicker(strip) => {
            controller.state.focus = FocusArea::Mixer;
            controller.state.selected_channel = strip.saturating_sub(1) as usize;
            if !zen_go_tui::protocol::MixerStrip::assignment_write_is_grounded(strip) {
                controller.state.last_message =
                    "Assignment picking is not grounded for the selected strip.".to_string();
            } else {
                controller.state.popup_selected_index = controller.state.mixer_channels
                    [MixerSurface::from_surface(controller.state.surface).index()]
                    [controller.state.selected_channel]
                    .assignment
                    .and_then(|current| {
                        MixerAssignment::grounded_choices()
                            .iter()
                            .position(|assignment| *assignment == current)
                    })
                    .unwrap_or(0);
                controller.state.assignment_picker =
                    Some(zen_go_tui::app::AssignmentPickerState { strip });
                controller.state.last_message = format!("Pick source assignment for CH {strip:02}");
            }
        }
        ui::MouseAction::PickAssignment { strip, assignment } => {
            controller.state.assignment_picker = None;
            controller.state.popup_selected_index = 0;
            controller.send(Command::SetMixerAssignment { strip, assignment })?;
        }
        ui::MouseAction::CloseAssignmentPicker => {
            controller.state.assignment_picker = None;
            controller.state.popup_selected_index = 0;
            controller.state.last_message = "Closed assignment picker".to_string();
        }
        ui::MouseAction::CloseSelectorPopup => {
            controller.state.selector_popup = None;
            controller.state.popup_selected_index = 0;
            controller.state.last_message = "Closed selector".to_string();
        }
        ui::MouseAction::SelectPreampInput(input) => {
            controller.state.focus = FocusArea::Preamp;
            controller.state.selected_preamp_input = input.min(1);
        }
        ui::MouseAction::AdjustPreampGain { input, increase } => {
            controller.state.focus = FocusArea::Preamp;
            controller.state.selected_preamp_input = input.min(1) as usize;
            let current = if input == 0 {
                controller.state.preamp.input1
            } else {
                controller.state.preamp.input2
            };
            controller.send(Command::SetPreampGain {
                input,
                raw: next_preamp_gain_raw(current, increase),
            })?;
        }
        ui::MouseAction::SetPreampGain { input, raw } => {
            controller.state.focus = FocusArea::Preamp;
            controller.state.selected_preamp_input = input.min(1) as usize;
            controller.send(Command::SetPreampGain {
                input: input.min(1),
                raw,
            })?;
        }
        ui::MouseAction::OpenPreampModeSelector(input) => {
            controller.state.focus = FocusArea::Preamp;
            controller.state.selected_preamp_input = input.min(1) as usize;
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
        }
        ui::MouseAction::CyclePreampMode(input) => {
            controller.state.focus = FocusArea::Preamp;
            controller.state.selected_preamp_input = input.min(1) as usize;
            let current = if input == 0 {
                controller.state.preamp.input1.mode
            } else {
                controller.state.preamp.input2.mode
            };
            let next = match current {
                PreampMode::Mic => PreampMode::Line,
                PreampMode::Line => PreampMode::HiZ,
                PreampMode::HiZ | PreampMode::Unknown(_) => PreampMode::Mic,
            };
            controller.send(Command::SetPreampMode { input, mode: next })?;
        }
        ui::MouseAction::PickSampleRate(rate) => {
            controller.state.selector_popup = None;
            controller.state.popup_selected_index = 0;
            controller.send(Command::SetSampleRate(rate))?;
        }
        ui::MouseAction::PickClockSource(source) => {
            controller.state.selector_popup = None;
            controller.state.popup_selected_index = 0;
            controller.send(Command::SetClockSource(source))?;
        }
        ui::MouseAction::PickPreampMode { input, mode } => {
            controller.state.selector_popup = None;
            controller.state.popup_selected_index = 0;
            controller.state.focus = FocusArea::Preamp;
            controller.state.selected_preamp_input = input.min(1) as usize;
            controller.send(Command::SetPreampMode { input, mode })?;
        }
        ui::MouseAction::TogglePreampPhase(input) => {
            controller.state.focus = FocusArea::Preamp;
            controller.state.selected_preamp_input = input.min(1) as usize;
            let mode_raw = if input == 0 {
                controller.state.preamp.input1.mode_raw
            } else {
                controller.state.preamp.input2.mode_raw
            };
            controller.send(Command::SetPreampPhase {
                input,
                enabled: mode_raw & 0x40 == 0,
            })?;
        }
        ui::MouseAction::TogglePreampPhantom(input) => {
            controller.state.focus = FocusArea::Preamp;
            controller.state.selected_preamp_input = input.min(1) as usize;
            let current = if input == 0 {
                controller.state.preamp.input1
            } else {
                controller.state.preamp.input2
            };
            controller.send(Command::SetPreampPhantom {
                input,
                enabled: !current.phantom_on,
            })?;
        }
    }

    Ok(())
}

fn cycle_sample_rate(controller: &mut Controller) -> Result<()> {
    if controller.state.device.clock_source != Some(ClockSource::Internal) {
        return Ok(());
    }

    let current = controller
        .state
        .device
        .sample_rate
        .unwrap_or(SampleRate::Hz48000);
    let all = SampleRate::all_confirmed();
    let position = all.iter().position(|rate| *rate == current).unwrap_or(2);
    let next = all[(position + 1) % all.len()];
    controller.send(Command::SetSampleRate(next))?;
    Ok(())
}

fn cycle_clock_source(controller: &mut Controller) -> Result<()> {
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

fn open_preamp_mode_selector(controller: &mut Controller) -> Result<()> {
    if controller.state.page != MainPage::Mixer {
        return Ok(());
    }

    if controller.state.focus != FocusArea::Preamp {
        return Ok(());
    }

    let input = controller.state.selected_preamp_input as u8;
    apply_mouse_action(controller, ui::MouseAction::OpenPreampModeSelector(input))
}

fn toggle_preamp_phase(controller: &mut Controller) -> Result<()> {
    if controller.state.page != MainPage::Mixer {
        return Ok(());
    }

    if controller.state.focus != FocusArea::Preamp {
        return Ok(());
    }

    let input = controller.state.selected_preamp_input as u8;
    let mode_raw = if input == 0 {
        controller.state.preamp.input1.mode_raw
    } else {
        controller.state.preamp.input2.mode_raw
    };
    controller.send(Command::SetPreampPhase {
        input,
        enabled: mode_raw & 0x40 == 0,
    })?;
    Ok(())
}

fn next_preamp_gain_raw(input: zen_go_tui::protocol::PreampInputState, up: bool) -> u8 {
    match input.mode {
        PreampMode::Mic => {
            if up {
                input.gain_raw.saturating_add(1).min(0x41)
            } else {
                input.gain_raw.saturating_sub(1)
            }
        }
        PreampMode::Line => {
            let current = i8::from_ne_bytes([input.gain_raw]);
            let next = if up {
                (current + 1).min(20)
            } else {
                (current - 1).max(-6)
            };
            next as u8
        }
        PreampMode::HiZ => {
            if up {
                input.gain_raw.saturating_add(1).min(0x2d)
            } else {
                input.gain_raw.saturating_sub(1)
            }
        }
        PreampMode::Unknown(_) => input.gain_raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use zen_go_tui::app::AssignmentPickerState;
    use zen_go_tui::protocol::{
        control_panel_startup_queries, MixerAssignment, MixerSurface, OutputState,
    };
    use zen_go_tui::transport::TransportError;

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

        open_mixer_assignment_picker(&mut controller).expect("open assignment picker");

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

        open_mixer_assignment_picker(&mut controller)
            .expect("open assignment picker from routing popup");

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

        open_preamp_mode_selector(&mut controller).expect("open preamp mode selector");

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
    fn toggle_mixer_solo_sends_selected_channel_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.focus = FocusArea::Mixer;
        controller.state.selected_channel = 0;
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].pan = PanState::center();
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].muted = Some(false);
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].soloed = Some(false);

        toggle_mixer_solo(&mut controller).expect("toggle solo");

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

        apply_mouse_action(&mut controller, ui::MouseAction::OpenAssignmentPicker(5))
            .expect("open picker");
        assert_eq!(
            controller.state.assignment_picker,
            Some(AssignmentPickerState { strip: 5 })
        );

        apply_mouse_action(
            &mut controller,
            ui::MouseAction::PickAssignment {
                strip: 5,
                assignment: MixerAssignment::Oscillator(1),
            },
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

        apply_mouse_action(&mut controller, ui::MouseAction::OpenAssignmentPicker(5))
            .expect("open picker");

        assert_eq!(controller.state.popup_selected_index, 13);
    }

    #[test]
    fn mouse_output_mute_uses_selected_output_target() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.outputs[1] = OutputState::new(OutputTarget::Hp1, 0x30, OutputMode::Normal);

        apply_mouse_action(&mut controller, ui::MouseAction::ToggleOutputMute(1))
            .expect("toggle output mute");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0x48, 0x01, 0x01]);
    }

    #[test]
    fn mouse_output_level_action_sends_exact_step() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        apply_mouse_action(
            &mut controller,
            ui::MouseAction::SetOutputLevel {
                index: 1,
                step: 0x12,
            },
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

        apply_mouse_action(
            &mut controller,
            ui::MouseAction::SetPreampGain {
                input: 1,
                raw: 0x11,
            },
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

        apply_mouse_action(
            &mut controller,
            ui::MouseAction::SetMixerLevel {
                index: 0,
                level: 0x15,
            },
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

        apply_mouse_action(
            &mut controller,
            ui::MouseAction::SetMixerPan {
                index: 0,
                pan: PanState::from_raw(0x12),
            },
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

        apply_mouse_action(
            &mut controller,
            ui::MouseAction::AdjustMixerLevel {
                index: 0,
                increase: true,
            },
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

        apply_mouse_action(
            &mut controller,
            ui::MouseAction::AdjustMixerPan {
                index: 0,
                right: true,
            },
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

        apply_mouse_action(&mut controller, ui::MouseAction::PageMixerStripsRight)
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

        apply_mouse_action(&mut controller, ui::MouseAction::ToggleHotkeysPopup)
            .expect("open hotkeys");
        assert!(controller.state.hotkeys_popup_open);

        apply_mouse_action(&mut controller, ui::MouseAction::ToggleHotkeysPopup)
            .expect("close hotkeys");
        assert!(!controller.state.hotkeys_popup_open);
    }

    #[test]
    fn mouse_sample_rate_selector_opens_and_pick_sends_exact_rate() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.device.clock_source = Some(ClockSource::Internal);

        apply_mouse_action(&mut controller, ui::MouseAction::OpenSampleRateSelector)
            .expect("open sample rate selector");
        assert_eq!(
            controller.state.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::SampleRate,
            })
        );

        apply_mouse_action(
            &mut controller,
            ui::MouseAction::PickSampleRate(SampleRate::Hz48000),
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

        apply_mouse_action(&mut controller, ui::MouseAction::OpenSampleRateSelector)
            .expect("open sample rate selector");
        assert_eq!(controller.state.selector_popup, None);

        cycle_sample_rate(&mut controller).expect("cycle sample rate should no-op");
        assert!(transport.take_writes().is_empty());
    }

    #[test]
    fn mouse_preamp_mode_selector_pick_sends_exact_mode() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        apply_mouse_action(&mut controller, ui::MouseAction::OpenPreampModeSelector(1))
            .expect("open preamp mode selector");
        assert_eq!(
            controller.state.selector_popup,
            Some(SelectorPopupState {
                kind: SelectorPopupKind::PreampMode { input: 1 },
            })
        );

        apply_mouse_action(
            &mut controller,
            ui::MouseAction::PickPreampMode {
                input: 1,
                mode: PreampMode::HiZ,
            },
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

        move_popup_selection(&mut controller, false);
        assert_eq!(
            controller.state.popup_selected_index,
            MixerAssignment::grounded_choices().len() - 1
        );

        move_popup_selection(&mut controller, true);
        assert_eq!(controller.state.popup_selected_index, 0);
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

        let _transport = wait_for_transport(
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
}
