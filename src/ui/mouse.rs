use ratatui::layout::Rect;

use crate::app::{
    AppState, AssignmentPickerState, Intent, RawPacketTab, SelectorPopupKind, SelectorPopupState,
    QUERY_REPLY_VISIBLE_COUNT,
};
use antelope_protocol::{ClockSource, MixerAssignment, PreampMode, SampleRate};
use antelope_protocol::{
    OFFSET_MIX1_LANE_A, OFFSET_MIX1_LANE_B, OFFSET_MIX2_LANE_A, OFFSET_MIX2_LANE_B,
    OFFSET_SURFACE_SELECTOR, SNAPSHOT_PAYLOAD_OFFSET, SURFACE_CODE_HP2, SURFACE_CODE_MONITOR_HP1,
};

use super::layouts::*;

pub(crate) fn contains_point(area: Rect, point: (u16, u16)) -> bool {
    point.0 >= area.x
        && point.0 < area.x.saturating_add(area.width)
        && point.1 >= area.y
        && point.1 < area.y.saturating_add(area.height)
}

pub fn mouse_action(area: Rect, state: &AppState, x: u16, y: u16) -> Option<Intent> {
    let point = (x, y);
    let chunks = root_chunks(area);

    if state.popup.hotkeys_open {
        return Some(Intent::ToggleHotkeysPopup);
    }

    if let Some(action) = device_header_mouse_action(titlebar_layout(chunks[0])[0], state, point) {
        return Some(action);
    }

    if contains_point(titlebar_layout(chunks[0])[1], point) {
        return system_panel_mouse_action(titlebar_layout(chunks[0])[1], state, point);
    }

    if state.popup.raw_view_open {
        return raw_mouse_action(area, state, point);
    }

    if state.popup.profile_editor.is_some() {
        return profile_editor_mouse_action(area, point);
    }

    if state.popup.profiles_open {
        return profiles_popup_mouse_action(area, state, point);
    }

    if let Some(popup) = state.popup.selector_popup {
        return selector_popup_mouse_action(area, popup, point);
    }

    if let Some(picker) = state.popup.assignment_picker {
        return assignment_picker_mouse_action(area, picker, point);
    }

    if state.popup.routing_open {
        return routing_popup_mouse_action(area, state, point);
    }

    if state.popup.options_open {
        return options_popup_mouse_action(area, state, point);
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
    if let Some(action) = mixer_panel_mouse_action(mixer_sections[1], point) {
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

pub fn slider_mouse_action(area: Rect, state: &AppState, x: u16, y: u16) -> Option<Intent> {
    if state.popup.hotkeys_open
        || state.popup.raw_view_open
        || state.popup.profiles_open
        || state.popup.selector_popup.is_some()
        || state.popup.assignment_picker.is_some()
        || state.popup.routing_open
        || state.popup.options_open
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
) -> Option<Intent> {
    if state.popup.hotkeys_open
        || state.popup.profiles_open
        || state.popup.selector_popup.is_some()
        || state.popup.assignment_picker.is_some()
        || state.popup.routing_open
        || state.popup.options_open
    {
        return None;
    }

    let point = (x, y);

    if state.popup.raw_view_open && state.raw_view.selected_tab == RawPacketTab::Query75 {
        if let Some(action) = query_reply_wheel_action(area, state, point, increase) {
            return Some(action);
        }
        return None;
    }

    if state.popup.raw_view_open {
        return None;
    }

    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout(page[0]);
    let mixer_sections = mixer_layout(main[1]);

    output_list_slider_wheel_action(page[1], state, point, increase)
        .or_else(|| mixer_list_slider_wheel_action(mixer_sections[1], state, point, increase))
        .or_else(|| preamp_slider_wheel_action(main[0], state, point, increase))
}

fn routing_popup_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    let popup = routing_popup_area(area);
    if !contains_point(popup, point) {
        return Some(Intent::CloseRoutingPopup);
    }
    afx_routing_mouse_action(popup, state, point)
}

fn options_popup_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    let popup = options_popup_area(area);
    if !contains_point(popup, point) {
        return Some(Intent::CloseOptionsPopup);
    }

    let rows = options_popup_layout(popup);
    let refresh_row = rows[1];
    let peak_threshold_row = rows[2];
    let peak_toggle_row = rows[3];
    let peak_hold_row = rows[4];
    let auto_save_row = rows[5];
    let button_rects = options_popup_button_rects(popup);

    if contains_point(button_rects[0], point) {
        return Some(Intent::CloseOptionsPopup);
    }

    if contains_point(refresh_row, point) {
        let refresh_rates = crate::app::RefreshRate::all();
        let prefix = "Refresh: ";
        let mut x = refresh_row.x + prefix.chars().count() as u16;
        for r in refresh_rates {
            let label = if *r == state.ui.settings.refresh_rate {
                format!("* {}", r.label())
            } else {
                r.label().to_string()
            };
            let w = crate::ui::styles::chip_width(&label);
            let rect = Rect::new(x, refresh_row.y, w, 1);
            if contains_point(rect, point) {
                return Some(Intent::SetRefreshRate(*r));
            }
            x += w + 1;
        }
    }

    if contains_point(peak_threshold_row, point) {
        let prefix = "Peaks:  ";
        let status_text = if state.ui.settings.peak_enabled {
            format!("ON ({} dB)", state.ui.settings.peak_threshold_db())
        } else {
            "OFF".to_string()
        };
        let mut x = peak_threshold_row.x + prefix.chars().count() as u16;
        let status_w = crate::ui::styles::chip_width(&status_text);
        x += status_w + 1;
        x += 2;
        let down_w = crate::ui::styles::chip_width("↓");
        let down_rect = Rect::new(x, peak_threshold_row.y, down_w, 1);
        if contains_point(down_rect, point) {
            return Some(Intent::CyclePeakThreshold(false));
        }
        x += down_w + 1;
        let up_w = crate::ui::styles::chip_width("↑");
        let up_rect = Rect::new(x, peak_threshold_row.y, up_w, 1);
        if contains_point(up_rect, point) {
            return Some(Intent::CyclePeakThreshold(true));
        }
    }

    if contains_point(peak_toggle_row, point) {
        let prefix = "Toggle: ";
        let label = if state.ui.settings.peak_enabled {
            "Disable"
        } else {
            "Enable"
        };
        let x = peak_toggle_row.x + prefix.chars().count() as u16;
        let w = crate::ui::styles::chip_width(label);
        let rect = Rect::new(x, peak_toggle_row.y, w, 1);
        if contains_point(rect, point) {
            return Some(Intent::TogglePeakEnabled);
        }
    }

    if contains_point(peak_hold_row, point) {
        let hold_durations = crate::app::PeakHoldDuration::all();
        let prefix = "Hold:   ";
        let mut x = peak_hold_row.x + prefix.chars().count() as u16;
        for h in hold_durations {
            let label = if *h == state.ui.settings.peak_hold_duration {
                format!("* {}", h.label())
            } else {
                h.label().to_string()
            };
            let w = crate::ui::styles::chip_width(&label);
            let rect = Rect::new(x, peak_hold_row.y, w, 1);
            if contains_point(rect, point) {
                return Some(Intent::CyclePeakHoldDuration(*h));
            }
            x += w + 1;
        }
    }

    if contains_point(auto_save_row, point) {
        return Some(Intent::ToggleAutoSave);
    }

    None
}

fn profiles_popup_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    let popup = profiles_popup_area(area);
    if !contains_point(popup, point) {
        return Some(Intent::CloseProfilesPopup);
    }

    let button_rects = profiles_popup_button_rects(popup);
    if contains_point(button_rects[0], point) {
        return Some(Intent::LoadSelectedProfile);
    }
    if contains_point(button_rects[1], point) {
        return Some(Intent::StartSaveProfile);
    }
    if contains_point(button_rects[2], point) {
        return Some(Intent::StartRenameProfile);
    }
    if contains_point(button_rects[3], point) {
        return Some(Intent::DeleteSelectedProfile);
    }
    if contains_point(button_rects[4], point) {
        return Some(Intent::CloseProfilesPopup);
    }

    let list_area = profiles_popup_layout(popup)[0];
    if !contains_point(list_area, point) || state.popup.profile_names.is_empty() {
        return None;
    }
    let index = point.1.saturating_sub(list_area.y) as usize;
    state
        .popup
        .profile_names
        .get(index)
        .map(|_| Intent::SelectProfile(index))
}

