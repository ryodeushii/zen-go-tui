use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget, Wrap};
use ratatui::Frame;

use crate::app::{
    AppState, FocusArea, ProfileEditorMode, RawPacketTab, RefreshRate, SelectorPopupKind,
    QUERY_REPLY_VISIBLE_COUNT,
};
use crate::terminal;
#[cfg(test)]
use antelope_protocol::PreampInputState;
use antelope_protocol::{
    ClockSource, MixerAssignment, MixerSurface, PreampMode, SampleRate, OFFSET_MIX1_LANE_A,
    OFFSET_MIX1_LANE_B, OFFSET_MIX2_LANE_A, OFFSET_MIX2_LANE_B, OFFSET_SURFACE_SELECTOR,
    SNAPSHOT_PAYLOAD_OFFSET, SURFACE_CODE_HP2, SURFACE_CODE_MONITOR_HP1,
};

use super::layouts::*;
use super::mouse::mix_meter;
use super::styles::*;
use super::widgets::mixer;
use super::widgets::signals;

// Re-export widget functions for tests and other modules
pub(crate) use mixer::*;
pub(crate) use signals::*;

pub fn draw(frame: &mut Frame<'_>, state: &AppState) {
    if state.popup.raw_view_open {
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
    let editor = state.popup.profile_editor.as_ref()?;
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
    if !state.popup.routing_open {
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
        Span::styled(&state.ui.last_message, strong_style(Color::LightCyan)),
    ]))
    .wrap(Wrap { trim: false })
    .render(rows[9], frame.buffer_mut());
}

