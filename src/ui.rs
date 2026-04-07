use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Gauge, LineGauge, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use crate::app::{AppState, FocusArea};
use crate::protocol::{MixerSurface, OutputMode, PreampInputState, PreampMode};

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
    let query00 = state
        .startup_query_summary(0x00)
        .unwrap_or("Capability/default block pending");
    let query11 = state
        .startup_query_summary(0x11)
        .unwrap_or("Status/capability value pending");
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
        Line::from(vec![Span::raw(format!("Startup: {}", query00))]),
        Line::from(vec![Span::raw(format!("         {}", query11))]),
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
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(7),
        ])
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

    frame.render_widget(
        Paragraph::new(render_experimental_pair_state_line(state))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Exp Pair State"),
            )
            .wrap(Wrap { trim: true }),
        mixer_layout[1],
    );

    let items: Vec<ListItem<'_>> = state
        .active_mixer_channels()
        .iter()
        .enumerate()
        .map(|(index, channel)| ListItem::new(render_mixer_strip_line(state, index, channel)))
        .collect();
    let list = List::new(items).block(section_block(
        "Mixer Strips",
        state.focus == FocusArea::Mixer,
    ));
    frame.render_widget(list, mixer_layout[2]);

    draw_preamp_panel(frame, sections[1], state);
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
        Line::from("Latest full `0x73` and `0x83` state packets. `b` capture baseline, `x` clear."),
    ])
    .block(section_block("Raw", true));
    frame.render_widget(header, layout[0]);

    let content = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(35),
            Constraint::Percentage(35),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ])
        .split(layout[1]);

    let dump_73 = state
        .latest_raw_73
        .as_deref()
        .map(|bytes| render_full_packet_dump(bytes, state.baseline_raw_73.as_deref()))
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
        .map(|bytes| render_full_packet_dump(bytes, state.baseline_raw_83.as_deref()))
        .unwrap_or_else(|| Text::from("Waiting for first 0x83 auxiliary packet..."));
    frame.render_widget(
        Paragraph::new(dump_83)
            .block(section_block("0x83 State", true))
            .wrap(Wrap { trim: false }),
        content[1],
    );

    let dump_75 = state
        .latest_raw_75
        .as_deref()
        .map(|bytes| render_full_packet_dump(bytes, state.baseline_raw_75.as_deref()))
        .unwrap_or_else(|| Text::from("Waiting for first 0x75 query reply..."));
    frame.render_widget(
        Paragraph::new(dump_75)
            .block(section_block("0x75 Query Reply", true))
            .wrap(Wrap { trim: false }),
        content[2],
    );

    let dump_81 = state
        .latest_raw_81
        .as_deref()
        .map(|bytes| render_full_packet_dump(bytes, state.baseline_raw_81.as_deref()))
        .unwrap_or_else(|| Text::from("Waiting for first 0x81 notification..."));
    frame.render_widget(
        Paragraph::new(dump_81)
            .block(section_block("0x81 Notification", true))
            .wrap(Wrap { trim: false }),
        content[3],
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
    "Tab focus | ←/→ select | +/- adjust | m mute/phantom | d dim | [ ] pan | a assign | l link | 3 preamp mode | p preamp phase | s sample-rate | c clock | 1/2 surface | b baseline | x clear | Raw shows 0x73/0x83/0x75/0x81 | ? help | q quit".to_string()
}

fn render_experimental_pair_state_line(state: &AppState) -> String {
    let Some(bytes) = state.latest_raw_73.as_deref() else {
        return "exp pair pending: waiting for 0x73 snapshot".to_string();
    };
    let Some(payload) = bytes.get(0x10..) else {
        return "exp pair pending: short 0x73 snapshot".to_string();
    };

    match payload.get(0x6a).copied() {
        Some(0x0f) => format!(
            "MIX 1 exp lanes={:02x}/{:02x} mirror={:02x}/{:02x} e0/e1={:02x}/{:02x}",
            payload.get(0xda).copied().unwrap_or(0),
            payload.get(0xdb).copied().unwrap_or(0),
            payload.get(0xdc).copied().unwrap_or(0),
            payload.get(0xdd).copied().unwrap_or(0),
            payload.get(0xe0).copied().unwrap_or(0),
            payload.get(0xe1).copied().unwrap_or(0),
        ),
        Some(0x0c) => format!(
            "MIX 2 exp lanes={:02x}/{:02x} e0/e1={:02x}/{:02x}",
            payload.get(0xde).copied().unwrap_or(0),
            payload.get(0xdf).copied().unwrap_or(0),
            payload.get(0xe0).copied().unwrap_or(0),
            payload.get(0xe1).copied().unwrap_or(0),
        ),
        Some(surface) => format!("exp pair pending: unsupported surface {:02x}", surface),
        None => "exp pair pending: missing surface byte".to_string(),
    }
}

