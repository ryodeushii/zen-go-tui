use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::app::AppState;

use super::styles::chip_width;

// Constants
pub(crate) const MIXER_STRIP_CARD_WIDTH: u16 = 18;
pub(crate) const MIXER_STRIP_GAP: u16 = 1;
pub(crate) const MIXER_STRIP_DB_MARKERS: [i16; 8] = [0, 5, 10, 15, 20, 30, 40, 60];
pub(crate) const ADJUST_DOWN_BUTTON_LABEL: &str = "↓";
pub(crate) const ADJUST_UP_BUTTON_LABEL: &str = "↑";
pub(crate) const SIGNAL_LABEL_WIDTH: u16 = 12;
pub(crate) const MAX_SIGNAL_ROW_WIDTH: u16 = 40;
pub(crate) const CONNECTION_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(2);
pub(crate) const MIX_METER_YELLOW_START_RATIO: f64 = 0.8;
pub(crate) const MIX_METER_RED_START_RATIO: f64 = 0.95;
pub(crate) const MIX_METER_CHANNEL_LABEL_WIDTH: u16 = 2;
pub(crate) const MIX_METER_DB_WIDTH: u16 = 7;

pub(crate) fn root_chunks(area: Rect) -> [Rect; 2] {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(17)])
        .split(area);
    [chunks[0], chunks[1]]
}

pub(crate) fn titlebar_layout(area: Rect) -> [Rect; 2] {
    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(24), Constraint::Length(21)])
        .split(area);
    [sections[0], sections[1]]
}

pub(crate) fn device_metadata_width(state: &AppState) -> u16 {
    let Some(metadata) = state.device.metadata.as_ref() else {
        return "metadata pending".chars().count() as u16;
    };

    chip_width(&format!("SN {}", metadata.serial))
        .saturating_add(1)
        .saturating_add(chip_width(&format!("HW {}", metadata.hardware_version)))
}

pub(crate) fn device_panel_layout(area: Rect, state: &AppState) -> [Rect; 2] {
    let inner = inner_area(area);
    let metadata_width = device_metadata_width(state).min(inner.width.saturating_sub(24));
    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(24), Constraint::Length(metadata_width)])
        .split(inner);
    [sections[0], sections[1]]
}

pub(crate) fn current_sample_rate_label(state: &AppState) -> String {
    if let Some(hz) = state.device.sample_rate_hz {
        if hz % 1000 == 0 {
            return format!("{} kHz", hz / 1000);
        }
        let khz = hz as f64 / 1000.0;
        return format!("{khz:.1} kHz");
    }

    state
        .device
        .sample_rate
        .map(|value| value.label())
        .unwrap_or_else(|| "rate ?".to_string())
}

pub(crate) fn device_header_hit_areas(area: Rect, state: &AppState) -> Vec<Rect> {
    let inner = device_panel_layout(area, state)[0];
    let product = state
        .device
        .metadata
        .as_ref()
        .map(|metadata| metadata.product_name.clone())
        .unwrap_or_else(|| "ZEN GO SYNERGY CORE".to_string());
    let sample = current_sample_rate_label(state);
    let clock = state
        .device
        .clock_source
        .map(|value| value.label().to_string())
        .unwrap_or_else(|| "clock ?".to_string());

    let mut x = inner.x + product.chars().count() as u16 + 2;
    let connection_rect = Rect::new(x, inner.y, chip_width("CONNECTED"), 1);
    x = x.saturating_add(connection_rect.width + 1);
    let sample_rect = Rect::new(x, inner.y, chip_width(&sample), 1);
    x = x.saturating_add(sample_rect.width + 1);
    let clock_rect = Rect::new(x, inner.y, chip_width(&clock), 1);

    vec![connection_rect, sample_rect, clock_rect]
}

pub(crate) fn mixer_page_layout(area: Rect) -> [Rect; 2] {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(14), Constraint::Length(8)])
        .split(area);
    [sections[0], sections[1]]
}

pub(crate) fn mixer_main_layout(area: Rect) -> [Rect; 2] {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(12)])
        .split(area);
    [sections[0], sections[1]]
}

