use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Clear, Gauge, LineGauge, List, ListItem, Paragraph, Tabs, Wrap,
};
use ratatui::Frame;

use crate::app::{AppState, AssignmentPickerState, FocusArea, RawPacketTab};
use crate::protocol::{
    MixerAssignment, MixerSurface, OutputMode, PreampInputState, PreampMode, Surface,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    ToggleRawView,
    SelectRawPacketTab(RawPacketTab),
    SelectQueryReplyEntry(usize),
    SelectSurface(Surface),
    SelectMixerChannel(usize),
    ToggleMixerMute(u8),
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
    CyclePreampMode(u8),
    TogglePreampPhase(u8),
    TogglePreampPhantom(u8),
}

pub fn draw(frame: &mut Frame<'_>, state: &AppState) {
    if state.raw_view_open {
        draw_raw_page(frame, frame.area(), state);
        return;
    }

    let chunks = root_chunks(frame.area());

    draw_status(frame, chunks[0], state);
    draw_outputs(frame, chunks[1], state);
    draw_mixer_and_preamp(frame, chunks[2], state);
    draw_footer(frame, chunks[3], state);
    draw_assignment_picker(frame, frame.area(), state);
}

fn root_chunks(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Min(14),
            Constraint::Length(3),
        ])
        .split(area)
        .to_vec()
}

fn mixer_preamp_sections(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area)
        .to_vec()
}

fn mixer_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(9),
        ])
        .split(area)
        .to_vec()
}

fn preamp_panel_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(4),
            Constraint::Min(3),
        ])
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

fn preamp_button_rects(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Min(12),
        ])
        .split(area)
        .to_vec()
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

fn status_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(18)])
        .split(area)
        .to_vec()
}

fn raw_header_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(18)])
        .split(area)
        .to_vec()
}

fn raw_tab_hit_areas(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Min(0),
        ])
        .split(inner_area(area))
        .iter()
        .copied()
        .take(5)
        .collect()
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

fn draw_status(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let sections = status_layout(area);
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
    frame.render_widget(text, sections[0]);
    frame.render_widget(
        Paragraph::new(if state.raw_view_open {
            "Back To Main"
        } else {
            "Open Raw View"
        })
        .block(Block::default().borders(Borders::ALL).title("View"))
        .wrap(Wrap { trim: true }),
        sections[1],
    );
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
    let sections = mixer_preamp_sections(area);

    let titles = ["MIX 1 / Monitor-HP1", "MIX 2 / HP2"];
    let active = match MixerSurface::from_surface(state.surface) {
        MixerSurface::Mix1 => 0,
        MixerSurface::Mix2 => 1,
    };
    let mixer_layout = mixer_layout(sections[0]);

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
            .block(Block::default().borders(Borders::ALL).title("Mix Meter"))
            .wrap(Wrap { trim: true }),
        mixer_layout[1],
    );

    let items: Vec<ListItem<'_>> = state
        .active_mixer_channels()
        .iter()
        .enumerate()
        .map(|(index, channel)| ListItem::new(render_mixer_strip_item(state, index, channel)))
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
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Assign CH {:02}", picker.strip)),
        ),
        popup,
    );
}

pub fn mouse_action(area: Rect, state: &AppState, x: u16, y: u16) -> Option<MouseAction> {
    let point = (x, y);

    if contains_point(status_layout(root_chunks(area)[0])[1], point) {
        return Some(MouseAction::ToggleRawView);
    }

    if state.raw_view_open {
        return raw_mouse_action(area, state, point);
    }

    if let Some(picker) = state.assignment_picker {
        return assignment_picker_mouse_action(area, picker, point);
    }

    let chunks = root_chunks(area);
    let main_sections = mixer_preamp_sections(chunks[2]);
    let mixer_sections = mixer_layout(main_sections[0]);

    if let Some(action) = mixer_tab_mouse_action(mixer_sections[0], point) {
        return Some(action);
    }
    if let Some(action) = mixer_list_mouse_action(mixer_sections[2], state, point) {
        return Some(action);
    }
    if let Some(action) = preamp_mouse_action(main_sections[1], point) {
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

fn mixer_tab_mouse_action(area: Rect, point: (u16, u16)) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    let inner = inner_area(area);
    let tabs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);
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
    let row = point.1.saturating_sub(inner.y) / 2;
    let index = row as usize;
    let channel = state.active_mixer_channels().get(index)?.channel;
    let row_area = Rect {
        x: inner.x,
        y: inner.y + row * 2,
        width: inner.width,
        height: 2,
    };
    if point.1 == row_area.y {
        return Some(MouseAction::SelectMixerChannel(index));
    }

    let controls = mixer_control_button_rects(row_area, channel % 2 == 1);
    if contains_point(controls[0], point) {
        return Some(MouseAction::ToggleMixerMute(channel));
    }
    if channel % 2 == 1 && contains_point(controls[1], point) {
        return Some(MouseAction::ToggleMixerLink(channel));
    }
    let src_rect = if channel % 2 == 1 {
        controls[2]
    } else {
        controls[1]
    };
    if contains_point(src_rect, point) {
        return Some(MouseAction::OpenAssignmentPicker(channel));
    }

    Some(MouseAction::SelectMixerChannel(index))
}

