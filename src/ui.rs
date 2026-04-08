use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Widget, Wrap};
use ratatui::Frame;
use tui_slider::{Slider, SliderOrientation, SliderState};

use crate::app::{
    AppState, AssignmentPickerState, FocusArea, MainPage, RawPacketTab, SelectorPopupKind,
    SelectorPopupState,
};
use crate::protocol::{
    meter_display_db, meter_ratio, ClockSource, MixerAssignment, MixerSurface, OutputMode,
    OutputState, PreampInputState, PreampMode, SampleRate, Surface,
};
use crate::terminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    ToggleRawView,
    OpenSampleRateSelector,
    OpenClockSourceSelector,
    SelectPage(MainPage),
    SelectOutput(usize),
    AdjustOutputLevel {
        index: usize,
        increase: bool,
    },
    ToggleOutputDim(usize),
    ToggleOutputMute(usize),
    SelectRawPacketTab(RawPacketTab),
    SelectQueryReplyEntry(usize),
    SelectSurface(Surface),
    SelectMixerChannel(usize),
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
        return;
    }

    let chunks = root_chunks(frame.area());

    draw_titlebar(frame, chunks[0], state);
    draw_page_tabs(frame, chunks[1], state);
    match state.page {
        MainPage::Mixer => draw_mixer_page(frame, chunks[2], state),
        MainPage::AfxDsp => draw_afx_page(frame, chunks[2], state),
    }
    draw_footer(frame, chunks[3], state);
    draw_assignment_picker(frame, frame.area(), state);
    draw_selector_popup(frame, frame.area(), state);
}

fn root_chunks(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(14),
            Constraint::Length(3),
        ])
        .split(area)
        .to_vec()
}

fn titlebar_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(24), Constraint::Length(18)])
        .split(area)
        .to_vec()
}

fn device_header_hit_areas(area: Rect, state: &AppState) -> Vec<Rect> {
    let inner = inner_area(area);
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

fn page_tab_hit_areas(area: Rect) -> Vec<Rect> {
    inline_chip_rects(
        inner_area(area).x,
        inner_area(area).y,
        &["Mixer", "AFX / DSP"],
    )
}

fn mixer_page_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(26), Constraint::Min(40)])
        .split(area)
        .to_vec()
}

fn output_panel_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(6)])
        .split(area)
        .to_vec()
}

fn mixer_main_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(12)])
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

fn mixer_workspace_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(3)])
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

fn preamp_card_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Length(3)])
        .split(area)
        .to_vec()
}

fn preamp_button_rects(area: Rect, input: PreampInputState) -> Vec<Rect> {
    let inner = inner_area(area);
    inline_chip_rects(
        inner.x,
        inner.y,
        &[
            "-",
            "+",
            input.mode.label(),
            preamp_phantom_label(input),
            preamp_phase_label(input),
        ],
    )
}

fn surface_tab_hit_areas(area: Rect) -> Vec<Rect> {
    inline_chip_rects(
        inner_area(area).x,
        inner_area(area).y,
        &["MIX 1 / Monitor-HP1", "MIX 2 / HP2"],
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
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

fn draw_titlebar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let sections = titlebar_layout(area);
    let text = Paragraph::new(render_device_header(state))
        .block(panel_block("Device", Color::DarkGray, true))
        .wrap(Wrap { trim: false });
    frame.render_widget(text, sections[0]);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(chip("RAW", Color::Black, Color::LightRed)),
            Line::from(Span::styled("[r] inspector", muted_style())),
        ])
        .block(panel_block("Inspector", Color::LightRed, false))
        .wrap(Wrap { trim: false }),
        sections[1],
    );
}

