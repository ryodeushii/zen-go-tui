use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::app::{AppState, FocusArea};
use crate::protocol::{MixerSurface, OutputMode};

pub fn draw(frame: &mut Frame<'_>, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());

    draw_status(frame, chunks[0], state);
    draw_outputs(frame, chunks[1], state);
    draw_mixer_and_preamp(frame, chunks[2], state);
    draw_footer(frame, chunks[3], state);
}

fn draw_status(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = section_block("Status", state.focus == FocusArea::Status);
    let sample = state
        .device
        .sample_rate
        .map(|value| value.label())
        .unwrap_or_else(|| "unknown".to_string());
    let clock = state
        .device
        .clock_source
        .map(|value| value.label().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let lock = if state.device.lock_known {
        if state.device.locked == Some(true) {
            "locked"
        } else {
            "unlocked"
        }
    } else {
        "experimental/unknown"
    };
    let connected = if state.connection.connected {
        "connected"
    } else {
        "disconnected"
    };
    let meta = state
        .device
        .metadata
        .as_ref()
        .map(|m| format!("{} v{}", m.product_name, m.version))
        .unwrap_or_else(|| "metadata pending".to_string());
    let text = Paragraph::new(vec![
        Line::from(vec![Span::raw(format!("Device: {}", meta))]),
        Line::from(vec![Span::raw(format!(
            "Clock: {}   Rate: {}   Lock: {}   Surface: {}",
            clock,
            sample,
            lock,
            state.surface.label()
        ))]),
        Line::from(vec![Span::raw(format!(
            "Connection: {}   Last: {}",
            connected, state.device.last_refresh_summary
        ))]),
    ])
    .block(block)
    .wrap(Wrap { trim: true });
    frame.render_widget(text, area);
}

fn draw_outputs(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(24),
            Constraint::Length(24),
            Constraint::Length(24),
        ])
        .split(area);

    for (index, output) in state.outputs.iter().enumerate() {
        let title = if state.selected_output == index && state.focus == FocusArea::Outputs {
            format!("{} ←", output.target.label())
        } else {
            output.target.label().to_string()
        };
        let gauge = Gauge::default()
            .block(section_block(
                &title,
                state.focus == FocusArea::Outputs && state.selected_output == index,
            ))
            .gauge_style(Style::default().fg(match output.mode {
                OutputMode::Normal => Color::Green,
                OutputMode::Mute => Color::Red,
                OutputMode::Dim => Color::Yellow,
                OutputMode::Unknown(_) => Color::Gray,
            }))
            .label(format!(
                "{} dB / {} / raw {:02x}",
                output.display_db(),
                output.mode.label(),
                output.volume
            ))
            .ratio(output.gain_ratio());
        frame.render_widget(gauge, sections[index]);
    }
}

fn draw_mixer_and_preamp(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area);

    let titles = ["MIX 1 / Monitor-HP1", "MIX 2 / HP2"];
    let active = match MixerSurface::from_surface(state.surface) {
        MixerSurface::Mix1 => 0,
        MixerSurface::Mix2 => 1,
    };
    let mixer_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(7)])
        .split(sections[0]);

    let tabs = Tabs::new(
        titles
            .iter()
            .map(|title| Line::from(*title))
            .collect::<Vec<_>>(),
    )
    .block(section_block(
        "Mixer Surface",
        state.focus == FocusArea::Mixer,
    ))
    .select(active)
    .highlight_style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(tabs, mixer_layout[0]);

    let items: Vec<ListItem<'_>> = state
        .mixer_channels
        .iter()
        .enumerate()
        .map(|(index, channel)| {
            let selected = state.focus == FocusArea::Mixer && state.selected_channel == index;
            let label = format!(
                "CH {:02}  level={}  mute={}  {}",
                channel.channel,
                channel
                    .level
                    .map(|value| format!("0x{:02x}", value))
                    .unwrap_or_else(|| "undecoded".to_string()),
                channel
                    .muted
                    .map(|value| if value { "on" } else { "off" })
                    .unwrap_or("undecoded"),
                if selected { "←" } else { "" }
            );
            ListItem::new(label)
        })
        .collect();
    let list = List::new(items).block(section_block(
        "Mixer Strips",
        state.focus == FocusArea::Mixer,
    ));
    frame.render_widget(list, mixer_layout[1]);

    let preamp = Paragraph::new(vec![
        Line::from("Preamp / DSP"),
        Line::from(format!("Front bytes: {:02x?}", state.dsp_cluster)),
        Line::from("Read-only unless protocol confidence is strong."),
        Line::from("Extended DSP/preamp bytes are shown as experimental."),
        Line::from(state.last_message.clone()),
    ])
    .block(section_block(
        "Preamp / DSP (experimental)",
        state.focus == FocusArea::Preamp,
    ))
    .wrap(Wrap { trim: true });
    frame.render_widget(preamp, sections[1]);
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let footer = Paragraph::new(render_footer_text(state))
        .block(Block::default().borders(Borders::ALL).title("Help"));
    frame.render_widget(footer, area);
}

fn section_block(title: &str, focused: bool) -> Block<'_> {
    let style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, style))
}

pub fn render_footer_text(_state: &AppState) -> String {
    "Tab focus | ←/→ select | +/- adjust | m mute | d dim | s sample-rate | c clock | 1/2 surface | ? help | q quit".to_string()
}

#[cfg(test)]
mod tests {
    use crate::app::AppState;
    use crate::protocol::{
        ClockSource, OutputMode, OutputState, OutputTarget, SampleRate, Surface,
    };

    use super::*;

    #[test]
    fn footer_contains_keybindings() {
        let mut state = AppState::default();
        state.device.sample_rate = Some(SampleRate::Hz48000);
        state.device.clock_source = Some(ClockSource::Internal);
        state.outputs = [
            OutputState::new(OutputTarget::Monitor, 0x40, OutputMode::Normal),
            OutputState::new(OutputTarget::Hp1, 0x30, OutputMode::Mute),
            OutputState::new(OutputTarget::Hp2, 0x20, OutputMode::Dim),
        ];
        state.surface = Surface::MonitorHp1;

        let footer = render_footer_text(&state);
        assert!(footer.contains("Tab"));
        assert!(footer.contains("m mute"));
        assert!(footer.contains("d dim"));
        assert!(footer.contains("q quit"));
    }
}
