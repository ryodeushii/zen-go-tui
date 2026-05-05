use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget, Wrap};
use ratatui::Frame;

use crate::app::{
    AppState, FocusArea, ProfileEditorMode, RawPacketTab, RefreshRate, SelectorPopupKind,
};
use crate::terminal;
use antelope_protocol::{
    ClockSource, MixerAssignment, MixerSurface, PreampMode, SampleRate,
};

use super::layouts::*;
use super::mouse::mix_meter;
use super::styles::*;
use super::widgets::mixer;

mod text;

// Re-export widget functions for tests and other modules
pub(crate) use mixer::*;

// Re-export text rendering helpers
pub(crate) use text::build_query_reply_list_items;
pub(crate) use text::connection_badge_color;
pub(crate) use text::render_device_header;
pub(crate) use text::render_device_metadata;
pub(crate) use text::render_full_packet_dump;
pub(crate) use text::render_hotkeys_popup_text;
pub(crate) use text::render_mix_meter_state_line;
pub(crate) use text::render_query_reply_panel;
pub(crate) use text::render_query_request_panel;
pub(crate) use text::render_status_strip;
pub(crate) use text::render_system_summary;

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
        .zip(output_card_areas(inner))
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

fn render_popup_list(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    items: Vec<ListItem<'_>>,
    selected_index: usize,
    highlight_color: Color,
) {
    let mut list_state = ListState::default();
    list_state.select(Some(selected_index.min(items.len().saturating_sub(1))));
    frame.render_widget(Clear, area);
    frame.render_stateful_widget(
        List::new(items)
            .block(panel_block(title, highlight_color, true))
            .highlight_style(terminal::adapt_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(highlight_color)
                    .add_modifier(Modifier::BOLD),
            )),
        area,
        &mut list_state,
    );
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

    render_popup_list(
        frame,
        popup,
        &format!("Assign CH {:02}", picker.strip),
        items,
        state.popup.selected_index,
        Color::Yellow,
    );
}

fn draw_selector_popup(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(popup_state) = state.popup.selector_popup else {
        return;
    };

    let popup = assignment_picker_area(area);
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

    render_popup_list(
        frame,
        popup,
        title,
        items,
        state.popup.selected_index,
        Color::Yellow,
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

