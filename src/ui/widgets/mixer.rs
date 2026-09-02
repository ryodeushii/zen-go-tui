use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use tui_slider::{Slider, SliderOrientation};

use crate::app::{AppState, FocusArea};
use crate::terminal;
use antelope_protocol::{
    meter_display_db, DynamicMixerStrip, OutputMode, OutputState, PreampInputState, PreampMode,
};

use super::super::layouts::*;
use super::super::styles::*;
use super::signals::*;

pub(crate) fn render_output_card_widget(
    area: Rect,
    buffer: &mut Buffer,
    output: &OutputState,
    active: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

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
    Paragraph::new(Line::from(header)).render(rows[0], buffer);
    render_labeled_slider(
        rows[1],
        buffer,
        &signal_slider_label("LVL", Some(format!("{} dB", output.display_db()))),
        Some(output.gain_ratio()),
        Color::LightGreen,
        true,
    );
    Paragraph::new(Line::from(vec![
        chip(ADJUST_DOWN_BUTTON_LABEL, Color::Black, Color::Gray),
        Span::raw(" "),
        chip(ADJUST_UP_BUTTON_LABEL, Color::Black, Color::Gray),
        Span::raw(" "),
        chip("DIM", Color::Black, dim_bg),
        Span::raw(" "),
        chip("MUTE", Color::Black, mute_bg),
    ]))
    .render(rows[2], buffer);
}

pub(crate) fn render_dynamic_output_card_widget(
    controls: DynamicOutputControlRects,
    buffer: &mut Buffer,
    state: &AppState,
    index: usize,
    active: bool,
) {
    let Some(output) = state.outputs().get(index) else {
        return;
    };
    let accent = if active {
        Color::LightGreen
    } else {
        Color::LightBlue
    };
    Paragraph::new(Line::from(chip(&output.name, Color::Black, accent))).render(
        Rect::new(
            controls.row.x,
            controls.row.y,
            controls.row.width.min(19),
            1,
        ),
        buffer,
    );
    if let Some(rect) = controls.level {
        let enabled = state
            .ui_profile
            .supports_output(output.address, antelope_protocol::OutputControl::Level);
        let label = output
            .level
            .map_or_else(|| "LVL ?".into(), |value| format!("LVL {value}"));
        Paragraph::new(Line::from(chip(
            &label,
            Color::Black,
            if enabled {
                Color::LightGreen
            } else {
                Color::DarkGray
            },
        )))
        .render(rect, buffer);
    }
    if let Some(rect) = controls.dim {
        let enabled = state
            .ui_profile
            .supports_output(output.address, antelope_protocol::OutputControl::Dim);
        Paragraph::new(Line::from(chip(
            "DIM",
            Color::Black,
            if enabled && output.dimmed == Some(true) {
                Color::Yellow
            } else if enabled {
                Color::Gray
            } else {
                Color::DarkGray
            },
        )))
        .render(rect, buffer);
    }
    if let Some(rect) = controls.mute {
        let enabled = state
            .ui_profile
            .supports_output(output.address, antelope_protocol::OutputControl::Mute);
        Paragraph::new(Line::from(chip(
            "MUTE",
            Color::Black,
            if enabled && output.muted == Some(true) {
                Color::LightRed
            } else if enabled {
                Color::Gray
            } else {
                Color::DarkGray
            },
        )))
        .render(rect, buffer);
    }
}

