use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::terminal;
use antelope_protocol::{meter_display_db, meter_ratio};

use super::super::layouts::*;
use super::super::styles::*;

pub(crate) fn render_colored_meter_bar(area: Rect, buffer: &mut Buffer, ratio: f64) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let filled_cells = (ratio.clamp(0.0, 1.0) * area.width as f64).round() as u16;
    let yellow_start = (area.width as f64 * MIX_METER_YELLOW_START_RATIO).floor() as u16;
    let red_start = (area.width as f64 * MIX_METER_RED_START_RATIO).floor() as u16;

    for offset in 0..area.width {
        let x = area.x + offset;
        let filled = offset < filled_cells;
        let color = if !filled {
            Color::DarkGray
        } else if offset >= red_start {
            Color::LightRed
        } else if offset >= yellow_start {
            Color::Yellow
        } else {
            Color::LightGreen
        };
        buffer[(x, area.y)]
            .set_symbol(if filled { "█" } else { "░" })
            .set_style(terminal::adapt_style(Style::default().fg(color)));
    }
}

pub(crate) fn render_mix_meter(raw: u8) -> String {
    let bar = render_symbol_bar(meter_ratio(raw), 8, '█', '░');
    format!(
        "{} {}",
        bar,
        format_meter_value_label(meter_display_db(raw))
    )
}
