use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget, Wrap};
use ratatui::Frame;
use tui_slider::{Slider, SliderOrientation, SliderState};

use crate::app::{
    AppState, AssignmentPickerState, FocusArea, MainPage, RawPacketTab, SelectorPopupKind,
    SelectorPopupState,
};
use crate::protocol::{
    meter_display_db, meter_ratio, ClockSource, MixerAssignment, MixerSurface, OutputMode,
    OutputState, PanState, PreampInputState, PreampMode, SampleRate, Surface,
};
use crate::terminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    ToggleRawView,
    ToggleHotkeysPopup,
    OpenRoutingPopup,
    CloseRoutingPopup,
    OpenSampleRateSelector,
    OpenClockSourceSelector,
    SelectPage(MainPage),
    SelectOutput(usize),
    AdjustOutputLevel {
        index: usize,
        increase: bool,
    },
    SetOutputLevel {
        index: usize,
        step: u8,
    },
    ToggleOutputDim(usize),
    ToggleOutputMute(usize),
    SelectRawPacketTab(RawPacketTab),
    SelectQueryReplyEntry(usize),
    SelectSurface(Surface),
    SelectMixerChannel(usize),
    AdjustMixerLevel {
        index: usize,
        increase: bool,
    },
    SetMixerLevel {
        index: usize,
        level: u8,
    },
    AdjustMixerPan {
        index: usize,
        right: bool,
    },
    SetMixerPan {
        index: usize,
        pan: PanState,
    },
    ToggleMixerMute(u8),
    ToggleMixerSolo(u8),
    ToggleMixerLink(u8),
    OpenAssignmentPicker(u8),
    PickAssignment {
        strip: u8,
        assignment: MixerAssignment,
    },
    CloseAssignmentPicker,
    SelectPreampInput(usize),
    AdjustPreampGain {
        input: u8,
        increase: bool,
    },
    SetPreampGain {
        input: u8,
        raw: u8,
    },
    OpenPreampModeSelector(u8),
    CyclePreampMode(u8),
    PickSampleRate(SampleRate),
    PickClockSource(ClockSource),
    PickPreampMode {
        input: u8,
        mode: PreampMode,
    },
    CloseSelectorPopup,
    TogglePreampPhase(u8),
    TogglePreampPhantom(u8),
}

pub fn draw(frame: &mut Frame<'_>, state: &AppState) {
    if state.raw_view_open {
        draw_raw_page(frame, frame.area(), state);
        draw_hotkeys_popup(frame, frame.area(), state);
        return;
    }

    let chunks = root_chunks(frame.area());

    draw_titlebar(frame, chunks[0], state);
    draw_mixer_page(frame, chunks[1], state);
    draw_routing_popup(frame, frame.area(), state);
    draw_assignment_picker(frame, frame.area(), state);
    draw_selector_popup(frame, frame.area(), state);
    draw_hotkeys_popup(frame, frame.area(), state);
}

fn root_chunks(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(17)])
        .split(area)
        .to_vec()
}

fn titlebar_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(24), Constraint::Length(21)])
        .split(area)
        .to_vec()
}

fn device_panel_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(24), Constraint::Length(26)])
        .split(inner_area(area))
        .to_vec()
}

fn device_header_hit_areas(area: Rect, state: &AppState) -> Vec<Rect> {
    let inner = device_panel_layout(area)[0];
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

fn mixer_page_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(14), Constraint::Length(8)])
        .split(area)
        .to_vec()
}

fn mixer_main_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(12)])
        .split(area)
        .to_vec()
}

fn preamp_bar_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area)
        .to_vec()
}

fn mixer_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(9)])
        .split(area)
        .to_vec()
}

fn mixer_strip_panel_layout(area: Rect, with_mix_meter: bool) -> Vec<Rect> {
    let inner = inner_area(area);
    if with_mix_meter && inner.height >= 3 {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(2)])
            .split(inner)
            .to_vec()
    } else {
        vec![
            inner,
            Rect::new(inner.x, inner.y + inner.height, inner.width, 0),
        ]
    }
}

const MIXER_STRIP_CARD_WIDTH: u16 = 18;
const MIXER_STRIP_GAP: u16 = 1;
const MIXER_STRIP_DB_MARKERS: [i16; 8] = [0, 5, 10, 15, 20, 30, 40, 60];

fn mixer_strip_card_width(area: Rect) -> u16 {
    area.width.min(MIXER_STRIP_CARD_WIDTH).max(1)
}

fn mixer_strip_viewport_capacity_for_inner(area: Rect) -> usize {
    if area.width == 0 {
        return 1;
    }

    let card_width = mixer_strip_card_width(area);
    ((area.width.saturating_add(MIXER_STRIP_GAP)) / (card_width + MIXER_STRIP_GAP)).max(1) as usize
}

fn mixer_strip_visible_bounds(area: Rect, state: &AppState) -> (usize, usize) {
    let visible = mixer_strip_viewport_capacity_for_inner(area);
    let total = state.active_mixer_channels().len();
    let start = state.mixer_strip_scroll.min(total.saturating_sub(visible));
    let end = usize::min(start + visible, total);
    (start, end)
}

fn mixer_strip_card_area(area: Rect, slot: usize) -> Rect {
    let card_width = mixer_strip_card_width(area);
    Rect::new(
        area.x + slot as u16 * (card_width + MIXER_STRIP_GAP),
        area.y,
        card_width,
        area.height,
    )
}

fn mixer_strip_inner_area(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

fn centered_inline_chip_rects(area: Rect, labels: &[&str]) -> Vec<Rect> {
    let total_width = inline_chip_rects(0, 0, labels)
        .last()
        .map(|rect| rect.x + rect.width)
        .unwrap_or(0);
    let x = area.x + area.width.saturating_sub(total_width) / 2;
    inline_chip_rects(x, area.y, labels)
}

fn mixer_header_chip_rects(area: Rect, source: &str) -> (Rect, Rect) {
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

fn preamp_card_inner_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(1)])
        .split(inner_area(area))
        .to_vec()
}

const ADJUST_DOWN_BUTTON_LABEL: &str = "↓";
const ADJUST_UP_BUTTON_LABEL: &str = "↑";

fn preamp_button_rects(area: Rect, input: PreampInputState) -> Vec<Rect> {
    let controls = preamp_card_inner_layout(area)[1];
    inline_chip_rects(
        controls.x,
        controls.y,
        &[
            ADJUST_DOWN_BUTTON_LABEL,
            ADJUST_UP_BUTTON_LABEL,
            input.mode.label(),
            preamp_phantom_label(input),
            preamp_phase_label(input),
        ],
    )
}

fn surface_tab_hit_areas(area: Rect) -> Vec<Rect> {
    let inner = inner_area(area);
    inline_chip_rects(inner.x, inner.y, &["MIX 1 / Monitor-HP1", "MIX 2 / HP2"])
}

fn routing_button_rect(area: Rect) -> Rect {
    let inner = inner_area(area);
    let width = chip_width("ROUTING").min(inner.width);
    Rect::new(
        inner.x + inner.width.saturating_sub(width),
        inner.y,
        width,
        1,
    )
}

fn inline_chip_rects(x: u16, y: u16, labels: &[&str]) -> Vec<Rect> {
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

fn assignment_picker_area(area: Rect) -> Rect {
    let width = area.width.min(42).max(28);
    let height = area.height.min(22).max(8);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn popup_list_inner_area(popup: Rect, title: &str) -> Rect {
    panel_block(title, Color::Yellow, true).inner(popup)
}

fn hotkeys_popup_area(area: Rect) -> Rect {
    let width = area.width.min(86).max(54);
    let height = area.height.min(16).max(10);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn raw_header_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(18)])
        .split(area)
        .to_vec()
}

fn raw_tab_hit_areas(area: Rect) -> Vec<Rect> {
    inline_chip_rects(
        inner_area(area).x,
        inner_area(area).y,
        &["0x74", "0x73", "0x83", "0x75", "0x81"],
    )
}

fn raw_page_layout(area: Rect) -> Vec<Rect> {
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

fn query_reply_history_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(4)])
        .split(area)
        .to_vec()
}

fn inner_area(area: Rect) -> Rect {
    let vertical_inset = if area.height >= 6 { 2 } else { 1 };
    let vertical_padding = vertical_inset * 2;
    Rect {
        x: area.x.saturating_add(2),
        y: area.y.saturating_add(vertical_inset),
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(vertical_padding),
    }
}

fn draw_titlebar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let sections = titlebar_layout(area);
    frame.render_widget(panel_block("Device", Color::DarkGray, true), sections[0]);
    let device_sections = device_panel_layout(sections[0]);
    frame.render_widget(
        Paragraph::new(render_device_header(state)).wrap(Wrap { trim: false }),
        device_sections[0],
    );
    frame.render_widget(
        Paragraph::new(render_device_metadata(state))
            .alignment(Alignment::Right)
            .wrap(Wrap { trim: false }),
        device_sections[1],
    );
    frame.render_widget(
        Paragraph::new(render_inspector_summary())
            .block(panel_block("Inspector", Color::LightRed, false))
            .wrap(Wrap { trim: false }),
        sections[1],
    );
}

fn draw_mixer_page(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let sections = mixer_page_layout(area);
    let main = mixer_main_layout(sections[0]);
    draw_preamp_bar(frame, main[0], state);

    draw_mixer_main(frame, main[1], state);
    draw_output_panel(frame, sections[1], state);
}