pub(crate) fn render_preamp_visual_widget(
    area: Rect,
    buffer: &mut Buffer,
    title: &str,
    input: PreampInputState,
    focused: bool,
    peak_raw: Option<u8>,
) {
    let block = if input.phantom_on {
        warning_section_block(title, focused)
    } else {
        section_block(title, focused)
    };
    block.render(area, buffer);

    let inner = inner_area(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let sections = preamp_card_inner_layout(area);
    render_stacked_signal_rows(
        sections[0],
        buffer,
        &meter_slider_label("OBS", input.observed_meter_db()),
        input.observed_meter_ratio(),
        &signal_slider_label("GAIN", Some(input.gain_db_label())),
        Some(input.gain_ratio()),
        style_for_preamp_mode(input.mode),
    );
    if let Some(peak_raw) = peak_raw {
        if let Some(peak_db) = meter_display_db(peak_raw) {
            let peak_text = format!("PEAK {} dB", peak_db);
            let peak_style = terminal::adapt_style(Style::default().fg(Color::Red));
            if sections[0].y + 2 < area.y + area.height.saturating_sub(1) {
                buffer.set_string(sections[0].x, sections[0].y + 2, &peak_text, peak_style);
            }
        }
    }
    Paragraph::new(render_preamp_controls_text(input)).render(sections[1], buffer);
}

pub(crate) fn render_pan_slider(area: Rect, buffer: &mut Buffer, ratio: f64) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let y = area.y + area.height / 2;
    for offset in 0..area.width {
        let x = area.x + offset;
        buffer[(x, y)]
            .set_symbol("─")
            .set_style(terminal::adapt_style(Style::default().fg(Color::DarkGray)));
    }

    let center_x = area.x + area.width / 2;
    buffer[(center_x, y)]
        .set_symbol("┼")
        .set_style(terminal::adapt_style(Style::default().fg(Color::LightBlue)));

    let handle_x =
        area.x + ((area.width.saturating_sub(1)) as f64 * ratio.clamp(0.0, 1.0)).round() as u16;
    buffer[(handle_x, y)]
        .set_symbol("●")
        .set_style(terminal::adapt_style(Style::default().fg(Color::LightBlue)));
}

pub(crate) fn render_pan_scale(area: Rect, buffer: &mut Buffer) {
    if area.width < 5 || area.height == 0 {
        return;
    }

    let style = terminal::adapt_style(Style::default().fg(Color::DarkGray));
    buffer.set_string(area.x, area.y, "-30", style);
    let center = area.x + area.width / 2;
    buffer.set_string(center, area.y, "0", style);
    let right_x = area.x + area.width.saturating_sub(2);
    buffer.set_string(right_x, area.y, "30", style);
}

pub(crate) fn render_vertical_combo_strip(
    area: Rect,
    buffer: &mut Buffer,
    meter_db: Option<i16>,
    level_ratio: Option<f64>,
    peak_raw: Option<u8>,
) {
    if area.width < 4 || area.height == 0 {
        return;
    }

    let content_width = 6.min(area.width);
    let content_area = Rect::new(
        area.x + area.width.saturating_sub(content_width) / 2,
        area.y,
        content_width,
        area.height,
    );

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(content_area);
    let scale = columns[0];
    let level = columns[2];
    let meter = columns[4];

    let mut previous_y: Option<u16> = None;
    for marker in MIXER_STRIP_DB_MARKERS {
        let ratio = 1.0 - (marker as f64 / 90.0);
        let mut y = vertical_ratio_row(scale, ratio);
        if let Some(prev) = previous_y {
            y = y.max(prev.saturating_add(1));
        }
        y = y.min(scale.y + scale.height.saturating_sub(1));
        previous_y = Some(y);
        buffer.set_string(
            scale.x,
            y,
            format!("{:>2}", marker),
            terminal::adapt_style(Style::default().fg(Color::DarkGray)),
        );
    }

    let meter_ratio = meter_db_ratio_option(meter_db);
    let peak_active = peak_raw.is_some();
    let peak_y = if peak_active { Some(meter.y) } else { None };
    let level_handle_y = level_ratio.map(|ratio| vertical_ratio_row(level, ratio));

    for step in 0..meter.height {
        let y = meter.y + meter.height.saturating_sub(1) - step;
        let cell_ratio = (step + 1) as f64 / meter.height.max(1) as f64;
        let meter_filled = meter_ratio
            .map(|ratio| cell_ratio <= ratio)
            .unwrap_or(false);
        let level_filled = level_ratio
            .map(|ratio| cell_ratio <= ratio)
            .unwrap_or(false);
        let is_peak = peak_y == Some(y);

        let (meter_symbol, meter_color) = if is_peak {
            ("▇", Color::Red)
        } else if meter_filled {
            ("█", meter_bar_color(cell_ratio))
        } else {
            ("░", Color::DarkGray)
        };
        buffer[(meter.x, y)]
            .set_symbol(meter_symbol)
            .set_style(terminal::adapt_style(Style::default().fg(meter_color)));

        let level_symbol = if level_handle_y == Some(y) {
            "●"
        } else if level_filled {
            "█"
        } else {
            "┆"
        };
        let level_color = if level_handle_y == Some(y) {
            Color::White
        } else if level_filled {
            Color::Yellow
        } else {
            Color::DarkGray
        };
        buffer[(level.x, y)]
            .set_symbol(level_symbol)
            .set_style(terminal::adapt_style(Style::default().fg(level_color)));
    }
}