fn draw_profiles_popup(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if !state.popup.profiles_open {
        return;
    }

    let popup = profiles_popup_area(area);
    frame.render_widget(Clear, popup);
    frame.render_widget(panel_block("Profiles", Color::LightGreen, true), popup);

    let sections = profiles_popup_layout(popup);
    if state.popup.profile_names.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            "No saved profiles yet.",
            muted_style(),
        )))
        .render(sections[0], frame.buffer_mut());
    } else {
        let items = state
            .popup
            .profile_names
            .iter()
            .map(|name| ListItem::new(name.as_str()))
            .collect::<Vec<_>>();
        let mut list_state = ListState::default();
        list_state.select(Some(
            state
                .popup
                .selected_index
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

    if let Some(editor) = state.popup.profile_editor.as_ref() {
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
            state.ui.focus == FocusArea::Outputs,
        ),
        area,
    );
    let inner = inner_area(area);
    for (index, (output, card)) in state
        .output
        .states
        .iter()
        .zip(output_card_areas(inner).into_iter())
        .enumerate()
    {
        render_output_card_widget(
            card,
            frame.buffer_mut(),
            output,
            state.ui.focus == FocusArea::Outputs && state.output.selected == index,
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
            state.preamp.state.input1
        } else {
            state.preamp.state.input2
        };
        let title = if state.ui.focus == FocusArea::Preamp && state.preamp.selected_input == index {
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
            state.ui.focus == FocusArea::Preamp && state.preamp.selected_input == index,
            state.preamp.peaks[index].as_ref().map(|p| p.raw),
        );
    }
}

fn draw_mixer_main(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let layout = mixer_layout(area);

    let surface = MixerSurface::from_surface(state.mixer.surface);
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
                state.ui.focus == FocusArea::Mixer,
            ))
            .wrap(Wrap { trim: false }),
        layout[0],
    );
    let header_buttons = mixer_header_button_rects(layout[0]);
    Paragraph::new(Line::from(vec![chip(
        "PROFILES",
        Color::Black,
        if state.popup.profiles_open {
            Color::Yellow
        } else {
            Color::LightGreen
        },
    )]))
    .render(header_buttons[0], frame.buffer_mut());
    Paragraph::new(Line::from(vec![chip(
        "ROUTING",
        Color::Black,
        if state.popup.routing_open {
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
            state.ui.focus == FocusArea::Mixer,
        ),
        layout[1],
    );
    let page_buttons = mixer_strip_page_button_rects(layout[1]);
    let visible = visible_end.saturating_sub(visible_start);
    let can_page_left = state.mixer.strip_scroll > 0;
    let can_page_right = state.mixer.strip_scroll + visible < total;
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
    let Some(picker) = state.popup.assignment_picker else {
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
            .popup
            .selected_index
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
    let Some(popup_state) = state.popup.selector_popup else {
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
            .popup
            .selected_index
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
    if !state.popup.hotkeys_open {
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
    if !state.popup.options_open {
        return;
    }

    let popup = options_popup_area(area);
    frame.render_widget(Clear, popup);
    frame.render_widget(panel_block("Options", Color::Cyan, true), popup);

    let rows = options_popup_layout(popup);
    Paragraph::new(Line::from(vec![chip("OPTIONS", Color::Black, Color::Cyan)]))
        .render(rows[0], frame.buffer_mut());

    let refresh_rates = RefreshRate::all();
    let current_refresh = state.ui.settings.refresh_rate;
    let mut refresh_spans = vec![Span::styled("Refresh: ", subdued_style())];
    for r in refresh_rates {
        if *r == current_refresh {
            let text = format!("* {}", r.label());
            refresh_spans.push(chip(&text, Color::Black, Color::LightCyan));
        } else {
            refresh_spans.push(chip(r.label(), Color::Black, Color::Gray));
        }
        refresh_spans.push(Span::raw(" "));
    }
    Paragraph::new(Line::from(refresh_spans)).render(rows[1], frame.buffer_mut());

    let peak_db = state.ui.settings.peak_threshold_db();
    let peak_status = if state.ui.settings.peak_enabled {
        format!("ON ({} dB)", peak_db)
    } else {
        "OFF".to_string()
    };
    let peak_color = if state.ui.settings.peak_enabled {
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
            if state.ui.settings.peak_enabled {
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
    let current_hold = state.ui.settings.peak_hold_duration;
    let mut hold_spans = vec![Span::styled("Hold:   ", subdued_style())];
    for h in hold_durations {
        if *h == current_hold {
            let text = format!("* {}", h.label());
            hold_spans.push(chip(&text, Color::Black, Color::LightCyan));
        } else {
            hold_spans.push(chip(h.label(), Color::Black, Color::Gray));
        }
        hold_spans.push(Span::raw(" "));
    }
    Paragraph::new(Line::from(hold_spans)).render(rows[4], frame.buffer_mut());

    let auto_save_status = if state.ui.settings.auto_save {
        "ON"
    } else {
        "OFF"
    };
    let auto_save_color = if state.ui.settings.auto_save {
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

    let selected = state.raw_view.selected_tab;
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

    let (title, text) = match state.raw_view.selected_tab {
        RawPacketTab::Query74 => (
            "0x74 Query Requests",
            state
                .raw_view
                .latest_raw_74
                .as_ref()
                .map(|a| a.as_slice())
                .map(|bytes| render_query_request_panel(bytes, state))
                .unwrap_or_else(|| Text::from("Waiting for first 0x74 query request...")),
        ),
        RawPacketTab::State73 => (
            "0x73 State",
            state
                .raw_view
                .latest_raw_73
                .as_ref()
                .map(|a| a.as_slice())
                .map(|bytes| {
                    render_full_packet_dump(
                        bytes,
                        state
                            .raw_view
                            .baseline_raw_73
                            .as_ref()
                            .map(|a| a.as_slice()),
                    )
                })
                .unwrap_or_else(|| Text::from("Waiting for first 0x73 snapshot...")),
        ),
        RawPacketTab::Auxiliary => (
            "0x83 State",
            state
                .raw_view
                .latest_raw_83
                .as_ref()
                .map(|a| a.as_slice())
                .map(|bytes| {
                    render_full_packet_dump(
                        bytes,
                        state
                            .raw_view
                            .baseline_raw_83
                            .as_ref()
                            .map(|a| a.as_slice()),
                    )
                })
                .unwrap_or_else(|| Text::from("Waiting for first 0x83 auxiliary packet...")),
        ),
        RawPacketTab::Query75 => (
            "0x75 Query Replies",
            state
                .raw_view
                .latest_raw_75
                .as_ref()
                .map(|a| a.as_slice())
                .map(|bytes| render_query_reply_panel(bytes, state))
                .unwrap_or_else(|| Text::from("Waiting for first 0x75 query reply...")),
        ),
        RawPacketTab::DeviceNotification => (
            "0x81 Notification",
            state
                .raw_view
                .latest_raw_81
                .as_ref()
                .map(|a| a.as_slice())
                .map(|bytes| {
                    render_full_packet_dump(
                        bytes,
                        state
                            .raw_view
                            .baseline_raw_81
                            .as_ref()
                            .map(|a| a.as_slice()),
                    )
                })
                .unwrap_or_else(|| Text::from("Waiting for first 0x81 notification...")),
        ),
    };
    if state.raw_view.selected_tab == RawPacketTab::Query75 {
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
    let selected_left =
        state.ui.focus == FocusArea::Mixer && state.mixer.selected_channel == left_index;
    let selected_right =
        state.ui.focus == FocusArea::Mixer && state.mixer.selected_channel == right_index;
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
        &labels[0],
        Color::Black,
        Color::LightMagenta,
    )]))
    .render(columns[0], buffer);
    Paragraph::new(Line::from(vec![chip(
        &labels[1],
        Color::Black,
        Color::Gray,
    )]))
    .render(columns[1], buffer);
    Paragraph::new(Line::from(vec![chip(
        &labels[2],
        Color::Black,
        if selected_left {
            Color::Yellow
        } else {
            Color::LightCyan
        },
    )]))
    .render(columns[2], buffer);
    Paragraph::new(Line::from(vec![chip(
        &labels[3],
        Color::Black,
        Color::Gray,
    )]))
    .render(columns[3], buffer);
    Paragraph::new(Line::from(vec![chip(
        &labels[4],
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
    let assignments = &state.mixer.channels[MixerSurface::Mix1.index()];
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
        Span::styled(
            state.ui.last_message.clone(),
            strong_style(Color::LightCyan),
        ),
    ]));
    Text::from(lines)
}

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
    let Some(bytes) = state.raw_view.latest_raw_73.as_ref().map(|a| a.as_slice()) else {
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

#[cfg(test)]
pub(crate) fn render_mixer_strip_line(
    state: &AppState,
    index: usize,
    channel: &antelope_protocol::MixerChannelState,
) -> String {
    let selected = state.ui.focus == FocusArea::Mixer && state.mixer.selected_channel == index;
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
