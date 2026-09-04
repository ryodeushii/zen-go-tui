use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget, Wrap};
use ratatui::Frame;

use crate::app::{
    AppState, FocusArea, ProfileEditorMode, RawMapScope, RawPacketTab, RefreshRate,
    SelectorPopupKind,
};
use crate::device::DevicePickerState;
use crate::terminal;
use antelope_protocol::{ClockSource, MixerAssignment, PreampMode, RuntimeDriverKind, SampleRate};

use super::layouts::*;
use super::mouse::mix_meter;
use super::raw_map::build_raw_packet_map;
use super::styles::*;
use super::widgets::mixer;

mod text;

// Re-export widget functions for tests and other modules
pub(crate) use mixer::*;

// Re-export text rendering helpers
pub(crate) use text::build_query_reply_list_items;
pub(crate) use text::render_device_header;
pub(crate) use text::render_device_metadata;
#[cfg(test)]
pub(crate) use text::render_full_packet_dump;
pub(crate) use text::render_hotkeys_popup_text;
#[cfg(test)]
pub(crate) use text::render_query_reply_panel;
#[cfg(test)]
pub(crate) use text::render_query_request_panel;
pub(crate) use text::render_status_strip;
pub(crate) use text::render_system_summary;

// Re-exports for tests only
#[cfg(test)]
pub(crate) use text::connection_badge_color;
#[cfg(test)]
pub(crate) use text::raw_map_entry_style;
#[cfg(test)]
pub(crate) use text::render_mix_meter_state_line;
#[cfg(test)]
pub(crate) use text::render_raw_map_text;
#[cfg(test)]
pub(crate) use text::selected_query_reply_bytes;
pub fn draw_device_picker(frame: &mut Frame<'_>, picker: &DevicePickerState) {
    let area = device_picker_area(frame.area());
    frame.render_widget(Clear, area);
    let rows = if picker.entries().is_empty() {
        vec![ListItem::new(
            "Waiting for an Antelope HID control interface…",
        )]
    } else {
        picker
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let marker = if picker.selected_index() == Some(index) {
                    ">"
                } else {
                    " "
                };
                let serial = entry.candidate.serial().unwrap_or("serial ?");
                let reason = if entry.diagnostic.is_empty() {
                    "ready"
                } else {
                    entry.diagnostic.as_str()
                };
                let line = format!(
                    "{marker} {} | {} | {} | {} | {reason}",
                    entry.profile_name, serial, entry.candidate.path, entry.status,
                );
                let style = if entry.is_selectable() {
                    Style::default().fg(Color::LightGreen)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                ListItem::new(line).style(style)
            })
            .collect()
    };
    frame.render_widget(List::new(rows).block(device_picker_block()), area);
}

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
    let main = mixer_main_layout_for_state(sections[0], state);
    draw_preamp_bar(frame, main[0], state);

    draw_mixer_main(frame, main[1], state);
    draw_output_panel(frame, sections[1], state);
}

fn draw_routing_popup(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if !state.popup.routing_open {
        return;
    }
    if state.ui_profile.driver_kind == RuntimeDriverKind::ZenGo {
        draw_zen_go_routing_popup(frame, area, state);
        return;
    }

    let popup = dynamic_routing_popup_area(area, state);
    frame.render_widget(Clear, popup);
    frame.render_widget(panel_block("Routing", Color::Magenta, true), popup);

    let inner = Rect::new(
        popup.x.saturating_add(1),
        popup.y.saturating_add(1),
        popup.width.saturating_sub(2),
        popup.height.saturating_sub(2),
    );
    if inner.height > 0 {
        Paragraph::new(Line::from(vec![chip(
            "ROUTING",
            Color::Black,
            Color::LightMagenta,
        )]))
        .render(
            Rect::new(inner.x, inner.y, inner.width, 1),
            frame.buffer_mut(),
        );
    }
    for (row, capability) in state
        .routing_capabilities
        .iter()
        .take(usize::from(inner.height.saturating_sub(1)))
        .enumerate()
    {
        let observed = state
            .routing_group(capability.destination)
            .map_or(0, |group| group.sources.len());
        let label = format!(
            "{}  {} ch  observed {observed}",
            capability.name, capability.channel_count
        );
        Paragraph::new(Line::from(label)).render(
            Rect::new(
                inner.x,
                inner.y.saturating_add(1).saturating_add(row as u16),
                inner.width,
                1,
            ),
            frame.buffer_mut(),
        );
    }
    // Routing groups are profile-defined; assignment editing happens from each strip source chip.
}

