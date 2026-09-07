use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use tui_slider::{Slider, SliderOrientation};

use crate::app::{AppState, FocusArea};
use crate::terminal;
use antelope_protocol::{
    meter_display_db, meter_ratio, DynamicMixerStrip, OutputMode, OutputState, PreampInputState,
    PreampMode,
};

use super::super::layouts::*;
#[cfg(test)]
use super::super::mouse::MixMeterState;
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
    let level_row = Rect::new(area.x, area.y.saturating_add(1), area.width, 1);
    let controls_row = Rect::new(area.x, area.y.saturating_add(2), area.width, 1);

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
    Paragraph::new(Line::from(header)).render(Rect::new(area.x, area.y, area.width, 1), buffer);
    render_labeled_slider(
        level_row,
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
    .render(controls_row, buffer);
}

fn output_meter_values(
    state: &AppState,
    index: usize,
) -> Option<(bool, Vec<(String, Option<u8>)>)> {
    let output = state.outputs().get(index)?;
    let lanes = state.output_meter_lanes(output.address.id);
    if lanes.is_empty() {
        return None;
    }
    let is_stereo = lanes.len() == 2 && lanes.iter().map(|(lane, _)| *lane).eq([0_u8, 1_u8]);
    let values = lanes
        .into_iter()
        .map(|(lane, value)| {
            let label = if is_stereo {
                if lane == 0 {
                    "L".to_string()
                } else {
                    "R".to_string()
                }
            } else {
                format!("LANE{}", lane.saturating_add(1))
            };
            (label, value)
        })
        .collect();
    Some((state.output_meter_is_provisional(output.address.id), values))
}

fn output_meter_parts(state: &AppState, index: usize) -> Option<(bool, Vec<(String, String)>)> {
    let (provisional, lanes) = output_meter_values(state, index)?;
    let values = lanes
        .into_iter()
        .map(|(label, value)| {
            let reading = match value {
                None => "--".to_string(),
                Some(0x60) => "silence".to_string(),
                Some(raw) => meter_display_db(raw)
                    .map(|db| format!("{db}dB"))
                    .unwrap_or_else(|| "<-60dB".to_string()),
            };
            (label, reading)
        })
        .collect();
    Some((provisional, values))
}

pub(crate) fn output_meter_status(state: &AppState, index: usize) -> Option<String> {
    let (provisional, lanes) = output_meter_parts(state, index)?;
    let values = lanes
        .iter()
        .map(|(label, reading)| format!("{label}:{reading}"))
        .collect::<Vec<_>>()
        .join(" ");
    let status = if provisional {
        "P-FEED(stage?)"
    } else {
        "METER"
    };
    Some(format!("{status} {values}"))
}

fn render_dynamic_output_name(
    area: Rect,
    buffer: &mut Buffer,
    name: &str,
    accent: Color,
    provisional: bool,
) {
    let mut spans = vec![chip(name, Color::Black, accent)];
    if provisional {
        spans.push(Span::raw(" "));
        spans.push(Span::styled("P-F", muted_style()));
    }
    Paragraph::new(Line::from(spans)).render(area, buffer);
}

fn render_output_meter_lane(area: Rect, buffer: &mut Buffer, label: &str, value: Option<u8>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let compact_label = if label.starts_with("LANE") && area.width < 9 {
        label.strip_prefix("LANE").unwrap_or(label)
    } else {
        label
    };
    let label_width = u16::try_from(compact_label.chars().count())
        .unwrap_or(u16::MAX)
        .saturating_add(1)
        .min(area.width);
    Paragraph::new(Line::from(Span::styled(
        format!("{compact_label} "),
        strong_style(Color::White),
    )))
    .render(Rect::new(area.x, area.y, label_width, 1), buffer);
    let reading = Rect::new(
        area.x.saturating_add(label_width),
        area.y,
        area.width.saturating_sub(label_width),
        1,
    );
    match value {
        Some(raw) => render_colored_meter_bar(reading, buffer, meter_ratio(raw)),
        None => {
            Paragraph::new(Line::from(Span::styled("--", muted_style()))).render(reading, buffer)
        }
    }
}

