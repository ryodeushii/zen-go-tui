use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders};

use crate::app::{AppState, RawMapScope, RawPacketTab, MIXER_STRIP_PAGE_SIZE};
use antelope_protocol::{
    FaderDirection, FaderSemantics, InputControl, MixerAddress, MixerControl, OutputControl,
    RuntimeInputControlKind,
};

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
    let Some(metadata) = state.device.status.metadata.as_ref() else {
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
    if let Some(hz) = state.device.status.sample_rate_hz {
        if hz % 1000 == 0 {
            return format!("{} kHz", hz / 1000);
        }
        let khz = hz as f64 / 1000.0;
        return format!("{khz:.1} kHz");
    }

    state
        .device
        .status
        .sample_rate
        .map(|value| value.label())
        .unwrap_or_else(|| "rate ?".to_string())
}

pub(crate) fn device_header_hit_areas(area: Rect, state: &AppState) -> Vec<Rect> {
    let inner = device_panel_layout(area, state)[0];
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

pub(crate) fn mixer_main_layout_for_state(area: Rect, state: &AppState) -> [Rect; 2] {
    let input_rows = state
        .input_spaces
        .iter()
        .map(|space| space.inputs.len())
        .max()
        .unwrap_or(0);
    let input_height = u16::try_from(input_rows)
        .unwrap_or(u16::MAX)
        .saturating_add(3)
        .min(area.height.saturating_sub(8));
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(input_height), Constraint::Min(8)])
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
    area.width.min(MIXER_STRIP_CARD_WIDTH).clamp(1, u16::MAX)
}

pub(crate) fn mixer_strip_viewport_capacity_for_inner(area: Rect) -> usize {
    if area.width == 0 {
        return 1;
    }

    let card_width = mixer_strip_card_width(area);
    let stride = card_width + MIXER_STRIP_GAP;
    // How many full strips fit: each strip needs `stride` width except the last which needs `card_width`.
    // Formula: (area.width + GAP) / stride, clamped to at least 1.
    ((area.width.saturating_add(MIXER_STRIP_GAP)) / stride).clamp(1, u16::MAX) as usize
}

pub(crate) fn mixer_strip_visible_bounds(area: Rect, state: &AppState) -> (usize, usize) {
    let _ = area;
    let visible = visible_mixer_strips(state);
    (visible.start, visible.end)
}

pub(crate) fn mixer_strip_card_area(area: Rect, slot: usize) -> Rect {
    let card_width = mixer_strip_card_width(area);
    let stride = card_width + MIXER_STRIP_GAP;
    let x = area.x + slot as u16 * stride;
    let width = card_width.min(area.width.saturating_sub(slot as u16 * stride));
    Rect::new(x, area.y, width, area.height)
}

pub(crate) fn mixer_input_strip_area(area: Rect, state: &AppState) -> Rect {
    let master_width =
        mixer_master_area(area, state).map_or(0, |master| master.width.saturating_add(1));
    Rect::new(
        area.x.saturating_add(master_width),
        area.y,
        area.width.saturating_sub(master_width),
        area.height,
    )
}

pub(crate) fn mixer_master_area(area: Rect, state: &AppState) -> Option<Rect> {
    state
        .active_mixer_surface()
        .and_then(|index| state.mixers().get(index))
        .and_then(|surface| surface.master.as_ref())
        .map(|_| Rect::new(area.x, area.y, area.width.min(14), area.height))
}