fn preamp_mouse_action(area: Rect, point: (u16, u16)) -> Option<MouseAction> {
    if !contains_point(area, point) {
        return None;
    }
    let layout = preamp_panel_layout(area);
    for (input, card) in [layout[0], layout[1]].into_iter().enumerate() {
        if !contains_point(card, point) {
            continue;
        }
        let parts = preamp_card_layout(card);
        let buttons = preamp_button_rects(parts[1]);
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
            return Some(MouseAction::CyclePreampMode(input as u8));
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

fn mixer_control_button_rects(area: Rect, has_link: bool) -> Vec<Rect> {
    let controls = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: 1,
    };
    if has_link {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(14),
                Constraint::Length(14),
                Constraint::Min(18),
            ])
            .split(controls)
            .to_vec()
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(14), Constraint::Min(18)])
            .split(controls)
            .to_vec()
    }
}

fn contains_point(area: Rect, point: (u16, u16)) -> bool {
    point.0 >= area.x
        && point.0 < area.x.saturating_add(area.width)
        && point.1 >= area.y
        && point.1 < area.y.saturating_add(area.height)
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

    let tabs = Tabs::new(vec![
        Line::from("0x74"),
        Line::from("0x73"),
        Line::from("0x83"),
        Line::from("0x75"),
        Line::from("0x81"),
    ])
    .block(section_block("Packet Tabs", true))
    .select(match state.selected_raw_packet {
        RawPacketTab::Query74 => 0,
        RawPacketTab::State73 => 1,
        RawPacketTab::Auxiliary83 => 2,
        RawPacketTab::Query75 => 3,
        RawPacketTab::Notification81 => 4,
    })
    .highlight_style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(tabs, layout[1]);

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
    "Tab focus | mouse: preamp buttons, mixer mute/link/src, tabs | r raw view | R refresh queries | +/- adjust | m mute/phantom | d dim | [ ] pan | a assign | l link | 3 preamp mode | p preamp phase | s sample-rate | c clock | 1/2 surface | b baseline | x clear | Raw shows 0x74/0x73/0x83/0x75/0x81 | ? help | q quit".to_string()
}

fn render_preamp_controls_text(input: PreampInputState) -> Text<'static> {
    let phantom = if matches!(input.mode, PreampMode::Mic) {
        if input.phantom_on {
            "[48V on]"
        } else {
            "[48V off]"
        }
    } else {
        "[48V n/a]"
    };
    let phase = if input.mode_raw & 0x40 != 0 {
        "[Phase inv]"
    } else {
        "[Phase norm]"
    };
    Text::from(vec![Line::from(format!(
        "[-] [+] [Mode {}] {} {}",
        input.mode.label(),
        phantom,
        phase
    ))])
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