fn profile_editor_mouse_action(area: Rect, point: (u16, u16)) -> Option<Intent> {
    if contains_point(profile_editor_area(area), point) {
        None
    } else {
        Some(Intent::CloseProfilesPopup)
    }
}

fn raw_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    let layout = raw_page_layout(area);
    let header = raw_header_layout(layout[0]);
    if contains_point(header[1], point) {
        return Some(Intent::ToggleRawView);
    }
    if contains_point(layout[1], point) {
        let tabs = raw_tab_hit_areas(layout[1]);
        if contains_point(tabs[0], point) {
            return Some(Intent::SelectRawPacketTab(RawPacketTab::Query74));
        } else if contains_point(tabs[1], point) {
            return Some(Intent::SelectRawPacketTab(RawPacketTab::State73));
        } else if contains_point(tabs[2], point) {
            return Some(Intent::SelectRawPacketTab(RawPacketTab::Auxiliary));
        } else if contains_point(tabs[3], point) {
            return Some(Intent::SelectRawPacketTab(RawPacketTab::Query75));
        } else if contains_point(tabs[4], point) {
            return Some(Intent::SelectRawPacketTab(RawPacketTab::DeviceNotification));
        }
    }
    if state.raw_view.selected_tab == RawPacketTab::Query75 {
        let sections = query_reply_history_layout(layout[2]);
        if !contains_point(sections[0], point) {
            return None;
        }
        let inner = inner_area(sections[0]);
        if point.1 < inner.y + 1 {
            return None;
        }
        let total = state.raw_view.recent_query_reply_entries.len();
        let visible = QUERY_REPLY_VISIBLE_COUNT.min(total);
        let start = state
            .raw_view
            .query_reply_scroll
            .min(total.saturating_sub(visible));
        let row = point.1.saturating_sub(inner.y + 1) as usize;
        if row >= visible {
            return None;
        }
        let rev_index = start + row;
        let index = total - 1 - rev_index;
        Some(Intent::SelectQueryReplyEntry(index))
    } else {
        None
    }
}