fn routing_popup_area(area: Rect) -> Rect {
    let width = area.width.min(58).max(44);
    let height = area.height.min(14).max(11);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn draw_routing_popup(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if !state.routing_popup_open {
        return;
    }

    let popup = routing_popup_area(area);
    frame.render_widget(Clear, popup);
    frame.render_widget(panel_block("Routing", Color::Magenta, true), popup);

    let rows = afx_routing_layout(popup);
    Paragraph::new(Line::from(vec![chip(
        "ROUTING",
        Color::Black,
        Color::LightMagenta,
    )]))
    .render(rows[0], frame.buffer_mut());
    Paragraph::new(Line::from(
        "Zen Go USB recordings mirror mixer strip assignments instead of using a separate routing matrix.",
    ))
    .wrap(Wrap { trim: false })
    .render(rows[1], frame.buffer_mut());
    Paragraph::new(Line::from("Edit here and mixer strips update immediately."))
        .wrap(Wrap { trim: false })
        .render(rows[2], frame.buffer_mut());
    Paragraph::new(Line::from(vec![
        Span::styled("PAIR", subdued_style()),
        Span::raw("  "),
        Span::styled("REC 1", strong_style(Color::LightCyan)),
        Span::raw(" / "),
        Span::styled("REC 2", strong_style(Color::LightCyan)),
    ]))
    .render(rows[3], frame.buffer_mut());

    for pair in 0..4 {
        let pair_area = rows[4 + pair];
        render_afx_routing_row(pair_area, frame.buffer_mut(), state, pair);
    }

    Paragraph::new(Line::from(vec![
        Span::styled("TIP ", subdued_style()),
        Span::styled(
            "click a source chip or press `a` for the selected channel",
            muted_style(),
        ),
    ]))
    .wrap(Wrap { trim: false })
    .render(rows[8], frame.buffer_mut());
    Paragraph::new(Line::from(vec![
        Span::styled("STATUS ", subdued_style()),
        Span::styled(state.last_message.clone(), strong_style(Color::LightCyan)),
    ]))
    .wrap(Wrap { trim: false })
    .render(rows[9], frame.buffer_mut());
}

fn afx_routing_layout(area: Rect) -> Vec<Rect> {
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

fn afx_routing_pair_channels(pair: usize) -> (usize, usize) {
    (pair * 2, pair * 2 + 1)
}

fn afx_routing_row_columns(area: Rect) -> Vec<Rect> {
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

fn afx_routing_row_labels(state: &AppState, pair: usize) -> [String; 5] {
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

fn afx_routing_row_rects(area: Rect, state: &AppState, pair: usize) -> Vec<Rect> {
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

fn render_afx_routing_row(area: Rect, buffer: &mut Buffer, state: &AppState, pair: usize) {
    let labels = afx_routing_row_labels(state, pair);
    let (left_index, right_index) = afx_routing_pair_channels(pair);
    let selected_left = state.focus == FocusArea::Mixer && state.selected_channel == left_index;
    let selected_right = state.focus == FocusArea::Mixer && state.selected_channel == right_index;
    let columns = afx_routing_row_columns(area);
    let row_style = terminal::adapt_style(Style::default().fg(if pair % 2 == 0 {
        Color::DarkGray
    } else {
        Color::Gray
    }));
    for x in area.x..area.x + area.width {
        buffer[(x, area.y)].set_style(row_style);
    }

    Paragraph::new(Line::from(vec![chip(
        labels[0].clone(),
        Color::Black,
        Color::LightMagenta,
    )]))
    .render(columns[0], buffer);
    Paragraph::new(Line::from(vec![chip(
        labels[1].clone(),
        Color::Black,
        Color::Gray,
    )]))
    .render(columns[1], buffer);
    Paragraph::new(Line::from(vec![chip(
        labels[2].clone(),
        Color::Black,
        if selected_left {
            Color::Yellow
        } else {
            Color::LightCyan
        },
    )]))
    .render(columns[2], buffer);
    Paragraph::new(Line::from(vec![chip(
        labels[3].clone(),
        Color::Black,
        Color::Gray,
    )]))
    .render(columns[3], buffer);
    Paragraph::new(Line::from(vec![chip(
        labels[4].clone(),
        Color::Black,
        if selected_right {
            Color::Yellow
        } else {
            Color::LightCyan
        },
    )]))
    .render(columns[4], buffer);
}

#[cfg(test)]
fn afx_routing_source_label(assignment: Option<MixerAssignment>) -> String {
    assignment
        .map(|assignment| assignment.label())
        .unwrap_or_else(|| "?".to_string())
}

#[cfg(test)]
fn render_afx_routing_text(state: &AppState) -> Text<'static> {
    let assignments = &state.mixer_channels[MixerSurface::Mix1.index()];
    let mut lines = vec![
        Line::from(vec![chip("ROUTING", Color::Black, Color::LightMagenta)]),
        Line::from(""),
        Line::from("Zen Go USB recordings mirror mixer strip assignments instead of using a separate routing matrix."),
        Line::from("This view reformats shared CH 01-08 assignments into the 4 stereo recording pairs exposed to the host."),
        Line::from(""),
        Line::from(vec![
            Span::styled("PAIR    ", subdued_style()),
            Span::styled("LEFT", strong_style(Color::LightCyan)),
            Span::styled("                           ", subdued_style()),
            Span::styled("RIGHT", strong_style(Color::LightCyan)),
        ]),
    ];

    for pair in 0..4 {
        let left = &assignments[pair * 2];
        let right = &assignments[pair * 2 + 1];
        lines.push(Line::from(format!(
            "USB {:>1}/{:>1}  Zen Go Recording {:>1} <- {:<18}  Zen Go Recording {:>1} <- {}",
            left.channel,
            right.channel,
            left.channel,
            afx_routing_source_label(left.assignment),
            right.channel,
            afx_routing_source_label(right.assignment),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("STATUS ", subdued_style()),
        Span::styled(state.last_message.clone(), strong_style(Color::LightCyan)),
    ]));
    Text::from(lines)
}

fn afx_routing_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }

    let rows = afx_routing_layout(area);
    for pair in 0..4 {
        let row_area = rows[4 + pair];
        if !contains_point(row_area, point) {
            continue;
        }

        let rects = afx_routing_row_rects(row_area, state, pair);
        let (left_index, right_index) = afx_routing_pair_channels(pair);
        if contains_point(rects[2], point) {
            return Some(MouseAction::OpenAssignmentPicker((left_index + 1) as u8));
        }
        if contains_point(rects[4], point) {
            return Some(MouseAction::OpenAssignmentPicker((right_index + 1) as u8));
        }
        if point.0 < rects[3].x {
            return Some(MouseAction::SelectMixerChannel(left_index));
        }
        return Some(MouseAction::SelectMixerChannel(right_index));
    }

    None
}

fn draw_output_panel(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    frame.render_widget(
        panel_block(
            "Outputs",
            Color::Rgb(70, 120, 90),
            state.focus == FocusArea::Outputs,
        ),
        area,
    );
    let inner = inner_area(area);
    for (index, (output, card)) in state
        .outputs
        .iter()
        .zip(output_card_areas(inner).into_iter())
        .enumerate()
    {
        render_output_card_widget(
            card,
            frame.buffer_mut(),
            output,
            state.focus == FocusArea::Outputs && state.selected_output == index,
        );
    }
    let help_button = output_hotkeys_button_rect(area);
    if help_button.height > 0 {
        Paragraph::new(Line::from(chip(
            "? HOTKEYS",
            Color::Black,
            Color::LightYellow,
        )))
        .alignment(Alignment::Right)
        .render(help_button, frame.buffer_mut());
    }
}

fn draw_preamp_bar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let cards = preamp_bar_layout(area);
    for (index, card) in cards.into_iter().enumerate() {
        let input = if index == 0 {
            state.preamp.input1
        } else {
            state.preamp.input2
        };
        let title = if state.focus == FocusArea::Preamp && state.selected_preamp_input == index {
            if index == 0 {
                "Preamp 1 ←"
            } else {
                "Preamp 2 ←"
            }
        } else if index == 0 {
            "Preamp 1"
        } else {
            "Preamp 2"
        };

        render_preamp_visual_widget(
            card,
            frame.buffer_mut(),
            title,
            input,
            state.focus == FocusArea::Preamp && state.selected_preamp_input == index,
        );
    }
}

fn draw_mixer_main(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let layout = mixer_layout(area);

    let surface = MixerSurface::from_surface(state.surface);
    let line = Line::from(vec![
        tab_chip(
            "MIX 1 / Monitor-HP1",
            surface == MixerSurface::Mix1,
            Color::LightCyan,
        ),
        Span::raw(" "),
        tab_chip(
            "MIX 2 / HP2",
            surface == MixerSurface::Mix2,
            Color::LightBlue,
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line)
            .block(panel_block(
                "Mixer Surface",
                Color::Rgb(80, 110, 150),
                state.focus == FocusArea::Mixer,
            ))
            .wrap(Wrap { trim: false }),
        layout[0],
    );
    Paragraph::new(Line::from(vec![chip(
        "ROUTING",
        Color::Black,
        if state.routing_popup_open {
            Color::Yellow
        } else {
            Color::LightMagenta
        },
    )]))
    .alignment(Alignment::Right)
    .render(routing_button_rect(layout[0]), frame.buffer_mut());

    let content = mixer_strip_panel_layout(layout[1], experimental_mix_meter(state).is_some());
    let inner = content[0];
    let (visible_start, visible_end) = mixer_strip_visible_bounds(inner, state);
    let total = state.active_mixer_channels().len();
    let title = if total == 0 {
        "Mixer Strips".to_string()
    } else {
        format!(
            "Mixer Strips {}-{} / {}",
            visible_start + 1,
            visible_end,
            total
        )
    };
    frame.render_widget(
        panel_block(
            &title,
            Color::Rgb(70, 100, 130),
            state.focus == FocusArea::Mixer,
        ),
        layout[1],
    );

    for (slot, (index, channel)) in state
        .active_mixer_channels()
        .iter()
        .enumerate()
        .skip(visible_start)
        .take(visible_end.saturating_sub(visible_start))
        .enumerate()
    {
        let card = mixer_strip_card_area(inner, slot);
        if card.x >= inner.x + inner.width {
            break;
        }
        render_mixer_strip_widget(card, frame.buffer_mut(), state, index, channel);
    }

    if let Some((_, left_raw, right_raw)) = experimental_mix_meter(state) {
        render_mix_meter_widget(content[1], frame.buffer_mut(), left_raw, right_raw);
    } else if content[1].height > 0 {
        frame.render_widget(
            Paragraph::new(render_status_strip(state)).wrap(Wrap { trim: false }),
            content[1],
        );
    }
}

fn draw_assignment_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(picker) = state.assignment_picker else {
        return;
    };

    let popup = assignment_picker_area(area);
    let choices = MixerAssignment::grounded_choices();
    let items = choices
        .iter()
        .map(|assignment| ListItem::new(assignment.label()))
        .collect::<Vec<_>>();
    let mut list_state = ListState::default();
    list_state.select(Some(
        state
            .popup_selected_index
            .min(items.len().saturating_sub(1)),
    ));

    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(
        List::new(items)
            .block(panel_block(
                &format!("Assign CH {:02}", picker.strip),
                Color::Yellow,
                true,
            ))
            .highlight_style(terminal::adapt_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
        popup,
        &mut list_state,
    );
}

fn draw_selector_popup(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(popup_state) = state.selector_popup else {
        return;
    };

    let popup = assignment_picker_area(area);
    frame.render_widget(Clear, popup);

    let (title, items) = match popup_state.kind {
        SelectorPopupKind::SampleRate => (
            "Sample Rate",
            SampleRate::all_confirmed()
                .iter()
                .map(|rate| ListItem::new(rate.label()))
                .collect::<Vec<_>>(),
        ),
        SelectorPopupKind::ClockSource => (
            "Clock Source",
            ClockSource::all_confirmed()
                .iter()
                .map(|source| ListItem::new(source.label()))
                .collect::<Vec<_>>(),
        ),
        SelectorPopupKind::PreampMode { .. } => (
            "Preamp Mode",
            [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                .iter()
                .map(|mode| ListItem::new(mode.label()))
                .collect::<Vec<_>>(),
        ),
    };
    let mut list_state = ListState::default();
    list_state.select(Some(
        state
            .popup_selected_index
            .min(items.len().saturating_sub(1)),
    ));

    frame.render_stateful_widget(
        List::new(items)
            .block(panel_block(title, Color::Yellow, true))
            .highlight_style(terminal::adapt_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
        popup,
        &mut list_state,
    );
}

fn draw_hotkeys_popup(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if !state.hotkeys_popup_open {
        return;
    }

    let popup = hotkeys_popup_area(area);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(render_hotkeys_popup_text())
            .block(panel_block("Hotkeys", Color::Yellow, true))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

pub fn mouse_action(area: Rect, state: &AppState, x: u16, y: u16) -> Option<MouseAction> {
    let point = (x, y);
    let chunks = root_chunks(area);

    if state.hotkeys_popup_open {
        return Some(MouseAction::ToggleHotkeysPopup);
    }

    if let Some(action) = device_header_mouse_action(titlebar_layout(chunks[0])[0], state, point) {
        return Some(action);
    }

    if contains_point(titlebar_layout(chunks[0])[1], point) {
        return Some(MouseAction::ToggleRawView);
    }

    if state.raw_view_open {
        return raw_mouse_action(area, state, point);
    }

    if let Some(popup) = state.selector_popup {
        return selector_popup_mouse_action(area, popup, point);
    }

    if let Some(picker) = state.assignment_picker {
        return assignment_picker_mouse_action(area, picker, point);
    }

    if state.routing_popup_open {
        return routing_popup_mouse_action(area, state, point);
    }

    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout(page[0]);
    let mixer_sections = mixer_layout(main[1]);

    if let Some(action) = output_list_mouse_action(page[1], state, point) {
        return Some(action);
    }

    if let Some(action) = mixer_tab_mouse_action(mixer_sections[0], point) {
        return Some(action);
    }
    if let Some(action) = mixer_list_mouse_action(mixer_sections[1], state, point) {
        return Some(action);
    }
    if let Some(action) = preamp_mouse_action(main[0], state, point) {
        return Some(action);
    }

    None
}

pub fn slider_mouse_action(area: Rect, state: &AppState, x: u16, y: u16) -> Option<MouseAction> {
    if state.hotkeys_popup_open
        || state.raw_view_open
        || state.selector_popup.is_some()
        || state.assignment_picker.is_some()
        || state.routing_popup_open
    {
        return None;
    }

    let point = (x, y);
    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout(page[0]);
    let mixer_sections = mixer_layout(main[1]);

    output_list_slider_mouse_action(page[1], state, point)
        .or_else(|| mixer_list_slider_mouse_action(mixer_sections[1], state, point))
        .or_else(|| preamp_slider_mouse_action(main[0], state, point))
}

pub fn slider_wheel_action(
    area: Rect,
    state: &AppState,
    x: u16,
    y: u16,
    increase: bool,
) -> Option<MouseAction> {
    if state.hotkeys_popup_open
        || state.raw_view_open
        || state.selector_popup.is_some()
        || state.assignment_picker.is_some()
        || state.routing_popup_open
    {
        return None;
    }

    let point = (x, y);
    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout(page[0]);
    let mixer_sections = mixer_layout(main[1]);

    output_list_slider_wheel_action(page[1], state, point, increase)
        .or_else(|| mixer_list_slider_wheel_action(mixer_sections[1], state, point, increase))
        .or_else(|| preamp_slider_wheel_action(main[0], state, point, increase))
}

fn routing_popup_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<MouseAction> {
    let popup = routing_popup_area(area);
    if !contains_point(popup, point) {
        return Some(MouseAction::CloseRoutingPopup);
    }
    afx_routing_mouse_action(popup, state, point)
}

fn raw_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<MouseAction> {
    let layout = raw_page_layout(area);
    let header = raw_header_layout(layout[0]);
    if contains_point(header[1], point) {
        return Some(MouseAction::ToggleRawView);
    }
    if contains_point(layout[1], point) {
        let tabs = raw_tab_hit_areas(layout[1]);
        if contains_point(tabs[0], point) {
            return Some(MouseAction::SelectRawPacketTab(RawPacketTab::Query74));
        } else if contains_point(tabs[1], point) {
            return Some(MouseAction::SelectRawPacketTab(RawPacketTab::State73));
        } else if contains_point(tabs[2], point) {
            return Some(MouseAction::SelectRawPacketTab(RawPacketTab::Auxiliary83));
        } else if contains_point(tabs[3], point) {
            return Some(MouseAction::SelectRawPacketTab(RawPacketTab::Query75));
        } else if contains_point(tabs[4], point) {
            return Some(MouseAction::SelectRawPacketTab(
                RawPacketTab::Notification81,
            ));
        }
    }
    if state.selected_raw_packet == RawPacketTab::Query75 {
        let sections = query_reply_history_layout(layout[2]);
        if !contains_point(sections[0], point) {
            return None;
        }
        let inner = inner_area(sections[0]);
        if point.1 < inner.y + 1 {
            return None;
        }
        let visible = state
            .recent_query_reply_entries
            .iter()
            .enumerate()
            .rev()
            .take(8)
            .collect::<Vec<_>>();
        let row = point.1.saturating_sub(inner.y + 1) as usize;
        visible
            .get(row)
            .map(|(index, _)| MouseAction::SelectQueryReplyEntry(*index))
    } else {
        None
    }
}

fn assignment_picker_mouse_action(
    area: Rect,
    picker: AssignmentPickerState,
    point: (u16, u16),
) -> Option<MouseAction> {
    let popup = assignment_picker_area(area);
    if !contains_point(popup, point) {
        return Some(MouseAction::CloseAssignmentPicker);
    }

    let inner = popup_list_inner_area(popup, &format!("Assign CH {:02}", picker.strip));
    if point.1 < inner.y {
        return None;
    }
    let index = point.1.saturating_sub(inner.y) as usize;
    let assignment = *MixerAssignment::grounded_choices().get(index)?;
    Some(MouseAction::PickAssignment {
        strip: picker.strip,
        assignment,
    })
}

fn selector_popup_mouse_action(
    area: Rect,
    popup: SelectorPopupState,
    point: (u16, u16),
) -> Option<MouseAction> {
    let popup_area = assignment_picker_area(area);
    if !contains_point(popup_area, point) {
        return Some(MouseAction::CloseSelectorPopup);
    }

    let title = match popup.kind {
        SelectorPopupKind::SampleRate => "Sample Rate",
        SelectorPopupKind::ClockSource => "Clock Source",
        SelectorPopupKind::PreampMode { .. } => "Preamp Mode",
    };
    let inner = popup_list_inner_area(popup_area, title);
    if point.1 < inner.y {
        return None;
    }
    let index = point.1.saturating_sub(inner.y) as usize;
    match popup.kind {
        SelectorPopupKind::SampleRate => SampleRate::all_confirmed()
            .get(index)
            .copied()
            .map(MouseAction::PickSampleRate),
        SelectorPopupKind::ClockSource => ClockSource::all_confirmed()
            .get(index)
            .copied()
            .map(MouseAction::PickClockSource),
        SelectorPopupKind::PreampMode { input } => {
            [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                .get(index)
                .copied()
                .map(|mode| MouseAction::PickPreampMode { input, mode })
        }
    }
}

fn device_header_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    let chips = device_header_hit_areas(area, state);
    if contains_point(chips[1], point) {
        if state.device.clock_source == Some(ClockSource::Internal) {
            Some(MouseAction::OpenSampleRateSelector)
        } else {
            None
        }
    } else if contains_point(chips[2], point) {
        Some(MouseAction::OpenClockSourceSelector)
    } else {
        None
    }
}

fn mixer_tab_mouse_action(area: Rect, point: (u16, u16)) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    if contains_point(routing_button_rect(area), point) {
        return Some(MouseAction::OpenRoutingPopup);
    }
    let tabs = surface_tab_hit_areas(area);
    if contains_point(tabs[0], point) {
        Some(MouseAction::SelectSurface(Surface::MonitorHp1))
    } else if contains_point(tabs[1], point) {
        Some(MouseAction::SelectSurface(Surface::Hp2))
    } else {
        None
    }
}

fn mixer_list_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    let inner = mixer_strip_panel_layout(area, experimental_mix_meter(state).is_some())[0];
    if !contains_point(inner, point) {
        return None;
    }
    let (visible_start, visible_end) = mixer_strip_visible_bounds(inner, state);
    for (slot, index) in (visible_start..visible_end).enumerate() {
        let Some(channel) = state.active_mixer_channels().get(index) else {
            continue;
        };
        let card = mixer_strip_card_area(inner, slot);
        if !contains_point(card, point) {
            continue;
        }

        let source = channel
            .assignment
            .map(|value| value.short_label())
            .unwrap_or_else(|| "?".to_string());
        let (_, source_rect) = mixer_header_chip_rects(card, &source);
        if let Some(action) = mixer_strip_slider_mouse_action(card, index, channel, point) {
            return Some(action);
        }
        if contains_point(source_rect, point) {
            return Some(MouseAction::OpenAssignmentPicker(channel.channel));
        }

        let controls = mixer_control_button_rects(card, channel.channel % 2 == 1);
        if channel.channel % 2 == 1 && contains_point(controls[0], point) {
            return Some(MouseAction::ToggleMixerLink(channel.channel));
        }
        let solo_rect = if channel.channel % 2 == 1 {
            controls[1]
        } else {
            controls[0]
        };
        if contains_point(solo_rect, point) {
            return Some(MouseAction::ToggleMixerSolo(channel.channel));
        }
        let mute_rect = if channel.channel % 2 == 1 {
            controls[2]
        } else {
            controls[1]
        };
        if contains_point(mute_rect, point) {
            return Some(MouseAction::ToggleMixerMute(channel.channel));
        }

        return Some(MouseAction::SelectMixerChannel(index));
    }

    None
}

fn mixer_list_slider_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    let inner = mixer_strip_panel_layout(area, experimental_mix_meter(state).is_some())[0];
    if !contains_point(inner, point) {
        return None;
    }
    let (visible_start, visible_end) = mixer_strip_visible_bounds(inner, state);
    for (slot, index) in (visible_start..visible_end).enumerate() {
        let Some(channel) = state.active_mixer_channels().get(index) else {
            continue;
        };
        let card = mixer_strip_card_area(inner, slot);
        if !contains_point(card, point) {
            continue;
        }
        return mixer_strip_slider_mouse_action(card, index, channel, point);
    }
    None
}

fn mixer_list_slider_wheel_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
    increase: bool,
) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    let inner = mixer_strip_panel_layout(area, experimental_mix_meter(state).is_some())[0];
    if !contains_point(inner, point) {
        return None;
    }
    let (visible_start, visible_end) = mixer_strip_visible_bounds(inner, state);
    for (slot, index) in (visible_start..visible_end).enumerate() {
        let Some(channel) = state.active_mixer_channels().get(index) else {
            continue;
        };
        let card = mixer_strip_card_area(inner, slot);
        if !contains_point(card, point) {
            continue;
        }
        return mixer_strip_slider_wheel_action(card, index, channel, point, increase);
    }
    None
}

fn preamp_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    let layout = preamp_bar_layout(area);
    for (input, card) in layout.into_iter().enumerate() {
        if !contains_point(card, point) {
            continue;
        }
        let input_state = if input == 0 {
            state.preamp.input1
        } else {
            state.preamp.input2
        };
        if let Some(action) = preamp_card_slider_mouse_action(card, input as u8, input_state, point)
        {
            return Some(action);
        }
        let buttons = preamp_button_rects(card, input_state);
        if contains_point(buttons[0], point) {
            return Some(MouseAction::AdjustPreampGain {
                input: input as u8,
                increase: false,
            });
        }
        if contains_point(buttons[1], point) {
            return Some(MouseAction::AdjustPreampGain {
                input: input as u8,
                increase: true,
            });
        }
        if contains_point(buttons[2], point) {
            return Some(MouseAction::OpenPreampModeSelector(input as u8));
        }
        if contains_point(buttons[3], point) {
            return Some(MouseAction::TogglePreampPhantom(input as u8));
        }
        if contains_point(buttons[4], point) {
            return Some(MouseAction::TogglePreampPhase(input as u8));
        }
        return Some(MouseAction::SelectPreampInput(input));
    }
    None
}

