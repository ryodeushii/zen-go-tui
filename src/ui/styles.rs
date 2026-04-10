use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders};

use crate::terminal;
use crate::ui::layouts::{ADJUST_DOWN_BUTTON_LABEL, ADJUST_UP_BUTTON_LABEL};
use antelope_protocol::{OutputMode, OutputState, PreampInputState, PreampMode};

pub(crate) fn section_block(title: &str, focused: bool) -> Block<'_> {
    let style = if focused {
        strong_style(Color::LightCyan)
    } else {
        muted_style()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(terminal::adapt_style(Style::default().fg(if focused {
            Color::LightCyan
        } else {
            Color::DarkGray
        })))
        .title(Span::styled(title, style))
}

pub(crate) fn panel_block<'a>(title: &'a str, accent: Color, focused: bool) -> Block<'a> {
    let title_style = if focused {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(accent)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(terminal::adapt_style(Style::default().fg(if focused {
            accent
        } else {
            Color::DarkGray
        })))
        .title(Span::styled(title, title_style))
}

pub(crate) fn chip_text(label: &str) -> String {
    format!(" {} ", label)
}

pub(crate) fn chip_width(label: &str) -> u16 {
    chip_text(label).chars().count() as u16
}

pub(crate) fn chip<T: Into<String>>(label: T, fg: Color, bg: Color) -> Span<'static> {
    Span::styled(
        chip_text(&label.into()),
        terminal::adapt_style(Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)),
    )
}

pub(crate) fn labeled_value_chip(
    label: &str,
    value: &str,
    min_value_width: usize,
    fg: Color,
    bg: Color,
) -> Span<'static> {
    chip(
        format!(
            "{label} {:>width$}",
            value,
            width = value.chars().count().max(min_value_width)
        ),
        fg,
        bg,
    )
}

pub(crate) fn tab_chip(label: &str, active: bool, accent: Color) -> Span<'static> {
    if active {
        chip(label, Color::Black, accent)
    } else {
        Span::styled(chip_text(label), muted_style())
    }
}

pub(crate) fn muted_style() -> Style {
    terminal::adapt_style(Style::default().fg(Color::Gray))
}

pub(crate) fn subdued_style() -> Style {
    terminal::adapt_style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
}

pub(crate) fn strong_style(color: Color) -> Style {
    terminal::adapt_style(Style::default().fg(color).add_modifier(Modifier::BOLD))
}

pub(crate) fn render_symbol_bar(ratio: f64, width: usize, filled: char, empty: char) -> String {
    let filled_cells = (ratio.clamp(0.0, 1.0) * width as f64).round() as usize;
    let mut out = String::with_capacity(width);
    for index in 0..width {
        out.push(if index < filled_cells { filled } else { empty });
    }
    out
}

#[cfg(test)]
pub(crate) fn render_level_bar(ratio: f64, width: usize) -> String {
    render_symbol_bar(ratio, width, '#', '.')
}

pub(crate) fn preamp_phantom_label(input: PreampInputState) -> &'static str {
    if matches!(input.mode, PreampMode::Mic) {
        "48V"
    } else {
        "N/A"
    }
}

pub(crate) fn preamp_phase_label(input: PreampInputState) -> &'static str {
    if input.mode_raw & 0x40 != 0 {
        "INV"
    } else {
        "NORM"
    }
}

#[cfg(test)]
pub(crate) fn render_output_card(output: &OutputState, active: bool) -> Text<'static> {
    let dim_bg = if output.mode == OutputMode::Dim {
        Color::Yellow
    } else {
        Color::DarkGray
    };
    let mute_bg = if output.mode == OutputMode::Mute {
        Color::LightRed
    } else {
        Color::DarkGray
    };
    let mut header = vec![chip(output.target.label(), Color::Black, Color::LightBlue)];
    if active {
        header.push(Span::raw(" "));
        header.push(chip("ACTIVE", Color::Black, Color::LightGreen));
    }
    Text::from(vec![
        Line::from(header),
        Line::from(vec![
            Span::styled("LVL ", strong_style(Color::LightGreen)),
            Span::styled(
                format!("{} dB", output.display_db()),
                strong_style(Color::White),
            ),
            Span::raw("  "),
            Span::styled(
                render_level_bar(output.gain_ratio(), 8),
                strong_style(Color::LightGreen),
            ),
        ]),
        Line::from(vec![
            chip(ADJUST_DOWN_BUTTON_LABEL, Color::Black, Color::Gray),
            Span::raw(" "),
            chip(ADJUST_UP_BUTTON_LABEL, Color::Black, Color::Gray),
            Span::raw(" "),
            chip("DIM", Color::Black, dim_bg),
            Span::raw(" "),
            chip("MUTE", Color::Black, mute_bg),
        ]),
    ])
}

pub(crate) fn style_for_preamp_mode(mode: PreampMode) -> Color {
    terminal::adapt_color(match mode {
        PreampMode::Mic => Color::Green,
        PreampMode::Line => Color::Yellow,
        PreampMode::HiZ => Color::Magenta,
        PreampMode::Unknown(_) => Color::Gray,
    })
}

pub(crate) fn warning_section_block(title: &str, focused: bool) -> Block<'_> {
    let style = if focused {
        terminal::adapt_style(
            Style::default()
                .fg(Color::LightRed)
                .bg(Color::Rgb(60, 20, 0))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        terminal::adapt_style(Style::default().fg(Color::LightRed))
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(terminal::adapt_style(Style::default().fg(Color::LightRed)))
        .title(Span::styled(title, style))
}

pub(crate) fn style_for_hex_byte(byte: u8, first_in_row: bool, changed: bool) -> Style {
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

pub(crate) fn style_for_ascii_byte(byte: u8, changed: bool) -> Style {
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