fn query_reply_wheel_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
    increase: bool,
) -> Option<Intent> {
    let layout = raw_page_layout(area);
    if state.raw_view.selected_tab != RawPacketTab::Query75 {
        return None;
    }
    let sections = query_reply_history_layout(layout[2]);
    if !contains_point(sections[0], point) {
        return None;
    }
    let total = state.raw_view.recent_query_reply_entries.len();
    if total <= QUERY_REPLY_VISIBLE_COUNT {
        return None;
    }
    Some(Intent::ScrollQueryReplyList { increase })
}

fn assignment_picker_mouse_action(
    area: Rect,
    picker: AssignmentPickerState,
    point: (u16, u16),
) -> Option<Intent> {
    let popup = assignment_picker_area(area);
    if !contains_point(popup, point) {
        return Some(Intent::CloseAssignmentPicker);
    }

    let inner = popup_list_inner_area(popup, &format!("Assign CH {:02}", picker.strip));
    if point.1 < inner.y {
        return None;
    }
    let index = point.1.saturating_sub(inner.y) as usize;
    let assignment = *MixerAssignment::grounded_choices().get(index)?;
    Some(Intent::PickAssignment {
        strip: picker.strip,
        assignment,
    })
}

fn selector_popup_mouse_action(
    area: Rect,
    popup: SelectorPopupState,
    point: (u16, u16),
) -> Option<Intent> {
    let popup_area = assignment_picker_area(area);
    if !contains_point(popup_area, point) {
        return Some(Intent::CloseSelectorPopup);
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
            .map(Intent::PickSampleRate),
        SelectorPopupKind::ClockSource => ClockSource::all_confirmed()
            .get(index)
            .copied()
            .map(Intent::PickClockSource),
        SelectorPopupKind::PreampMode { input } => {
            [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                .get(index)
                .copied()
                .map(|mode| Intent::PickPreampMode { input, mode })
        }
    }
}

fn system_panel_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let inner = crate::ui::layouts::inner_area(area);
    let labels = ["RAW", "OPTNS", "X"];
    let rects = inline_chip_rects(inner.x, inner.y, &labels);
    if contains_point(rects[0], point) {
        return Some(Intent::ToggleRawView);
    }
    if contains_point(rects[1], point) {
        if state.popup.options_open {
            return Some(Intent::CloseOptionsPopup);
        } else {
            return Some(Intent::OpenOptionsPopup);
        }
    }
    if contains_point(rects[2], point) {
        return Some(Intent::Quit);
    }
    None
}