fn preamp_slider_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    let layout = preamp_bar_layout(area);
    for (input, card) in layout.into_iter().enumerate() {
        if !contains_point(card, point) {
            continue;
        }
        let input_state = if input == 0 {
            state.preamp.input1
        } else {
            state.preamp.input2
        };
        return preamp_card_slider_mouse_action(card, input as u8, input_state, point);
    }
    None
}

fn preamp_slider_wheel_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
    increase: bool,
) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    let layout = preamp_bar_layout(area);
    for (input, card) in layout.into_iter().enumerate() {
        if !contains_point(card, point) {
            continue;
        }
        let track = preamp_gain_slider_rect(card);
        if contains_point(track, point) {
            return Some(MouseAction::AdjustPreampGain {
                input: input as u8,
                increase,
            });
        }
        return None;
    }
    None
}

fn output_list_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }

    if contains_point(output_hotkeys_button_rect(area), point) {
        return Some(MouseAction::ToggleHotkeysPopup);
    }

    let inner = inner_area(area);
    if point.1 < inner.y {
        return None;
    }

    let (index, card) = output_card_areas(inner)
        .into_iter()
        .enumerate()
        .find(|(_, card)| contains_point(*card, point))?;
    state.outputs.get(index)?;
    let controls = output_control_rects(card);

    if let Some(action) = output_card_slider_mouse_action(card, index, point) {
        return Some(action);
    }

    if contains_point(controls[0], point) {
        return Some(MouseAction::AdjustOutputLevel {
            index,
            increase: false,
        });
    }
    if contains_point(controls[1], point) {
        return Some(MouseAction::AdjustOutputLevel {
            index,
            increase: true,
        });
    }
    if contains_point(controls[2], point) {
        return Some(MouseAction::ToggleOutputDim(index));
    }
    if contains_point(controls[3], point) {
        return Some(MouseAction::ToggleOutputMute(index));
    }

    Some(MouseAction::SelectOutput(index))
}

fn output_list_slider_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }

    let inner = inner_area(area);
    if point.1 < inner.y {
        return None;
    }

    let (index, card) = output_card_areas(inner)
        .into_iter()
        .enumerate()
        .find(|(_, card)| contains_point(*card, point))?;
    state.outputs.get(index)?;
    output_card_slider_mouse_action(card, index, point)
}

fn output_list_slider_wheel_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
    increase: bool,
) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }

    let inner = inner_area(area);
    if point.1 < inner.y {
        return None;
    }

    let (index, card) = output_card_areas(inner)
        .into_iter()
        .enumerate()
        .find(|(_, card)| contains_point(*card, point))?;
    state.outputs.get(index)?;
    let track = output_level_slider_rect(card);
    contains_point(track, point).then_some(MouseAction::AdjustOutputLevel { index, increase })
}

fn mixer_control_button_rects(area: Rect, has_link: bool) -> Vec<Rect> {
    let inner = mixer_strip_inner_area(area);
    let y = inner.y + inner.height.saturating_sub(1);
    if has_link {
        centered_inline_chip_rects(Rect::new(inner.x, y, inner.width, 1), &["L", "S", "M"])
    } else {
        centered_inline_chip_rects(Rect::new(inner.x, y, inner.width, 1), &["S", "M"])
    }
}