pub(crate) fn render_mixer_strip_widget(
    area: Rect,
    buffer: &mut Buffer,
    state: &AppState,
    index: usize,
    channel: &antelope_protocol::MixerChannelState,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let selected = state.ui.focus == FocusArea::Mixer && state.mixer.selected_channel == index;
    let source = channel
        .assignment
        .map(|value| value.short_label().to_string())
        .unwrap_or_else(|| "?".to_string());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(terminal::adapt_style(Style::default().fg(if selected {
            Color::LightGreen
        } else {
            Color::DarkGray
        })));
    let inner = block.inner(area);
    block.render(area, buffer);
    if inner.width == 0 || inner.height < 6 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let (channel_rect, source_rect) = mixer_header_chip_rects(area, &source);
    let ch_label = format!("CH {:02}", channel.channel);
    Paragraph::new(Line::from(vec![chip(
        &ch_label,
        Color::Black,
        if selected {
            Color::LightGreen
        } else {
            Color::Gray
        },
    )]))
    .render(channel_rect, buffer);
    Paragraph::new(Line::from(vec![chip(
        &source,
        Color::Black,
        Color::LightCyan,
    )]))
    .alignment(Alignment::Right)
    .render(source_rect, buffer);

    Paragraph::new(Line::from(Span::styled(
        mixer_pan_label(channel),
        strong_style(Color::LightBlue),
    )))
    .alignment(Alignment::Center)
    .render(rows[1], buffer);
    render_pan_slider(rows[2], buffer, channel.pan.ratio());
    render_pan_scale(rows[3], buffer);
    Paragraph::new(Line::from(Span::styled(
        format_meter_value_label(channel.meter_db()),
        strong_style(Color::LightGreen),
    )))
    .alignment(Alignment::Center)
    .render(rows[4], buffer);
    let peak_raw = state
        .mixer
        .peaks
        .get(state.active_mixer_surface().unwrap_or(0))
        .and_then(|mix| mix.get(index))
        .and_then(|peak| peak.as_ref())
        .map(|p| p.raw);
    render_vertical_combo_strip(
        rows[5],
        buffer,
        channel.meter_db(),
        channel.gain_ratio(),
        peak_raw,
    );

    Paragraph::new(Line::from(Span::styled(
        mixer_level_value_label(channel),
        strong_style(Color::Yellow),
    )))
    .alignment(Alignment::Center)
    .render(rows[6], buffer);

    let solo_on = channel.soloed == Some(true);
    let mute_on = channel.muted == Some(true);
    let link_on = channel.linked == Some(true);
    let mut controls = Vec::new();
    if channel.channel % 2 == 1 {
        controls.push(chip(
            "L",
            Color::Black,
            if link_on {
                Color::LightBlue
            } else {
                Color::DarkGray
            },
        ));
        controls.push(Span::raw(" "));
    }
    controls.push(chip(
        "S",
        Color::Black,
        if solo_on {
            Color::LightGreen
        } else {
            Color::DarkGray
        },
    ));
    controls.push(Span::raw(" "));
    controls.push(chip(
        "M",
        Color::Black,
        if mute_on {
            Color::LightRed
        } else {
            Color::DarkGray
        },
    ));
    Paragraph::new(Line::from(controls))
        .alignment(Alignment::Center)
        .render(rows[7], buffer);
}