pub(crate) fn preamp_bar_layout(area: Rect) -> [Rect; 2] {
    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    [sections[0], sections[1]]
}

pub(crate) fn mixer_layout(area: Rect) -> [Rect; 2] {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(9)])
        .split(area);
    [sections[0], sections[1]]
}

pub(crate) fn mixer_strip_panel_layout(area: Rect, with_mix_meter: bool) -> [Rect; 2] {
    let inner = inner_area(area);
    if with_mix_meter && inner.height >= 3 {
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(2)])
            .split(inner);
        [sections[0], sections[1]]
    } else {
        [
            inner,
            Rect::new(inner.x, inner.y + inner.height, inner.width, 0),
        ]
    }
}

pub(crate) fn mixer_strip_card_width(area: Rect) -> u16 {
    area.width.min(MIXER_STRIP_CARD_WIDTH).max(1)
}

pub(crate) fn mixer_strip_viewport_capacity_for_inner(area: Rect) -> usize {
    if area.width == 0 {
        return 1;
    }

    let card_width = mixer_strip_card_width(area);
    ((area.width.saturating_add(MIXER_STRIP_GAP)) / (card_width + MIXER_STRIP_GAP)).max(1) as usize
}

pub(crate) fn mixer_strip_visible_bounds(area: Rect, state: &AppState) -> (usize, usize) {
    let visible = mixer_strip_viewport_capacity_for_inner(area);
    let total = state.active_mixer_channels().len();
    let start = state.mixer_strip_scroll.min(total.saturating_sub(visible));
    let end = usize::min(start + visible, total);
    (start, end)
}

pub(crate) fn mixer_strip_card_area(area: Rect, slot: usize) -> Rect {
    let card_width = mixer_strip_card_width(area);
    Rect::new(
        area.x + slot as u16 * (card_width + MIXER_STRIP_GAP),
        area.y,
        card_width,
        area.height,
    )
}

pub(crate) fn mixer_strip_inner_area(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

pub(crate) fn centered_inline_chip_rects(area: Rect, labels: &[&str]) -> Vec<Rect> {
    let total_width = inline_chip_rects(0, 0, labels)
        .last()
        .map(|rect| rect.x + rect.width)
        .unwrap_or(0);
    let x = area.x + area.width.saturating_sub(total_width) / 2;
    inline_chip_rects(x, area.y, labels)
}

pub(crate) fn mixer_header_chip_rects(area: Rect, source: &str) -> (Rect, Rect) {
    let inner = mixer_strip_inner_area(area);
    let channel_rect = Rect::new(inner.x, inner.y, chip_width("CH 16").min(inner.width), 1);
    let source_width = chip_width(source).min(inner.width);
    let source_rect = Rect::new(
        inner.x + inner.width.saturating_sub(source_width),
        inner.y,
        source_width,
        1,
    );
    (channel_rect, source_rect)
}

pub(crate) fn preamp_card_inner_layout(area: Rect) -> [Rect; 2] {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(1)])
        .split(inner_area(area));
    [sections[0], sections[1]]
}

pub(crate) fn preamp_button_rects(
    area: Rect,
    input: antelope_protocol::PreampInputState,
) -> Vec<Rect> {
    let controls = preamp_card_inner_layout(area)[1];
    inline_chip_rects(
        controls.x,
        controls.y,
        &[
            ADJUST_DOWN_BUTTON_LABEL,
            ADJUST_UP_BUTTON_LABEL,
            input.mode.label(),
            super::styles::preamp_phantom_label(input),
            super::styles::preamp_phase_label(input),
        ],
    )
}

pub(crate) fn surface_tab_hit_areas(area: Rect) -> Vec<Rect> {
    let inner = inner_area(area);
    inline_chip_rects(inner.x, inner.y, &["MIX 1 / Monitor-HP1", "MIX 2 / HP2"])
}