pub(crate) fn dynamic_mixer_strip_card_area(
    area: Rect,
    state: &AppState,
    slot: usize,
    visible: usize,
) -> Rect {
    let total = state
        .active_mixer_surface()
        .and_then(|index| state.mixers().get(index))
        .map_or(0, |surface| surface.strips.len());
    if visible == 0 || total <= MIXER_STRIP_PAGE_SIZE * 2 {
        return mixer_strip_card_area(area, slot);
    }
    let visible = u16::try_from(visible).unwrap_or(u16::MAX).max(1);
    let gap_total = visible.saturating_sub(1).min(area.width);
    let available = area.width.saturating_sub(gap_total);
    let width = (available / visible).max(1);
    let stride = width.saturating_add(1);
    let offset = u16::try_from(slot)
        .unwrap_or(u16::MAX)
        .saturating_mul(stride);
    Rect::new(
        area.x.saturating_add(offset),
        area.y,
        width.min(area.width.saturating_sub(offset)),
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
    let channel_rect = Rect::new(inner.x, inner.y, chip_width("CH 00").min(inner.width), 1);
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

pub(crate) fn dynamic_surface_tab_hit_areas(area: Rect, state: &AppState) -> Vec<Rect> {
    let inner = inner_area(area);
    let labels = state
        .mixers()
        .iter()
        .map(|surface| surface.name.as_str())
        .collect::<Vec<_>>();
    inline_chip_rects(inner.x, inner.y, &labels)
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

pub(crate) fn device_picker_area(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(100);
    let height = area.height.saturating_sub(4).min(24);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

pub(crate) fn device_picker_block() -> Block<'static> {
    Block::default()
        .title(" Select Antelope device — Enter opens supported rows ")
        .borders(Borders::ALL)
}

pub(crate) fn device_picker_content_area(area: Rect) -> Rect {
    device_picker_block().inner(device_picker_area(area))
}

pub(crate) fn device_picker_row_areas(area: Rect, count: usize) -> Vec<Rect> {
    let content = device_picker_content_area(area);
    (0..count.min(usize::from(content.height)))
        .map(|row| Rect::new(content.x, content.y + row as u16, content.width, 1))
        .collect()
}

pub(crate) fn assignment_picker_area(area: Rect) -> Rect {
    let width = area.width.clamp(28, 42);
    let height = area.height.clamp(8, 22);
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
    let width = area.width.clamp(54, 86);
    let height = area.height.clamp(10, 16);
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
        .constraints([
            Constraint::Min(20),
            Constraint::Length(raw_back_button_chip_width()),
        ])
        .split(area)
        .to_vec()
}

pub(crate) fn raw_back_button_chip_width() -> u16 {
    chip_width("Back To Main")
}

pub(crate) fn raw_back_button_hit_area(header_right: Rect) -> Rect {
    let text = "Back To Main";
    let w = text.chars().count() as u16;
    // Block has 1-cell borders on all sides; text is left-aligned in inner area.
    Rect::new(header_right.x + 1, header_right.y + 1, w, 1)
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
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area)
        .to_vec()
}

pub(crate) fn raw_scope_hit_areas(area: Rect, tab: RawPacketTab) -> Vec<Rect> {
    let labels = RawMapScope::options_for(tab)
        .iter()
        .map(|scope| scope.label())
        .collect::<Vec<_>>();
    let inner = inner_area(area);
    inline_chip_rects(inner.x, inner.y, &labels)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawContentLayout {
    Wide {
        map: Rect,
        dump: Rect,
    },
    Narrow {
        map: Rect,
        dump: Rect,
    },
    WideQuery {
        history: Rect,
        map: Rect,
        dump: Rect,
    },
    NarrowQuery {
        history: Rect,
        map: Rect,
        dump: Rect,
    },
}

impl RawContentLayout {
    pub(crate) fn map(self) -> Rect {
        match self {
            Self::Wide { map, .. }
            | Self::Narrow { map, .. }
            | Self::WideQuery { map, .. }
            | Self::NarrowQuery { map, .. } => map,
        }
    }

    pub(crate) fn dump(self) -> Rect {
        match self {
            Self::Wide { dump, .. }
            | Self::Narrow { dump, .. }
            | Self::WideQuery { dump, .. }
            | Self::NarrowQuery { dump, .. } => dump,
        }
    }

    pub(crate) fn history(self) -> Option<Rect> {
        match self {
            Self::WideQuery { history, .. } | Self::NarrowQuery { history, .. } => Some(history),
            Self::Wide { .. } | Self::Narrow { .. } => None,
        }
    }

    pub(crate) fn compact_map(self) -> bool {
        matches!(self, Self::Narrow { .. } | Self::NarrowQuery { .. })
    }
}

pub(crate) fn raw_content_layout(area: Rect, query_replies: bool) -> RawContentLayout {
    if area.width >= 120 {
        if query_replies {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
                .split(area);
            let panes = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
                .split(columns[1]);
            RawContentLayout::WideQuery {
                history: columns[0],
                map: panes[0],
                dump: panes[1],
            }
        } else {
            let panes = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
                .split(area);
            RawContentLayout::Wide {
                map: panes[0],
                dump: panes[1],
            }
        }
    } else if query_replies {
        let (history, remaining) = stack_raw_history(area);
        let (map, dump) = stack_raw_map_dump(remaining);
        RawContentLayout::NarrowQuery { history, map, dump }
    } else {
        let (map, dump) = stack_raw_map_dump(area);
        RawContentLayout::Narrow { map, dump }
    }
}

fn stack_raw_history(area: Rect) -> (Rect, Rect) {
    if area.height == 0 {
        return (Rect::new(area.x, area.y, area.width, 0), area);
    }
    let history_height = area.height.min(4);
    (
        Rect::new(area.x, area.y, area.width, history_height),
        Rect::new(
            area.x,
            area.y.saturating_add(history_height),
            area.width,
            area.height.saturating_sub(history_height),
        ),
    )
}

fn stack_raw_map_dump(area: Rect) -> (Rect, Rect) {
    if area.height == 0 {
        return (Rect::new(area.x, area.y, area.width, 0), area);
    }
    let map_height = area.height.saturating_sub(1).min(6);
    (
        Rect::new(area.x, area.y, area.width, map_height),
        Rect::new(
            area.x,
            area.y.saturating_add(map_height),
            area.width,
            area.height.saturating_sub(map_height),
        ),
    )
}

#[cfg(test)]
#[allow(dead_code)]
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
    let width = area.width.clamp(44, 58);
    let height = area.height.clamp(11, 14);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn dynamic_routing_popup_area(area: Rect, state: &AppState) -> Rect {
    let compatibility_defaults =
        state.routing_assignment_available() && state.routing_capabilities.is_empty();
    let width = if compatibility_defaults {
        area.width.clamp(44, 58)
    } else {
        area.width.clamp(44, 72)
    };
    let wanted = u16::try_from(state.routing_capabilities.len())
        .unwrap_or(u16::MAX)
        .saturating_add(4);
    let height = if compatibility_defaults {
        area.height.clamp(11, 14)
    } else {
        wanted.clamp(6, area.height.max(1))
    };
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn profiles_popup_area(area: Rect) -> Rect {
    let width = area.width.clamp(44, 64);
    let height = area.height.clamp(12, 16);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn options_popup_area(area: Rect) -> Rect {
    let width = area.width.clamp(38, 52);
    let height = area.height.clamp(14, 18);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn options_popup_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner_area(area))
        .to_vec()
}

pub(crate) fn options_popup_button_rects(area: Rect) -> Vec<Rect> {
    let row = options_popup_layout(area)[4];
    inline_chip_rects(row.x, row.y, &["CLOSE"])
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
    let width = popup.width.saturating_sub(8).clamp(28, 40);
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
    let assignments = &state.mixer.channels[MixerSurface::Mix1.index()];
    let (left_index, right_index) = afx_routing_pair_channels(pair);
    let left = &assignments[left_index];
    let right = &assignments[right_index];
    [
        format!("USB {}/{}", left.channel, right.channel),
        format!("REC {}", left.channel),
        left.assignment
            .map(|assignment| assignment.short_label().to_string())
            .unwrap_or_else(|| "?".to_string()),
        format!("REC {}", right.channel),
        right
            .assignment
            .map(|assignment| assignment.short_label().to_string())
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct DynamicOutputControlRects {
    pub row: Rect,
    pub level: Option<Rect>,
    pub dim: Option<Rect>,
    pub mute: Option<Rect>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DynamicInputControlRects {
    pub row: Rect,
    pub gain: Option<Rect>,
    pub mode: Option<Rect>,
    pub phantom: Option<Rect>,
    pub phase: Option<Rect>,
    pub link: Option<Rect>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DynamicMixerControlRects {
    pub card: Rect,
    pub source: Option<Rect>,
    pub fader: Option<Rect>,
    pub pan: Option<Rect>,
    pub send: Option<Rect>,
    pub mute: Option<Rect>,
    pub solo: Option<Rect>,
    pub link: Option<Rect>,
}

pub(crate) fn dynamic_output_row_areas(area: Rect, count: usize) -> Vec<Rect> {
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    (0..count.min(usize::from(inner.height)))
        .map(|index| {
            Rect::new(
                inner.x,
                inner.y.saturating_add(index as u16),
                inner.width,
                1,
            )
        })
        .collect()
}

pub(crate) fn dynamic_output_control_rects(
    row: Rect,
    state: &AppState,
    index: usize,
) -> Option<DynamicOutputControlRects> {
    let output = state.outputs().get(index)?;
    if row.height >= output_card_height() {
        let buttons = output_control_rects(row);
        return Some(DynamicOutputControlRects {
            row,
            level: (state
                .ui_profile
                .declares_output(output.address, OutputControl::Level)
                && state.output_range(OutputControl::Level).is_some())
            .then(|| output_level_slider_rect(row)),
            dim: state
                .ui_profile
                .declares_output(output.address, OutputControl::Dim)
                .then_some(buttons[2]),
            mute: state
                .ui_profile
                .declares_output(output.address, OutputControl::Mute)
                .then_some(buttons[3]),
        });
    }
    let mut x = row.x.saturating_add(row.width.min(20));
    let end = row.x.saturating_add(row.width);
    let mut take = |width: u16| {
        let width = width.min(end.saturating_sub(x));
        let rect = Rect::new(x, row.y, width, row.height.min(1));
        x = x.saturating_add(width).saturating_add(1);
        (width > 0).then_some(rect)
    };
    let level = (state
        .ui_profile
        .declares_output(output.address, OutputControl::Level)
        && state.output_range(OutputControl::Level).is_some())
    .then(|| take(12))
    .flatten();
    let dim = state
        .ui_profile
        .declares_output(output.address, OutputControl::Dim)
        .then(|| take(5))
        .flatten();
    let mute = state
        .ui_profile
        .declares_output(output.address, OutputControl::Mute)
        .then(|| take(6))
        .flatten();
    Some(DynamicOutputControlRects {
        row,
        level,
        dim,
        mute,
    })
}

pub(crate) fn dynamic_input_rows(area: Rect, state: &AppState) -> Vec<(usize, usize, Rect)> {
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    let columns = state.input_spaces.len().max(1);
    let column_width = inner.width / u16::try_from(columns).unwrap_or(u16::MAX).max(1);
    let mut rows = Vec::new();
    for (space_index, space) in state.input_spaces.iter().enumerate() {
        let offset = u16::try_from(space_index)
            .unwrap_or(u16::MAX)
            .saturating_mul(column_width);
        let width = if space_index + 1 == columns {
            inner.width.saturating_sub(offset)
        } else {
            column_width
        };
        if state.input_spaces.len() == 1 && space.inputs.len() <= 2 {
            for (input_index, card) in preamp_bar_layout(area).into_iter().enumerate() {
                if input_index >= space.inputs.len() {
                    break;
                }
                rows.push((space_index, input_index, card));
            }
            continue;
        }
        for input_index in 0..space
            .inputs
            .len()
            .min(usize::from(inner.height.saturating_sub(1)))
        {
            rows.push((
                space_index,
                input_index,
                Rect::new(
                    inner.x.saturating_add(offset),
                    inner.y.saturating_add(1).saturating_add(input_index as u16),
                    width,
                    1,
                ),
            ));
        }
    }
    rows
}

pub(crate) fn dynamic_preamp_button_rects(
    row: Rect,
    state: &AppState,
    input: &antelope_protocol::DynamicInputState,
) -> Vec<(RuntimeInputControlKind, Rect)> {
    let mut labels = Vec::new();
    let mut kinds = Vec::new();
    let declares = |kind| {
        state
            .ui_profile
            .input_capabilities(input.address)
            .iter()
            .any(|capability| capability.kind == kind)
    };
    if declares(RuntimeInputControlKind::Gain) {
        labels.extend([ADJUST_DOWN_BUTTON_LABEL, ADJUST_UP_BUTTON_LABEL]);
        kinds.extend([RuntimeInputControlKind::Gain, RuntimeInputControlKind::Gain]);
    }
    for (kind, label) in [
        (
            RuntimeInputControlKind::Mode,
            input.mode.map_or("MODE", |_| "MODE"),
        ),
        (RuntimeInputControlKind::Phantom, "48V"),
        (RuntimeInputControlKind::Phase, "PH"),
        (RuntimeInputControlKind::Link, "LINK"),
    ] {
        if declares(kind) {
            labels.push(label);
            kinds.push(kind);
        }
    }
    inline_chip_rects(
        preamp_card_inner_layout(row)[1].x,
        preamp_card_inner_layout(row)[1].y,
        &labels,
    )
    .into_iter()
    .zip(kinds)
    .map(|(rect, kind)| (kind, rect))
    .collect()
}

pub(crate) fn dynamic_input_control_rects(
    row: Rect,
    state: &AppState,
    space_index: usize,
    input_index: usize,
) -> Option<DynamicInputControlRects> {
    let input = state
        .input_spaces
        .get(space_index)?
        .inputs
        .get(input_index)?;
    if row.height >= 3 {
        let buttons = dynamic_preamp_button_rects(row, state, input);
        let button_for = |kind| {
            buttons
                .iter()
                .find(|(candidate, _)| *candidate == kind)
                .map(|(_, rect)| *rect)
        };
        return Some(DynamicInputControlRects {
            row,
            gain: state
                .ui_profile
                .declares_input(input.address, InputControl::Gain)
                .then(|| preamp_gain_slider_rect(row)),
            mode: button_for(RuntimeInputControlKind::Mode),
            phantom: button_for(RuntimeInputControlKind::Phantom),
            phase: button_for(RuntimeInputControlKind::Phase),
            link: button_for(RuntimeInputControlKind::Link),
        });
    }
    let mut x = row.x.saturating_add(row.width.min(10));
    let end = row.x.saturating_add(row.width);
    let mut take = |width: u16| {
        let width = width.min(end.saturating_sub(x));
        let rect = Rect::new(x, row.y, width, row.height.min(1));
        x = x.saturating_add(width).saturating_add(1);
        (width > 0).then_some(rect)
    };
    let declared = |kind| {
        state
            .ui_profile
            .input_capabilities(input.address)
            .iter()
            .any(|capability| capability.kind == kind)
    };
    let gain = declared(RuntimeInputControlKind::Gain)
        .then(|| take(7))
        .flatten();
    let mode = declared(RuntimeInputControlKind::Mode)
        .then(|| take(4))
        .flatten();
    let phantom = declared(RuntimeInputControlKind::Phantom)
        .then(|| take(3))
        .flatten();
    let phase = declared(RuntimeInputControlKind::Phase)
        .then(|| take(2))
        .flatten();
    let link = declared(RuntimeInputControlKind::Link)
        .then(|| take(4))
        .flatten();
    Some(DynamicInputControlRects {
        row,
        gain,
        mode,
        phantom,
        phase,
        link,
    })
}

pub(crate) fn dynamic_mixer_control_rects(
    card: Rect,
    state: &AppState,
    address: MixerAddress,
) -> Option<DynamicMixerControlRects> {
    let surface = state
        .mixers()
        .iter()
        .find(|surface| surface.surface == address.surface)?;
    if address.strip == 0 {
        surface.master.as_ref()?;
    } else {
        surface
            .strips
            .iter()
            .find(|strip| strip.strip == address.strip)?;
    }
    let rows = mixer_strip_rows(card);
    let source = address.strip != 0 && state.routing_assignment_available();
    let fader = (state
        .ui_profile
        .declares_mixer(address.surface, MixerControl::Fader)
        && state.mixer_fader(address.surface).is_some())
    .then(|| mixer_level_slider_rect(card));
    let pan = (state
        .ui_profile
        .declares_mixer(address.surface, MixerControl::Pan)
        && state
            .mixer_range(address.surface, MixerControl::Pan)
            .is_some())
    .then_some(rows[2]);
    let send = (state
        .ui_profile
        .declares_mixer(address.surface, MixerControl::Send)
        && state
            .mixer_range(address.surface, MixerControl::Send)
            .is_some())
    .then_some(rows[4]);
    let labels = [
        state.ui_profile.declares_link(address.surface)
            && address.strip != 0
            && address.strip % 2 == 1,
        state
            .ui_profile
            .declares_mixer(address.surface, MixerControl::Solo),
        state
            .ui_profile
            .declares_mixer(address.surface, MixerControl::Mute),
    ];
    let names = labels
        .iter()
        .zip(["L", "S", "M"])
        .filter_map(|(visible, name)| visible.then_some(name))
        .collect::<Vec<_>>();
    let rects = centered_inline_chip_rects(rows[7], &names);
    let mut cursor = 0;
    let link = labels[0].then(|| {
        let rect = rects[cursor];
        cursor += 1;
        rect
    });
    let solo = labels[1].then(|| {
        let rect = rects[cursor];
        cursor += 1;
        rect
    });
    let mute = labels[2].then(|| rects[cursor]);
    let source_label = state
        .active_mixer_surface()
        .and_then(|surface| {
            address
                .strip
                .checked_sub(1)
                .and_then(|index| state.mixer.channels.get(surface)?.get(usize::from(index)))
        })
        .and_then(|channel| channel.assignment)
        .map_or("SOURCE ?", |assignment| assignment.label());
    Some(DynamicMixerControlRects {
        card,
        source: source.then(|| {
            let (_, rect) = mixer_header_chip_rects(card, source_label);
            Rect {
                x: card.x.saturating_add(card.width / 2),
                y: rect.y,
                width: card.width.saturating_sub(card.width / 2),
                height: rect.height.max(3),
            }
        }),
        fader,
        pan,
        send,
        mute,
        solo,
        link,
    })
}

#[cfg(test)]
pub(crate) fn dynamic_output_control_rects_for_test(
    state: &AppState,
    index: usize,
) -> Option<DynamicOutputControlRects> {
    let area = Rect::new(0, 0, 140, 48);
    let page = mixer_page_layout(root_chunks(area)[1]);
    let row = *dynamic_output_row_areas(page[1], state.outputs().len()).get(index)?;
    dynamic_output_control_rects(row, state, index)
}

#[cfg(test)]
pub(crate) fn dynamic_input_control_rects_for_test(
    state: &AppState,
    space_index: usize,
    input_index: usize,
) -> Option<DynamicInputControlRects> {
    let area = Rect::new(0, 0, 140, 48);
    let page = mixer_page_layout(root_chunks(area)[1]);
    let main = mixer_main_layout_for_state(page[0], state);
    let (_, _, row) = dynamic_input_rows(main[0], state)
        .into_iter()
        .find(|(space, input, _)| *space == space_index && *input == input_index)?;
    dynamic_input_control_rects(row, state, space_index, input_index)
}

#[cfg(test)]
pub(crate) fn dynamic_mixer_control_rects_for_test(
    state: &AppState,
    address: MixerAddress,
) -> Option<DynamicMixerControlRects> {
    let area = Rect::new(0, 0, 140, 48);
    let page = mixer_page_layout(root_chunks(area)[1]);
    let main = mixer_main_layout_for_state(page[0], state);
    let mixer = mixer_layout(main[1]);
    let inner = mixer_strip_panel_layout(mixer[1], false)[0];
    let card = if address.strip == 0 {
        mixer_master_area(inner, state)?
    } else {
        let surface = state
            .mixers()
            .iter()
            .find(|surface| surface.surface == address.surface)?;
        let index = surface
            .strips
            .iter()
            .position(|strip| strip.strip == address.strip)?;
        let (start, end) = mixer_strip_visible_bounds(inner, state);
        if !(start..end).contains(&index) {
            return None;
        }
        dynamic_mixer_strip_card_area(
            mixer_input_strip_area(inner, state),
            state,
            index.saturating_sub(start),
            end.saturating_sub(start),
        )
    };
    dynamic_mixer_control_rects(card, state, address)
}

#[cfg(test)]
pub(crate) fn dynamic_output_row_count_for_test(state: &AppState) -> usize {
    state.outputs().len()
}

#[cfg(test)]
pub(crate) fn dynamic_input_row_count_for_test(state: &AppState) -> usize {
    state
        .input_spaces
        .iter()
        .map(|space| space.inputs.len())
        .sum()
}

pub(crate) fn dynamic_output_card_areas(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 || area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    if count == 3 {
        return output_card_areas(area).to_vec();
    }
    let columns = count.min(3);
    let rows = count.saturating_add(columns - 1) / columns;
    let row_height = area.height / u16::try_from(rows).unwrap_or(u16::MAX).max(1);
    let column_width = area.width / u16::try_from(columns).unwrap_or(u16::MAX).max(1);
    (0..count)
        .map(|index| {
            let row = index / columns;
            let column = index % columns;
            let x_offset = u16::try_from(column)
                .unwrap_or(u16::MAX)
                .saturating_mul(column_width);
            let y_offset = u16::try_from(row)
                .unwrap_or(u16::MAX)
                .saturating_mul(row_height);
            let width = if column + 1 == columns {
                area.width.saturating_sub(x_offset)
            } else {
                column_width
            };
            let height = if row + 1 == rows {
                area.height.saturating_sub(y_offset)
            } else {
                row_height
            };
            Rect::new(
                area.x.saturating_add(x_offset),
                area.y.saturating_add(y_offset),
                width,
                height,
            )
        })
        .collect()
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

pub(crate) fn output_step_from_ratio(ratio: f64, range: (i32, i32)) -> u8 {
    let (min, max) = range;
    let value = max as f64 - ratio.clamp(0.0, 1.0) * (max - min) as f64;
    value.round().clamp(0.0, f64::from(u8::MAX)) as u8
}

pub(crate) fn fader_ratio(value: i32, semantics: FaderSemantics) -> f64 {
    let value = value.clamp(semantics.min, semantics.max);
    let span = (semantics.max - semantics.min) as f64;
    if span == 0.0 {
        return 0.0;
    }
    let ratio = (value - semantics.min) as f64 / span;
    match semantics.direction {
        FaderDirection::Direct => ratio,
        FaderDirection::Attenuation => 1.0 - ratio,
    }
}

pub(crate) fn fader_display_db(value: i32, semantics: FaderSemantics) -> i16 {
    let value = value.clamp(semantics.min, semantics.max);
    let displayed = match semantics.direction {
        FaderDirection::Direct => value,
        FaderDirection::Attenuation => semantics.unity - value,
    };
    displayed.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

pub(crate) fn mixer_level_from_ratio(ratio: f64, semantics: FaderSemantics) -> u8 {
    let ratio = ratio.clamp(0.0, 1.0);
    let value = match semantics.direction {
        FaderDirection::Direct => {
            semantics.min as f64 + ratio * (semantics.max - semantics.min) as f64
        }
        FaderDirection::Attenuation => {
            semantics.max as f64 - ratio * (semantics.max - semantics.min) as f64
        }
    }
    .round()
    .clamp(0.0, f64::from(u8::MAX)) as u8;
    value
}

pub(crate) fn visible_mixer_strips(state: &AppState) -> std::ops::Range<usize> {
    let Some(surface) = state
        .active_mixer_surface()
        .and_then(|index| state.mixers().get(index))
    else {
        return 0..0;
    };
    let start = state.mixer.strip_scroll.min(surface.strips.len());
    let end = start
        .saturating_add(state.mixer.visible_strip_count.max(1))
        .min(surface.strips.len());
    start..end
}

pub(crate) fn pan_from_ratio(ratio: f64, range: (i32, i32)) -> antelope_protocol::PanState {
    let (min, max) = range;
    let raw = min as f64 + (max - min) as f64 * ratio.clamp(0.0, 1.0);
    antelope_protocol::PanState::from_raw(raw.round().clamp(0.0, f64::from(u8::MAX)) as u8)
}

pub(crate) fn value_from_ratio(ratio: f64, range: (i32, i32)) -> i32 {
    let (min, max) = range;
    (min as f64 + (max - min) as f64 * ratio.clamp(0.0, 1.0))
        .round()
        .clamp(min as f64, max as f64) as i32
}

pub(crate) fn preamp_gain_from_ratio(range: (i32, i32), ratio: f64) -> Option<i32> {
    let (min, max) = range;
    Some(
        (min as f64 + (max - min) as f64 * ratio.clamp(0.0, 1.0))
            .round()
            .clamp(min as f64, max as f64) as i32,
    )
}

pub(crate) fn slider_state(ratio: Option<f64>) -> tui_slider::SliderState {
    tui_slider::SliderState::new(ratio.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0, 0.0, 100.0)
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
