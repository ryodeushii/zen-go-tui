use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::ListItem;

use crate::app::{AppState, QUERY_REPLY_VISIBLE_COUNT};
use antelope_protocol::{
    OFFSET_MIX1_LANE_A, OFFSET_MIX1_LANE_B, OFFSET_MIX2_LANE_A, OFFSET_MIX2_LANE_B,
    OFFSET_SURFACE_SELECTOR, SNAPSHOT_PAYLOAD_OFFSET, SURFACE_CODE_HP2, SURFACE_CODE_MONITOR_HP1,
};

use super::super::layouts::CONNECTION_STALE_AFTER;
use super::super::layouts::current_sample_rate_label;
use super::super::styles::{
    chip, labeled_value_chip, muted_style, strong_style, style_for_ascii_byte,
    style_for_hex_byte,
};
use super::super::widgets::signals::render_mix_meter;

pub(crate) fn render_query_reply_panel(_state_bytes: &[u8], state: &AppState) -> Text<'static> {
    state
        .selected_query_reply_entry()
        .map(|entry| {
            render_full_packet_dump(
                &entry.raw,
                state
                    .raw_view
                    .baseline_raw_75
                    .as_ref()
                    .map(|a| a.as_slice()),
            )
        })
        .unwrap_or_else(|| Text::from("No 0x75 reply selected yet."))
}

pub(crate) fn render_query_request_panel(state_bytes: &[u8], state: &AppState) -> Text<'static> {
    let mut lines = render_full_packet_dump(
        state_bytes,
        state
            .raw_view
            .baseline_raw_74
            .as_ref()
            .map(|a| a.as_slice()),
    )
    .lines;
    if !state.raw_view.recent_query_request_log.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from("Recent 0x74 requests:"));
        for entry in state.raw_view.recent_query_request_log.iter().rev().take(8) {
            lines.push(Line::from(entry.clone()));
        }
    }
    Text::from(lines)
}

pub(crate) fn build_query_reply_list_items(state: &AppState) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    if state.raw_view.recent_query_reply_entries.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "Waiting for first 0x75 query reply...",
            muted_style(),
        ))));
        return items;
    }
    let total = state.raw_view.recent_query_reply_entries.len();
    let visible = QUERY_REPLY_VISIBLE_COUNT.min(total);
    let start = state
        .raw_view
        .query_reply_scroll
        .min(total.saturating_sub(visible));
    let end = (start + visible).min(total);
    for rev_index in start..end {
        let index = total - 1 - rev_index;
        let entry = &state.raw_view.recent_query_reply_entries[index];
        let marker = if state.raw_view.selected_query_reply_entry == Some(index) {
            ">"
        } else {
            " "
        };
        items.push(ListItem::new(Line::from(format!(
            "{} {}",
            marker, entry.summary
        ))));
    }
    items
}

pub(crate) fn render_mix_meter_state_line(state: &AppState) -> String {
    let Some(bytes) = state.raw_view.latest_raw_73.as_ref().map(|a| a.as_slice()) else {
        return "Mix meter: waiting for 0x73 snapshot".to_string();
    };
    let Some(payload) = bytes.get(SNAPSHOT_PAYLOAD_OFFSET..) else {
        return "Mix meter: short 0x73 snapshot".to_string();
    };

    match payload.get(OFFSET_SURFACE_SELECTOR).copied() {
        Some(SURFACE_CODE_MONITOR_HP1) => {
            let lane_a = payload.get(OFFSET_MIX1_LANE_A).copied().unwrap_or(0);
            let lane_b = payload.get(OFFSET_MIX1_LANE_B).copied().unwrap_or(0);
            format!(
                "MIX 1 L {} R {}",
                render_mix_meter(lane_a),
                render_mix_meter(lane_b),
            )
        }
        Some(SURFACE_CODE_HP2) => {
            let lane_a = payload.get(OFFSET_MIX2_LANE_A).copied().unwrap_or(0);
            let lane_b = payload.get(OFFSET_MIX2_LANE_B).copied().unwrap_or(0);
            format!(
                "MIX 2 L {} R {}",
                render_mix_meter(lane_a),
                render_mix_meter(lane_b),
            )
        }
        Some(surface) => format!("Mix meter: unsupported surface {:02x}", surface),
        None => "Mix meter: missing surface byte".to_string(),
    }
}

