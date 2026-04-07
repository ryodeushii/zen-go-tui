use std::io;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use zen_go_tui::app::{Controller, FocusArea};
use zen_go_tui::protocol::{
    ClockSource, Command, MixerSurface, OutputMode, OutputTarget, PanState, PreampMode, SampleRate,
    Surface,
};
use zen_go_tui::transport::{HidTransport, MockTransport, Transport};
use zen_go_tui::ui;

#[derive(Parser, Debug)]
#[command(author, version, about = "Zen Go Synergy Core terminal control panel")]
struct Cli {
    #[arg(long)]
    mock: bool,
}

const ZEN_GO_VID: u16 = 0x23e5;
const ZEN_GO_PID: u16 = 0xa015;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let transport: Box<dyn Transport> = if cli.mock {
        Box::new(MockTransport::default())
    } else {
        Box::new(HidTransport::open(ZEN_GO_VID, ZEN_GO_PID)?)
    };

    run_app(transport)
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
    loop {
        controller.poll_device(Duration::from_millis(5))?;
        terminal.draw(|frame| ui::draw(frame, &controller.state))?;

        if !event::poll(Duration::from_millis(10))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('r') => controller.state.toggle_raw_view(),
                    KeyCode::Char('R') => {
                        controller.refresh_queried_state()?;
                        controller.state.last_message =
                            "Sent captured 0x74 startup/state refresh sweep".to_string();
                    }
                    KeyCode::Tab => controller.state.cycle_focus(),
                    KeyCode::Char('?') => {
                        controller.state.last_message =
                            "Status: s/c with grounded startup 0x75 summaries. Outputs: +/- m d. Mixer: +/- m [ ] pan a assign l link. Preamp: ←/→ select, +/- gain, m phantom, p phase, 3 mode. Surface: 1/2. Raw: r open, ←/→ tabs, Query75 ←/→ history, b/x baseline. R sends the captured 0x74 refresh sweep.".to_string();
                    }
                    KeyCode::Left if controller.state.raw_view_open => {
                        if controller.state.selected_raw_packet
                            == zen_go_tui::app::RawPacketTab::Query75
                        {
                            controller.state.cycle_query_reply_entry(false)
                        } else {
                            controller.state.cycle_raw_packet(false)
                        }
                    }
                    KeyCode::Right if controller.state.raw_view_open => {
                        if controller.state.selected_raw_packet
                            == zen_go_tui::app::RawPacketTab::Query75
                        {
                            controller.state.cycle_query_reply_entry(true)
                        } else {
                            controller.state.cycle_raw_packet(true)
                        }
                    }
                    KeyCode::Left => move_selection(controller, false),
                    KeyCode::Right => move_selection(controller, true),
                    KeyCode::Char('+') | KeyCode::Char('=') => adjust_focused(controller, true)?,
                    KeyCode::Char('-') => adjust_focused(controller, false)?,
                    KeyCode::Char('m') => toggle_mute(controller)?,
                    KeyCode::Char('d') => toggle_dim(controller)?,
                    KeyCode::Char('a') => cycle_mixer_assignment(controller)?,
                    KeyCode::Char('l') => toggle_mixer_link(controller)?,
                    KeyCode::Char('[') => adjust_mixer_pan(controller, false)?,
                    KeyCode::Char(']') => adjust_mixer_pan(controller, true)?,
                    KeyCode::Char('p') => toggle_preamp_phase(controller)?,
                    KeyCode::Char('3') => cycle_preamp_mode(controller)?,
                    KeyCode::Char('s') => cycle_sample_rate(controller)?,
                    KeyCode::Char('c') => cycle_clock_source(controller)?,
                    KeyCode::Char('1') => {
                        controller.send(Command::SelectSurface(Surface::MonitorHp1))?
                    }
                    KeyCode::Char('2') => controller.send(Command::SelectSurface(Surface::Hp2))?,
                    KeyCode::Char('b') if controller.state.raw_view_open => {
                        controller.state.capture_raw_baseline();
                        controller.state.last_message =
                            "Captured raw baseline for 0x73/0x83/0x75/0x81".to_string();
                    }
                    KeyCode::Char('x') if controller.state.raw_view_open => {
                        controller.state.clear_raw_baseline();
                        controller.state.last_message = "Cleared raw baseline".to_string();
                    }
                    KeyCode::Esc if controller.state.assignment_picker.is_some() => {
                        controller.state.assignment_picker = None;
                        controller.state.last_message = "Closed assignment picker".to_string();
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse) => {
                let size = terminal.size()?;
                handle_mouse_event(
                    ratatui::layout::Rect::new(0, 0, size.width, size.height),
                    controller,
                    mouse,
                )?
            }
            _ => {}
        }
    }

    Ok(())
}

fn move_selection(controller: &mut Controller, right: bool) {
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
                target: OutputTarget::from_index(index),
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
    match controller.state.focus {
        FocusArea::Outputs => {
            let index = controller.state.selected_output;
            let output = controller.state.outputs[index];
            controller.send(Command::SetOutputMute {
                target: OutputTarget::from_index(index),
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
    if controller.state.focus != FocusArea::Outputs {
        return Ok(());
    }
    let index = controller.state.selected_output;
    let output = controller.state.outputs[index];
    controller.send(Command::SetOutputDim {
        target: OutputTarget::from_index(index),
        enabled: output.mode != OutputMode::Dim,
    })?;
    Ok(())
}

fn adjust_mixer_pan(controller: &mut Controller, right: bool) -> Result<()> {
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
    })?;
    Ok(())
}

fn cycle_mixer_assignment(controller: &mut Controller) -> Result<()> {
    if controller.state.focus != FocusArea::Mixer {
        return Ok(());
    }

    let active_channel =
        controller.state.active_mixer_channels()[controller.state.selected_channel];
    if !zen_go_tui::protocol::MixerStrip::assignment_write_is_grounded(active_channel.channel) {
        controller.state.last_message =
            "Assignment cycling is only grounded for strip 11 currently; broader strip mapping remains deferred.".to_string();
        return Ok(());
    }

    controller.state.last_message =
        "Assignment writes are disabled until the full d3 41 table can be reconstructed safely."
            .to_string();
    return Ok(());
}

fn toggle_mixer_link(controller: &mut Controller) -> Result<()> {
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
    mouse: MouseEvent,
) -> Result<()> {
    if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
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
                    "Assignment picking is only grounded for strip 11 currently; broader strip mapping remains deferred.".to_string();
            } else {
                controller.state.last_message =
                    "Assignment writes are disabled until the full d3 41 table can be reconstructed safely.".to_string();
            }
        }
        ui::MouseAction::PickAssignment { strip, assignment } => {
            controller.state.assignment_picker = None;
            let _ = (strip, assignment);
            controller.state.last_message =
                "Assignment writes are disabled until the full d3 41 table can be reconstructed safely.".to_string();
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