pub(crate) fn mixer_header_button_rects(area: Rect) -> Vec<Rect> {
    let inner = inner_area(area);
    let labels = ["PROFILES", "ROUTING"];
    let total_width = inline_chip_rects(0, 0, &labels)
        .last()
        .map(|rect| rect.x + rect.width)
        .unwrap_or(0);
    let start_x = inner.x + inner.width.saturating_sub(total_width);
    inline_chip_rects(start_x, inner.y, &labels)
}

pub(crate) fn mixer_strip_page_button_rects(area: Rect) -> Vec<Rect> {
    let labels = ["←", "→"];
    let total_width = inline_chip_rects(0, 0, &labels)
        .last()
        .map(|rect| rect.x + rect.width)
        .unwrap_or(0);
    let x = area
        .x
        .saturating_add(area.width.saturating_sub(total_width.saturating_add(2)));
    inline_chip_rects(x, area.y, &labels)
}

pub(crate) fn inline_chip_rects(x: u16, y: u16, labels: &[&str]) -> Vec<Rect> {
    let mut offset = x;
    labels
        .iter()
        .map(|label| {
            let rect = Rect::new(offset, y, chip_width(label), 1);
            offset = offset.saturating_add(rect.width).saturating_add(1);
            rect
        })
        .collect()
}

pub(crate) fn assignment_picker_area(area: Rect) -> Rect {
    let width = area.width.min(42).max(28);
    let height = area.height.min(22).max(8);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn popup_list_inner_area(popup: Rect, title: &str) -> Rect {
    super::styles::panel_block(title, ratatui::style::Color::Yellow, true).inner(popup)
}

pub(crate) fn hotkeys_popup_area(area: Rect) -> Rect {
    let width = area.width.min(86).max(54);
    let height = area.height.min(16).max(10);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn raw_header_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(18)])
        .split(area)
        .to_vec()
}

pub(crate) fn raw_tab_hit_areas(area: Rect) -> Vec<Rect> {
    inline_chip_rects(
        inner_area(area).x,
        inner_area(area).y,
        &["0x74", "0x73", "0x83", "0x75", "0x81"],
    )
}

pub(crate) fn raw_page_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area)
        .to_vec()
}

pub(crate) fn query_reply_history_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(4)])
        .split(area)
        .to_vec()
}

pub(crate) fn inner_area(area: Rect) -> Rect {
    let vertical_inset = if area.height >= 6 { 2 } else { 1 };
    let vertical_padding = vertical_inset * 2;
    Rect {
        x: area.x.saturating_add(2),
        y: area.y.saturating_add(vertical_inset),
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(vertical_padding),
    }
}

pub(crate) fn routing_popup_area(area: Rect) -> Rect {
    let width = area.width.min(58).max(44);
    let height = area.height.min(14).max(11);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn profiles_popup_area(area: Rect) -> Rect {
    let width = area.width.min(64).max(44);
    let height = area.height.min(16).max(12);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn profiles_popup_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner_area(area))
        .to_vec()
}

pub(crate) fn profiles_popup_button_rects(area: Rect) -> Vec<Rect> {
    let row = profiles_popup_layout(area)[1];
    inline_chip_rects(row.x, row.y, &["LOAD", "SAVE", "RENAME", "DELETE", "CLOSE"])
}

pub(crate) fn profile_editor_area(area: Rect) -> Rect {
    let popup = profiles_popup_area(area);
    let width = popup.width.saturating_sub(8).min(40).max(28);
    let height = 5;
    Rect {
        x: popup.x + popup.width.saturating_sub(width) / 2,
        y: popup.y + popup.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn afx_routing_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner_area(area))
        .to_vec()
}

pub(crate) fn afx_routing_pair_channels(pair: usize) -> (usize, usize) {
    (pair * 2, pair * 2 + 1)
}

pub(crate) fn afx_routing_row_columns(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(6),
        ])
        .split(area)
        .to_vec()
}