fn draw_page_tabs(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let line = Line::from(vec![
        tab_chip("Mixer", state.page == MainPage::Mixer, Color::LightGreen),
        Span::raw(" "),
        tab_chip(
            "AFX / DSP",
            state.page == MainPage::AfxDsp,
            Color::LightMagenta,
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line)
            .block(panel_block("Pages", Color::DarkGray, false))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_mixer_page(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let sections = mixer_page_layout(area);
    draw_output_panel(frame, sections[0], state);

    let main = mixer_main_layout(sections[1]);
    draw_preamp_bar(frame, main[0], state);

    let workspace = mixer_workspace_layout(main[1]);
    draw_mixer_main(frame, workspace[0], state);
    draw_status_strip(frame, workspace[1], state);
}

fn draw_afx_page(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let placeholder = Paragraph::new(vec![
        Line::from(vec![chip("AFX / DSP", Color::Black, Color::LightMagenta)]),
        Line::from(""),
        Line::from("Reserved for future DSP controls and insert-style visual routing."),
        Line::from("Raw inspector remains available with `r`."),
        Line::from(""),
        Line::from(vec![
            chip("PLANNED", Color::Black, Color::Yellow),
            Span::raw(" "),
            Span::styled(
                "slots, returns, DSP chains, per-engine state",
                muted_style(),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("STATUS ", subdued_style()),
            Span::styled(state.last_message.clone(), strong_style(Color::LightCyan)),
        ]),
    ])
    .block(panel_block("AFX / DSP", Color::Magenta, true))
    .wrap(Wrap { trim: false });
    frame.render_widget(placeholder, area);
}

fn draw_output_panel(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let sections = output_panel_layout(area);
    frame.render_widget(
        panel_block(
            "Outputs",
            Color::Rgb(70, 120, 90),
            state.focus == FocusArea::Outputs,
        ),
        sections[0],
    );
    let inner = inner_area(sections[0]);
    for (index, output) in state.outputs.iter().enumerate() {
        let y = inner.y + index as u16 * output_card_height();
        if y + output_card_height() > inner.y + inner.height {
            break;
        }
        render_output_card_widget(
            Rect::new(inner.x, y, inner.width, output_card_height()),
            frame.buffer_mut(),
            output,
            state.focus == FocusArea::Outputs && state.selected_output == index,
        );
    }
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                chip("f", Color::Black, Color::LightGreen),
                Span::raw(" focus mixer"),
            ]),
            Line::from(vec![
                chip("+/-", Color::Black, Color::Yellow),
                Span::raw(" output level"),
            ]),
            Line::from(vec![
                chip("m", Color::Black, Color::LightRed),
                Span::raw(" mute   "),
                chip("d", Color::Black, Color::Yellow),
                Span::raw(" dim"),
            ]),
            Line::from(vec![
                chip("1/2", Color::Black, Color::LightBlue),
                Span::raw(" surface"),
            ]),
        ])
        .block(panel_block("Keys", Color::DarkGray, false))
        .wrap(Wrap { trim: false }),
        sections[1],
    );
}

fn draw_preamp_bar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let cards = preamp_bar_layout(area);
    for (index, card) in cards.into_iter().enumerate() {
        let parts = preamp_card_layout(card);
        let input = if index == 0 {
            state.preamp.input1
        } else {
            state.preamp.input2
        };
        let title = if state.focus == FocusArea::Preamp && state.selected_preamp_input == index {
            if index == 0 {
                "CH 1 ←"
            } else {
                "CH 2 ←"
            }
        } else if index == 0 {
            "CH 1"
        } else {
            "CH 2"
        };

        render_preamp_visual_widget(
            parts[0],
            frame.buffer_mut(),
            title,
            input,
            state.focus == FocusArea::Preamp && state.selected_preamp_input == index,
        );
        frame.render_widget(preamp_controls_paragraph(input), parts[1]);
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

    frame.render_widget(
        panel_block(
            "Mixer Strips",
            Color::Rgb(70, 100, 130),
            state.focus == FocusArea::Mixer,
        ),
        layout[1],
    );
    let inner = inner_area(layout[1]);
    for (index, channel) in state.active_mixer_channels().iter().enumerate() {
        let y = inner.y + index as u16 * mixer_strip_height();
        if y + mixer_strip_height() > inner.y + inner.height {
            break;
        }
        render_mixer_strip_widget(
            Rect::new(inner.x, y, inner.width, mixer_strip_height()),
            frame.buffer_mut(),
            state,
            index,
            channel,
        );
    }
}