fn render_mixer_strip_line(
    state: &AppState,
    index: usize,
    channel: &crate::protocol::MixerChannelState,
) -> String {
    let selected = state.focus == FocusArea::Mixer && state.selected_channel == index;
    let bar = channel
        .meter_ratio()
        .or_else(|| channel.gain_ratio())
        .map(render_thin_bar)
        .unwrap_or_else(|| "........".to_string());
    let assignment = channel
        .assignment
        .map(|value| value.label())
        .unwrap_or_else(|| "assignment?".to_string());
    let pan = channel.pan.display_percent();
    let pan_label = if pan < 0 {
        format!("L{}", pan.unsigned_abs())
    } else if pan > 0 {
        format!("R{}", pan)
    } else {
        "C".to_string()
    };
    format!(
        "CH {:02} {:<8} src={:<16} level={} meter={} mute={} pan={} link={} {}",
        channel.channel,
        bar,
        assignment,
        channel
            .display_db()
            .map(|value| format!("{} dB", value))
            .unwrap_or_else(|| "undecoded".to_string()),
        channel
            .meter
            .map(|value| format!("raw {:02x}", value))
            .unwrap_or_else(|| "undecoded".to_string()),
        channel
            .muted
            .map(|value| if value { "on" } else { "off" })
            .unwrap_or("undecoded"),
        pan_label,
        channel
            .linked
            .map(|value| if value { "on" } else { "off" })
            .unwrap_or("unknown"),
        if selected { "←" } else { "" }
    )
}

fn draw_preamp_panel(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(4),
            Constraint::Min(3),
        ])
        .split(area);

    let input1_title = if state.focus == FocusArea::Preamp && state.selected_preamp_input == 0 {
        "A1 ←"
    } else {
        "A1"
    };
    let input2_title = if state.focus == FocusArea::Preamp && state.selected_preamp_input == 1 {
        "A2 ←"
    } else {
        "A2"
    };

    frame.render_widget(
        render_preamp_gauge(
            input1_title,
            state.preamp.input1,
            state.focus == FocusArea::Preamp && state.selected_preamp_input == 0,
        ),
        layout[0],
    );
    frame.render_widget(
        render_preamp_gauge(
            input2_title,
            state.preamp.input2,
            state.focus == FocusArea::Preamp && state.selected_preamp_input == 1,
        ),
        layout[1],
    );
    if let Some(area) = inner_bottom_line(layout[1]) {
        frame.render_widget(render_preamp_observed_meter(state.preamp.input2), area);
    }

    let status = Paragraph::new(vec![
        Line::from("Preamp Controls"),
        Line::from("Left/Right select input   +/- gain   3 mode   m phantom   p phase"),
        Line::from(format!("Raw cluster: {:02x?}", state.dsp_cluster)),
    ])
    .block(section_block("Preamp", state.focus == FocusArea::Preamp))
    .wrap(Wrap { trim: true });
    frame.render_widget(status, layout[2]);

    frame.render_widget(
        Paragraph::new(state.last_message.clone())
            .block(Block::default().borders(Borders::ALL).title("Status"))
            .wrap(Wrap { trim: true }),
        layout[3],
    );
}

fn render_preamp_gauge<'a>(title: &'a str, input: PreampInputState, focused: bool) -> Gauge<'a> {
    let phase_on = input.mode_raw & 0x40 != 0;
    let phantom = if matches!(input.mode, PreampMode::Mic) {
        if input.phantom_on {
            "48V"
        } else {
            "48v off"
        }
    } else {
        "n/a"
    };

    let block = if input.phantom_on {
        warning_section_block(title, focused)
    } else {
        section_block(title, focused)
    };

    Gauge::default()
        .block(block)
        .gauge_style(Style::default().fg(style_for_preamp_mode(input.mode)))
        .label(format!(
            "{}  {}  phantom:{}  phase:{}  raw {:02x}",
            input.mode.label(),
            input.gain_db_label(),
            phantom,
            if phase_on { "inv" } else { "norm" },
            input.gain_raw,
        ))
        .ratio(input.gain_ratio())
}

fn render_preamp_observed_meter<'a>(input: PreampInputState) -> LineGauge<'a> {
    LineGauge::default()
        .filled_style(Style::default().fg(Color::LightCyan))
        .unfilled_style(Style::default().fg(Color::DarkGray))
        .label(observed_meter_label(input))
        .ratio(input.observed_meter_ratio().unwrap_or(0.0))
}

fn observed_meter_label(input: PreampInputState) -> String {
    match input.observed_meter {
        Some(raw) => format!("obs meter raw {:02x}", raw),
        None => "obs meter pending".to_string(),
    }
}