pub(crate) fn afx_routing_row_labels(state: &AppState, pair: usize) -> [String; 5] {
    use antelope_protocol::MixerSurface;
    let assignments = &state.mixer_channels[MixerSurface::Mix1.index()];
    let (left_index, right_index) = afx_routing_pair_channels(pair);
    let left = &assignments[left_index];
    let right = &assignments[right_index];
    [
        format!("USB {}/{}", left.channel, right.channel),
        format!("REC {}", left.channel),
        left.assignment
            .map(|assignment| assignment.short_label())
            .unwrap_or_else(|| "?".to_string()),
        format!("REC {}", right.channel),
        right
            .assignment
            .map(|assignment| assignment.short_label())
            .unwrap_or_else(|| "?".to_string()),
    ]
}

pub(crate) fn afx_routing_row_rects(area: Rect, state: &AppState, pair: usize) -> Vec<Rect> {
    let columns = afx_routing_row_columns(area);
    vec![
        Rect::new(
            columns[0].x,
            columns[0].y,
            chip_width("USB 7/8").min(columns[0].width),
            1,
        ),
        Rect::new(
            columns[1].x,
            columns[1].y,
            chip_width("REC 8").min(columns[1].width),
            1,
        ),
        Rect::new(
            columns[2].x,
            columns[2].y,
            columns[2]
                .width
                .min(chip_width(&afx_routing_row_labels(state, pair)[2])),
            1,
        ),
        Rect::new(
            columns[3].x,
            columns[3].y,
            chip_width("REC 8").min(columns[3].width),
            1,
        ),
        Rect::new(
            columns[4].x,
            columns[4].y,
            columns[4]
                .width
                .min(chip_width(&afx_routing_row_labels(state, pair)[4])),
            1,
        ),
    ]
}

pub(crate) fn output_card_height() -> u16 {
    3
}

pub(crate) fn output_card_areas(area: Rect) -> [Rect; 3] {
    let areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(Rect::new(area.x, area.y, area.width, output_card_height()));
    [areas[0], areas[1], areas[2]]
}

pub(crate) fn output_control_rects(area: Rect) -> Vec<Rect> {
    inline_chip_rects(
        area.x,
        area.y + output_card_height() - 1,
        &[
            ADJUST_DOWN_BUTTON_LABEL,
            ADJUST_UP_BUTTON_LABEL,
            "DIM",
            "MUTE",
        ],
    )
}

pub(crate) fn output_level_slider_rect(area: Rect) -> Rect {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    horizontal_labeled_slider_track(rows[1])
}

#[cfg(test)]
pub(crate) fn mixer_strip_height() -> u16 {
    18
}

pub(crate) fn output_hotkeys_button_rect(area: Rect) -> Rect {
    let inner = inner_area(area);
    let y = inner.y.saturating_add(output_card_height());
    if inner.height <= output_card_height() {
        return Rect::new(inner.x, y, 0, 0);
    }

    let width = chip_width("? HOTKEYS");
    Rect::new(
        inner.x + inner.width.saturating_sub(width),
        y,
        width.min(inner.width),
        1,
    )
}

pub(crate) fn preamp_gain_slider_rect(area: Rect) -> Rect {
    let signal = preamp_card_inner_layout(area)[0];
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(signal);
    horizontal_labeled_slider_track(rows[1])
}

pub(crate) fn mixer_strip_rows(area: Rect) -> [Rect; 8] {
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
        .split(mixer_strip_inner_area(area));
    [
        rows[0], rows[1], rows[2], rows[3], rows[4], rows[5], rows[6], rows[7],
    ]
}

pub(crate) fn mixer_pan_slider_rect(area: Rect) -> Rect {
    mixer_strip_rows(area)[2]
}

pub(crate) fn mixer_level_slider_rect(area: Rect) -> Rect {
    let combo = mixer_strip_rows(area)[5];
    if combo.width < 4 || combo.height == 0 {
        return Rect::new(combo.x, combo.y, 0, 0);
    }
    let content_width = 6.min(combo.width);
    let content_area = Rect::new(
        combo.x + combo.width.saturating_sub(content_width) / 2,
        combo.y,
        content_width,
        combo.height,
    );
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(content_area)[2]
}