fn device_header_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let chips = device_header_hit_areas(area, state);
    if contains_point(chips[1], point) {
        if state.device.status.clock_source == Some(ClockSource::Internal) {
            Some(Intent::OpenSampleRateSelector)
        } else {
            None
        }
    } else if contains_point(chips[2], point) {
        Some(Intent::OpenClockSourceSelector)
    } else {
        None
    }
}

fn mixer_tab_mouse_action(area: Rect, point: (u16, u16)) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let buttons = mixer_header_button_rects(area);
    if contains_point(buttons[0], point) {
        return Some(Intent::OpenProfilesPopup);
    }
    if contains_point(buttons[1], point) {
        return Some(Intent::OpenRoutingPopup);
    }
    let tabs = surface_tab_hit_areas(area);
    if contains_point(tabs[0], point) {
        Some(Intent::SelectSurface(
            antelope_protocol::Surface::MonitorHp1,
        ))
    } else if contains_point(tabs[1], point) {
        Some(Intent::SelectSurface(antelope_protocol::Surface::Hp2))
    } else {
        None
    }
}

fn mixer_panel_mouse_action(area: Rect, point: (u16, u16)) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let buttons = mixer_strip_page_button_rects(area);
    if contains_point(buttons[0], point) {
        Some(Intent::PageMixerStripsLeft)
    } else if contains_point(buttons[1], point) {
        Some(Intent::PageMixerStripsRight)
    } else {
        None
    }
}

pub(crate) fn mixer_list_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let inner = mixer_strip_panel_layout(area, mix_meter(state).is_some())[0];
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
            return Some(Intent::OpenAssignmentPicker(channel.channel));
        }

        let controls = mixer_control_button_rects(card, channel.channel % 2 == 1);
        if channel.channel % 2 == 1 && contains_point(controls[0], point) {
            return Some(Intent::ToggleMixerLink(channel.channel));
        }
        let solo_rect = if channel.channel % 2 == 1 {
            controls[1]
        } else {
            controls[0]
        };
        if contains_point(solo_rect, point) {
            return Some(Intent::ToggleMixerSolo(channel.channel));
        }
        let mute_rect = if channel.channel % 2 == 1 {
            controls[2]
        } else {
            controls[1]
        };
        if contains_point(mute_rect, point) {
            return Some(Intent::ToggleMixerMute(channel.channel));
        }

        return Some(Intent::SelectMixerChannel(index));
    }

    None
}

pub(crate) fn mixer_list_slider_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let inner = mixer_strip_panel_layout(area, mix_meter(state).is_some())[0];
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

pub(crate) fn mixer_list_slider_wheel_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
    increase: bool,
) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let inner = mixer_strip_panel_layout(area, mix_meter(state).is_some())[0];
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

