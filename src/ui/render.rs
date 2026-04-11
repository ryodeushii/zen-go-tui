use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget, Wrap};
use ratatui::Frame;
use tui_slider::{Slider, SliderOrientation};

use crate::app::{
    AppState, FocusArea, ProfileEditorMode, RawPacketTab, RefreshRate, SelectorPopupKind,
    QUERY_REPLY_VISIBLE_COUNT,
};
use crate::terminal;
use antelope_protocol::{
    meter_display_db, meter_ratio, ClockSource, MixerAssignment, MixerSurface, OutputMode,
    OutputState, PreampInputState, PreampMode, SampleRate, OFFSET_MIX1_LANE_A, OFFSET_MIX1_LANE_B,
    OFFSET_MIX2_LANE_A, OFFSET_MIX2_LANE_B, OFFSET_SURFACE_SELECTOR, SNAPSHOT_PAYLOAD_OFFSET,
    SURFACE_CODE_HP2, SURFACE_CODE_MONITOR_HP1,
};

use super::layouts::*;
use super::mouse::mix_meter;
use super::styles::*;

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
    draw_profiles_popup(frame, frame.area(), state);
    draw_assignment_picker(frame, frame.area(), state);
    draw_selector_popup(frame, frame.area(), state);
    draw_hotkeys_popup(frame, frame.area(), state);
    draw_options_popup(frame, frame.area(), state);
}

pub fn profile_editor_cursor(area: Rect, state: &AppState) -> Option<(u16, u16)> {
    let editor = state.profile_editor.as_ref()?;
    let editor_area = profile_editor_area(area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner_area(editor_area));
    let row = rows[0];
    if row.width == 0 {
        return None;
    }

    let x = row.x.saturating_add(
        editor
            .value
            .chars()
            .count()
            .min(row.width.saturating_sub(1) as usize) as u16,
    );
    Some((x, row.y))
}

fn draw_titlebar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let sections = titlebar_layout(area);
    frame.render_widget(panel_block("Device", Color::DarkGray, true), sections[0]);
    let device_sections = device_panel_layout(sections[0], state);
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
        Paragraph::new(render_system_summary(state))
            .block(panel_block("System", Color::LightRed, false))
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

