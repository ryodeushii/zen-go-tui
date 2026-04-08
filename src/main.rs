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

use zen_go_tui::app::{Controller, FocusArea, MainPage};
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
                        if !controller.state.raw_view_open
                            && controller.state.assignment_picker.is_none()
                        {
                            controller.state.cycle_page(true);
                        }
                        Ok(())
                    }
                    AppKeyCode::BackTab => {
                        if !controller.state.raw_view_open
                            && controller.state.assignment_picker.is_none()
                        {
                            controller.state.cycle_page(false);
                        }
                        Ok(())
                    }
                    AppKeyCode::Char('f') => {
                        if controller.state.page == MainPage::Mixer {
                            controller.state.cycle_focus();
                        }
                        Ok(())
                    }
                    AppKeyCode::Char('?') => {
                        controller.state.last_message =
                            "Tab/Shift+Tab switch pages. f cycles mixer focus. Outputs: +/- m d. Mixer: +/- m o [ ] pan a assign l link. Preamp: ←/→ select, +/- gain, m phantom, p phase, 3 mode. Surface: 1/2. Raw: r open, ←/→ tabs, Query75 ←/→ history, b/x baseline. R sends the captured 0x74 refresh sweep.".to_string();
                        Ok(())
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
                        move_selection(controller, false);
                        Ok(())
                    }
                    AppKeyCode::Right => {
                        move_selection(controller, true);
                        Ok(())
                    }
                    AppKeyCode::Char('+') | AppKeyCode::Char('=') => {
                        adjust_focused(controller, true)
                    }
                    AppKeyCode::Char('-') => adjust_focused(controller, false),
                    AppKeyCode::Char('m') => toggle_mute(controller),
                    AppKeyCode::Char('o') => toggle_mixer_solo(controller),
                    AppKeyCode::Char('d') => toggle_dim(controller),
                    AppKeyCode::Char('a') => cycle_mixer_assignment(controller),
                    AppKeyCode::Char('l') => toggle_mixer_link(controller),
                    AppKeyCode::Char('[') => adjust_mixer_pan(controller, false),
                    AppKeyCode::Char(']') => adjust_mixer_pan(controller, true),
                    AppKeyCode::Char('p') => toggle_preamp_phase(controller),
                    AppKeyCode::Char('3') => cycle_preamp_mode(controller),
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
                    AppKeyCode::Esc if controller.state.assignment_picker.is_some() => {
                        controller.state.assignment_picker = None;
                        controller.state.last_message = "Closed assignment picker".to_string();
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

fn move_selection(controller: &mut Controller, right: bool) {
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
        }
        FocusArea::Preamp => {
            controller.state.selected_preamp_input = if right { 1 } else { 0 };
        }
        _ => {}
    }
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

fn cycle_mixer_assignment(controller: &mut Controller) -> Result<()> {
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
            "Assignment cycling is not grounded for the selected strip.".to_string();
        return Ok(());
    }

    let choices = MixerAssignment::grounded_choices();
    let current = active_channel
        .assignment
        .and_then(|assignment| {
            choices
                .iter()
                .position(|candidate| *candidate == assignment)
        })
        .unwrap_or(0);
    let next = choices[(current + 1) % choices.len()];
    controller.send(Command::SetMixerAssignment {
        strip: active_channel.channel,
        assignment: next,
    })
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
    if mouse.kind != AppMouseEventKind::Down(AppMouseButton::Left) {
        return Ok(());
    }

    if let Some(action) = ui::mouse_action(area, &controller.state, mouse.column, mouse.row) {
        apply_mouse_action(controller, action)?;
    }

    Ok(())
}

fn apply_mouse_action(controller: &mut Controller, action: ui::MouseAction) -> Result<()> {
    match action {
        ui::MouseAction::ToggleRawView => controller.state.toggle_raw_view(),
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
                controller.state.assignment_picker =
                    Some(zen_go_tui::app::AssignmentPickerState { strip });
                controller.state.last_message = format!("Pick source assignment for CH {strip:02}");
            }
        }
        ui::MouseAction::PickAssignment { strip, assignment } => {
            controller.state.assignment_picker = None;
            controller.send(Command::SetMixerAssignment { strip, assignment })?;
        }
        ui::MouseAction::CloseAssignmentPicker => {
            controller.state.assignment_picker = None;
            controller.state.last_message = "Closed assignment picker".to_string();
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

fn cycle_preamp_mode(controller: &mut Controller) -> Result<()> {
    if controller.state.page != MainPage::Mixer {
        return Ok(());
    }

    if controller.state.focus != FocusArea::Preamp {
        return Ok(());
    }

    let input = controller.state.selected_preamp_input as u8;
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
    Ok(())
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
    fn cycle_mixer_assignment_sends_next_assignment_for_early_strip() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller);
        controller.state.focus = FocusArea::Mixer;
        controller.state.selected_channel = 0;
        controller.state.mixer_channels[MixerSurface::Mix1.index()][0].assignment =
            Some(MixerAssignment::Preamp(1));

        cycle_mixer_assignment(&mut controller).expect("cycle assignment");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x05]);
        assert_eq!(&writes[0][0x10 + 0x03..0x10 + 0x05], &[0x00, 0x01]);
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
