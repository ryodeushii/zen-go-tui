use std::io;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use zen_go_tui::app::{Controller, FocusArea};
use zen_go_tui::protocol::{
    ClockSource, Command, MixerAssignment, MixerLinkTarget, MixerSurface, OutputMode, OutputTarget,
    PanState, PreampMode, SampleRate, Surface,
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
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut controller = Controller::new(transport);
    controller.bootstrap()?;
    let result = app_loop(&mut terminal, &mut controller);
    disable_raw_mode()?;
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

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Tab => controller.state.cycle_focus(),
                KeyCode::Char('?') => {
                    controller.state.last_message =
                        "Status: s/c with grounded startup 0x75 summaries. Outputs: +/- m d. Mixer: +/- m [ ] pan a assign l link. Preamp: ←/→ select, +/- gain, m phantom, p phase, 3 mode. Surface: 1/2. Raw: b/x baseline for 0x73/0x83/0x75/0x81.".to_string();
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
                KeyCode::Char('b') if controller.state.focus == FocusArea::Raw => {
                    controller.state.capture_raw_baseline();
                    controller.state.last_message =
                        "Captured raw baseline for 0x73/0x83/0x75/0x81".to_string();
                }
                KeyCode::Char('x') if controller.state.focus == FocusArea::Raw => {
                    controller.state.clear_raw_baseline();
                    controller.state.last_message = "Cleared raw baseline".to_string();
                }
                _ => {}
            }
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
            controller.send(Command::SetMixerLevel {
                mixer: MixerSurface::from_surface(controller.state.surface),
                channel,
                level: next,
                pan_state: active_channel.pan,
            })?;
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
            controller.send(Command::SetMixerMute {
                mixer: MixerSurface::from_surface(controller.state.surface),
                channel,
                muted,
                pan_state: active_channel.pan,
            })?;
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
    if active_channel.channel <= 4 {
        controller.state.last_message =
            "Assignment cycling is limited to ordinary strips 5..16; early AFX-adjacent strips remain deferred.".to_string();
        return Ok(());
    }

    let current = active_channel.assignment.unwrap_or(MixerAssignment::Mute);
    let choices = [
        MixerAssignment::Mute,
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
        MixerAssignment::SpdifIn(1),
        MixerAssignment::SpdifIn(2),
        MixerAssignment::Oscillator(1),
        MixerAssignment::Oscillator(2),
        MixerAssignment::EmuMic(1),
        MixerAssignment::EmuMic(2),
    ];
    let position = choices
        .iter()
        .position(|item| *item == current)
        .unwrap_or(0);
    let next = choices[(position + 1) % choices.len()];

    controller.send(Command::SetMixerAssignment {
        strip: active_channel.channel,
        assignment: next,
    })?;
    Ok(())
}

fn toggle_mixer_link(controller: &mut Controller) -> Result<()> {
    if controller.state.focus != FocusArea::Mixer {
        return Ok(());
    }

    let active_channel =
        controller.state.active_mixer_channels()[controller.state.selected_channel];
    let mixer = MixerSurface::from_surface(controller.state.surface);
    if let Some(target) = MixerLinkTarget::from_channel(mixer, active_channel.channel) {
        controller.send(Command::SetLinkState {
            selector: target.selector,
            enabled: !active_channel.linked.unwrap_or(false),
            companion_bank: target.companion_bank(),
        })?;
    } else {
        controller.state.last_message =
            "Link toggling is only exposed for currently grounded selector mappings.".to_string();
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