fn draw_zen_go_routing_popup(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
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
        render_afx_routing_row(rows[4 + pair], frame.buffer_mut(), state, pair);
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
            "letters, digits, spaces, -, _ and .",
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
    for (index, row) in dynamic_output_card_areas(inner, state.outputs().len())
        .into_iter()
        .enumerate()
    {
        let Some(controls) = dynamic_output_control_rects(row, state, index) else {
            continue;
        };
        render_dynamic_output_card_widget(
            controls,
            frame.buffer_mut(),
            state,
            index,
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
    draw_dynamic_input_banks(frame, area, state);
}

fn draw_dynamic_input_banks(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = panel_block(
        "Inputs",
        Color::LightMagenta,
        state.ui.focus == FocusArea::Preamp,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    for (space, column) in state
        .input_spaces
        .iter()
        .zip(dynamic_input_space_areas(inner, state))
    {
        Paragraph::new(Line::from(Span::styled(
            &space.name,
            strong_style(Color::LightMagenta),
        )))
        .render(
            Rect::new(column.x, inner.y, column.width, 1),
            frame.buffer_mut(),
        );
    }
    for (space_index, input_index, row) in dynamic_input_rows(area, state) {
        let Some(controls) = dynamic_input_control_rects(row, state, space_index, input_index)
        else {
            continue;
        };
        render_dynamic_input_row(
            frame.buffer_mut(),
            state,
            space_index,
            input_index,
            controls,
        );
    }
}

pub(crate) fn dynamic_input_control_color(
    state: &AppState,
    input: &antelope_protocol::DynamicInputState,
    kind: antelope_protocol::RuntimeInputControlKind,
) -> Color {
    let control = state
        .ui_profile
        .input_capabilities(input.address)
        .iter()
        .find(|capability| capability.kind == kind)
        .and_then(|capability| capability.control);
    if !control.is_some_and(|control| state.ui_profile.supports_input(input.address, control)) {
        return Color::DarkGray;
    }
    if kind == antelope_protocol::RuntimeInputControlKind::Phase && input.phase == Some(true) {
        Color::Yellow
    } else if kind == antelope_protocol::RuntimeInputControlKind::Phase {
        Color::Green
    } else {
        Color::LightGreen
    }
}

fn render_dynamic_input_row(
    buffer: &mut Buffer,
    state: &AppState,
    space_index: usize,
    input_index: usize,
    controls: DynamicInputControlRects,
) {
    let Some(input) = state
        .input_spaces
        .get(space_index)
        .and_then(|space| space.inputs.get(input_index))
    else {
        return;
    };
    let rich_card = controls.row.height >= PREAMP_CARD_HEIGHT;
    if rich_card {
        let focused =
            state.ui.focus == FocusArea::Preamp && state.preamp.selected_input == input_index;
        let block = if input.phantom == Some(true) {
            warning_section_block(&input.name, focused)
        } else {
            section_block(&input.name, focused)
        };
        block.render(controls.row, buffer);
        let sections = preamp_card_inner_layout(controls.row);
        let meter_db = input.meter.and_then(antelope_protocol::meter_display_db);
        let gain_range = state.input_range(input.address, input.mode);
        let gain_label = input
            .gain
            .map_or_else(|| "? dB".to_string(), |value| format!("{value} dB"));
        let gain_ratio = input
            .gain
            .zip(gain_range)
            .map(|(value, range)| value_ratio(value, range));
        let gain_color = match input.mode {
            Some(0) => style_for_preamp_mode(PreampMode::Mic),
            Some(1) => style_for_preamp_mode(PreampMode::Line),
            Some(2) => style_for_preamp_mode(PreampMode::HiZ),
            _ => Color::LightGreen,
        };
        render_stacked_signal_rows(
            sections[0],
            buffer,
            &meter_slider_label("OBS", meter_db),
            input.meter.map(antelope_protocol::meter_ratio),
            &signal_slider_label("GAIN", Some(gain_label)),
            gain_ratio,
            gain_color,
        );
    } else {
        let input_label = input
            .meter
            .and_then(antelope_protocol::meter_display_db)
            .map_or_else(
                || input.name.clone(),
                |meter| format!("{} {} dB", input.name, meter),
            );
        Paragraph::new(Line::from(chip(
            &input_label,
            Color::Black,
            Color::LightBlue,
        )))
        .render(
            Rect::new(
                controls.row.x,
                controls.row.y,
                controls.row.width.min(10),
                1,
            ),
            buffer,
        );
    }
    let color = |kind| dynamic_input_control_color(state, input, kind);
    if !rich_card {
        if let Some(rect) = controls.gain {
            let label = input
                .gain
                .map_or_else(|| "GAIN ?".into(), |value| format!("GAIN {value}"));
            Paragraph::new(Line::from(chip(
                &label,
                Color::Black,
                color(antelope_protocol::RuntimeInputControlKind::Gain),
            )))
            .render(rect, buffer);
        }
    }
    if let Some(rect) = controls.mode {
        let label = input.mode.map_or_else(
            || "MODE".into(),
            |value| {
                state
                    .ui_profile
                    .input_value_label(input.address, antelope_protocol::InputControl::Mode, value)
                    .map_or_else(|| format!("M{value}"), str::to_owned)
            },
        );
        Paragraph::new(Line::from(chip(
            &label,
            Color::Black,
            color(antelope_protocol::RuntimeInputControlKind::Mode),
        )))
        .render(rect, buffer);
    }
    if let Some(rect) = controls.phantom {
        Paragraph::new(Line::from(chip(
            "48V",
            Color::Black,
            if state
                .ui_profile
                .supports_input(input.address, antelope_protocol::InputControl::Phantom)
                && input.phantom == Some(true)
            {
                Color::LightRed
            } else {
                color(antelope_protocol::RuntimeInputControlKind::Phantom)
            },
        )))
        .render(rect, buffer);
    }
    if let Some(rect) = controls.phase {
        Paragraph::new(Line::from(chip(
            "PH",
            Color::Black,
            color(antelope_protocol::RuntimeInputControlKind::Phase),
        )))
        .render(rect, buffer);
    }
    if let Some(rect) = controls.link {
        Paragraph::new(Line::from(chip(
            "LINK",
            Color::Black,
            color(antelope_protocol::RuntimeInputControlKind::Link),
        )))
        .render(rect, buffer);
    }
}

fn draw_mixer_main(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let layout = mixer_layout(area);

    let mut tabs = Vec::new();
    for (index, surface) in state.mixers().iter().enumerate() {
        if index > 0 {
            tabs.push(Span::raw(" "));
        }
        let name = surface.name.as_str();
        tabs.push(tab_chip(
            name,
            state.active_mixer_surface() == Some(index),
            if index.is_multiple_of(2) {
                Color::LightCyan
            } else {
                Color::LightBlue
            },
        ));
    }
    let line = Line::from(tabs);
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
    let total = state
        .active_mixer_surface()
        .and_then(|index| state.mixers().get(index))
        .map_or(0, |surface| surface.strips.len());
    let mut title = if total == 0 {
        "Mixer Strips".to_string()
    } else {
        format!(
            "Mixer Strips {}-{} / {}",
            visible_start + 1,
            visible_end,
            total
        )
    };
    if state
        .active_mixer_surface()
        .and_then(|index| state.mixers().get(index))
        .is_some_and(|surface| surface.master.is_some())
    {
        title.push_str(" | Master");
    }
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

    let visible_count = visible_end.saturating_sub(visible_start);
    if let Some((surface, master, card)) = state
        .active_mixer_surface()
        .and_then(|index| state.mixers().get(index))
        .and_then(|surface| {
            Some((
                surface,
                surface.master.as_ref()?,
                mixer_master_area(inner, state)?,
            ))
        })
    {
        let address = antelope_protocol::MixerAddress {
            surface: surface.surface,
            strip: master.strip,
        };
        if let Some(controls) = dynamic_mixer_control_rects(card, state, address) {
            render_dynamic_mixer_strip_widget(
                controls,
                frame.buffer_mut(),
                state,
                address,
                None,
                master,
            );
        }
    }
    let strip_area = mixer_input_strip_area(inner, state);
    let active_surface = state
        .active_mixer_surface()
        .and_then(|index| state.mixers().get(index));
    for (slot, index) in (visible_start..visible_end).enumerate() {
        let Some(strip) = active_surface.and_then(|surface| surface.strips.get(index)) else {
            continue;
        };
        let card = dynamic_mixer_strip_card_area(strip_area, state, slot, visible_count);
        if card.x >= strip_area.x + strip_area.width
            || card.x + card.width > strip_area.x + strip_area.width
        {
            break;
        }
        let address = antelope_protocol::MixerAddress {
            surface: active_surface.map_or(0, |surface| surface.surface),
            strip: strip.strip,
        };
        if let Some(controls) = dynamic_mixer_control_rects(card, state, address) {
            render_dynamic_mixer_strip_widget(
                controls,
                frame.buffer_mut(),
                state,
                address,
                Some(index),
                strip,
            );
        }
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

fn raw_packet_source(
    tab: RawPacketTab,
    state: &AppState,
) -> (&'static str, Option<&[u8]>, Option<&[u8]>, &'static str) {
    match tab {
        RawPacketTab::Query74 => (
            "0x74 Query Requests",
            state
                .raw_view
                .latest_raw_74
                .as_ref()
                .map(|bytes| bytes.as_slice()),
            state
                .raw_view
                .baseline_raw_74
                .as_ref()
                .map(|bytes| bytes.as_slice()),
            "Waiting for first 0x74 query request...",
        ),
        RawPacketTab::State73 => (
            "0x73 State",
            state
                .raw_view
                .latest_raw_73
                .as_ref()
                .map(|bytes| bytes.as_slice()),
            state
                .raw_view
                .baseline_raw_73
                .as_ref()
                .map(|bytes| bytes.as_slice()),
            "Waiting for first 0x73 snapshot...",
        ),
        RawPacketTab::Auxiliary => (
            "0x83 State",
            state
                .raw_view
                .latest_raw_83
                .as_ref()
                .map(|bytes| bytes.as_slice()),
            state
                .raw_view
                .baseline_raw_83
                .as_ref()
                .map(|bytes| bytes.as_slice()),
            "Waiting for first 0x83 auxiliary packet...",
        ),
        RawPacketTab::Query75 => {
            let latest = state
                .raw_view
                .latest_raw_75
                .as_ref()
                .map(|bytes| bytes.as_slice());
            let bytes = if latest.is_some() || state.selected_query_reply_entry().is_some() {
                Some(text::selected_query_reply_bytes(
                    latest.unwrap_or(&[]),
                    state,
                ))
            } else {
                None
            };
            (
                "0x75 Query Replies",
                bytes,
                state
                    .raw_view
                    .baseline_raw_75
                    .as_ref()
                    .map(|bytes| bytes.as_slice()),
                "Waiting for first 0x75 query reply...",
            )
        }
        RawPacketTab::DeviceNotification => (
            "0x81 Notification",
            state
                .raw_view
                .latest_raw_81
                .as_ref()
                .map(|bytes| bytes.as_slice()),
            state
                .raw_view
                .baseline_raw_81
                .as_ref()
                .map(|bytes| bytes.as_slice()),
            "Waiting for first 0x81 notification...",
        ),
    }
}

fn raw_scope_accent(scope: RawMapScope) -> Color {
    match scope {
        RawMapScope::All => Color::LightCyan,
        RawMapScope::Base => Color::Green,
        RawMapScope::Outputs => Color::LightGreen,
        RawMapScope::Preamps => Color::LightMagenta,
        RawMapScope::Mixer => Color::Yellow,
        RawMapScope::Query => Color::LightYellow,
        RawMapScope::Metadata => Color::LightBlue,
        RawMapScope::Status => Color::LightGreen,
        RawMapScope::Parser => Color::Cyan,
        RawMapScope::Unmapped => Color::LightRed,
    }
}

fn raw_text_height(text: &Text<'_>, width: u16, wrapped: bool) -> usize {
    if width == 0 {
        return 0;
    }
    if !wrapped {
        return text.lines.len();
    }

    // Paragraph's wrapped line composer is not exposed without Ratatui's unstable feature.
    // Render into a tall buffer instead, using the same Paragraph and Wrap configuration as
    // the visible pane, so word boundaries, grapheme widths, and trim behavior stay identical.
    let estimated_height = text
        .lines
        .iter()
        .map(|line| line.width().saturating_add(1))
        .sum::<usize>()
        .max(1)
        .min(usize::from(u16::MAX)) as u16;
    let area = Rect::new(0, 0, width, estimated_height);
    let mut buffer = Buffer::empty(area);
    Paragraph::new(text.clone())
        .wrap(Wrap { trim: false })
        .render(area, &mut buffer);

    buffer
        .content()
        .chunks(usize::from(width))
        .enumerate()
        .rev()
        .find(|(_, row)| row.iter().any(|cell| cell.symbol() != " "))
        .map_or(0, |(row, _)| row + 1)
}

fn raw_scroll_offset(scroll: usize, text: &Text<'_>, viewport: Rect, wrapped: bool) -> u16 {
    let content_height = raw_text_height(text, viewport.width, wrapped);
    let max_scroll = content_height.saturating_sub(usize::from(viewport.height));
    scroll.min(max_scroll).min(usize::from(u16::MAX)) as u16
}

#[cfg(test)]
pub(crate) fn raw_scroll_offset_for_test(
    scroll: usize,
    text: &Text<'_>,
    viewport: Rect,
    wrapped: bool,
) -> u16 {
    raw_scroll_offset(scroll, text, viewport, wrapped)
}

const RAW_COVERAGE_LEGEND: &str =
    "USED green | READBACK blue | OBSERVED amber | PARSER cyan | UNMAPPED red | PADDING gray";

fn raw_footer_lines(width: u16, map_scroll: u16, dump_scroll: u16) -> Vec<Line<'static>> {
    let footer =
        format!("[/] scope   PageUp/PageDown scroll   map {map_scroll}   dump {dump_scroll}");
    let width = usize::from(width);
    if RAW_COVERAGE_LEGEND.chars().count() <= width {
        return vec![
            Line::from(Span::styled(RAW_COVERAGE_LEGEND, muted_style())),
            Line::from(Span::styled(footer, muted_style())),
        ];
    }

    let split = RAW_COVERAGE_LEGEND
        .get(..width)
        .and_then(|prefix| prefix.rfind(" | "))
        .unwrap_or(width.min(RAW_COVERAGE_LEGEND.len()));
    let (first, second) = RAW_COVERAGE_LEGEND.split_at(split);
    vec![
        Line::from(Span::styled(first, muted_style())),
        Line::from(vec![
            Span::styled(second, muted_style()),
            Span::raw("   "),
            Span::styled(footer, muted_style()),
        ]),
    ]
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

    let scopes = RawMapScope::options_for(selected);
    let scope_line = scopes
        .iter()
        .enumerate()
        .flat_map(|(index, scope)| {
            let mut spans = Vec::with_capacity(2);
            if index > 0 {
                spans.push(Span::raw(" "));
            }
            spans.push(tab_chip(
                scope.label(),
                *scope == state.raw_view.raw_map_scope,
                raw_scope_accent(*scope),
            ));
            spans
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(Line::from(scope_line))
            .block(section_block("Map Scope", true))
            .wrap(Wrap { trim: false }),
        layout[2],
    );

    let query_replies = selected == RawPacketTab::Query75;
    let content = raw_content_layout(layout[3], query_replies);
    let (title, bytes, baseline, waiting) = raw_packet_source(selected, state);
    let (map_text, mut dump_text) = if let Some(bytes) = bytes {
        let map = build_raw_packet_map(selected, bytes);
        let map_text =
            text::render_raw_map_text(&map, state.raw_view.raw_map_scope, content.compact_map());
        let dump_text =
            text::render_full_packet_dump(bytes, baseline, &map, state.raw_view.raw_map_scope);
        (map_text, dump_text)
    } else {
        (Text::from(waiting), Text::from(waiting))
    };

    if selected == RawPacketTab::Query74 && !state.raw_view.recent_query_request_log.is_empty() {
        let mut lines = dump_text.lines;
        lines.push(Line::from(""));
        lines.push(Line::from("Recent 0x74 requests:"));
        for entry in state.raw_view.recent_query_request_log.iter().rev().take(8) {
            lines.push(Line::from(entry.clone()));
        }
        dump_text = Text::from(lines);
    }

    if let Some(history) = content.history() {
        let list_items = build_query_reply_list_items(state);
        frame.render_widget(
            List::new(list_items)
                .block(section_block("Recent 0x75 Replies", true))
                .highlight_style(strong_style(Color::LightCyan)),
            history,
        );
    }

    let map_area = content.map();
    let map_block = section_block("Field Map", true);
    let map_inner = map_block.inner(map_area);
    let map_scroll = raw_scroll_offset(state.raw_view.raw_map_scroll, &map_text, map_inner, true);
    frame.render_widget(
        Paragraph::new(map_text)
            .block(map_block)
            .wrap(Wrap { trim: false })
            .scroll((map_scroll, 0)),
        map_area,
    );

    let dump_area = content.dump();
    let dump_block = section_block(title, true);
    let dump_inner = dump_block.inner(dump_area);
    let dump_scroll =
        raw_scroll_offset(state.raw_view.raw_dump_scroll, &dump_text, dump_inner, true);
    frame.render_widget(
        Paragraph::new(dump_text)
            .block(dump_block)
            .wrap(Wrap { trim: false })
            .scroll((dump_scroll, 0)),
        dump_area,
    );

    frame.render_widget(
        Paragraph::new(raw_footer_lines(layout[4].width, map_scroll, dump_scroll)),
        layout[4],
    );
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