fn draw_status_strip(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    frame.render_widget(
        Paragraph::new(render_status_strip(state))
            .block(panel_block("Mix", Color::DarkGray, false))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let footer = Paragraph::new(render_footer_text(state))
        .block(Block::default().borders(Borders::ALL).title("Help"));
    frame.render_widget(footer, area);
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

    frame.render_widget(Clear, popup);
    frame.render_widget(
        List::new(items).block(panel_block(
            &format!("Assign CH {:02}", picker.strip),
            Color::Yellow,
            true,
        )),
        popup,
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

    frame.render_widget(
        List::new(items).block(panel_block(title, Color::Yellow, true)),
        popup,
    );
}

pub fn mouse_action(area: Rect, state: &AppState, x: u16, y: u16) -> Option<MouseAction> {
    let point = (x, y);
    let chunks = root_chunks(area);

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

    if let Some(action) = page_tab_mouse_action(chunks[1], point) {
        return Some(action);
    }

    if let Some(picker) = state.assignment_picker {
        return assignment_picker_mouse_action(area, picker, point);
    }

    if state.page != MainPage::Mixer {
        return None;
    }

    let page = mixer_page_layout(chunks[2]);
    let main = mixer_main_layout(page[1]);
    let workspace = mixer_workspace_layout(main[1]);
    let mixer_sections = mixer_layout(workspace[0]);

    if let Some(action) = output_list_mouse_action(page[0], state, point) {
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

    let inner = inner_area(popup);
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

    let inner = inner_area(popup_area);
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

fn page_tab_mouse_action(area: Rect, point: (u16, u16)) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    let tabs = page_tab_hit_areas(area);
    if contains_point(tabs[0], point) {
        Some(MouseAction::SelectPage(MainPage::Mixer))
    } else if contains_point(tabs[1], point) {
        Some(MouseAction::SelectPage(MainPage::AfxDsp))
    } else {
        None
    }
}

fn mixer_tab_mouse_action(area: Rect, point: (u16, u16)) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
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
    let inner = inner_area(area);
    if point.1 < inner.y {
        return None;
    }
    let row = point.1.saturating_sub(inner.y) / mixer_strip_height();
    let index = row as usize;
    let channel = state.active_mixer_channels().get(index)?.channel;
    let row_area = Rect {
        x: inner.x,
        y: inner.y + row * mixer_strip_height(),
        width: inner.width,
        height: mixer_strip_height(),
    };
    if point.1 == row_area.y {
        return Some(MouseAction::SelectMixerChannel(index));
    }

    let controls = mixer_control_button_rects(row_area, channel % 2 == 1);
    if contains_point(controls[0], point) {
        return Some(MouseAction::ToggleMixerSolo(channel));
    }
    if contains_point(controls[1], point) {
        return Some(MouseAction::ToggleMixerMute(channel));
    }
    if channel % 2 == 1 && contains_point(controls[2], point) {
        return Some(MouseAction::ToggleMixerLink(channel));
    }
    let src_rect = if channel % 2 == 1 {
        controls[3]
    } else {
        controls[2]
    };
    if contains_point(src_rect, point) {
        return Some(MouseAction::OpenAssignmentPicker(channel));
    }

    Some(MouseAction::SelectMixerChannel(index))
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
        let parts = preamp_card_layout(card);
        let input_state = if input == 0 {
            state.preamp.input1
        } else {
            state.preamp.input2
        };
        let buttons = preamp_button_rects(parts[1], input_state);
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
        if contains_point(parts[0], point) {
            return Some(MouseAction::SelectPreampInput(input));
        }
        return Some(MouseAction::SelectPreampInput(input));
    }
    None
}

fn output_list_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<MouseAction> {
    let layout = output_panel_layout(area);
    let list_area = layout[0];
    if !contains_point(list_area, point) {
        return None;
    }

    let inner = inner_area(list_area);
    if point.1 < inner.y {
        return None;
    }

    let row = point.1.saturating_sub(inner.y) / output_card_height();
    let index = row as usize;
    state.outputs.get(index)?;
    let card = Rect::new(
        inner.x,
        inner.y + row * output_card_height(),
        inner.width,
        output_card_height(),
    );
    let controls = output_control_rects(card);

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

fn mixer_control_button_rects(area: Rect, has_link: bool) -> Vec<Rect> {
    let y = area.y + 1;
    if has_link {
        inline_chip_rects(area.x, y, &["S", "M", "L", "SRC"])
    } else {
        inline_chip_rects(area.x, y, &["S", "M", "SRC"])
    }
}

fn output_card_height() -> u16 {
    4
}

fn mixer_strip_height() -> u16 {
    3
}

fn output_control_rects(area: Rect) -> Vec<Rect> {
    inline_chip_rects(
        area.x,
        area.y + output_card_height() - 1,
        &["-", "+", "DIM", "MUTE"],
    )
}

fn contains_point(area: Rect, point: (u16, u16)) -> bool {
    point.0 >= area.x
        && point.0 < area.x.saturating_add(area.width)
        && point.1 >= area.y
        && point.1 < area.y.saturating_add(area.height)
}

const SIGNAL_LABEL_WIDTH: u16 = 12;

fn slider_state(ratio: Option<f64>) -> SliderState {
    SliderState::new(ratio.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0, 0.0, 100.0)
}

fn meter_slider(ratio: Option<f64>, color: Color) -> Slider<'static> {
    let state = slider_state(ratio);
    Slider::from_state(&state)
        .orientation(SliderOrientation::Horizontal)
        .show_value(false)
        .show_handle(false)
        .filled_symbol("█")
        .empty_symbol("░")
        .filled_color(terminal::adapt_color(color))
        .empty_color(terminal::adapt_color(Color::DarkGray))
}

fn level_slider(ratio: Option<f64>, color: Color) -> Slider<'static> {
    let state = slider_state(ratio);
    Slider::from_state(&state)
        .orientation(SliderOrientation::Horizontal)
        .show_value(false)
        .show_handle(true)
        .filled_symbol("─")
        .empty_symbol("┄")
        .handle_symbol("●")
        .filled_color(terminal::adapt_color(color))
        .empty_color(terminal::adapt_color(Color::DarkGray))
        .handle_color(terminal::adapt_color(Color::White))
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
        level_slider(ratio, color).render(sections[1], buffer);
    } else {
        meter_slider(ratio, color).render(sections[1], buffer);
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
    Paragraph::new(Line::from(vec![Span::styled(
        format!("LEVEL {:>3} dB", output.display_db()),
        strong_style(Color::White),
    )]))
    .render(rows[1], buffer);
    render_labeled_slider(
        rows[2],
        buffer,
        &signal_slider_label("LVL", Some(format!("{} dB", output.display_db()))),
        Some(output.gain_ratio()),
        Color::LightGreen,
        true,
    );
    Paragraph::new(Line::from(vec![
        chip("-", Color::Black, Color::Gray),
        Span::raw(" "),
        chip("+", Color::Black, Color::Gray),
        Span::raw(" "),
        chip("DIM", Color::Black, dim_bg),
        Span::raw(" "),
        chip("MUTE", Color::Black, mute_bg),
    ]))
    .render(rows[3], buffer);
}