fn inner_bottom_line(area: Rect) -> Option<Rect> {
    if area.width <= 2 || area.height <= 2 {
        return None;
    }

    Some(Rect {
        x: area.x + 1,
        y: area.y + area.height - 2,
        width: area.width - 2,
        height: 1,
    })
}

fn style_for_preamp_mode(mode: PreampMode) -> Color {
    match mode {
        PreampMode::Mic => Color::Green,
        PreampMode::Line => Color::Yellow,
        PreampMode::HiZ => Color::Magenta,
        PreampMode::Unknown(_) => Color::Gray,
    }
}

fn warning_section_block(title: &str, focused: bool) -> Block<'_> {
    let style = if focused {
        Style::default()
            .fg(Color::LightRed)
            .bg(Color::Rgb(60, 20, 0))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::LightRed)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::LightRed))
        .title(Span::styled(title, style))
}

fn render_full_packet_dump(bytes: &[u8], baseline: Option<&[u8]>) -> Text<'static> {
    Text::from(
        bytes
            .chunks(16)
            .enumerate()
            .map(|(row, chunk)| {
                let offset = row * 16;
                let baseline_chunk =
                    baseline.and_then(|all| all.get(offset..usize::min(offset + 16, all.len())));
                render_dump_line(offset, chunk, baseline_chunk)
            })
            .collect::<Vec<_>>(),
    )
}

fn render_dump_line(offset: usize, chunk: &[u8], baseline: Option<&[u8]>) -> Line<'static> {
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
            let changed = baseline
                .and_then(|base| base.get(index))
                .is_some_and(|base_byte| *base_byte != *byte);
            spans.push(Span::styled(
                format!("{:02x} ", byte),
                style_for_hex_byte(*byte, index == 0, changed),
            ));
        } else {
            spans.push(Span::raw("   "));
        }
    }

    spans.push(Span::raw(" |"));
    for (index, byte) in chunk.iter().enumerate() {
        let ch = if byte.is_ascii_graphic() || *byte == b' ' {
            *byte as char
        } else {
            '.'
        };
        let changed = baseline
            .and_then(|base| base.get(index))
            .is_some_and(|base_byte| *base_byte != *byte);
        spans.push(Span::styled(
            ch.to_string(),
            style_for_ascii_byte(*byte, changed),
        ));
    }
    spans.push(Span::raw("|"));

    Line::from(spans)
}

fn style_for_hex_byte(byte: u8, first_in_row: bool, changed: bool) -> Style {
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

    if changed {
        style = style.bg(Color::DarkGray).add_modifier(Modifier::UNDERLINED);
    }

    style
}

fn style_for_ascii_byte(byte: u8, changed: bool) -> Style {
    let style = match byte {
        0x00 => Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM | Modifier::ITALIC),
        value if value.is_ascii_graphic() || value == b' ' => Style::default().fg(Color::LightCyan),
        _ => Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::ITALIC),
    };

    if changed {
        style.bg(Color::DarkGray).add_modifier(Modifier::UNDERLINED)
    } else {
        style
    }
}

#[cfg(test)]
mod tests {
    use crate::app::AppState;
    use crate::protocol::{
        ClockSource, MixerAssignment, MixerLinkTarget, MixerSurface, OutputMode, OutputState,
        OutputTarget, PanState, PreampInputState, SampleRate, Surface,
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
        assert!(footer.contains("0x75/0x81"));
        assert!(footer.contains("q quit"));
    }

