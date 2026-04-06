use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::app::{AppState, FocusArea};
use crate::protocol::{MixerSurface, OutputMode};

pub fn draw(frame: &mut Frame<'_>, state: &AppState) {
    if state.focus == FocusArea::Raw {
        draw_raw_page(frame, frame.area(), state);
        return;
    }

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
        .active_mixer_channels()
        .iter()
        .enumerate()
        .map(|(index, channel)| {
            let selected = state.focus == FocusArea::Mixer && state.selected_channel == index;
            let bar = channel
                .gain_ratio()
                .map(render_thin_bar)
                .unwrap_or_else(|| "........".to_string());
            let label = format!(
                "CH {:02}  {:<8}  level={}  mute={}  {}",
                channel.channel,
                bar,
                channel
                    .display_db()
                    .map(|value| format!("{} dB", value))
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
        Line::from(
            "Startup strip decode is still unresolved; confirmed writes populate per-surface mixer state.",
        ),
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

fn draw_raw_page(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);

    let header = Paragraph::new(vec![
        Line::from("Live Raw State View"),
        Line::from("Latest full `0x73` and `0x83` state packets. Tab cycles back."),
    ])
    .block(section_block("Raw", true));
    frame.render_widget(header, layout[0]);

    let content = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(layout[1]);

    let dump_73 = state
        .latest_raw_73
        .as_deref()
        .map(render_full_packet_dump)
        .unwrap_or_else(|| Text::from("Waiting for first 0x73 snapshot..."));
    frame.render_widget(
        Paragraph::new(dump_73)
            .block(section_block("0x73 State", true))
            .wrap(Wrap { trim: false }),
        content[0],
    );

    let dump_83 = state
        .latest_raw_83
        .as_deref()
        .map(render_full_packet_dump)
        .unwrap_or_else(|| Text::from("Waiting for first 0x83 auxiliary packet..."));
    frame.render_widget(
        Paragraph::new(dump_83)
            .block(section_block("0x83 State", true))
            .wrap(Wrap { trim: false }),
        content[1],
    );

    frame.render_widget(
        Paragraph::new(render_footer_text(state))
            .block(Block::default().borders(Borders::ALL).title("Help")),
        layout[2],
    );
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

fn render_thin_bar(ratio: f64) -> String {
    const WIDTH: usize = 8;
    let filled = (ratio.clamp(0.0, 1.0) * WIDTH as f64).round() as usize;
    let mut out = String::with_capacity(WIDTH);
    for index in 0..WIDTH {
        out.push(if index < filled { '|' } else { '.' });
    }
    out
}

pub fn render_footer_text(_state: &AppState) -> String {
    "Tab focus | ←/→ select | +/- adjust | m mute | d dim | s sample-rate | c clock | 1/2 surface | ? help | q quit".to_string()
}

fn render_full_packet_dump(bytes: &[u8]) -> Text<'static> {
    Text::from(
        bytes
            .chunks(16)
            .enumerate()
            .map(|(row, chunk)| render_dump_line(row * 16, chunk))
            .collect::<Vec<_>>(),
    )
}

fn render_dump_line(offset: usize, chunk: &[u8]) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("{:04x}: ", offset),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];

    for index in 0..16 {
        if index == 8 {
            spans.push(Span::raw(" "));
        }

        if let Some(byte) = chunk.get(index) {
            spans.push(Span::styled(
                format!("{:02x} ", byte),
                style_for_hex_byte(*byte, index == 0),
            ));
        } else {
            spans.push(Span::raw("   "));
        }
    }

    spans.push(Span::raw(" |"));
    for byte in chunk {
        let ch = if byte.is_ascii_graphic() || *byte == b' ' {
            *byte as char
        } else {
            '.'
        };
        spans.push(Span::styled(ch.to_string(), style_for_ascii_byte(*byte)));
    }
    spans.push(Span::raw("|"));

    Line::from(spans)
}

fn style_for_hex_byte(byte: u8, first_in_row: bool) -> Style {
    let mut style = match byte {
        0x00 => Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
        0x73 | 0x83 | 0x74 | 0x75 | 0x81 => Style::default()
            .fg(Color::LightMagenta)
            .add_modifier(Modifier::BOLD),
        0x60 | 0x5a | 0x54 | 0x51 | 0x32 => Style::default().fg(Color::Yellow),
        value if value.is_ascii_graphic() || value == b' ' => Style::default().fg(Color::LightCyan),
        _ => Style::default().fg(Color::White),
    };

    if first_in_row && byte != 0x00 {
        style = style.add_modifier(Modifier::BOLD);
    }

    style
}

fn style_for_ascii_byte(byte: u8) -> Style {
    match byte {
        0x00 => Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM | Modifier::ITALIC),
        value if value.is_ascii_graphic() || value == b' ' => Style::default().fg(Color::LightCyan),
        _ => Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::ITALIC),
    }
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

    #[test]
    fn hex_dump_renders_offset_and_ascii() {
        let dump = render_full_packet_dump(&[0x83, 0x00, 0x41, 0x42, 0x0a]);
        let first = &dump.lines[0];
        let rendered: String = first
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(rendered.contains("0000:"));
        assert!(rendered.contains("83 00 41 42 0a"));
        assert!(rendered.contains("|..AB.|"));
    }

    #[test]
    fn zero_bytes_are_dimmed_and_offsets_are_bold() {
        let dump = render_full_packet_dump(&[0x00]);
        let first = &dump.lines[0];
        assert!(first.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(first.spans[1].style.add_modifier.contains(Modifier::DIM));
    }
}