fn preamp_slider_label(input: PreampInputState) -> String {
    let phantom = preamp_phantom_label(input);
    let phase = preamp_phase_label(input);
    format!(
        "GAIN {}  {}  48V:{}  PH:{}  {:02x}",
        input.gain_db_label(),
        input.mode.label(),
        phantom,
        phase,
        input.gain_raw,
    )
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

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(inner);
    Paragraph::new(Line::from(vec![Span::styled(
        preamp_slider_label(input),
        strong_style(style_for_preamp_mode(input.mode)),
    )]))
    .render(rows[0], buffer);
    render_stacked_signal_rows(
        rows[1],
        buffer,
        &meter_slider_label("OBS", input.observed_meter_db()),
        input.observed_meter_ratio(),
        &signal_slider_label("GAIN", Some(input.gain_db_label())),
        Some(input.gain_ratio()),
        style_for_preamp_mode(input.mode),
    );
}

fn mixer_controls_line(channel: &crate::protocol::MixerChannelState) -> Line<'static> {
    let solo_on = channel.soloed == Some(true);
    let mute_on = channel.muted == Some(true);
    let link_on = channel.linked == Some(true);
    let mut controls = vec![
        chip(
            "S",
            Color::Black,
            if solo_on {
                Color::LightGreen
            } else {
                Color::DarkGray
            },
        ),
        Span::raw(" "),
        chip(
            "M",
            Color::Black,
            if mute_on {
                Color::LightRed
            } else {
                Color::DarkGray
            },
        ),
    ];
    if channel.channel % 2 == 1 {
        controls.push(Span::raw(" "));
        controls.push(chip(
            "L",
            Color::Black,
            if link_on {
                Color::LightBlue
            } else {
                Color::DarkGray
            },
        ));
    }
    controls.push(Span::raw(" "));
    controls.push(chip("SRC", Color::Black, Color::Gray));
    Line::from(controls)
}