pub fn mixer_strip_viewport_capacity(area: Rect, state: &AppState) -> usize {
    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout(page[0]);
    let mixer = mixer_layout(main[1]);
    let list = mixer_strip_panel_layout(mixer[1], experimental_mix_meter(state).is_some());
    mixer_strip_viewport_capacity_for_inner(list[0])
}

pub fn mixer_strip_panel_contains(area: Rect, state: &AppState, x: u16, y: u16) -> bool {
    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout(page[0]);
    let mixer = mixer_layout(main[1]);
    let list = mixer_strip_panel_layout(mixer[1], experimental_mix_meter(state).is_some());
    contains_point(list[0], (x, y))
}

fn output_card_height() -> u16 {
    3
}

fn output_card_areas(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(Rect::new(area.x, area.y, area.width, output_card_height()))
        .to_vec()
}

fn output_hotkeys_button_rect(area: Rect) -> Rect {
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

#[cfg(test)]
fn mixer_strip_height() -> u16 {
    18
}

fn output_control_rects(area: Rect) -> Vec<Rect> {
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

fn contains_point(area: Rect, point: (u16, u16)) -> bool {
    point.0 >= area.x
        && point.0 < area.x.saturating_add(area.width)
        && point.1 >= area.y
        && point.1 < area.y.saturating_add(area.height)
}

fn slider_ratio_for_horizontal_point(area: Rect, point: (u16, u16)) -> Option<f64> {
    if !contains_point(area, point) || area.width == 0 {
        return None;
    }
    if area.width <= 1 {
        return Some(1.0);
    }
    Some(
        ((point.0.saturating_sub(area.x)) as f64 / area.width.saturating_sub(1) as f64)
            .clamp(0.0, 1.0),
    )
}

fn slider_ratio_for_vertical_point(area: Rect, point: (u16, u16)) -> Option<f64> {
    if !contains_point(area, point) || area.height == 0 {
        return None;
    }
    if area.height <= 1 {
        return Some(1.0);
    }
    Some(
        (1.0 - (point.1.saturating_sub(area.y)) as f64 / area.height.saturating_sub(1) as f64)
            .clamp(0.0, 1.0),
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

fn output_level_slider_rect(area: Rect) -> Rect {
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

fn preamp_gain_slider_rect(area: Rect) -> Rect {
    let signal = preamp_card_inner_layout(area)[0];
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(signal);
    horizontal_labeled_slider_track(rows[1])
}

fn mixer_strip_rows(area: Rect) -> Vec<Rect> {
    Layout::default()
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
        .split(mixer_strip_inner_area(area))
        .to_vec()
}

fn mixer_pan_slider_rect(area: Rect) -> Rect {
    mixer_strip_rows(area)[2]
}

fn mixer_level_slider_rect(area: Rect) -> Rect {
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

fn output_step_from_ratio(ratio: f64) -> u8 {
    ((1.0 - ratio.clamp(0.0, 1.0)) * 96.0).round() as u8
}

fn mixer_level_from_ratio(ratio: f64) -> u8 {
    ((1.0 - ratio.clamp(0.0, 1.0)) * 90.0).round() as u8
}

fn pan_from_ratio(ratio: f64) -> PanState {
    let span = (PanState::MAX - PanState::MIN) as f64;
    let raw = PanState::MIN as f64 + span * ratio.clamp(0.0, 1.0);
    PanState::from_raw(raw.round() as u8)
}

fn preamp_gain_from_ratio(input: PreampInputState, ratio: f64) -> Option<u8> {
    let ratio = ratio.clamp(0.0, 1.0);
    match input.mode {
        PreampMode::Mic => Some((ratio * 65.0).round() as u8),
        PreampMode::Line => Some((-6 + (ratio * 26.0).round() as i8) as u8),
        PreampMode::HiZ => Some((ratio * 45.0).round() as u8),
        PreampMode::Unknown(_) => None,
    }
}

fn output_card_slider_mouse_action(
    area: Rect,
    index: usize,
    point: (u16, u16),
) -> Option<MouseAction> {
    let track = output_level_slider_rect(area);
    let ratio = slider_ratio_for_horizontal_point(track, point)?;
    Some(MouseAction::SetOutputLevel {
        index,
        step: output_step_from_ratio(ratio).min(0x60),
    })
}

fn preamp_card_slider_mouse_action(
    area: Rect,
    input: u8,
    input_state: PreampInputState,
    point: (u16, u16),
) -> Option<MouseAction> {
    let track = preamp_gain_slider_rect(area);
    let ratio = slider_ratio_for_horizontal_point(track, point)?;
    Some(MouseAction::SetPreampGain {
        input,
        raw: preamp_gain_from_ratio(input_state, ratio)?,
    })
}

fn mixer_strip_slider_mouse_action(
    area: Rect,
    index: usize,
    _channel: &crate::protocol::MixerChannelState,
    point: (u16, u16),
) -> Option<MouseAction> {
    let pan = mixer_pan_slider_rect(area);
    if let Some(ratio) = slider_ratio_for_horizontal_point(pan, point) {
        return Some(MouseAction::SetMixerPan {
            index,
            pan: pan_from_ratio(ratio),
        });
    }

    let level = mixer_level_slider_rect(area);
    let ratio = slider_ratio_for_vertical_point(level, point)?;
    Some(MouseAction::SetMixerLevel {
        index,
        level: mixer_level_from_ratio(ratio).min(0x5a),
    })
}

fn mixer_strip_slider_wheel_action(
    area: Rect,
    index: usize,
    _channel: &crate::protocol::MixerChannelState,
    point: (u16, u16),
    increase: bool,
) -> Option<MouseAction> {
    let pan = mixer_pan_slider_rect(area);
    if contains_point(pan, point) {
        return Some(MouseAction::AdjustMixerPan {
            index,
            right: increase,
        });
    }

    let level = mixer_level_slider_rect(area);
    contains_point(level, point).then_some(MouseAction::AdjustMixerLevel { index, increase })
}

const SIGNAL_LABEL_WIDTH: u16 = 12;
const MAX_SIGNAL_ROW_WIDTH: u16 = 40;

fn slider_state(ratio: Option<f64>) -> SliderState {
    SliderState::new(ratio.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0, 0.0, 100.0)
}

fn level_slider(ratio: Option<f64>, color: Color) -> Slider<'static> {
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

fn render_level_slider(area: Rect, buffer: &mut Buffer, ratio: Option<f64>, color: Color) {
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

fn signal_slider_label(prefix: &str, value: Option<String>) -> String {
    value
        .filter(|value| !value.is_empty())
        .map(|value| format!("{prefix} {value}"))
        .unwrap_or_else(|| prefix.to_string())
}

fn format_meter_value_label(value: Option<i16>) -> String {
    let mapped = value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-∞".to_string());
    format!("{:>3} dB", mapped)
}

fn meter_slider_label(prefix: &str, value: Option<i16>) -> String {
    format!("{prefix} {}", format_meter_value_label(value))
}

fn bounded_signal_area(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y,
        area.width.min(MAX_SIGNAL_ROW_WIDTH),
        area.height,
    )
}

fn render_labeled_slider(
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

fn render_stacked_signal_rows(
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

fn render_output_card_widget(area: Rect, buffer: &mut Buffer, output: &OutputState, active: bool) {
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

fn render_preamp_visual_widget(
    area: Rect,
    buffer: &mut Buffer,
    title: &str,
    input: PreampInputState,
    focused: bool,
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
    Paragraph::new(render_preamp_controls_text(input)).render(sections[1], buffer);
}

fn mixer_pan_label(channel: &crate::protocol::MixerChannelState) -> String {
    format!("PAN {}", channel.pan.display_percent())
}

fn mixer_level_value_label(channel: &crate::protocol::MixerChannelState) -> String {
    channel
        .display_db()
        .map(|value| format!("LVL {} dB", value))
        .unwrap_or_else(|| "LVL ?".to_string())
}

fn strip_db_ratio(value: Option<i16>) -> Option<f64> {
    value.map(|db| ((db.clamp(-60, 0) + 60) as f64 / 60.0).clamp(0.0, 1.0))
}

fn meter_bar_color(cell_ratio: f64) -> Color {
    if cell_ratio >= MIX_METER_RED_START_RATIO {
        Color::LightRed
    } else if cell_ratio >= MIX_METER_YELLOW_START_RATIO {
        Color::Yellow
    } else {
        Color::LightGreen
    }
}

fn vertical_ratio_row(area: Rect, ratio: f64) -> u16 {
    let height = area.height.saturating_sub(1) as f64;
    area.y + area.height.saturating_sub(1) - (height * ratio.clamp(0.0, 1.0)).round() as u16
}

fn render_pan_slider(area: Rect, buffer: &mut Buffer, ratio: f64) {
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

fn render_pan_scale(area: Rect, buffer: &mut Buffer) {
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

fn render_vertical_combo_strip(
    area: Rect,
    buffer: &mut Buffer,
    meter_db: Option<i16>,
    level_db: Option<i16>,
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
        let mut y = vertical_ratio_row(scale, strip_db_ratio(Some(-marker)).unwrap_or(0.0));
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

    let meter_ratio = strip_db_ratio(meter_db);
    let level_ratio = strip_db_ratio(level_db);
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

        buffer[(meter.x, y)]
            .set_symbol(if meter_filled { "█" } else { "░" })
            .set_style(terminal::adapt_style(Style::default().fg(
                if meter_filled {
                    meter_bar_color(cell_ratio)
                } else {
                    Color::DarkGray
                },
            )));

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

fn render_mixer_strip_widget(
    area: Rect,
    buffer: &mut Buffer,
    state: &AppState,
    index: usize,
    channel: &crate::protocol::MixerChannelState,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let selected = state.focus == FocusArea::Mixer && state.selected_channel == index;
    let source = channel
        .assignment
        .map(|value| value.short_label())
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
    Paragraph::new(Line::from(vec![chip(
        format!("CH {:02}", channel.channel),
        Color::Black,
        if selected {
            Color::LightGreen
        } else {
            Color::Gray
        },
    )]))
    .render(channel_rect, buffer);
    Paragraph::new(Line::from(vec![chip(
        source.clone(),
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
    render_vertical_combo_strip(rows[5], buffer, channel.meter_db(), channel.display_db());

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

fn draw_raw_page(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let layout = raw_page_layout(area);

    let header = raw_header_layout(layout[0]);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("Live Raw State View"),
            Line::from("One packet type at a time. `b` capture baseline, `x` clear."),
        ])
        .block(section_block("Raw", true))
        .wrap(Wrap { trim: true }),
        header[0],
    );
    frame.render_widget(
        Paragraph::new("Back To Main")
            .block(Block::default().borders(Borders::ALL).title("View"))
            .wrap(Wrap { trim: true }),
        header[1],
    );

    let selected = state.selected_raw_packet;
    let tabs = Line::from(vec![
        tab_chip(
            "0x74",
            selected == RawPacketTab::Query74,
            Color::LightYellow,
        ),
        Span::raw(" "),
        tab_chip("0x73", selected == RawPacketTab::State73, Color::LightCyan),
        Span::raw(" "),
        tab_chip(
            "0x83",
            selected == RawPacketTab::Auxiliary83,
            Color::LightBlue,
        ),
        Span::raw(" "),
        tab_chip("0x75", selected == RawPacketTab::Query75, Color::LightGreen),
        Span::raw(" "),
        tab_chip(
            "0x81",
            selected == RawPacketTab::Notification81,
            Color::LightMagenta,
        ),
    ]);
    frame.render_widget(
        Paragraph::new(tabs)
            .block(section_block("Packet Tabs", true))
            .wrap(Wrap { trim: false }),
        layout[1],
    );

    let (title, text) = match state.selected_raw_packet {
        RawPacketTab::Query74 => (
            "0x74 Query Requests",
            state
                .latest_raw_74
                .as_deref()
                .map(|bytes| render_query_request_panel(bytes, state))
                .unwrap_or_else(|| Text::from("Waiting for first 0x74 query request...")),
        ),
        RawPacketTab::State73 => (
            "0x73 State",
            state
                .latest_raw_73
                .as_deref()
                .map(|bytes| render_full_packet_dump(bytes, state.baseline_raw_73.as_deref()))
                .unwrap_or_else(|| Text::from("Waiting for first 0x73 snapshot...")),
        ),
        RawPacketTab::Auxiliary83 => (
            "0x83 State",
            state
                .latest_raw_83
                .as_deref()
                .map(|bytes| render_full_packet_dump(bytes, state.baseline_raw_83.as_deref()))
                .unwrap_or_else(|| Text::from("Waiting for first 0x83 auxiliary packet...")),
        ),
        RawPacketTab::Query75 => (
            "0x75 Query Replies",
            state
                .latest_raw_75
                .as_deref()
                .map(|bytes| render_query_reply_panel(bytes, state))
                .unwrap_or_else(|| Text::from("Waiting for first 0x75 query reply...")),
        ),
        RawPacketTab::Notification81 => (
            "0x81 Notification",
            state
                .latest_raw_81
                .as_deref()
                .map(|bytes| render_full_packet_dump(bytes, state.baseline_raw_81.as_deref()))
                .unwrap_or_else(|| Text::from("Waiting for first 0x81 notification...")),
        ),
    };
    if state.selected_raw_packet == RawPacketTab::Query75 {
        let sections = query_reply_history_layout(layout[2]);
        frame.render_widget(
            Paragraph::new(render_query_reply_history_list(state))
                .block(section_block("Recent 0x75 Replies", true))
                .wrap(Wrap { trim: false }),
            sections[0],
        );
        frame.render_widget(
            Paragraph::new(text)
                .block(section_block(title, true))
                .wrap(Wrap { trim: false }),
            sections[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new(text)
                .block(section_block(title, true))
                .wrap(Wrap { trim: false }),
            layout[2],
        );
    }
}

fn section_block(title: &str, focused: bool) -> Block<'_> {
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

fn panel_block<'a>(title: &'a str, accent: Color, focused: bool) -> Block<'a> {
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

fn chip_text(label: &str) -> String {
    format!(" {} ", label)
}

fn chip_width(label: &str) -> u16 {
    chip_text(label).chars().count() as u16
}

fn chip<T: Into<String>>(label: T, fg: Color, bg: Color) -> Span<'static> {
    Span::styled(
        chip_text(&label.into()),
        terminal::adapt_style(Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)),
    )
}

fn labeled_value_chip(
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

fn tab_chip(label: &str, active: bool, accent: Color) -> Span<'static> {
    if active {
        chip(label, Color::Black, accent)
    } else {
        Span::styled(chip_text(label), muted_style())
    }
}

fn muted_style() -> Style {
    terminal::adapt_style(Style::default().fg(Color::Gray))
}

fn subdued_style() -> Style {
    terminal::adapt_style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
}

fn strong_style(color: Color) -> Style {
    terminal::adapt_style(Style::default().fg(color).add_modifier(Modifier::BOLD))
}

fn render_symbol_bar(ratio: f64, width: usize, filled: char, empty: char) -> String {
    let filled_cells = (ratio.clamp(0.0, 1.0) * width as f64).round() as usize;
    let mut out = String::with_capacity(width);
    for index in 0..width {
        out.push(if index < filled_cells { filled } else { empty });
    }
    out
}

#[cfg(test)]
fn render_level_bar(ratio: f64, width: usize) -> String {
    render_symbol_bar(ratio, width, '#', '.')
}

fn preamp_phantom_label(input: PreampInputState) -> &'static str {
    if matches!(input.mode, PreampMode::Mic) {
        "48V"
    } else {
        "N/A"
    }
}

fn preamp_phase_label(input: PreampInputState) -> &'static str {
    if input.mode_raw & 0x40 != 0 {
        "INV"
    } else {
        "NORM"
    }
}

#[cfg(test)]
fn render_output_card(output: &OutputState, active: bool) -> Text<'static> {
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

fn render_device_header(state: &AppState) -> Line<'static> {
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
    let lock = if state.device.lock_known {
        if state.device.locked == Some(true) {
            "locked"
        } else {
            "unlocked"
        }
    } else {
        "lock ?"
    };
    let connection = if state.connection.connected {
        "connected"
    } else {
        "waiting"
    };
    Line::from(vec![
        Span::styled(product, strong_style(Color::LightGreen)),
        Span::raw("  "),
        chip(
            connection.to_uppercase(),
            Color::Black,
            connection_badge_color(state),
        ),
        Span::raw(" "),
        chip(sample, Color::Black, Color::Yellow),
        Span::raw(" "),
        chip(clock, Color::Black, Color::LightBlue),
        Span::raw(" "),
        chip(lock.to_uppercase(), Color::Black, Color::Magenta),
    ])
}

fn render_device_metadata(state: &AppState) -> Line<'static> {
    if let Some(metadata) = state.device.metadata.as_ref() {
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

fn render_inspector_summary() -> Line<'static> {
    Line::from(vec![
        chip("RAW", Color::Black, Color::LightRed),
        Span::raw(" "),
        Span::styled("[r] inspector", muted_style()),
    ])
}

fn connection_badge_color(state: &AppState) -> Color {
    if state.connection.connected {
        Color::LightGreen
    } else if state
        .connection
        .last_snapshot_at
        .is_some_and(|instant| instant.elapsed() >= CONNECTION_STALE_AFTER)
    {
        Color::LightRed
    } else {
        Color::Rgb(255, 165, 0)
    }
}

const CONNECTION_STALE_AFTER: Duration = Duration::from_secs(2);

fn current_sample_rate_label(state: &AppState) -> String {
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

fn render_status_strip(state: &AppState) -> Line<'static> {
    Line::from(Span::styled(
        render_experimental_pair_state_line(state),
        muted_style(),
    ))
}

fn render_hotkeys_popup_text() -> Text<'static> {
    Text::from(vec![
        Line::from("Global"),
        Line::from("  q quit   ? hotkeys   Esc close popup"),
        Line::from(""),
        Line::from("Mixer Page"),
        Line::from("  Tab focus cycle   Left/Right move selection   +/- adjust focused control"),
        Line::from("  Outputs: m mute   d dim"),
        Line::from("  Mixer: o solo   a assignment   l link   [ ] pan   1/2 surface"),
        Line::from("  Routing: click ROUTING in Mixer Surface header, then a opens source picker"),
        Line::from("  Preamp: m phantom   p phase   3 mode"),
        Line::from(""),
        Line::from("Device / Inspector"),
        Line::from("  s sample rate   c clock source   r raw inspector   R refresh queries"),
        Line::from("  Raw view: Left/Right tabs or Query75 history   b capture baseline   x clear"),
        Line::from(""),
        Line::from(Span::styled(
            "Click ? HOTKEYS or press ? again to close.",
            muted_style(),
        )),
    ])
}

const MIX_METER_YELLOW_START_RATIO: f64 = 0.8;
const MIX_METER_RED_START_RATIO: f64 = 0.95;
const MIX_METER_CHANNEL_LABEL_WIDTH: u16 = 2;
const MIX_METER_DB_WIDTH: u16 = 7;

fn render_mix_meter_widget(area: Rect, buffer: &mut Buffer, left_raw: u8, right_raw: u8) {
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

fn render_mix_meter_channel(area: Rect, buffer: &mut Buffer, label: &str, raw: u8) {
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

fn render_colored_meter_bar(area: Rect, buffer: &mut Buffer, ratio: f64) {
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

fn experimental_mix_meter(state: &AppState) -> Option<(&'static str, u8, u8)> {
    let bytes = state.latest_raw_73.as_deref()?;
    let payload = bytes.get(0x10..)?;

    match payload.get(0x6a).copied() {
        Some(0x0f) => Some((
            "MIX 1",
            payload.get(0xda).copied().unwrap_or(0),
            payload.get(0xdb).copied().unwrap_or(0),
        )),
        Some(0x0c) => Some((
            "MIX 2",
            payload.get(0xde).copied().unwrap_or(0),
            payload.get(0xdf).copied().unwrap_or(0),
        )),
        _ => None,
    }
}

fn render_preamp_controls_text(input: PreampInputState) -> Text<'static> {
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

fn render_query_reply_panel(_state_bytes: &[u8], state: &AppState) -> Text<'static> {
    state
        .selected_query_reply_entry()
        .map(|entry| render_full_packet_dump(&entry.raw, state.baseline_raw_75.as_deref()))
        .unwrap_or_else(|| Text::from("No 0x75 reply selected yet."))
}

fn render_query_request_panel(state_bytes: &[u8], state: &AppState) -> Text<'static> {
    let mut lines = render_full_packet_dump(state_bytes, state.baseline_raw_74.as_deref()).lines;
    if !state.recent_query_request_log.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from("Recent 0x74 requests:"));
        for entry in state.recent_query_request_log.iter().rev().take(8) {
            lines.push(Line::from(entry.clone()));
        }
    }
    Text::from(lines)
}

fn render_query_reply_history_list(state: &AppState) -> Text<'static> {
    let mut lines = vec![Line::from("Select one reply to inspect its raw bytes:")];
    if state.recent_query_reply_entries.is_empty() {
        lines.push(Line::from("Waiting for first 0x75 query reply..."));
        return Text::from(lines);
    }
    for (index, entry) in state
        .recent_query_reply_entries
        .iter()
        .enumerate()
        .rev()
        .take(8)
    {
        let marker = if state.selected_query_reply_entry == Some(index) {
            ">"
        } else {
            " "
        };
        lines.push(Line::from(format!("{} {}", marker, entry.summary)));
    }
    Text::from(lines)
}

#[cfg(test)]
fn render_mixer_strip_controls(
    _state: &AppState,
    _index: usize,
    channel: &crate::protocol::MixerChannelState,
) -> String {
    let mute = channel
        .muted
        .map(|value| if value { "on" } else { "off" })
        .unwrap_or("?");
    let src = channel
        .assignment
        .map(|value| value.label())
        .unwrap_or_else(|| "assignment?".to_string());
    let solo = channel
        .soloed
        .map(|value| if value { "on" } else { "off" })
        .unwrap_or("?");
    let link = if channel.channel % 2 == 1 {
        let value = channel
            .linked
            .map(|flag| if flag { "on" } else { "off" })
            .unwrap_or("?");
        format!(" [Link {}]", value)
    } else {
        String::new()
    };
    format!("    [Mute {}] [Solo {}]{} [Src {}]", mute, solo, link, src)
}

fn render_experimental_pair_state_line(state: &AppState) -> String {
    let Some(bytes) = state.latest_raw_73.as_deref() else {
        return "exp pair pending: waiting for 0x73 snapshot".to_string();
    };
    let Some(payload) = bytes.get(0x10..) else {
        return "exp pair pending: short 0x73 snapshot".to_string();
    };

    match payload.get(0x6a).copied() {
        Some(0x0f) => {
            let lane_a = payload.get(0xda).copied().unwrap_or(0);
            let lane_b = payload.get(0xdb).copied().unwrap_or(0);
            format!(
                "MIX 1 L {} R {}",
                render_mix_meter(lane_a),
                render_mix_meter(lane_b),
            )
        }
        Some(0x0c) => {
            let lane_a = payload.get(0xde).copied().unwrap_or(0);
            let lane_b = payload.get(0xdf).copied().unwrap_or(0);
            format!(
                "MIX 2 L {} R {}",
                render_mix_meter(lane_a),
                render_mix_meter(lane_b),
            )
        }
        Some(surface) => format!("exp pair pending: unsupported surface {:02x}", surface),
        None => "exp pair pending: missing surface byte".to_string(),
    }
}

fn render_mix_meter(raw: u8) -> String {
    let bar = render_symbol_bar(meter_ratio(raw), 8, '█', '░');
    format!(
        "{} {}",
        bar,
        format_meter_value_label(meter_display_db(raw))
    )
}

#[cfg(test)]
fn render_mixer_strip_line(
    state: &AppState,
    index: usize,
    channel: &crate::protocol::MixerChannelState,
) -> String {
    let selected = state.focus == FocusArea::Mixer && state.selected_channel == index;
    let bar = channel
        .meter_ratio()
        .or_else(|| channel.gain_ratio())
        .map(|ratio| render_symbol_bar(ratio, 8, '|', '.'))
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
        "CH {:02} {:<8} src={:<16} level={} meter={} mute={} solo={} pan={} link={} {}",
        channel.channel,
        bar,
        assignment,
        channel
            .display_db()
            .map(|value| format!("{} dB", value))
            .unwrap_or_else(|| "undecoded".to_string()),
        channel
            .meter_db()
            .map(|value| format!("{} dB", value))
            .or_else(|| channel.meter.map(|_| String::new()))
            .unwrap_or_else(|| "undecoded".to_string()),
        channel
            .muted
            .map(|value| if value { "on" } else { "off" })
            .unwrap_or("undecoded"),
        channel
            .soloed
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

#[cfg(test)]
fn observed_meter_label(input: PreampInputState) -> String {
    match input.observed_meter {
        Some(_) => input
            .observed_meter_db()
            .map(|value| format!("obs meter {} dB", value))
            .unwrap_or_default(),
        None => String::new(),
    }
}

fn style_for_preamp_mode(mode: PreampMode) -> Color {
    terminal::adapt_color(match mode {
        PreampMode::Mic => Color::Green,
        PreampMode::Line => Color::Yellow,
        PreampMode::HiZ => Color::Magenta,
        PreampMode::Unknown(_) => Color::Gray,
    })
}

fn warning_section_block(title: &str, focused: bool) -> Block<'_> {
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
    use std::time::Instant;

    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::Widget;

    use crate::app::{AppState, FocusArea};
    use crate::protocol::{
        ClockSource, MixerAssignment, MixerChannelState, MixerLinkTarget, MixerSurface, OutputMode,
        OutputState, OutputTarget, PanState, PreampInputState, SampleRate, Surface,
    };

    use super::*;

    fn render_buffer(area: Rect, render: impl FnOnce(Rect, &mut Buffer)) -> String {
        let mut buffer = Buffer::empty(area);
        render(area, &mut buffer);
        let mut out = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn output_card_rendering_surfaces_level_mode_and_focus() {
        let output = OutputState::new(OutputTarget::Hp1, 0x30, OutputMode::Mute);

        let rendered = render_buffer(Rect::new(0, 0, 40, output_card_height()), |area, buffer| {
            render_output_card_widget(area, buffer, &output, true);
        });

        assert!(rendered.contains("HP1"));
        assert!(rendered.contains("48 dB"));
        assert!(rendered.contains("ACTIVE"));
        assert!(rendered.contains("LVL -48 dB"));
        assert!(rendered.contains("─"));
        assert!(rendered.contains("●"));
        assert!(rendered.contains(" - "));
        assert!(rendered.contains(" + "));
        assert!(rendered.contains(" DIM "));
        assert!(rendered.contains(" MUTE "));
        assert!(!rendered.contains("raw 30"));
    }

    #[test]
    fn output_card_areas_split_horizontally_across_bottom_panel() {
        let areas = output_card_areas(Rect::new(10, 5, 90, output_card_height()));

        assert_eq!(areas.len(), 3);
        assert_eq!(areas[0].y, areas[1].y);
        assert_eq!(areas[1].y, areas[2].y);
        assert!(areas[0].x < areas[1].x);
        assert!(areas[1].x < areas[2].x);
    }

    #[test]
    fn hotkeys_popup_text_lists_core_shortcuts() {
        let rendered = render_hotkeys_popup_text().to_string();

        assert!(rendered.contains("Global"));
        assert!(rendered.contains("? hotkeys"));
        assert!(rendered.contains("Outputs: m mute   d dim"));
        assert!(rendered.contains("r raw inspector"));
    }

    #[test]
    fn mouse_action_hits_output_hotkeys_button() {
        let area = Rect::new(0, 0, 120, 50);
        let page = mixer_page_layout(root_chunks(area)[1]);
        let button = output_hotkeys_button_rect(page[1]);

        assert_eq!(
            mouse_action(area, &AppState::default(), button.x + 1, button.y),
            Some(MouseAction::ToggleHotkeysPopup)
        );
    }

    #[test]
    fn meter_value_labels_reserve_width_and_use_negative_infinity() {
        assert_eq!(format_meter_value_label(Some(0)), "  0 dB");
        assert_eq!(format_meter_value_label(Some(-48)), "-48 dB");
        assert_eq!(format_meter_value_label(None), " -∞ dB");
        assert_eq!(format_meter_value_label(Some(0)).chars().count(), 6);
        assert_eq!(format_meter_value_label(Some(-48)).chars().count(), 6);
        assert_eq!(format_meter_value_label(None).chars().count(), 6);
    }

    #[test]
    fn mixer_strip_widget_renders_compact_vertical_strip_layout() {
        let mut state = AppState::default();
        state.focus = FocusArea::Mixer;
        state.selected_channel = 10;
        state.mixer_channels[0][10] = MixerChannelState {
            channel: 11,
            level: Some(0x10),
            meter: Some(0x30),
            muted: Some(false),
            soloed: Some(true),
            pan: PanState::from_raw(0x3e),
            assignment: Some(MixerAssignment::ComputerPlay(8)),
            linked: Some(true),
        };

        let rendered = render_buffer(Rect::new(0, 0, 18, mixer_strip_height()), |area, buffer| {
            render_mixer_strip_widget(area, buffer, &state, 10, &state.mixer_channels[0][10]);
        });

        assert!(rendered.contains("CH 11"));
        assert!(rendered.contains(" C8 "));
        assert!(!rendered.contains("Computer Play 8"));
        assert!(rendered.contains("PAN 30"));
        assert!(rendered.contains("-30"));
        assert!(rendered.contains(" 30"));
        assert!(rendered.contains("-48 dB"));
        assert!(rendered.contains("-16 dB"));
        assert!(rendered.contains(" 60"));
        assert!(rendered.contains(" 40"));
        assert!(rendered.contains(" 30"));
        assert!(rendered.contains(" 20"));
        assert!(rendered.contains(" 15"));
        assert!(rendered.contains(" 10"));
        assert!(rendered.contains("  5"));
        assert!(rendered.contains("  0"));
        assert!(rendered.contains("█"));
        assert!(rendered.contains("●"));
    }

    #[test]
    fn preamp_visual_stacks_observed_meter_and_gain_sliders() {
        let mut input = PreampInputState::from_raw(0x14, 0x10);
        input.observed_meter = Some(0x30);

        let rendered = render_buffer(Rect::new(0, 0, 44, 5), |area, buffer| {
            render_preamp_visual_widget(area, buffer, "Preamp 1", input, true);
        });

        assert!(rendered.contains("Preamp 1"));
        assert!(rendered.contains("GAIN 20 dB"));
        assert!(rendered.contains("OBS -48 dB"));
        assert!(rendered.contains("█"));
        assert!(rendered.contains("░"));
        assert!(rendered.contains("─"));
        assert!(rendered.contains("●"));
        assert!(!rendered.contains("48V:"));
        assert!(!rendered.contains("PH:"));
        assert!(!rendered.contains("raw "));
    }

    #[test]
    fn mixer_strip_widget_uses_reserved_meter_width_for_silence() {
        let mut state = AppState::default();
        state.focus = FocusArea::Mixer;
        state.selected_channel = 0;
        state.mixer_channels[0][0].level = Some(0x00);
        state.mixer_channels[0][0].meter = Some(0x60);

        let rendered = render_buffer(Rect::new(0, 0, 72, mixer_strip_height()), |area, buffer| {
            render_mixer_strip_widget(area, buffer, &state, 0, &state.mixer_channels[0][0]);
        });

        assert!(rendered.contains(" -∞ dB"));
    }

    #[test]
    fn mixer_strip_widget_keeps_db_scale_markers_in_wide_area() {
        let mut state = AppState::default();
        state.focus = FocusArea::Mixer;
        state.selected_channel = 0;
        state.mixer_channels[0][0].level = Some(0x00);
        state.mixer_channels[0][0].meter = Some(0x10);

        let rendered = render_buffer(
            Rect::new(0, 0, 120, mixer_strip_height()),
            |area, buffer| {
                render_mixer_strip_widget(area, buffer, &state, 0, &state.mixer_channels[0][0]);
            },
        );

        assert!(rendered.contains("60"));
        assert!(rendered.contains("30"));
        assert!(rendered.contains("LVL 0 dB"));
    }

    #[test]
    fn labeled_level_slider_keeps_handle_visible_at_maximum() {
        let area = Rect::new(0, 0, 24, 1);
        let mut buffer = Buffer::empty(area);

        render_labeled_slider(
            area,
            &mut buffer,
            "LVL   0 dB",
            Some(1.0),
            Color::Yellow,
            true,
        );

        assert_eq!(buffer[(23, 0)].symbol(), "●");
    }

    #[test]
    fn status_strip_surfaces_message_surface_and_output() {
        let mut state = AppState::default();
        state.surface = Surface::Hp2;
        state.selected_output = 1;
        state.last_message = "Applied dim change".to_string();

        let rendered = render_status_strip(&state).to_string();

        assert!(!rendered.contains("STATUS"));
        assert!(!rendered.contains("Applied dim change"));
        assert_eq!(rendered, render_experimental_pair_state_line(&state));
    }

    #[test]
    fn experimental_mix_meter_extracts_mix1_lane_pair() {
        let mut state = AppState::default();
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x6a] = 0x0f;
        frame[0x10 + 0xda] = 0x0a;
        frame[0x10 + 0xdb] = 0x05;
        state.latest_raw_73 = Some(frame);

        assert_eq!(experimental_mix_meter(&state), Some(("MIX 1", 0x0a, 0x05)));
    }

    #[test]
    fn mixer_strip_panel_layout_reserves_two_rows_for_embedded_mix_meter() {
        let layout = mixer_strip_panel_layout(Rect::new(0, 0, 80, 14), true);

        assert_eq!(layout[1].height, 2);
        assert_eq!(
            layout[0].height + layout[1].height,
            inner_area(Rect::new(0, 0, 80, 14)).height
        );
    }

    #[test]
    fn mixer_list_mouse_action_ignores_embedded_mix_meter_rows() {
        let mut state = AppState::default();
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x6a] = 0x0f;
        frame[0x10 + 0xda] = 0x0a;
        frame[0x10 + 0xdb] = 0x05;
        state.latest_raw_73 = Some(frame);

        let mixer = mixer_layout(Rect::new(0, 0, 100, 20));
        let meter_area = mixer_strip_panel_layout(mixer[1], true)[1];

        assert_eq!(
            mixer_list_mouse_action(mixer[1], &state, (meter_area.x + 1, meter_area.y)),
            None
        );
    }

    #[test]
    fn mix_meter_widget_renders_two_row_stereo_bar_and_fixed_db_labels() {
        let rendered = render_buffer(Rect::new(0, 0, 56, 2), |area, buffer| {
            render_mix_meter_widget(area, buffer, 0x00, 0x3c);
        });

        assert!(rendered.contains("L"));
        assert!(rendered.contains("R"));
        assert!(rendered.contains("  0 dB"));
        assert!(rendered.contains("-60 dB"));
        assert!(rendered.contains("█"));
        assert!(rendered.contains("░"));
        let lines = rendered.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("L"));
        assert!(lines[1].contains("R"));
    }

    #[test]
    fn device_header_surfaces_serial_and_hw_without_duplicate_status_line() {
        let mut state = AppState::default();
        state.device.metadata = Some(crate::protocol::DeviceMetadata {
            product_name: "Zen Go Synergy Core".to_string(),
            serial: "1234567890".to_string(),
            hardware_version: "6.6".to_string(),
        });
        state.device.sample_rate = Some(SampleRate::Hz48000);
        state.device.clock_source = Some(ClockSource::Internal);
        state.device.lock_known = true;
        state.device.locked = Some(true);

        let rendered = render_device_header(&state).to_string();
        let metadata = render_device_metadata(&state).to_string();

        assert!(metadata.contains("1234567890"));
        assert!(metadata.contains("6.6"));
        assert!(metadata.contains(" HW  6.6 "));
        assert!(!rendered.contains("SURFACE"));
        assert!(!rendered.contains("PAGE"));
        assert!(!rendered.contains("Last"));
        assert!(!rendered.contains('\n'));
    }

    #[test]
    fn device_header_prefers_live_sample_rate_readout_over_configured_rate() {
        let mut state = AppState::default();
        state.device.sample_rate = Some(SampleRate::Hz96000);
        state.device.sample_rate_hz = Some(44_100);

        let rendered = render_device_header(&state).to_string();

        assert!(rendered.contains("44.1 kHz"));
        assert!(!rendered.contains("96000 Hz"));
    }

    #[test]
    fn afx_page_renders_usb_recording_pairs_from_mixer_assignments() {
        let mut state = AppState::default();
        state.mixer_channels[MixerSurface::Mix1.index()][0].assignment =
            Some(MixerAssignment::Preamp(1));
        state.mixer_channels[MixerSurface::Mix1.index()][1].assignment =
            Some(MixerAssignment::Preamp(2));
        for channel in 2..8 {
            state.mixer_channels[MixerSurface::Mix1.index()][channel].assignment =
                Some(MixerAssignment::Mute);
        }

        let rendered = render_afx_routing_text(&state).to_string();

        assert!(rendered.contains("Zen Go USB recordings mirror mixer strip assignments"));
        assert!(rendered.contains("USB 1/2  Zen Go Recording 1 <- Preamp 1"));
        assert!(rendered.contains("Zen Go Recording 2 <- Preamp 2"));
        assert!(rendered.contains("USB 7/8  Zen Go Recording 7 <- Mute"));
        assert!(rendered.contains("Zen Go Recording 8 <- Mute"));
    }

    #[test]
    fn titlebar_renders_inspector_hint_on_single_row() {
        let rendered = render_inspector_summary().to_string();

        assert!(rendered.contains("RAW"));
        assert!(rendered.contains("[r] inspector"));
        assert!(!rendered.contains('\n'));
    }

    #[test]
    fn connection_badge_uses_green_orange_and_red_states() {
        let mut state = AppState::default();
        assert_eq!(connection_badge_color(&state), Color::Rgb(255, 165, 0));

        state.connection.connected = true;
        assert_eq!(connection_badge_color(&state), Color::LightGreen);

        state.connection.connected = false;
        state.connection.last_snapshot_at = Some(Instant::now() - Duration::from_secs(3));
        assert_eq!(connection_badge_color(&state), Color::LightRed);
    }

    #[test]
    fn mixer_strip_rendering_includes_solo_state() {
        let mut state = AppState::default();
        state.focus = crate::app::FocusArea::Mixer;
        state.selected_channel = 0;
        state.mixer_channels[MixerSurface::Mix1.index()][0].soloed = Some(true);

        let line = render_mixer_strip_line(
            &state,
            0,
            &state.mixer_channels[MixerSurface::Mix1.index()][0],
        );
        let controls = render_mixer_strip_controls(
            &state,
            0,
            &state.mixer_channels[MixerSurface::Mix1.index()][0],
        );

        assert!(line.contains("solo=on"));
        assert!(controls.contains("[Solo on]"));
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
    fn query_reply_panel_includes_recent_reply_log() {
        let mut state = AppState::default();
        state.recent_query_reply_entries = vec![
            crate::app::QueryReplyLogEntry {
                summary: "0x75 03/05 [64 bytes] 05 00 00 00 01 01 00 01".to_string(),
                raw: vec![0x75, 0x05],
            },
            crate::app::QueryReplyLogEntry {
                summary: "0x75 03/06 [64 bytes] 06 03 00 03 01 03 02 03".to_string(),
                raw: vec![0x75, 0x06],
            },
        ];
        state.selected_query_reply_entry = Some(1);

        let text = render_query_reply_panel(&[0x75, 0x00, 0x00, 0x00], &state).to_string();

        assert!(text.contains("0000: 75 06"));
    }

    #[test]
    fn query_request_panel_includes_recent_request_log() {
        let mut state = AppState::default();
        state.recent_query_request_log = vec!["0x74 03/05".to_string(), "0x74 03/06".to_string()];

        let text = render_query_request_panel(&[0x74, 0x00, 0x00, 0x00], &state).to_string();

        assert!(text.contains("Recent 0x74 requests:"));
        assert!(text.contains("0x74 03/05"));
        assert!(text.contains("0x74 03/06"));
    }

    #[test]
    fn mouse_action_hits_status_raw_view_toggle() {
        let area = Rect::new(0, 0, 120, 50);
        let button = titlebar_layout(root_chunks(area)[0])[1];
        let point = (button.x + button.width / 2, button.y + 1);

        assert_eq!(
            mouse_action(area, &AppState::default(), point.0, point.1),
            Some(MouseAction::ToggleRawView)
        );
    }

    #[test]
    fn mouse_action_selects_raw_packet_tab_when_raw_view_is_open() {
        let area = Rect::new(0, 0, 120, 50);
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(2),
            ])
            .split(area);
        let tabs = raw_tab_hit_areas(layout[1]);
        let point = (tabs[3].x + tabs[3].width / 2, tabs[3].y);
        let mut state = AppState::default();
        state.raw_view_open = true;

        assert_eq!(
            mouse_action(area, &state, point.0, point.1),
            Some(MouseAction::SelectRawPacketTab(RawPacketTab::Query75))
        );
    }

    #[test]
    fn mouse_action_opens_routing_popup_from_mixer_surface_button() {
        let area = Rect::new(0, 0, 120, 50);
        let page = mixer_page_layout(root_chunks(area)[1]);
        let main = mixer_main_layout(page[0]);
        let mixer = mixer_layout(main[1]);
        let button = routing_button_rect(mixer[0]);
        let point = (button.x + button.width / 2, button.y);

        assert_eq!(
            mouse_action(area, &AppState::default(), point.0, point.1),
            Some(MouseAction::OpenRoutingPopup)
        );
    }

    #[test]
    fn mouse_action_opens_assignment_picker_from_afx_routing_source_chip() {
        let area = Rect::new(0, 0, 120, 50);
        let mut state = AppState::default();
        state.routing_popup_open = true;
        state.focus = FocusArea::Mixer;
        state.mixer_channels[MixerSurface::Mix1.index()][0].assignment =
            Some(MixerAssignment::Preamp(1));
        state.mixer_channels[MixerSurface::Mix1.index()][1].assignment =
            Some(MixerAssignment::Preamp(2));
        let row_area = afx_routing_layout(routing_popup_area(area))[4];
        let rects = afx_routing_row_rects(row_area, &state, 0);
        let point = (rects[2].x + rects[2].width / 2, rects[2].y);

        assert_eq!(
            mouse_action(area, &state, point.0, point.1),
            Some(MouseAction::OpenAssignmentPicker(1))
        );
    }

    #[test]
    fn afx_routing_source_columns_stay_aligned_for_different_label_lengths() {
        let area = Rect::new(0, 0, 120, 1);
        let mut state = AppState::default();
        state.mixer_channels[MixerSurface::Mix1.index()][0].assignment =
            Some(MixerAssignment::Mute);
        state.mixer_channels[MixerSurface::Mix1.index()][1].assignment =
            Some(MixerAssignment::Preamp(2));
        state.mixer_channels[MixerSurface::Mix1.index()][2].assignment =
            Some(MixerAssignment::ComputerPlay(8));
        state.mixer_channels[MixerSurface::Mix1.index()][3].assignment =
            Some(MixerAssignment::Oscillator(1));

        let first = afx_routing_row_rects(area, &state, 0);
        let second = afx_routing_row_rects(area, &state, 1);

        assert_eq!(first[2].x, second[2].x);
        assert_eq!(first[4].x, second[4].x);
    }

    #[test]
    fn mouse_action_opens_sample_rate_selector_from_device_chip() {
        let area = Rect::new(0, 0, 120, 50);
        let mut state = AppState::default();
        state.device.clock_source = Some(ClockSource::Internal);
        let chips = device_header_hit_areas(titlebar_layout(root_chunks(area)[0])[0], &state);

        assert_eq!(
            mouse_action(area, &state, chips[1].x + 1, chips[1].y),
            Some(MouseAction::OpenSampleRateSelector)
        );
    }

    #[test]
    fn mouse_action_does_not_open_sample_rate_selector_when_clock_is_external() {
        let area = Rect::new(0, 0, 120, 50);
        let mut state = AppState::default();
        state.device.clock_source = Some(ClockSource::Usb);
        let chips = device_header_hit_areas(titlebar_layout(root_chunks(area)[0])[0], &state);

        assert_eq!(mouse_action(area, &state, chips[1].x + 1, chips[1].y), None);
    }

    #[test]
    fn mouse_action_hits_visible_surface_tab_position() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let mixer = mixer_layout(main[1]);
        let tabs = surface_tab_hit_areas(mixer[0]);

        assert_eq!(
            mouse_action(area, &AppState::default(), tabs[1].x + 1, tabs[1].y),
            Some(MouseAction::SelectSurface(Surface::Hp2))
        );
    }

    #[test]
    fn mouse_action_hits_visible_output_dim_chip_position() {
        let area = Rect::new(0, 0, 120, 50);
        let page = mixer_page_layout(root_chunks(area)[1]);
        let list_inner = inner_area(page[1]);
        let row_area = output_card_areas(list_inner)[0];
        let state = AppState::default();
        let dim = output_control_rects(row_area)[2];

        assert_eq!(
            mouse_action(area, &state, dim.x + dim.width / 2, dim.y),
            Some(MouseAction::ToggleOutputDim(0))
        );
    }

    #[test]
    fn mouse_action_hits_visible_output_mute_chip_position_on_hp1() {
        let area = Rect::new(0, 0, 120, 50);
        let page = mixer_page_layout(root_chunks(area)[1]);
        let list_inner = inner_area(page[1]);
        let row_area = output_card_areas(list_inner)[1];
        let state = AppState::default();
        let mute = output_control_rects(row_area)[3];

        assert_eq!(
            mouse_action(area, &state, mute.x + mute.width / 2, mute.y),
            Some(MouseAction::ToggleOutputMute(1))
        );
    }

    #[test]
    fn mouse_action_selects_recent_query_reply_entry_when_raw_query_tab_is_open() {
        let area = Rect::new(0, 0, 120, 50);
        let layout = raw_page_layout(area);
        let sections = query_reply_history_layout(layout[2]);
        let inner = inner_area(sections[0]);
        let mut state = AppState::default();
        state.raw_view_open = true;
        state.selected_raw_packet = RawPacketTab::Query75;
        state.recent_query_reply_entries = vec![
            crate::app::QueryReplyLogEntry {
                summary: "0x75 03/05".to_string(),
                raw: vec![0x75, 0x05],
            },
            crate::app::QueryReplyLogEntry {
                summary: "0x75 03/06".to_string(),
                raw: vec![0x75, 0x06],
            },
        ];
        let point = (inner.x + 1, inner.y + 1);

        assert_eq!(
            mouse_action(area, &state, point.0, point.1),
            Some(MouseAction::SelectQueryReplyEntry(1))
        );
    }

    #[test]
    fn mouse_action_hits_preamp_gain_up_button() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let cards = preamp_bar_layout(main[0]);
        let buttons = preamp_button_rects(cards[0], AppState::default().preamp.input1);
        let point = (buttons[1].x + buttons[1].width / 2, buttons[1].y);

        assert_eq!(
            mouse_action(area, &AppState::default(), point.0, point.1),
            Some(MouseAction::AdjustPreampGain {
                input: 0,
                increase: true,
            })
        );
    }

    #[test]
    fn output_card_renders_arrow_adjust_buttons() {
        let rendered = render_output_card(&AppState::default().outputs[0], true).to_string();

        assert!(rendered.contains(" ↑ "));
        assert!(rendered.contains(" ↓ "));
        assert!(!rendered.contains(" + "));
        assert!(!rendered.contains(" - "));
    }

    #[test]
    fn preamp_controls_render_arrow_adjust_buttons() {
        let rendered = render_preamp_controls_text(AppState::default().preamp.input1).to_string();

        assert!(rendered.contains(" ↑ "));
        assert!(rendered.contains(" ↓ "));
        assert!(!rendered.contains(" + "));
        assert!(!rendered.contains(" - "));
    }

    #[test]
    fn slider_wheel_action_adjusts_output_level_one_step() {
        let area = Rect::new(0, 0, 120, 50);
        let page = mixer_page_layout(root_chunks(area)[1]);
        let card = output_card_areas(inner_area(page[1]))[0];
        let track = output_level_slider_rect(card);

        assert_eq!(
            slider_wheel_action(area, &AppState::default(), track.x, track.y, true),
            Some(MouseAction::AdjustOutputLevel {
                index: 0,
                increase: true,
            })
        );
    }

    #[test]
    fn slider_wheel_action_adjusts_preamp_gain_one_step() {
        let area = Rect::new(0, 0, 120, 50);
        let card =
            preamp_bar_layout(mixer_main_layout(mixer_page_layout(root_chunks(area)[1])[0])[0])[0];
        let track = preamp_gain_slider_rect(card);

        assert_eq!(
            slider_wheel_action(area, &AppState::default(), track.x, track.y, true),
            Some(MouseAction::AdjustPreampGain {
                input: 0,
                increase: true,
            })
        );
    }

    #[test]
    fn slider_wheel_action_adjusts_mixer_pan_inside_strip_panel() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let mixer = mixer_layout(main[1]);
        let list_inner = mixer_strip_panel_layout(mixer[1], false)[0];
        let card = mixer_strip_card_area(list_inner, 0);
        let track = mixer_pan_slider_rect(card);

        assert_eq!(
            slider_wheel_action(area, &AppState::default(), track.x, track.y, true),
            Some(MouseAction::AdjustMixerPan {
                index: 0,
                right: true,
            })
        );
    }

    #[test]
    fn slider_wheel_action_adjusts_mixer_level_inside_strip_panel() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let mixer = mixer_layout(main[1]);
        let list_inner = mixer_strip_panel_layout(mixer[1], false)[0];
        let card = mixer_strip_card_area(list_inner, 0);
        let track = mixer_level_slider_rect(card);

        assert_eq!(
            slider_wheel_action(area, &AppState::default(), track.x, track.y, true),
            Some(MouseAction::AdjustMixerLevel {
                index: 0,
                increase: true,
            })
        );
    }

    #[test]
    fn mouse_action_hits_visible_output_level_slider_position() {
        let area = Rect::new(0, 0, 120, 50);
        let page = mixer_page_layout(root_chunks(area)[1]);
        let card = output_card_areas(inner_area(page[1]))[0];
        let slider_row = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(card)[1];
        let slider_area = bounded_signal_area(slider_row);
        let label_width = SIGNAL_LABEL_WIDTH
            .min(slider_area.width.saturating_sub(1))
            .max(1);
        let track = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(label_width), Constraint::Min(1)])
            .split(slider_area)[1];

        assert_eq!(
            mouse_action(
                area,
                &AppState::default(),
                track.x + track.width.saturating_sub(1),
                track.y
            ),
            Some(MouseAction::SetOutputLevel { index: 0, step: 0 })
        );
    }

    #[test]
    fn mouse_action_hits_visible_preamp_gain_slider_position() {
        let area = Rect::new(0, 0, 120, 50);
        let card =
            preamp_bar_layout(mixer_main_layout(mixer_page_layout(root_chunks(area)[1])[0])[0])[0];
        let signal_area = preamp_card_inner_layout(card)[0];
        let gain_row = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(signal_area)[1];
        let slider_area = bounded_signal_area(gain_row);
        let label_width = SIGNAL_LABEL_WIDTH
            .min(slider_area.width.saturating_sub(1))
            .max(1);
        let track = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(label_width), Constraint::Min(1)])
            .split(slider_area)[1];

        assert_eq!(
            mouse_action(
                area,
                &AppState::default(),
                track.x + track.width.saturating_sub(1),
                track.y
            ),
            Some(MouseAction::SetPreampGain {
                input: 0,
                raw: 0x41
            })
        );
    }

    #[test]
    fn mouse_action_hits_visible_mixer_pan_slider_position() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let mixer = mixer_layout(main[1]);
        let list_inner = mixer_strip_panel_layout(mixer[1], false)[0];
        let card = mixer_strip_card_area(list_inner, 0);
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
            .split(mixer_strip_inner_area(card));

        assert_eq!(
            mouse_action(
                area,
                &AppState::default(),
                rows[2].x + rows[2].width.saturating_sub(1),
                rows[2].y
            ),
            Some(MouseAction::SetMixerPan {
                index: 0,
                pan: PanState::right(),
            })
        );
    }

    #[test]
    fn mouse_action_hits_visible_mixer_level_slider_position() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let mixer = mixer_layout(main[1]);
        let list_inner = mixer_strip_panel_layout(mixer[1], false)[0];
        let card = mixer_strip_card_area(list_inner, 0);
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
            .split(mixer_strip_inner_area(card));
        let combo = rows[5];
        let content_width = 6.min(combo.width);
        let content_area = Rect::new(
            combo.x + combo.width.saturating_sub(content_width) / 2,
            combo.y,
            content_width,
            combo.height,
        );
        let level = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(content_area)[2];

        assert_eq!(
            mouse_action(area, &AppState::default(), level.x, level.y),
            Some(MouseAction::SetMixerLevel { index: 0, level: 0 })
        );
    }

    #[test]
    fn mouse_action_hits_visible_preamp_mode_chip_position() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let cards = preamp_bar_layout(main[0]);
        let state = AppState::default();
        let mode = preamp_button_rects(cards[0], state.preamp.input1)[2];

        assert_eq!(
            mouse_action(area, &state, mode.x + mode.width / 2, mode.y),
            Some(MouseAction::OpenPreampModeSelector(0))
        );
    }

    #[test]
    fn mouse_action_picks_preamp_mode_from_selector_popup() {
        let area = Rect::new(0, 0, 120, 50);
        let mut state = AppState::default();
        state.selector_popup = Some(SelectorPopupState {
            kind: SelectorPopupKind::PreampMode { input: 0 },
        });
        let popup = assignment_picker_area(area);
        let inner = popup_list_inner_area(popup, "Preamp Mode");

        assert_eq!(
            mouse_action(area, &state, inner.x + 1, inner.y + 1),
            Some(MouseAction::PickPreampMode {
                input: 0,
                mode: PreampMode::Line,
            })
        );
    }

    #[test]
    fn mouse_action_picks_first_assignment_from_first_popup_row() {
        let area = Rect::new(0, 0, 120, 50);
        let popup = assignment_picker_area(area);
        let inner = popup_list_inner_area(popup, "Assign CH 11");
        let mut state = AppState::default();
        state.assignment_picker = Some(AssignmentPickerState { strip: 11 });

        assert_eq!(
            mouse_action(area, &state, inner.x + 1, inner.y),
            Some(MouseAction::PickAssignment {
                strip: 11,
                assignment: MixerAssignment::Mute,
            })
        );
    }

    #[test]
    fn preamp_control_row_keeps_leading_chip_padding_when_rendered() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 1));
        Paragraph::new(render_preamp_controls_text(
            AppState::default().preamp.input1,
        ))
        .render(Rect::new(0, 0, 40, 1), &mut buffer);

        assert_eq!(buffer[(0, 0)].symbol(), " ");
        assert_eq!(buffer[(1, 0)].symbol(), "↓");
    }

    #[test]
    fn mouse_action_hits_mixer_link_button_on_odd_strip() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let mixer = mixer_layout(main[1]);
        let list_inner = mixer_strip_panel_layout(mixer[1], false)[0];
        let card = mixer_strip_card_area(list_inner, 0);
        let buttons = mixer_control_button_rects(card, true);
        let point = (buttons[0].x + buttons[0].width / 2, buttons[0].y);

        assert_eq!(
            mouse_action(area, &AppState::default(), point.0, point.1),
            Some(MouseAction::ToggleMixerLink(1))
        );
    }

    #[test]
    fn mouse_action_hits_mixer_solo_button() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let mixer = mixer_layout(main[1]);
        let list_inner = mixer_strip_panel_layout(mixer[1], false)[0];
        let card = mixer_strip_card_area(list_inner, 0);
        let buttons = mixer_control_button_rects(card, true);
        let point = (buttons[1].x + buttons[1].width / 2, buttons[1].y);

        assert_eq!(
            mouse_action(area, &AppState::default(), point.0, point.1),
            Some(MouseAction::ToggleMixerSolo(1))
        );
    }

    #[test]
    fn mouse_action_hits_visible_mixer_solo_chip_position() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let mixer = mixer_layout(main[1]);
        let list_inner = mixer_strip_panel_layout(mixer[1], false)[0];
        let card = mixer_strip_card_area(list_inner, 0);
        let state = AppState::default();
        let buttons = mixer_control_button_rects(card, true);
        let point = (buttons[1].x + buttons[1].width / 2, buttons[1].y);

        assert_eq!(
            mouse_action(area, &state, point.0, point.1),
            Some(MouseAction::ToggleMixerSolo(1))
        );
    }

    #[test]
    fn mouse_action_opens_assignment_picker_from_src_button() {
        let area = Rect::new(0, 0, 120, 60);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[1]);
        let main = mixer_main_layout(page[0]);
        let mixer = mixer_layout(main[1]);
        let list_inner = mixer_strip_panel_layout(mixer[1], false)[0];
        let mut state = AppState::default();
        state.selected_channel = 3;
        state.mixer_channels[0][3].assignment = Some(MixerAssignment::ComputerPlay(2));
        let card = mixer_strip_card_area(list_inner, 3);
        let (_, source_rect) = mixer_header_chip_rects(card, "C2");
        let point = (source_rect.x + source_rect.width / 2, source_rect.y);

        assert_eq!(
            mouse_action(area, &state, point.0, point.1),
            Some(MouseAction::OpenAssignmentPicker(4))
        );
    }

    #[test]
    fn mouse_action_picks_assignment_from_modal() {
        let area = Rect::new(0, 0, 120, 50);
        let popup = assignment_picker_area(area);
        let inner = popup_list_inner_area(popup, "Assign CH 11");
        let mut state = AppState::default();
        state.assignment_picker = Some(AssignmentPickerState { strip: 11 });

        assert_eq!(
            mouse_action(area, &state, inner.x + inner.width / 2, inner.y + 4),
            Some(MouseAction::PickAssignment {
                strip: 11,
                assignment: MixerAssignment::ComputerPlay(2),
            })
        );
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
        assert!(line.contains("pan=R30"));
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
        assert!(line.contains("meter=-48 dB"));
    }

    #[test]
    fn mixer_strip_line_hides_meter_value_below_ui_floor() {
        let mut state = AppState::default();
        state.mixer_channels[0][0].level = Some(0x00);
        state.mixer_channels[0][0].meter = Some(0x60);
        state.mixer_channels[0][0].muted = Some(false);

        let line = render_mixer_strip_line(&state, 0, &state.mixer_channels[0][0]);

        assert!(line.contains("meter= mute=off"));
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
        assert!(line.contains("L ███████░ -10 dB"));
        assert!(line.contains("R ███████░  -5 dB"));
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
        assert!(line.contains("L ████████   0 dB"));
        assert!(line.contains("R ███████░  -6 dB"));
    }

    #[test]
    fn experimental_pair_state_line_surfaces_no_signal_family_as_pending_meter() {
        let mut state = AppState::default();
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x6a] = 0x0c;
        frame[0x10 + 0xde] = 0x5a;
        frame[0x10 + 0xdf] = 0x5a;
        frame[0x10 + 0x6e] = 0x60;
        frame[0x10 + 0x8e] = 0x60;
        frame[0x10 + 0xe2] = 0x60;
        state.latest_raw_73 = Some(frame);

        let line = render_experimental_pair_state_line(&state);

        assert!(line.contains("MIX 2"));
        assert!(line.contains("L ░░░░░░░░  -∞ dB"));
        assert!(line.contains("R ░░░░░░░░  -∞ dB"));
    }

    #[test]
    fn experimental_pair_state_line_keeps_unknown_meter_bytes_visible() {
        let mut state = AppState::default();
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x6a] = 0x0c;
        frame[0x10 + 0xde] = 0x12;
        frame[0x10 + 0xdf] = 0x34;
        state.latest_raw_73 = Some(frame);

        let line = render_experimental_pair_state_line(&state);

        assert!(line.contains("L ██████░░ -18 dB"));
        assert!(line.contains("R █░░░░░░░ -52 dB"));
    }

    #[test]
    fn observed_meter_label_mentions_raw_value() {
        let mut input = PreampInputState::from_raw(0x2a, 0x00);
        input.observed_meter = Some(0x30);

        assert_eq!(observed_meter_label(input), "obs meter -48 dB");
    }

    #[test]
    fn observed_meter_label_mentions_pending_state() {
        assert_eq!(
            observed_meter_label(PreampInputState::from_raw(0x2a, 0x00)),
            ""
        );
    }

    #[test]
    fn observed_meter_label_hides_values_below_ui_floor() {
        let mut input = PreampInputState::from_raw(0x2a, 0x00);
        input.observed_meter = Some(0x60);

        assert_eq!(observed_meter_label(input), "");
    }
}