fn preamp_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let layout = preamp_bar_layout(area);
    for (input, card) in layout.into_iter().enumerate() {
        if !contains_point(card, point) {
            continue;
        }
        let input_state = if input == 0 {
            state.preamp.state.input1
        } else {
            state.preamp.state.input2
        };
        if let Some(action) = preamp_card_slider_mouse_action(card, input as u8, input_state, point)
        {
            return Some(action);
        }
        let buttons = preamp_button_rects(card, input_state);
        if contains_point(buttons[0], point) {
            return Some(Intent::AdjustPreampGain {
                input: input as u8,
                increase: false,
            });
        }
        if contains_point(buttons[1], point) {
            return Some(Intent::AdjustPreampGain {
                input: input as u8,
                increase: true,
            });
        }
        if contains_point(buttons[2], point) {
            return Some(Intent::OpenPreampModeSelector(input as u8));
        }
        if contains_point(buttons[3], point) {
            return Some(Intent::TogglePreampPhantom(input as u8));
        }
        if contains_point(buttons[4], point) {
            return Some(Intent::TogglePreampPhase(input as u8));
        }
        return Some(Intent::SelectPreampInput(input));
    }
    None
}

fn preamp_slider_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let layout = preamp_bar_layout(area);
    for (input, card) in layout.into_iter().enumerate() {
        if !contains_point(card, point) {
            continue;
        }
        let input_state = if input == 0 {
            state.preamp.state.input1
        } else {
            state.preamp.state.input2
        };
        return preamp_card_slider_mouse_action(card, input as u8, input_state, point);
    }
    None
}

fn preamp_slider_wheel_action(
    area: Rect,
    _state: &AppState,
    point: (u16, u16),
    increase: bool,
) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let layout = preamp_bar_layout(area);
    for (input, card) in layout.into_iter().enumerate() {
        if !contains_point(card, point) {
            continue;
        }
        let track = preamp_gain_slider_rect(card);
        if contains_point(wheel_hitbox(track), point) {
            return Some(Intent::AdjustPreampGain {
                input: input as u8,
                increase,
            });
        }
        return None;
    }
    None
}

fn output_list_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }

    if contains_point(output_hotkeys_button_rect(area), point) {
        return Some(Intent::ToggleHotkeysPopup);
    }

    let inner = inner_area(area);
    if point.1 < inner.y {
        return None;
    }

    let (index, card) = output_card_areas(inner)
        .into_iter()
        .enumerate()
        .find(|(_, card)| contains_point(*card, point))?;
    state.output.states.get(index)?;
    let controls = output_control_rects(card);

    if let Some(action) = output_card_slider_mouse_action(card, index, point) {
        return Some(action);
    }

    if contains_point(controls[0], point) {
        return Some(Intent::AdjustOutputLevel {
            index,
            increase: false,
        });
    }
    if contains_point(controls[1], point) {
        return Some(Intent::AdjustOutputLevel {
            index,
            increase: true,
        });
    }
    if contains_point(controls[2], point) {
        return Some(Intent::ToggleOutputDim(index));
    }
    if contains_point(controls[3], point) {
        return Some(Intent::ToggleOutputMute(index));
    }

    Some(Intent::SelectOutput(index))
}

fn output_list_slider_mouse_action(
    area: Rect,
    _state: &AppState,
    point: (u16, u16),
) -> Option<Intent> {
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
    output_card_slider_mouse_action(card, index, point)
}

fn output_list_slider_wheel_action(
    area: Rect,
    _state: &AppState,
    point: (u16, u16),
    increase: bool,
) -> Option<Intent> {
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
    let track = output_level_slider_rect(card);
    contains_point(wheel_hitbox(track), point)
        .then_some(Intent::AdjustOutputLevel { index, increase })
}

pub(crate) fn mixer_control_button_rects(area: Rect, has_link: bool) -> Vec<Rect> {
    let inner = mixer_strip_inner_area(area);
    let y = inner.y + inner.height.saturating_sub(1);
    if has_link {
        centered_inline_chip_rects(Rect::new(inner.x, y, inner.width, 1), &["L", "S", "M"])
    } else {
        centered_inline_chip_rects(Rect::new(inner.x, y, inner.width, 1), &["S", "M"])
    }
}

pub fn mixer_strip_viewport_capacity(area: Rect, _state: &AppState) -> usize {
    // Replicate the layout chain to get the actual inner width of the mixer strip panel.
    // root_chunks -> mixer_page_layout -> mixer_main_layout -> mixer_layout -> mixer_strip_panel_layout
    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout(page[0]);
    let mixer = mixer_layout(main[1]);
    let inner = mixer_strip_inner_area(mixer[1]);
    mixer_strip_viewport_capacity_for_inner(inner)
}

