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
    ClockSource, Command, MixerSurface, OutputMode, OutputTarget, SampleRate, Surface,
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
                        "Status: s/c. Outputs: +/- m d. Mixer: +/- m. Surface: 1/2.".to_string();
                }
                KeyCode::Left => move_selection(controller, false),
                KeyCode::Right => move_selection(controller, true),
                KeyCode::Char('+') | KeyCode::Char('=') => adjust_focused(controller, true)?,
                KeyCode::Char('-') => adjust_focused(controller, false)?,
                KeyCode::Char('m') => toggle_mute(controller)?,
                KeyCode::Char('d') => toggle_dim(controller)?,
                KeyCode::Char('s') => cycle_sample_rate(controller)?,
                KeyCode::Char('c') => cycle_clock_source(controller)?,
                KeyCode::Char('1') => {
                    controller.send(Command::SelectSurface(Surface::MonitorHp1))?
                }
                KeyCode::Char('2') => controller.send(Command::SelectSurface(Surface::Hp2))?,
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
                pan_state: zen_go_tui::protocol::PanState::Center,
            })?;
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
                pan_state: zen_go_tui::protocol::PanState::Center,
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