fn mixer_controls_width(has_link: bool) -> u16 {
    let rects = if has_link {
        inline_chip_rects(0, 0, &["S", "M", "L", "SRC"])
    } else {
        inline_chip_rects(0, 0, &["S", "M", "SRC"])
    };
    rects.last().map(|rect| rect.x + rect.width).unwrap_or(0)
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
    let header = Line::from(vec![
        chip(
            format!("CH {:02}", channel.channel),
            Color::Black,
            if selected {
                Color::LightGreen
            } else {
                Color::Gray
            },
        ),
        Span::raw(" "),
        Span::styled(truncate_label(&source, 18), strong_style(Color::LightCyan)),
        Span::raw(" "),
        chip(format!("PAN {pan_label}"), Color::Black, Color::LightBlue),
        Span::raw(" "),
        chip(
            channel
                .display_db()
                .map(|value| format!("{} dB", value))
                .unwrap_or_else(|| "LEVEL ?".to_string()),
            Color::Black,
            Color::Yellow,
        ),
    ]);
    Paragraph::new(header).render(Rect::new(area.x, area.y, area.width, 1), buffer);

    let content_area = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    );
    let controls_width = mixer_controls_width(channel.channel % 2 == 1).min(content_area.width);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(controls_width), Constraint::Min(1)])
        .split(content_area);
    Paragraph::new(mixer_controls_line(channel)).render(columns[0], buffer);

    if columns[1].width == 0 || columns[1].height < 2 {
        return;
    }
    let slider_area = Rect::new(
        columns[1].x.saturating_add(1),
        columns[1].y,
        columns[1].width.saturating_sub(1),
        columns[1].height,
    );
    render_stacked_signal_rows(
        slider_area,
        buffer,
        &meter_slider_label("MTR", channel.meter_db()),
        channel.meter_ratio(),
        &signal_slider_label(
            "LVL",
            channel.display_db().map(|value| format!("{} dB", value)),
        ),
        channel.gain_ratio(),
        Color::Yellow,
    );
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

    frame.render_widget(
        Paragraph::new(render_footer_text(state))
            .block(Block::default().borders(Borders::ALL).title("Help")),
        layout[3],
    );
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

fn truncate_label(label: &str, width: usize) -> String {
    if label.chars().count() <= width {
        return label.to_string();
    }
    label
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>()
        + "~"
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
            Span::styled(
                format!("LEVEL {:>3} dB", output.display_db()),
                strong_style(Color::White),
            ),
            Span::raw("  "),
            Span::styled(
                render_level_bar(output.gain_ratio(), 10),
                strong_style(Color::LightGreen),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            chip("-", Color::Black, Color::Gray),
            Span::raw(" "),
            chip("+", Color::Black, Color::Gray),
            Span::raw(" "),
            chip("DIM", Color::Black, dim_bg),
            Span::raw(" "),
            chip("MUTE", Color::Black, mute_bg),
        ]),
    ])
}

fn render_device_header(state: &AppState) -> Text<'static> {
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
    let metadata_line = if let Some(metadata) = state.device.metadata.as_ref() {
        Line::from(vec![
            chip(
                format!("SN {}", metadata.serial),
                Color::Black,
                Color::LightCyan,
            ),
            Span::raw(" "),
            chip(
                format!("HW {}", metadata.hardware_version),
                Color::Black,
                Color::LightMagenta,
            ),
        ])
    } else {
        Line::from(Span::styled("metadata pending", muted_style()))
    };

    Text::from(vec![
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
        ]),
        metadata_line,
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

pub fn render_footer_text(_state: &AppState) -> String {
    "Tab page | f focus | mouse: raw button, page tabs, preamp buttons, mixer mute/solo/link/src | r raw view | R refresh queries | +/- adjust | m mute/phantom | o solo | d dim | [ ] pan | a assign | l link | 3 preamp mode | p preamp phase | s sample-rate | c clock | 1/2 surface | b baseline | x clear | Raw shows 0x74/0x73/0x83/0x75/0x81 | ? help | q quit".to_string()
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
    Text::from(vec![
        Line::from(vec![
            chip("-", Color::Black, Color::Gray),
            Span::raw(" "),
            chip("+", Color::Black, Color::Gray),
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
        ]),
        Line::from(vec![
            Span::styled("GAIN ", subdued_style()),
            Span::styled(input.gain_db_label(), strong_style(Color::White)),
            Span::raw("  "),
            Span::styled(format!("raw {:02x}", input.gain_raw), muted_style()),
        ]),
    ])
}