fn render_inline_output_meters(
    area: Rect,
    buffer: &mut Buffer,
    name: &str,
    accent: Color,
    provisional: bool,
    lanes: &[(String, Option<u8>)],
) {
    if area.width == 0 || area.height == 0 || lanes.is_empty() {
        render_dynamic_output_name(area, buffer, name, accent, provisional);
        return;
    }

    let marker_width: u16 = if provisional { 4 } else { 0 };
    let name_width = u16::try_from(name.chars().count())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let lane_minimum = u16::try_from(lanes.len())
        .unwrap_or(u16::MAX)
        .saturating_mul(3);
    let show_name = area.width
        >= marker_width
            .saturating_add(name_width)
            .saturating_add(lane_minimum);
    let mut x = area.x;
    if show_name {
        let rect = Rect::new(x, area.y, name_width.min(area.width), 1);
        Paragraph::new(Line::from(chip(name, Color::Black, accent))).render(rect, buffer);
        x = x.saturating_add(rect.width);
    }
    if provisional {
        let width = marker_width.min(area.x.saturating_add(area.width).saturating_sub(x));
        Paragraph::new(Line::from(Span::styled("P-F ", muted_style())))
            .render(Rect::new(x, area.y, width, 1), buffer);
        x = x.saturating_add(width);
    }

    for (lane_index, (label, value)) in lanes.iter().enumerate() {
        let lanes_left = u16::try_from(lanes.len().saturating_sub(lane_index))
            .unwrap_or(u16::MAX)
            .max(1);
        let available = area.x.saturating_add(area.width).saturating_sub(x);
        let width = if lane_index + 1 == lanes.len() {
            available
        } else {
            available / lanes_left
        };
        render_output_meter_lane(Rect::new(x, area.y, width, 1), buffer, label, *value);
        x = x.saturating_add(width);
    }
}