    #[test]
    fn hex_dump_renders_offset_and_ascii() {
        let dump = render_full_packet_dump(&[0x83, 0x00, 0x41, 0x42, 0x0a], None);
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
        let dump = render_full_packet_dump(&[0x00], None);
        let first = &dump.lines[0];
        assert!(first.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(first.spans[1].style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn footer_mentions_assignment_pan_and_link_controls() {
        let footer = render_footer_text(&AppState::default());

        assert!(footer.contains("a assign"));
        assert!(footer.contains("[ ] pan"));
        assert!(footer.contains("l link"));
        assert!(footer.contains("Raw shows 0x73/0x83/0x75/0x81"));
    }

    #[test]
    fn status_panel_surfaces_grounded_non_metadata_startup_queries() {
        let mut state = AppState::default();
        state.device.startup_query_summaries[1] =
            Some("Capability/default block: 3 bytes [aa bb cc]".to_string());
        state.device.startup_query_summaries[2] =
            Some("Status/capability value: 1 bytes [12]".to_string());

        let lines = vec![
            Line::from(format!(
                "Startup: {}",
                state.startup_query_summary(0x00).unwrap_or_default()
            )),
            Line::from(format!(
                "         {}",
                state.startup_query_summary(0x11).unwrap_or_default()
            )),
        ];
        let rendered = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("Capability/default block: 3 bytes [aa bb cc]"));
        assert!(rendered.contains("Status/capability value: 1 bytes [12]"));
    }

    #[test]
    fn mixer_strip_line_includes_assignment_pan_and_link() {
        let mut state = AppState::default();
        state.mixer_channels[0][10].assignment = Some(MixerAssignment::ComputerPlay(8));
        state.mixer_channels[0][10].pan = PanState::from_raw(0x3e);
        state.mixer_channels[0][10].linked = Some(true);
        state.mixer_channels[0][10].level = Some(0x10);
        state.mixer_channels[0][10].meter = Some(0x08);
        state.mixer_channels[0][10].muted = Some(false);

        let channel = &state.mixer_channels[0][10];
        let line = render_mixer_strip_line(&state, 10, channel);

        assert!(line.contains("Computer Play 8"));
        assert!(line.contains("pan=R100"));
        assert!(line.contains("link=on"));
        assert!(line.contains("meter="));
    }

    #[test]
    fn mixer_strip_line_renders_meter_separately_from_level_value() {
        let mut state = AppState::default();
        state.mixer_channels[0][0].level = Some(0x00);
        state.mixer_channels[0][0].meter = Some(0x30);
        state.mixer_channels[0][0].muted = Some(false);

        let line = render_mixer_strip_line(&state, 0, &state.mixer_channels[0][0]);

        assert!(line.contains("level=0 dB"));
        assert!(line.contains("meter="));
    }

    #[test]
    fn mixer_strip_line_renders_newly_grounded_pair_link() {
        let mut state = AppState::default();
        let target = MixerLinkTarget::from_channel(MixerSurface::Mix1, 7).expect("grounded pair");
        state.mixer_channels[target.mixer.index()][target.left_channel as usize - 1].linked =
            Some(true);
        state.mixer_channels[target.mixer.index()][target.left_channel as usize - 1].assignment =
            Some(MixerAssignment::SpdifIn(1));

        let line = render_mixer_strip_line(
            &state,
            target.left_channel as usize - 1,
            &state.mixer_channels[target.mixer.index()][target.left_channel as usize - 1],
        );

        assert!(line.contains("CH 07"));
        assert!(line.contains("SPDIF In 1"));
        assert!(line.contains("link=on"));
    }

    #[test]
    fn experimental_pair_state_line_surfaces_mix1_mirrored_lanes() {
        let mut state = AppState::default();
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x6a] = 0x0f;
        frame[0x10 + 0xda] = 0x0a;
        frame[0x10 + 0xdb] = 0x05;
        frame[0x10 + 0xdc] = 0x0a;
        frame[0x10 + 0xdd] = 0x05;
        frame[0x10 + 0xe0] = 0x60;
        frame[0x10 + 0xe1] = 0x60;
        state.latest_raw_73 = Some(frame);

        let line = render_experimental_pair_state_line(&state);

        assert!(line.contains("MIX 1"));
        assert!(line.contains("lanes=0a/05"));
        assert!(line.contains("mirror=0a/05"));
        assert!(line.contains("ch1=unmuted ch2=unmuted"));
    }

    #[test]
    fn experimental_pair_state_line_surfaces_mix2_compact_lanes() {
        let mut state = AppState::default();
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x6a] = 0x0c;
        frame[0x10 + 0xde] = 0x00;
        frame[0x10 + 0xdf] = 0x06;
        frame[0x10 + 0xe0] = 0x60;
        frame[0x10 + 0xe1] = 0x60;
        state.latest_raw_73 = Some(frame);

        let line = render_experimental_pair_state_line(&state);

        assert!(line.contains("MIX 2"));
        assert!(line.contains("lanes=00/06"));
        assert!(line.contains("e0/e1=60/60"));
        assert!(line.contains("ch1=unmuted ch2=muted"));
    }

    #[test]
    fn experimental_pair_state_line_marks_unresolved_codebook_values() {
        let mut state = AppState::default();
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x6a] = 0x0c;
        frame[0x10 + 0xde] = 0x5a;
        frame[0x10 + 0xdf] = 0x5a;
        state.latest_raw_73 = Some(frame);

        let line = render_experimental_pair_state_line(&state);

        assert!(line.contains("MIX 2"));
        assert!(line.contains("ch1/ch2=unresolved"));
    }

    #[test]
    fn observed_meter_label_mentions_raw_value() {
        let mut input = PreampInputState::from_raw(0x2a, 0x00);
        input.observed_meter = Some(0x30);

        assert_eq!(observed_meter_label(input), "obs meter raw 30");
    }

    #[test]
    fn observed_meter_label_mentions_pending_state() {
        assert_eq!(
            observed_meter_label(PreampInputState::from_raw(0x2a, 0x00)),
            "obs meter pending"
        );
    }
}