pub(crate) fn render_device_header(state: &AppState) -> Line<'static> {
    let product = state
        .device
        .status
        .metadata
        .as_ref()
        .map(|metadata| metadata.product_name.clone())
        .unwrap_or_else(|| "ZEN GO SYNERGY CORE".to_string());
    let sample = current_sample_rate_label(state);
    let clock = state
        .device
        .status
        .clock_source
        .map(|value| value.label().to_string())
        .unwrap_or_else(|| "clock ?".to_string());
    let lock = if state.device.status.lock_known {
        if state.device.status.locked == Some(true) {
            "locked"
        } else {
            "unlocked"
        }
    } else {
        "lock ?"
    };
    let connection = if state.device.connection.connected {
        "connected"
    } else {
        "waiting"
    };
    Line::from(vec![
        Span::styled(product, strong_style(Color::LightGreen)),
        Span::raw("  "),
        chip(
            &connection.to_uppercase(),
            Color::Black,
            connection_badge_color(state),
        ),
        Span::raw(" "),
        chip(&sample, Color::Black, Color::Yellow),
        Span::raw(" "),
        chip(&clock, Color::Black, Color::LightBlue),
        Span::raw(" "),
        chip(&lock.to_uppercase(), Color::Black, Color::Magenta),
    ])
}

pub(crate) fn render_device_metadata(state: &AppState) -> Line<'static> {
    if let Some(metadata) = state.device.status.metadata.as_ref() {
        Line::from(vec![
            labeled_value_chip(
                "SN",
                &metadata.serial,
                metadata.serial.chars().count(),
                Color::Black,
                Color::LightCyan,
            ),
            Span::raw(" "),
            labeled_value_chip(
                "HW",
                &metadata.hardware_version,
                4,
                Color::Black,
                Color::LightMagenta,
            ),
        ])
    } else {
        Line::from(Span::styled("metadata pending", muted_style()))
    }
}

pub(crate) fn render_system_summary(state: &AppState) -> Line<'static> {
    let raw_color = if state.popup.raw_view_open {
        Color::Yellow
    } else {
        Color::LightRed
    };
    let options_color = if state.popup.options_open {
        Color::Yellow
    } else {
        Color::Cyan
    };
    Line::from(vec![
        chip("RAW", Color::Black, raw_color),
        Span::raw(" "),
        chip("OPTNS", Color::Black, options_color),
        Span::raw(" "),
        chip("X", Color::Black, Color::DarkGray),
    ])
}

pub(crate) fn connection_badge_color(state: &AppState) -> Color {
    if state.device.connection.connected {
        Color::LightGreen
    } else if state
        .device
        .connection
        .last_snapshot_at
        .is_some_and(|instant| instant.elapsed() >= CONNECTION_STALE_AFTER)
    {
        Color::LightRed
    } else {
        Color::Rgb(255, 165, 0)
    }
}

pub(crate) fn render_status_strip(state: &AppState) -> Line<'static> {
    Line::from(Span::styled(
        render_mix_meter_state_line(state),
        muted_style(),
    ))
}

pub(crate) fn render_hotkeys_popup_text() -> Text<'static> {
    Text::from(vec![
        Line::from("Global"),
        Line::from("  q quit   ? hotkeys   Esc close popup"),
        Line::from("  Ctrl+c quit   Ctrl+d raw inspector"),
        Line::from(""),
        Line::from("Navigation"),
        Line::from("  Tab cycle focus   Left/Right move selection"),
        Line::from("  Up/Down adjust focused control or popup selection"),
        Line::from("  Enter confirm popup selection"),
        Line::from(""),
        Line::from("Mixer Page"),
        Line::from("  Outputs: m mute   d dim   Up/Down volume"),
        Line::from("  Mixer strips: o solo   a assignment   l link"),
        Line::from("  [ ] pan   1/2 surface"),
        Line::from("  Preamp: m phantom   3 mode   Up/Down gain"),
        Line::from(""),
        Line::from("Popups"),
        Line::from("  r routing (USB recording assignments)"),
        Line::from("  p profiles (save/load/rename/delete)"),
        Line::from("  Profiles: s save   r rename   d delete"),
        Line::from(""),
        Line::from("Raw Inspector (Ctrl+d)"),
        Line::from("  Left/Right cycle tabs or Query75 history"),
        Line::from("  b capture baseline   x clear baseline"),
        Line::from("  R refresh queries"),
        Line::from(""),
        Line::from("Device"),
        Line::from("  s cycle sample rate   c cycle clock source"),
        Line::from(""),
        Line::from(Span::styled(
            "Mouse: click controls, scroll sliders, wheel raw list",
            muted_style(),
        )),
    ])
}

pub(crate) fn render_full_packet_dump(bytes: &[u8], baseline: Option<&[u8]>) -> Text<'static> {
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

pub(crate) fn render_dump_line(
    offset: usize,
    chunk: &[u8],
    baseline: Option<&[u8]>,
) -> Line<'static> {
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