fn preamp_controls_paragraph(input: PreampInputState) -> Paragraph<'static> {
    Paragraph::new(render_preamp_controls_text(input))
        .block(panel_block(
            "Controls",
            style_for_preamp_mode(input.mode),
            false,
        ))
        .wrap(Wrap { trim: false })
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
fn render_mixer_strip_item(
    state: &AppState,
    index: usize,
    channel: &crate::protocol::MixerChannelState,
) -> Text<'static> {
    let selected = state.focus == FocusArea::Mixer && state.selected_channel == index;
    let source = channel
        .assignment
        .map(|value| value.label())
        .unwrap_or_else(|| "assignment?".to_string());
    let meter_bar = render_level_bar(channel.meter_ratio().unwrap_or(0.0), 8);
    let level_bar = render_level_bar(channel.gain_ratio().unwrap_or(0.0), 8);
    let pan = channel.pan.display_percent();
    let pan_label = if pan < 0 {
        format!("L{}", pan.unsigned_abs())
    } else if pan > 0 {
        format!("R{}", pan)
    } else {
        "C".to_string()
    };
    let solo_on = channel.soloed == Some(true);
    let mute_on = channel.muted == Some(true);
    let link_on = channel.linked == Some(true);

    let mut header = vec![chip(
        format!("CH {:02}", channel.channel),
        Color::Black,
        if selected {
            Color::LightGreen
        } else {
            Color::Gray
        },
    )];
    header.push(Span::raw(" "));
    header.push(Span::styled(
        truncate_label(&source, 18),
        strong_style(Color::LightCyan),
    ));
    header.push(Span::raw("  "));
    header.push(chip(
        format!("PAN {pan_label}"),
        Color::Black,
        Color::LightBlue,
    ));
    header.push(Span::raw(" "));
    header.push(chip(
        channel
            .display_db()
            .map(|value| format!("{} dB", value))
            .unwrap_or_else(|| "LEVEL ?".to_string()),
        Color::Black,
        Color::Yellow,
    ));

    let mut controls = vec![
        chip(
            "S",
            Color::Black,
            if solo_on {
                Color::LightGreen
            } else {
                Color::DarkGray
            },
        ),
        Span::raw(" "),
        chip(
            "M",
            Color::Black,
            if mute_on {
                Color::LightRed
            } else {
                Color::DarkGray
            },
        ),
    ];
    if channel.channel % 2 == 1 {
        controls.push(Span::raw(" "));
        controls.push(chip(
            "L",
            Color::Black,
            if link_on {
                Color::LightBlue
            } else {
                Color::DarkGray
            },
        ));
    }
    controls.push(Span::raw("  "));
    controls.push(chip("SRC", Color::Black, Color::Gray));
    controls.push(Span::raw("  "));
    controls.push(Span::styled(
        format!("LVL {}", level_bar),
        strong_style(Color::Yellow),
    ));
    controls.push(Span::raw("  "));
    controls.push(Span::styled(
        format!("MTR {}", meter_bar),
        strong_style(Color::LightGreen),
    ));

    Text::from(vec![Line::from(header), Line::from(controls)])
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

    use crate::app::{AppState, FocusArea, MainPage};
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
        assert!(footer.contains("o solo"));
        assert!(footer.contains("d dim"));
        assert!(footer.contains("0x75/0x81"));
        assert!(footer.contains("q quit"));
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
    fn meter_value_labels_reserve_width_and_use_negative_infinity() {
        assert_eq!(format_meter_value_label(Some(0)), "  0 dB");
        assert_eq!(format_meter_value_label(Some(-48)), "-48 dB");
        assert_eq!(format_meter_value_label(None), " -∞ dB");
        assert_eq!(format_meter_value_label(Some(0)).chars().count(), 6);
        assert_eq!(format_meter_value_label(Some(-48)).chars().count(), 6);
        assert_eq!(format_meter_value_label(None).chars().count(), 6);
    }

    #[test]
    fn mixer_strip_widget_stacks_meter_and_level_in_one_signal_area() {
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

        let rendered = render_buffer(Rect::new(0, 0, 72, mixer_strip_height()), |area, buffer| {
            render_mixer_strip_widget(area, buffer, &state, 10, &state.mixer_channels[0][10]);
        });

        assert!(rendered.contains("Computer Play 8"));
        assert!(rendered.contains("MTR -48 dB"));
        assert!(rendered.contains("LVL -16 dB"));
        assert!(rendered.contains("█"));
        assert!(rendered.contains("─"));
        assert!(rendered.contains("●"));
    }

    #[test]
    fn preamp_visual_stacks_observed_meter_and_gain_sliders() {
        let mut input = PreampInputState::from_raw(0x14, 0x10);
        input.observed_meter = Some(0x30);

        let rendered = render_buffer(Rect::new(0, 0, 44, 6), |area, buffer| {
            render_preamp_visual_widget(area, buffer, "CH 1", input, true);
        });

        assert!(rendered.contains("CH 1"));
        assert!(rendered.contains("GAIN 20 dB"));
        assert!(rendered.contains("OBS -48 dB"));
        assert!(rendered.contains("█"));
        assert!(rendered.contains("─"));
        assert!(rendered.contains("●"));
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

        assert!(rendered.contains("MTR  -∞ dB"));
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

        assert!(rendered.contains("1234567890"));
        assert!(rendered.contains("6.6"));
        assert!(!rendered.contains("SURFACE"));
        assert!(!rendered.contains("PAGE"));
        assert!(!rendered.contains("Last"));
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
    fn footer_mentions_assignment_pan_and_link_controls() {
        let footer = render_footer_text(&AppState::default());

        assert!(footer.contains("mouse:"));
        assert!(footer.contains("Tab page"));
        assert!(footer.contains("r raw view"));
        assert!(footer.contains("a assign"));
        assert!(footer.contains("[ ] pan"));
        assert!(footer.contains("l link"));
        assert!(footer.contains("Raw shows 0x74/0x73/0x83/0x75/0x81"));
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
    fn mouse_action_selects_afx_page_tab() {
        let area = Rect::new(0, 0, 120, 50);
        let tabs = page_tab_hit_areas(root_chunks(area)[1]);
        let point = (tabs[1].x + tabs[1].width / 2, tabs[1].y);

        assert_eq!(
            mouse_action(area, &AppState::default(), point.0, point.1),
            Some(MouseAction::SelectPage(MainPage::AfxDsp))
        );
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
        let page = mixer_page_layout(chunks[2]);
        let main = mixer_main_layout(page[1]);
        let workspace = mixer_workspace_layout(main[1]);
        let mixer = mixer_layout(workspace[0]);
        let tabs = surface_tab_hit_areas(mixer[0]);

        assert_eq!(
            mouse_action(area, &AppState::default(), tabs[1].x + 1, tabs[1].y),
            Some(MouseAction::SelectSurface(Surface::Hp2))
        );
    }

    #[test]
    fn mouse_action_hits_visible_output_dim_chip_position() {
        let area = Rect::new(0, 0, 120, 50);
        let page = mixer_page_layout(root_chunks(area)[2]);
        let outputs = output_panel_layout(page[0]);
        let list_inner = inner_area(outputs[0]);
        let row_area = Rect::new(
            list_inner.x,
            list_inner.y,
            list_inner.width,
            output_card_height(),
        );
        let state = AppState::default();
        let line = render_output_card(&state.outputs[0], true);
        let rendered: String = line.lines[3]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let chip_x = rendered.find(" DIM ").expect("dim chip") as u16;

        assert_eq!(
            mouse_action(area, &state, row_area.x + chip_x + 1, row_area.y + 3),
            Some(MouseAction::ToggleOutputDim(0))
        );
    }

    #[test]
    fn mouse_action_hits_visible_output_mute_chip_position_on_hp1() {
        let area = Rect::new(0, 0, 120, 50);
        let page = mixer_page_layout(root_chunks(area)[2]);
        let outputs = output_panel_layout(page[0]);
        let list_inner = inner_area(outputs[0]);
        let row_area = Rect::new(
            list_inner.x,
            list_inner.y + output_card_height(),
            list_inner.width,
            output_card_height(),
        );
        let state = AppState::default();
        let line = render_output_card(&state.outputs[1], false);
        let rendered: String = line.lines[3]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let chip_x = rendered.find(" MUTE ").expect("mute chip") as u16;

        assert_eq!(
            mouse_action(area, &state, row_area.x + chip_x + 1, row_area.y + 3),
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
    fn mouse_action_hits_preamp_gain_plus_button() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[2]);
        let main = mixer_main_layout(page[1]);
        let cards = preamp_bar_layout(main[0]);
        let card = preamp_card_layout(cards[0]);
        let buttons = preamp_button_rects(card[1], AppState::default().preamp.input1);
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
    fn mouse_action_hits_visible_preamp_mode_chip_position() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[2]);
        let main = mixer_main_layout(page[1]);
        let cards = preamp_bar_layout(main[0]);
        let card = preamp_card_layout(cards[0]);
        let state = AppState::default();
        let controls = render_preamp_controls_text(state.preamp.input1);
        let rendered: String = controls.lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let mode_chip = format!(" {} ", state.preamp.input1.mode.label());
        let chip_x = rendered.find(&mode_chip).expect("mode chip") as u16;
        let inner = inner_area(card[1]);

        assert_eq!(
            mouse_action(area, &state, inner.x + chip_x, inner.y),
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
        let inner = inner_area(popup);

        assert_eq!(
            mouse_action(area, &state, inner.x + 1, inner.y + 1),
            Some(MouseAction::PickPreampMode {
                input: 0,
                mode: PreampMode::Line,
            })
        );
    }

    #[test]
    fn preamp_control_row_keeps_leading_chip_padding_when_rendered() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 3));
        preamp_controls_paragraph(AppState::default().preamp.input1)
            .render(Rect::new(0, 0, 40, 3), &mut buffer);

        assert_eq!(buffer[(1, 1)].symbol(), " ");
        assert_eq!(buffer[(2, 1)].symbol(), "-");
    }

    #[test]
    fn mouse_action_hits_mixer_link_button_on_odd_strip() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[2]);
        let main = mixer_main_layout(page[1]);
        let workspace = mixer_workspace_layout(main[1]);
        let mixer = mixer_layout(workspace[0]);
        let list_inner = inner_area(mixer[1]);
        let row_area = Rect::new(
            list_inner.x,
            list_inner.y,
            list_inner.width,
            mixer_strip_height(),
        );
        let buttons = mixer_control_button_rects(row_area, true);
        let point = (buttons[2].x + buttons[2].width / 2, buttons[2].y);

        assert_eq!(
            mouse_action(area, &AppState::default(), point.0, point.1),
            Some(MouseAction::ToggleMixerLink(1))
        );
    }

    #[test]
    fn mouse_action_hits_mixer_solo_button() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[2]);
        let main = mixer_main_layout(page[1]);
        let workspace = mixer_workspace_layout(main[1]);
        let mixer = mixer_layout(workspace[0]);
        let list_inner = inner_area(mixer[1]);
        let row_area = Rect::new(
            list_inner.x,
            list_inner.y,
            list_inner.width,
            mixer_strip_height(),
        );
        let buttons = mixer_control_button_rects(row_area, true);
        let point = (buttons[0].x + buttons[0].width / 2, buttons[0].y);

        assert_eq!(
            mouse_action(area, &AppState::default(), point.0, point.1),
            Some(MouseAction::ToggleMixerSolo(1))
        );
    }

    #[test]
    fn mouse_action_hits_visible_mixer_solo_chip_position() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[2]);
        let main = mixer_main_layout(page[1]);
        let workspace = mixer_workspace_layout(main[1]);
        let mixer = mixer_layout(workspace[0]);
        let list_inner = inner_area(mixer[1]);
        let row_area = Rect::new(
            list_inner.x,
            list_inner.y,
            list_inner.width,
            mixer_strip_height(),
        );
        let state = AppState::default();
        let line = render_mixer_strip_item(&state, 0, &state.mixer_channels[0][0]);
        let rendered: String = line.lines[1]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let chip_x = rendered.find(" S ").expect("solo chip") as u16;

        assert_eq!(
            mouse_action(area, &state, row_area.x + chip_x + 1, row_area.y + 1),
            Some(MouseAction::ToggleMixerSolo(1))
        );
    }

    #[test]
    fn mouse_action_opens_assignment_picker_from_src_button() {
        let area = Rect::new(0, 0, 120, 60);
        let chunks = root_chunks(area);
        let page = mixer_page_layout(chunks[2]);
        let main = mixer_main_layout(page[1]);
        let workspace = mixer_workspace_layout(main[1]);
        let mixer = mixer_layout(workspace[0]);
        let list_inner = inner_area(mixer[1]);
        let row_area = Rect::new(
            list_inner.x,
            list_inner.y + 10 * mixer_strip_height(),
            list_inner.width,
            mixer_strip_height(),
        );
        let mut state = AppState::default();
        state.selected_channel = 10;
        let line = render_mixer_strip_item(&state, 10, &state.mixer_channels[0][10]);
        let rendered: String = line.lines[1]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let chip_x = rendered.find(" SRC ").expect("src chip") as u16;

        assert_eq!(
            mouse_action(area, &state, row_area.x + chip_x + 1, row_area.y + 1),
            Some(MouseAction::OpenAssignmentPicker(11))
        );
    }

    #[test]
    fn mouse_action_picks_assignment_from_modal() {
        let area = Rect::new(0, 0, 120, 50);
        let popup = assignment_picker_area(area);
        let inner = inner_area(popup);
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