pub(crate) fn wheel_hitbox(area: Rect) -> Rect {
    const MIN_WHEEL_HIT_WIDTH: u16 = 5;

    if area.width == 0 || area.width >= MIN_WHEEL_HIT_WIDTH {
        return area;
    }

    let extra = MIN_WHEEL_HIT_WIDTH - area.width;
    let left = extra / 2;
    let right = extra - left;
    Rect::new(
        area.x.saturating_sub(left),
        area.y,
        area.width.saturating_add(left).saturating_add(right),
        area.height,
    )
}

fn horizontal_labeled_slider_track(area: Rect) -> Rect {
    let area = bounded_signal_area(area);
    if area.width == 0 || area.height == 0 {
        return Rect::new(area.x, area.y, 0, 0);
    }
    let label_width = SIGNAL_LABEL_WIDTH.min(area.width.saturating_sub(1)).max(1);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(label_width), Constraint::Min(1)])
        .split(area)[1]
}

pub(crate) fn bounded_signal_area(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y,
        area.width.min(MAX_SIGNAL_ROW_WIDTH),
        area.height,
    )
}

pub(crate) fn output_step_from_ratio(ratio: f64) -> u8 {
    ((1.0 - ratio.clamp(0.0, 1.0)) * 96.0).round() as u8
}

pub(crate) fn mixer_level_from_ratio(ratio: f64) -> u8 {
    ((1.0 - ratio.clamp(0.0, 1.0)) * 90.0).round() as u8
}

pub(crate) fn pan_from_ratio(ratio: f64) -> antelope_protocol::PanState {
    let span = (antelope_protocol::PanState::MAX - antelope_protocol::PanState::MIN) as f64;
    let raw = antelope_protocol::PanState::MIN as f64 + span * ratio.clamp(0.0, 1.0);
    antelope_protocol::PanState::from_raw(raw.round() as u8)
}

pub(crate) fn preamp_gain_from_ratio(
    input: antelope_protocol::PreampInputState,
    ratio: f64,
) -> Option<u8> {
    use antelope_protocol::PreampMode;
    let ratio = ratio.clamp(0.0, 1.0);
    match input.mode {
        PreampMode::Mic => Some((ratio * 65.0).round() as u8),
        PreampMode::Line => Some((-6 + (ratio * 26.0).round() as i8) as u8),
        PreampMode::HiZ => Some((ratio * 45.0).round() as u8),
        PreampMode::Unknown(_) => None,
    }
}

pub(crate) fn slider_state(ratio: Option<f64>) -> tui_slider::SliderState {
    tui_slider::SliderState::new(ratio.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0, 0.0, 100.0)
}

pub(crate) fn level_db_ratio(value: Option<i16>) -> Option<f64> {
    value.map(|db| ((db.clamp(-60, 0) + 60) as f64 / 60.0).clamp(0.0, 1.0))
}

pub(crate) fn meter_db_ratio_option(value: Option<i16>) -> Option<f64> {
    value.map(antelope_protocol::meter_db_ratio)
}

pub(crate) fn meter_bar_color(cell_ratio: f64) -> ratatui::style::Color {
    use ratatui::style::Color;
    if cell_ratio >= MIX_METER_RED_START_RATIO {
        Color::LightRed
    } else if cell_ratio >= MIX_METER_YELLOW_START_RATIO {
        Color::Yellow
    } else {
        Color::LightGreen
    }
}

pub(crate) fn vertical_ratio_row(area: Rect, ratio: f64) -> u16 {
    let height = area.height.saturating_sub(1) as f64;
    area.y + area.height.saturating_sub(1) - (height * ratio.clamp(0.0, 1.0)).round() as u16
}

pub(crate) fn format_meter_value_label(value: Option<i16>) -> String {
    let mapped = value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-∞".to_string());
    format!("{:>3} dB", mapped)
}

pub(crate) fn signal_slider_label(prefix: &str, value: Option<String>) -> String {
    value
        .filter(|value| !value.is_empty())
        .map(|value| format!("{prefix} {value}"))
        .unwrap_or_else(|| prefix.to_string())
}

pub(crate) fn meter_slider_label(prefix: &str, value: Option<i16>) -> String {
    format!("{prefix} {}", format_meter_value_label(value))
}