fn render_dynamic_output_header(
    area: Rect,
    buffer: &mut Buffer,
    state: &AppState,
    index: usize,
    name: &str,
    accent: Color,
) {
    let Some((provisional, lanes)) = output_meter_values(state, index) else {
        render_dynamic_output_name(area, buffer, name, accent, false);
        return;
    };
    render_inline_output_meters(area, buffer, name, accent, provisional, &lanes);
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
    if controls.row.height >= output_card_control_height() {
        let meter_values = output_meter_values(state, index);
        let meter_count = meter_values.as_ref().map_or(0, |(_, lanes)| {
            u16::try_from(lanes.len()).unwrap_or(u16::MAX)
        });
        let stacked_meters = meter_count > 1
            && controls.row.height
                >= output_card_control_height().saturating_add(meter_count.saturating_sub(1));
        if let Some((provisional, lanes)) = meter_values.as_ref().filter(|_| stacked_meters) {
            let dedicated_header =
                controls.row.height >= output_card_control_height().saturating_add(meter_count);
            if dedicated_header {
                render_dynamic_output_name(
                    controls.header,
                    buffer,
                    &output.name,
                    accent,
                    *provisional,
                );
            } else {
                render_inline_output_meters(
                    controls.header,
                    buffer,
                    &output.name,
                    accent,
                    *provisional,
                    &lanes[..1],
                );
            }
            for (lane_index, (label, value)) in lanes
                .iter()
                .enumerate()
                .skip(usize::from(!dedicated_header))
            {
                let meter_row = if dedicated_header {
                    lane_index
                } else {
                    lane_index.saturating_sub(1)
                };
                let y_offset = output_card_control_height()
                    .saturating_add(u16::try_from(meter_row).unwrap_or(u16::MAX));
                render_output_meter_lane(
                    Rect::new(
                        controls.row.x,
                        controls.row.y.saturating_add(y_offset),
                        controls.row.width,
                        1,
                    ),
                    buffer,
                    label,
                    *value,
                );
            }
        } else {
            render_dynamic_output_header(
                controls.header,
                buffer,
                state,
                index,
                &output.name,
                accent,
            );
        }
        if let (Some(_), Some(semantics)) = (
            controls.level,
            state.output_semantics(antelope_protocol::OutputControl::Level),
        ) {
            let value = output.level.unwrap_or(semantics.min);
            render_labeled_slider(
                Rect::new(
                    controls.row.x,
                    controls.row.y.saturating_add(1),
                    controls.row.width,
                    1,
                ),
                buffer,
                &format!("LVL {} dB", output_display_db(value, semantics)),
                Some(output_ratio(value, semantics)),
                Color::LightGreen,
                true,
            );
        }
        let dim = output.dimmed == Some(true);
        let mute = output.muted == Some(true);
        let mono = output.mono == Some(true);
        if let Some(rect) = controls.dim {
            Paragraph::new(Line::from(chip(
                "DIM",
                Color::Black,
                if dim { Color::Yellow } else { Color::Gray },
            )))
            .render(rect, buffer);
        }
        if let Some(rect) = controls.mute {
            Paragraph::new(Line::from(chip(
                "MUTE",
                Color::Black,
                if mute { Color::LightRed } else { Color::Gray },
            )))
            .render(rect, buffer);
        }
        if let Some(rect) = controls.mono {
            Paragraph::new(Line::from(chip(
                "MONO",
                Color::Black,
                if mono {
                    Color::LightCyan
                } else if output.mono.is_some() {
                    Color::Gray
                } else {
                    Color::DarkGray
                },
            )))
            .render(rect, buffer);
        }
        return;
    }
    render_dynamic_output_header(controls.header, buffer, state, index, &output.name, accent);
    if let Some(rect) = controls.level {
        let enabled = state
            .ui_profile
            .supports_output(output.address, antelope_protocol::OutputControl::Level);
        let label = match (
            output.level,
            state.output_semantics(antelope_protocol::OutputControl::Level),
        ) {
            (Some(value), Some(semantics)) => {
                format!("LVL {} dB", output_display_db(value, semantics))
            }
            _ => "LVL ?".into(),
        };
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
    if let Some(rect) = controls.mono {
        let enabled = state
            .ui_profile
            .supports_output(output.address, antelope_protocol::OutputControl::Mono);
        Paragraph::new(Line::from(chip(
            "MONO",
            Color::Black,
            if enabled && output.mono == Some(true) {
                Color::LightCyan
            } else if enabled && output.mono.is_some() {
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
        &meter_slider_label("OBS", input.observed_meter),
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

pub(crate) fn dynamic_meter_value_label(raw: Option<u8>) -> String {
    match raw {
        None => "?".into(),
        Some(raw) => {
            meter_display_db(raw).map_or_else(|| "-∞ dB".into(), |value| format!("{value} dB"))
        }
    }
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
    if let Some(rect) = controls.source {
        let source = mixer_source_label(state, address);
        Paragraph::new(Line::from(chip(&source, Color::Black, Color::LightCyan)))
            .alignment(Alignment::Right)
            .render(rect, buffer);
    }
    let enabled = |control| state.ui_profile.supports_mixer(address.surface, control);
    if let Some(rect) = controls.pan {
        let ratio = state
            .mixer_range(address.surface, antelope_protocol::MixerControl::Pan)
            .and_then(|(min, max)| {
                strip
                    .pan
                    .map(|value| ((value - min) as f64 / (max - min).max(1) as f64).clamp(0.0, 1.0))
            })
            .unwrap_or(0.5);
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
    let semantics = state.mixer_fader(address.surface);
    let rows = mixer_strip_rows(controls.card);
    Paragraph::new(Line::from(Span::styled(
        dynamic_meter_value_label(strip.meter),
        strong_style(Color::LightGreen),
    )))
    .alignment(Alignment::Center)
    .render(rows[4], buffer);
    render_vertical_combo_strip(
        rows[5],
        buffer,
        strip.meter.and_then(meter_display_db),
        semantics.and_then(|semantics| strip.fader.map(|value| fader_ratio(value, semantics))),
        None,
    );
    if controls.fader.is_some() {
        Paragraph::new(Line::from(Span::styled(
            semantics.map_or_else(
                || "LVL ?".into(),
                |semantics| {
                    strip.fader.map_or_else(
                        || "LVL ?".into(),
                        |value| format!("LVL {} dB", fader_display_db(value, semantics)),
                    )
                },
            ),
            strong_style(Color::Yellow),
        )))
        .render(rows[6], buffer);
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

#[cfg(test)]
pub(crate) fn render_mix_meter_widget(area: Rect, buffer: &mut Buffer, meter: &MixMeterState) {
    if area.width == 0 || area.height == 0 || meter.lanes.is_empty() {
        return;
    }

    let lane_count = meter.lanes.len();
    if usize::from(area.height) < lane_count {
        let columns = (0..lane_count)
            .map(|_| Constraint::Ratio(1, lane_count as u32))
            .collect::<Vec<_>>();
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(columns)
            .split(area);
        for (column, lane) in columns.iter().zip(&meter.lanes) {
            render_mix_meter_channel(*column, buffer, &meter.lane_label(lane.lane), lane.value);
        }
        return;
    }

    let rows = (0..lane_count)
        .map(|_| Constraint::Length(1))
        .collect::<Vec<_>>();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(rows)
        .split(Rect::new(area.x, area.y, area.width, lane_count as u16));
    for (row, lane) in rows.iter().zip(&meter.lanes) {
        render_mix_meter_channel(*row, buffer, &meter.lane_label(lane.lane), lane.value);
    }
}

#[cfg(test)]
pub(crate) fn render_mix_meter_channel(area: Rect, buffer: &mut Buffer, label: &str, raw: u8) {
    use antelope_protocol::{meter_display_db, meter_ratio};

    let label_width = MIX_METER_CHANNEL_LABEL_WIDTH.max(label.len() as u16);
    if area.width <= label_width + MIX_METER_DB_WIDTH {
        let text = format!("{label} {}", render_mix_meter(raw));
        Paragraph::new(Line::from(Span::styled(text, muted_style()))).render(area, buffer);
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(label_width),
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