fn draw_profiles_popup(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if !state.profiles_popup_open {
        return;
    }

    let popup = profiles_popup_area(area);
    frame.render_widget(Clear, popup);
    frame.render_widget(panel_block("Profiles", Color::LightGreen, true), popup);

    let sections = profiles_popup_layout(popup);
    if state.profile_names.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            "No saved profiles yet.",
            muted_style(),
        )))
        .render(sections[0], frame.buffer_mut());
    } else {
        let items = state
            .profile_names
            .iter()
            .map(|name| ListItem::new(name.clone()))
            .collect::<Vec<_>>();
        let mut list_state = ListState::default();
        list_state.select(Some(
            state
                .popup_selected_index
                .min(items.len().saturating_sub(1)),
        ));
        frame.render_stateful_widget(
            List::new(items).highlight_style(terminal::adapt_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            )),
            sections[0],
            &mut list_state,
        );
    }

    let has_selection = state.selected_profile_name().is_some();
    let button_rects = profiles_popup_button_rects(popup);
    let button_specs = [
        ("LOAD", has_selection, Color::LightCyan),
        ("SAVE", true, Color::LightGreen),
        ("RENAME", has_selection, Color::Yellow),
        ("DELETE", has_selection, Color::LightRed),
        ("CLOSE", true, Color::Gray),
    ];
    for (rect, (label, enabled, color)) in button_rects.into_iter().zip(button_specs) {
        Paragraph::new(Line::from(vec![chip(
            label,
            Color::Black,
            if enabled { color } else { Color::DarkGray },
        )]))
        .render(rect, frame.buffer_mut());
    }

    Paragraph::new(Line::from(vec![
        Span::styled("ENTER ", subdued_style()),
        Span::styled("load selected", muted_style()),
        Span::raw("   "),
        Span::styled("s/r/d ", subdued_style()),
        Span::styled("save, rename, delete", muted_style()),
    ]))
    .wrap(Wrap { trim: false })
    .render(sections[2], frame.buffer_mut());

    if let Some(editor) = state.profile_editor.as_ref() {
        let editor_area = profile_editor_area(area);
        frame.render_widget(Clear, editor_area);
        frame.render_widget(
            panel_block(
                match editor.mode {
                    ProfileEditorMode::Save => "Save Profile",
                    ProfileEditorMode::Rename => "Rename Profile",
                },
                Color::Yellow,
                true,
            ),
            editor_area,
        );
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner_area(editor_area));
        Paragraph::new(Line::from(Span::styled(
            if editor.value.is_empty() {
                "profile name"
            } else {
                &editor.value
            },
            strong_style(Color::LightYellow),
        )))
        .render(rows[0], frame.buffer_mut());
        Paragraph::new(Line::from(Span::styled(
            "letters, digits, - and _",
            muted_style(),
        )))
        .render(rows[1], frame.buffer_mut());
        Paragraph::new(Line::from(vec![
            Span::styled("ENTER ", subdued_style()),
            Span::styled("confirm", muted_style()),
            Span::raw("   "),
            Span::styled("ESC ", subdued_style()),
            Span::styled("cancel", muted_style()),
        ]))
        .render(rows[2], frame.buffer_mut());
    }
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
            state.preamp_peaks[index].as_ref().map(|p| p.raw),
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
    let header_buttons = mixer_header_button_rects(layout[0]);
    Paragraph::new(Line::from(vec![chip(
        "PROFILES",
        Color::Black,
        if state.profiles_popup_open {
            Color::Yellow
        } else {
            Color::LightGreen
        },
    )]))
    .render(header_buttons[0], frame.buffer_mut());
    Paragraph::new(Line::from(vec![chip(
        "ROUTING",
        Color::Black,
        if state.routing_popup_open {
            Color::Yellow
        } else {
            Color::LightMagenta
        },
    )]))
    .render(header_buttons[1], frame.buffer_mut());

    let content = mixer_strip_panel_layout(layout[1], mix_meter(state).is_some());
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
    let page_buttons = mixer_strip_page_button_rects(layout[1]);
    let visible = visible_end.saturating_sub(visible_start);
    let can_page_left = state.mixer_strip_scroll > 0;
    let can_page_right = state.mixer_strip_scroll + visible < total;
    Paragraph::new(Line::from(vec![chip(
        "←",
        Color::Black,
        if can_page_left {
            Color::LightBlue
        } else {
            Color::DarkGray
        },
    )]))
    .render(page_buttons[0], frame.buffer_mut());
    Paragraph::new(Line::from(vec![chip(
        "→",
        Color::Black,
        if can_page_right {
            Color::LightBlue
        } else {
            Color::DarkGray
        },
    )]))
    .render(page_buttons[1], frame.buffer_mut());

    for (slot, (index, channel)) in state
        .active_mixer_channels()
        .iter()
        .enumerate()
        .skip(visible_start)
        .take(visible_end.saturating_sub(visible_start))
        .enumerate()
    {
        let card = mixer_strip_card_area(inner, slot);
        if card.x >= inner.x + inner.width || card.x + card.width > inner.x + inner.width {
            break;
        }
        render_mixer_strip_widget(card, frame.buffer_mut(), state, index, channel);
    }

    if let Some((_, left_raw, right_raw)) = mix_meter(state) {
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

fn draw_options_popup(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if !state.options_popup_open {
        return;
    }

    let popup = options_popup_area(area);
    frame.render_widget(Clear, popup);
    frame.render_widget(panel_block("Options", Color::Cyan, true), popup);

    let rows = options_popup_layout(popup);
    Paragraph::new(Line::from(vec![chip("OPTIONS", Color::Black, Color::Cyan)]))
        .render(rows[0], frame.buffer_mut());

    let refresh_rates = RefreshRate::all();
    let current_refresh = state.settings.refresh_rate;
    let mut refresh_spans = vec![Span::styled("Refresh: ", subdued_style())];
    for r in refresh_rates {
        if *r == current_refresh {
            refresh_spans.push(chip(
                format!("* {}", r.label()),
                Color::Black,
                Color::LightCyan,
            ));
        } else {
            refresh_spans.push(chip(r.label(), Color::Black, Color::Gray));
        }
        refresh_spans.push(Span::raw(" "));
    }
    Paragraph::new(Line::from(refresh_spans)).render(rows[1], frame.buffer_mut());

    let peak_db = state.settings.peak_threshold_db();
    let peak_status = if state.settings.peak_enabled {
        format!("ON ({} dB)", peak_db)
    } else {
        "OFF".to_string()
    };
    let peak_color = if state.settings.peak_enabled {
        Color::LightGreen
    } else {
        Color::DarkGray
    };
    Paragraph::new(Line::from(vec![
        Span::styled("Peaks:  ", subdued_style()),
        chip(&peak_status, Color::Black, peak_color),
        Span::raw("  "),
        chip("↓", Color::Black, Color::Gray),
        Span::raw(" "),
        chip("↑", Color::Black, Color::Gray),
    ]))
    .render(rows[2], frame.buffer_mut());

    Paragraph::new(Line::from(vec![
        Span::styled("Toggle: ", subdued_style()),
        chip(
            if state.settings.peak_enabled {
                "Disable"
            } else {
                "Enable"
            },
            Color::Black,
            Color::Yellow,
        ),
        Span::raw("  "),
        Span::styled("p ", subdued_style()),
        Span::styled("toggle", muted_style()),
    ]))
    .render(rows[3], frame.buffer_mut());

    let hold_durations = crate::app::PeakHoldDuration::all();
    let current_hold = state.settings.peak_hold_duration;
    let mut hold_spans = vec![Span::styled("Hold:   ", subdued_style())];
    for h in hold_durations {
        if *h == current_hold {
            hold_spans.push(chip(
                format!("* {}", h.label()),
                Color::Black,
                Color::LightCyan,
            ));
        } else {
            hold_spans.push(chip(h.label(), Color::Black, Color::Gray));
        }
        hold_spans.push(Span::raw(" "));
    }
    Paragraph::new(Line::from(hold_spans)).render(rows[4], frame.buffer_mut());

    let auto_save_status = if state.settings.auto_save {
        "ON"
    } else {
        "OFF"
    };
    let auto_save_color = if state.settings.auto_save {
        Color::LightGreen
    } else {
        Color::DarkGray
    };
    Paragraph::new(Line::from(vec![
        Span::styled("Auto-save: ", subdued_style()),
        chip(auto_save_status, Color::Black, auto_save_color),
        Span::raw("  "),
        Span::styled("a ", subdued_style()),
        Span::styled("toggle", muted_style()),
    ]))
    .render(rows[5], frame.buffer_mut());

    let button_rects = options_popup_button_rects(popup);
    Paragraph::new(Line::from(vec![chip("CLOSE", Color::Black, Color::Gray)]))
        .render(button_rects[0], frame.buffer_mut());

    Paragraph::new(Line::from(vec![
        Span::styled("ESC ", subdued_style()),
        Span::styled("close", muted_style()),
        Span::raw("   "),
        Span::styled("1/2/3 ", subdued_style()),
        Span::styled("refresh", muted_style()),
        Span::raw("   "),
        Span::styled("↑/↓ ", subdued_style()),
        Span::styled("threshold", muted_style()),
        Span::raw("   "),
        Span::styled("p ", subdued_style()),
        Span::styled("peaks", muted_style()),
        Span::raw("   "),
        Span::styled("h/l ", subdued_style()),
        Span::styled("hold", muted_style()),
        Span::raw("   "),
        Span::styled("a ", subdued_style()),
        Span::styled("auto-save", muted_style()),
    ]))
    .wrap(Wrap { trim: false })
    .render(rows[7], frame.buffer_mut());
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
            selected == RawPacketTab::Auxiliary,
            Color::LightBlue,
        ),
        Span::raw(" "),
        tab_chip("0x75", selected == RawPacketTab::Query75, Color::LightGreen),
        Span::raw(" "),
        tab_chip(
            "0x81",
            selected == RawPacketTab::DeviceNotification,
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
        RawPacketTab::Auxiliary => (
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
        RawPacketTab::DeviceNotification => (
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
        let list_items = build_query_reply_list_items(state);
        frame.render_widget(
            List::new(list_items)
                .block(section_block("Recent 0x75 Replies", true))
                .highlight_style(strong_style(Color::LightCyan)),
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

fn render_afx_routing_row(area: Rect, buffer: &mut Buffer, state: &AppState, pair: usize) {
    let labels = afx_routing_row_labels(state, pair);
    let (left_index, right_index) = afx_routing_pair_channels(pair);
    let selected_left = state.focus == FocusArea::Mixer && state.selected_channel == left_index;
    let selected_right = state.focus == FocusArea::Mixer && state.selected_channel == right_index;
    let columns = afx_routing_row_columns(area);
    let row_style = terminal::adapt_style(Style::default().fg(if pair.is_multiple_of(2) {
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
pub(crate) fn afx_routing_source_label(assignment: Option<MixerAssignment>) -> String {
    assignment
        .map(|assignment| assignment.label())
        .unwrap_or_else(|| "?".to_string())
}

#[cfg(test)]
pub(crate) fn render_afx_routing_text(state: &AppState) -> Text<'static> {
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
    let peak_raw = state
        .mixer_peaks
        .get(state.active_mixer_surface().index())
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

pub(crate) fn render_query_reply_panel(_state_bytes: &[u8], state: &AppState) -> Text<'static> {
    state
        .selected_query_reply_entry()
        .map(|entry| render_full_packet_dump(&entry.raw, state.baseline_raw_75.as_deref()))
        .unwrap_or_else(|| Text::from("No 0x75 reply selected yet."))
}

pub(crate) fn render_query_request_panel(state_bytes: &[u8], state: &AppState) -> Text<'static> {
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

pub(crate) fn build_query_reply_list_items(state: &AppState) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    if state.recent_query_reply_entries.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "Waiting for first 0x75 query reply...",
            muted_style(),
        ))));
        return items;
    }
    let total = state.recent_query_reply_entries.len();
    let visible = QUERY_REPLY_VISIBLE_COUNT.min(total);
    let start = state.query_reply_scroll.min(total.saturating_sub(visible));
    let end = (start + visible).min(total);
    for rev_index in start..end {
        let index = total - 1 - rev_index;
        let entry = &state.recent_query_reply_entries[index];
        let marker = if state.selected_query_reply_entry == Some(index) {
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

#[cfg(test)]
pub(crate) fn render_mixer_strip_controls(
    _state: &AppState,
    _index: usize,
    channel: &antelope_protocol::MixerChannelState,
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

pub(crate) fn render_experimental_pair_state_line(state: &AppState) -> String {
    let Some(bytes) = state.latest_raw_73.as_deref() else {
        return "exp pair pending: waiting for 0x73 snapshot".to_string();
    };
    let Some(payload) = bytes.get(SNAPSHOT_PAYLOAD_OFFSET..) else {
        return "exp pair pending: short 0x73 snapshot".to_string();
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
        Some(surface) => format!("exp pair pending: unsupported surface {:02x}", surface),
        None => "exp pair pending: missing surface byte".to_string(),
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

#[cfg(test)]
pub(crate) fn render_mixer_strip_line(
    state: &AppState,
    index: usize,
    channel: &antelope_protocol::MixerChannelState,
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
pub(crate) fn observed_meter_label(input: PreampInputState) -> String {
    match input.observed_meter {
        Some(_) => input
            .observed_meter_db()
            .map(|value| format!("obs meter {} dB", value))
            .unwrap_or_default(),
        None => String::new(),
    }
}

pub(crate) fn render_device_header(state: &AppState) -> Line<'static> {
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

pub(crate) fn render_device_metadata(state: &AppState) -> Line<'static> {
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

pub(crate) fn render_system_summary(state: &AppState) -> Line<'static> {
    let raw_color = if state.raw_view_open {
        Color::Yellow
    } else {
        Color::LightRed
    };
    let options_color = if state.options_popup_open {
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

pub(crate) use super::layouts::current_sample_rate_label;

pub(crate) fn render_status_strip(state: &AppState) -> Line<'static> {
    Line::from(Span::styled(
        render_experimental_pair_state_line(state),
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