pub(crate) fn render_dynamic_mixer_strip_widget(
    controls: DynamicMixerControlRects,
    buffer: &mut Buffer,
    state: &AppState,
    address: antelope_protocol::MixerAddress,
    index: Option<usize>,
    strip: &DynamicMixerStrip,
) {
    if controls.card.width == 0 || controls.card.height == 0 {
        return;
    }
    let selected = index.is_some_and(|index| {
        state.ui.focus == FocusArea::Mixer && state.mixer.selected_channel == index
    });
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(terminal::adapt_style(Style::default().fg(if selected {
            Color::LightGreen
        } else {
            Color::DarkGray
        })));
    block.render(controls.card, buffer);
    let (label_rect, _) = mixer_header_chip_rects(controls.card, "");
    Paragraph::new(Line::from(chip(
        &strip.name,
        Color::Black,
        if selected {
            Color::LightGreen
        } else {
            Color::Gray
        },
    )))
    .render(label_rect, buffer);
    let enabled = |control| state.ui_profile.supports_mixer(address.surface, control);
    if let Some(rect) = controls.pan {
        let ratio = strip
            .pan
            .map_or(0.5, |value| (value as f64 / 64.0).clamp(0.0, 1.0));
        render_pan_slider(rect, buffer, ratio);
        Paragraph::new(Line::from(Span::styled(
            strip
                .pan
                .map_or_else(|| "PAN ?".into(), |value| format!("PAN {value}")),
            strong_style(if enabled(antelope_protocol::MixerControl::Pan) {
                Color::LightBlue
            } else {
                Color::DarkGray
            }),
        )))
        .render(mixer_strip_rows(controls.card)[1], buffer);
    }
    if let Some(rect) = controls.fader {
        let ratio = strip
            .fader
            .map(|value| 1.0 - (value as f64 / 90.0).clamp(0.0, 1.0));
        render_level_slider(
            rect,
            buffer,
            ratio,
            if enabled(antelope_protocol::MixerControl::Fader) {
                Color::Yellow
            } else {
                Color::DarkGray
            },
        );
        Paragraph::new(Line::from(Span::styled(
            strip
                .fader
                .map_or_else(|| "LVL ?".into(), |value| format!("LVL {value}")),
            strong_style(Color::Yellow),
        )))
        .render(mixer_strip_rows(controls.card)[6], buffer);
    }
    if let Some(rect) = controls.send {
        Paragraph::new(Line::from(chip(
            &strip
                .send
                .map_or_else(|| "SEND ?".into(), |value| format!("SEND {value}")),
            Color::Black,
            if enabled(antelope_protocol::MixerControl::Send) {
                Color::LightCyan
            } else {
                Color::DarkGray
            },
        )))
        .render(rect, buffer);
    }
    for (rect, label, on, control) in [
        (controls.link, "L", strip.linked, None),
        (
            controls.solo,
            "S",
            strip.soloed,
            Some(antelope_protocol::MixerControl::Solo),
        ),
        (
            controls.mute,
            "M",
            strip.muted,
            Some(antelope_protocol::MixerControl::Mute),
        ),
    ] {
        let Some(rect) = rect else { continue };
        let actionable =
            control.map_or_else(|| state.ui_profile.supports_link(address.surface), enabled);
        Paragraph::new(Line::from(chip(
            label,
            Color::Black,
            if !actionable {
                Color::DarkGray
            } else if on == Some(true) {
                Color::LightRed
            } else {
                Color::Gray
            },
        )))
        .render(rect, buffer);
    }
}

pub(crate) fn level_slider(ratio: Option<f64>, color: Color) -> Slider<'static> {
    let state = slider_state(ratio);
    Slider::from_state(&state)
        .orientation(SliderOrientation::Horizontal)
        .show_value(false)
        .show_handle(false)
        .filled_symbol("─")
        .empty_symbol("┄")
        .filled_color(terminal::adapt_color(color))
        .empty_color(terminal::adapt_color(Color::DarkGray))
}

pub(crate) fn render_level_slider(
    area: Rect,
    buffer: &mut Buffer,
    ratio: Option<f64>,
    color: Color,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let ratio = ratio.unwrap_or(0.0).clamp(0.0, 1.0);
    level_slider(Some(ratio), color).render(area, buffer);

    let handle_x = area.x + ((area.width.saturating_sub(1)) as f64 * ratio).round() as u16;
    let handle_y = area.y + area.height / 2;
    buffer.set_string(
        handle_x,
        handle_y,
        "●",
        terminal::adapt_style(Style::default().fg(Color::White)),
    );
}