pub fn mixer_strip_panel_contains(area: Rect, state: &AppState, x: u16, y: u16) -> bool {
    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout(page[0]);
    let mixer = mixer_layout(main[1]);
    let list = mixer_strip_panel_layout(mixer[1], mix_meter(state).is_some());
    contains_point(list[0], (x, y))
}

fn output_card_slider_mouse_action(area: Rect, index: usize, point: (u16, u16)) -> Option<Intent> {
    let track = output_level_slider_rect(area);
    let ratio = slider_ratio_for_horizontal_point(track, point)?;
    Some(Intent::SetOutputLevel {
        index,
        step: output_step_from_ratio(ratio).min(0x60),
    })
}

fn preamp_card_slider_mouse_action(
    area: Rect,
    input: u8,
    input_state: antelope_protocol::PreampInputState,
    point: (u16, u16),
) -> Option<Intent> {
    let track = preamp_gain_slider_rect(area);
    let ratio = slider_ratio_for_horizontal_point(track, point)?;
    Some(Intent::SetPreampGain {
        input,
        raw: preamp_gain_from_ratio(input_state, ratio)?,
    })
}

fn mixer_strip_slider_mouse_action(
    area: Rect,
    index: usize,
    _channel: &antelope_protocol::MixerChannelState,
    point: (u16, u16),
) -> Option<Intent> {
    let pan = mixer_pan_slider_rect(area);
    if let Some(ratio) = slider_ratio_for_horizontal_point(pan, point) {
        return Some(Intent::SetMixerPan {
            index,
            pan: pan_from_ratio(ratio),
        });
    }

    let level = mixer_level_slider_rect(area);
    let ratio = slider_ratio_for_vertical_point(level, point)?;
    Some(Intent::SetMixerLevel {
        index,
        level: mixer_level_from_ratio(ratio).min(0x5a),
    })
}

fn mixer_strip_slider_wheel_action(
    area: Rect,
    index: usize,
    _channel: &antelope_protocol::MixerChannelState,
    point: (u16, u16),
    increase: bool,
) -> Option<Intent> {
    let pan = mixer_pan_slider_rect(area);
    if contains_point(wheel_hitbox(pan), point) {
        return Some(Intent::AdjustMixerPan {
            index,
            right: increase,
        });
    }

    let level = mixer_level_slider_rect(area);
    contains_point(wheel_hitbox(level), point)
        .then_some(Intent::AdjustMixerLevel { index, increase })
}

fn afx_routing_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
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
            return Some(Intent::OpenAssignmentPicker((left_index + 1) as u8));
        }
        if contains_point(rects[4], point) {
            return Some(Intent::OpenAssignmentPicker((right_index + 1) as u8));
        }
        if point.0 < rects[3].x {
            return Some(Intent::SelectMixerChannel(left_index));
        }
        return Some(Intent::SelectMixerChannel(right_index));
    }

    None
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

pub(crate) fn mix_meter(state: &AppState) -> Option<(&'static str, u8, u8)> {
    let bytes = state.raw_view.latest_raw_73.as_deref()?;
    let payload = bytes.get(SNAPSHOT_PAYLOAD_OFFSET..)?;

    match payload.get(OFFSET_SURFACE_SELECTOR).copied() {
        Some(SURFACE_CODE_MONITOR_HP1) => Some((
            "MIX 1",
            payload.get(OFFSET_MIX1_LANE_A).copied().unwrap_or(0),
            payload.get(OFFSET_MIX1_LANE_B).copied().unwrap_or(0),
        )),
        Some(SURFACE_CODE_HP2) => Some((
            "MIX 2",
            payload.get(OFFSET_MIX2_LANE_A).copied().unwrap_or(0),
            payload.get(OFFSET_MIX2_LANE_B).copied().unwrap_or(0),
        )),
        _ => None,
    }
}