fn render_mixer_strip_item(
    state: &AppState,
    index: usize,
    channel: &crate::protocol::MixerChannelState,
) -> Text<'static> {
    Text::from(vec![
        Line::from(render_mixer_strip_line(state, index, channel)),
        Line::from(render_mixer_strip_controls(state, index, channel)),
    ])
}

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
    let link = if channel.channel % 2 == 1 {
        let value = channel
            .linked
            .map(|flag| if flag { "on" } else { "off" })
            .unwrap_or("?");
        format!(" [Link {}]", value)
    } else {
        String::new()
    };
    format!("    [Mute {}]{} [Src {}]", mute, link, src)
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
                render_mix_meter_bar(lane_a),
                render_mix_meter_bar(lane_b),
            )
        }
        Some(0x0c) => {
            let lane_a = payload.get(0xde).copied().unwrap_or(0);
            let lane_b = payload.get(0xdf).copied().unwrap_or(0);
            format!(
                "MIX 2 L {} R {}",
                render_mix_meter_bar(lane_a),
                render_mix_meter_bar(lane_b),
            )
        }
        Some(surface) => format!("exp pair pending: unsupported surface {:02x}", surface),
        None => "exp pair pending: missing surface byte".to_string(),
    }
}

fn render_mix_meter_bar(raw: u8) -> String {
    let ratio = 1.0 - (raw.min(0x60) as f64 / 96.0);
    render_thin_bar(ratio)
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
    let layout = preamp_panel_layout(area);

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

    let input1_layout = preamp_card_layout(layout[0]);
    let input2_layout = preamp_card_layout(layout[1]);

    frame.render_widget(
        render_preamp_gauge(
            input1_title,
            state.preamp.input1,
            state.focus == FocusArea::Preamp && state.selected_preamp_input == 0,
        ),
        input1_layout[0],
    );
    frame.render_widget(
        Paragraph::new(render_preamp_controls_text(state.preamp.input1))
            .block(Block::default().borders(Borders::ALL).title("Controls"))
            .wrap(Wrap { trim: true }),
        input1_layout[1],
    );
    frame.render_widget(
        render_preamp_gauge(
            input2_title,
            state.preamp.input2,
            state.focus == FocusArea::Preamp && state.selected_preamp_input == 1,
        ),
        input2_layout[0],
    );
    frame.render_widget(
        Paragraph::new(render_preamp_controls_text(state.preamp.input2))
            .block(Block::default().borders(Borders::ALL).title("Controls"))
            .wrap(Wrap { trim: true }),
        input2_layout[1],
    );
    if let Some(area) = inner_bottom_line(input1_layout[0]) {
        frame.render_widget(render_preamp_observed_meter(state.preamp.input1), area);
    }
    if let Some(area) = inner_bottom_line(input2_layout[0]) {
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

        assert!(footer.contains("mouse:"));
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
        let button = status_layout(root_chunks(area)[0])[1];
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
        let sections = mixer_preamp_sections(chunks[2]);
        let preamp = preamp_panel_layout(sections[1]);
        let card = preamp_card_layout(preamp[0]);
        let buttons = preamp_button_rects(card[1]);
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
    fn mouse_action_hits_mixer_link_button_on_odd_strip() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let sections = mixer_preamp_sections(chunks[2]);
        let mixer = mixer_layout(sections[0]);
        let list_inner = inner_area(mixer[2]);
        let row_area = Rect::new(list_inner.x, list_inner.y, list_inner.width, 2);
        let buttons = mixer_control_button_rects(row_area, true);
        let point = (buttons[1].x + buttons[1].width / 2, buttons[1].y);

        assert_eq!(
            mouse_action(area, &AppState::default(), point.0, point.1),
            Some(MouseAction::ToggleMixerLink(1))
        );
    }

    #[test]
    fn mouse_action_opens_assignment_picker_from_src_button() {
        let area = Rect::new(0, 0, 120, 50);
        let chunks = root_chunks(area);
        let sections = mixer_preamp_sections(chunks[2]);
        let mixer = mixer_layout(sections[0]);
        let list_inner = inner_area(mixer[2]);
        let row_area = Rect::new(list_inner.x, list_inner.y + 20, list_inner.width, 2);
        let buttons = mixer_control_button_rects(row_area, false);
        let point = (buttons[1].x + buttons[1].width / 2, buttons[1].y);
        let mut state = AppState::default();
        state.selected_channel = 10;

        assert_eq!(
            mouse_action(area, &state, point.0, point.1),
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
        assert!(line.contains("L |||||||."));
        assert!(line.contains("R ||||||||"));
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
        assert!(line.contains("L ||||||||"));
        assert!(line.contains("R ||||||||"));
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
        assert!(line.contains("L |......."));
        assert!(line.contains("R |......."));
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

        assert!(line.contains("L |||||||."));
        assert!(line.contains("R ||||...."));
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