pub(crate) fn render_labeled_slider(
    area: Rect,
    buffer: &mut Buffer,
    label: &str,
    ratio: Option<f64>,
    color: Color,
    show_handle: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let area = bounded_signal_area(area);
    let label_width = SIGNAL_LABEL_WIDTH.min(area.width.saturating_sub(1)).max(1);
    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(label_width), Constraint::Min(1)])
        .split(area);
    Paragraph::new(Line::from(Span::styled(
        format!("{label} "),
        strong_style(color),
    )))
    .render(sections[0], buffer);
    if show_handle {
        render_level_slider(sections[1], buffer, ratio, color);
    } else {
        render_colored_meter_bar(sections[1], buffer, ratio.unwrap_or(0.0));
    }
}

pub(crate) fn render_stacked_signal_rows(
    area: Rect,
    buffer: &mut Buffer,
    meter_label: &str,
    meter_ratio: Option<f64>,
    level_label: &str,
    level_ratio: Option<f64>,
    level_color: Color,
) {
    if area.width == 0 || area.height < 2 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    render_labeled_slider(
        rows[0],
        buffer,
        meter_label,
        meter_ratio,
        Color::LightGreen,
        false,
    );
    render_labeled_slider(rows[1], buffer, level_label, level_ratio, level_color, true);
}

pub(crate) fn mixer_pan_label(channel: &antelope_protocol::MixerChannelState) -> String {
    format!("PAN {}", channel.pan.display_percent())
}

pub(crate) fn mixer_level_value_label(channel: &antelope_protocol::MixerChannelState) -> String {
    channel
        .display_db()
        .map(|value| format!("LVL {} dB", value))
        .unwrap_or_else(|| "LVL ?".to_string())
}

pub(crate) fn render_mix_meter_widget(
    area: Rect,
    buffer: &mut Buffer,
    left_raw: u8,
    right_raw: u8,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    if area.height < 2 {
        let channels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        render_mix_meter_channel(channels[0], buffer, "L", left_raw);
        render_mix_meter_channel(channels[1], buffer, "R", right_raw);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(Rect::new(area.x, area.y, area.width, 2));
    render_mix_meter_channel(rows[0], buffer, "L", left_raw);
    render_mix_meter_channel(rows[1], buffer, "R", right_raw);
}

pub(crate) fn render_mix_meter_channel(area: Rect, buffer: &mut Buffer, label: &str, raw: u8) {
    use antelope_protocol::{meter_display_db, meter_ratio};

    if area.width <= MIX_METER_CHANNEL_LABEL_WIDTH + MIX_METER_DB_WIDTH {
        let text = format!("{label} {}", render_mix_meter(raw));
        Paragraph::new(Line::from(Span::styled(text, muted_style()))).render(area, buffer);
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(MIX_METER_CHANNEL_LABEL_WIDTH),
            Constraint::Min(1),
            Constraint::Length(MIX_METER_DB_WIDTH),
        ])
        .split(area);
    Paragraph::new(Line::from(Span::styled(label, strong_style(Color::White))))
        .render(sections[0], buffer);
    render_colored_meter_bar(sections[1], buffer, meter_ratio(raw));
    Paragraph::new(Line::from(Span::styled(
        format_meter_value_label(meter_display_db(raw)),
        muted_style(),
    )))
    .alignment(Alignment::Right)
    .render(sections[2], buffer);
}

pub(crate) fn render_preamp_controls_text(input: PreampInputState) -> Text<'static> {
    let phantom = if matches!(input.mode, PreampMode::Mic) {
        if input.phantom_on {
            chip(preamp_phantom_label(input), Color::Black, Color::LightRed)
        } else {
            chip(preamp_phantom_label(input), Color::Black, Color::DarkGray)
        }
    } else {
        chip(preamp_phantom_label(input), Color::Black, Color::Gray)
    };
    let phase = if input.mode_raw & 0x40 != 0 {
        chip(preamp_phase_label(input), Color::Black, Color::Yellow)
    } else {
        chip(preamp_phase_label(input), Color::Black, Color::LightGreen)
    };
    Text::from(Line::from(vec![
        chip(ADJUST_DOWN_BUTTON_LABEL, Color::Black, Color::Gray),
        Span::raw(" "),
        chip(ADJUST_UP_BUTTON_LABEL, Color::Black, Color::Gray),
        Span::raw(" "),
        chip(
            input.mode.label(),
            Color::Black,
            style_for_preamp_mode(input.mode),
        ),
        Span::raw(" "),
        phantom,
        Span::raw(" "),
        phase,
    ]))
}
