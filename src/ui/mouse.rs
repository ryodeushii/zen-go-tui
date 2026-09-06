use ratatui::layout::Rect;

use crate::app::{
    AppState, AssignmentPickerState, Intent, RawMapScope, RawPacketTab, SelectorPopupKind,
    SelectorPopupState, QUERY_REPLY_VISIBLE_COUNT,
};
use crate::device::DevicePickerState;
use antelope_protocol::{
    ClockSource, DynamicMeterState, GlobalControl, InputControl, MixerAddress, MixerAssignment,
    MixerControl, OutputControl, PreampMode, RuntimeMeterTarget, SampleRate,
};

use super::layouts::*;
use super::styles::section_block;

fn any_modal_popup_open(state: &AppState) -> bool {
    state.popup.hotkeys_open
        || state.popup.profiles_open
        || state.popup.selector_popup.is_some()
        || state.popup.assignment_picker.is_some()
        || state.popup.routing_open
        || state.popup.options_open
}

pub fn device_picker_activation_row(
    area: Rect,
    picker: &DevicePickerState,
    x: u16,
    y: u16,
) -> Option<usize> {
    device_picker_row_areas(area, picker.entries().len())
        .iter()
        .position(|row| contains_point(*row, (x, y)))
        .filter(|row| picker.entries()[*row].is_selectable())
}

fn contains_point(area: Rect, point: (u16, u16)) -> bool {
    point.0 >= area.x
        && point.0 < area.x.saturating_add(area.width)
        && point.1 >= area.y
        && point.1 < area.y.saturating_add(area.height)
}

fn intent_is_available(state: &AppState, intent: &Intent) -> bool {
    if !intent.writes_hardware() {
        return true;
    }
    match intent {
        Intent::AdjustOutputLevel { index, .. } | Intent::SetOutputLevel { index, .. } => {
            state.outputs().get(*index).is_some_and(|output| {
                state
                    .ui_profile
                    .supports_output(output.address, OutputControl::Level)
            })
        }
        Intent::ToggleOutputMute(index) => state.outputs().get(*index).is_some_and(|output| {
            state
                .ui_profile
                .supports_output(output.address, OutputControl::Mute)
        }),
        Intent::ToggleOutputDim(index) => state.outputs().get(*index).is_some_and(|output| {
            state
                .ui_profile
                .supports_output(output.address, OutputControl::Dim)
        }),
        Intent::AdjustMixerLevel { .. } | Intent::SetMixerLevel { .. } => {
            active_surface_supports(state, MixerControl::Fader)
        }
        Intent::AdjustMixerPan { .. } | Intent::SetMixerPan { .. } => {
            active_surface_supports(state, MixerControl::Pan)
        }
        Intent::ToggleMixerMute(_) => active_surface_supports(state, MixerControl::Mute),
        Intent::ToggleMixerSolo(_) => active_surface_supports(state, MixerControl::Solo),
        Intent::ToggleMixerLink(_) => state
            .active_mixer_surface()
            .and_then(|index| state.mixers().get(index))
            .is_some_and(|surface| state.ui_profile.supports_link(surface.surface)),
        Intent::OpenAssignmentPicker(strip) | Intent::PickAssignment { strip, .. } => {
            *strip > 0
                && antelope_protocol::MixerStrip::assignment_write_is_grounded(*strip)
                && state
                    .active_mixer_surface()
                    .and_then(|index| state.mixers().get(index))
                    .and_then(|mixer| {
                        mixer
                            .strips
                            .get(state.mixer.selected_channel)
                            .map(|strip| (mixer.surface, strip.strip))
                    })
                    .is_some_and(|(surface, strip)| {
                        state.routing_assignment_available(surface, strip)
                    })
        }
        Intent::OpenAssignmentPickerAt { address }
        | Intent::PickAssignmentAt { address, .. }
        | Intent::PickRoutingSourceAt { address, .. } => {
            address.strip > 0
                && state
                    .mixers()
                    .iter()
                    .find(|surface| surface.surface == address.surface)
                    .is_some_and(|surface| {
                        surface
                            .strips
                            .iter()
                            .any(|strip| strip.strip == address.strip)
                    })
                && antelope_protocol::MixerStrip::assignment_write_is_grounded(
                    u8::try_from(address.strip).unwrap_or(0),
                )
                && state.routing_assignment_available(address.surface, address.strip)
        }
        Intent::AdjustMixerLevelAt { address, .. } | Intent::SetMixerLevelAt { address, .. } => {
            state
                .ui_profile
                .supports_mixer(address.surface, MixerControl::Fader)
        }
        Intent::AdjustMixerPanAt { address, .. } | Intent::SetMixerPanAt { address, .. } => state
            .ui_profile
            .supports_mixer(address.surface, MixerControl::Pan),
        Intent::SetMixerSendAt { address, .. } => state
            .ui_profile
            .supports_mixer(address.surface, MixerControl::Send),
        Intent::ToggleMixerMuteAt { address } => state
            .ui_profile
            .supports_mixer(address.surface, MixerControl::Mute),
        Intent::ToggleMixerSoloAt { address } => state
            .ui_profile
            .supports_mixer(address.surface, MixerControl::Solo),
        Intent::ToggleMixerLinkAt { address } => state.ui_profile.supports_link(address.surface),
        Intent::AdjustPreampGain { input, .. } | Intent::SetPreampGain { input, .. } => {
            input_supports(state, *input, InputControl::Gain)
        }
        Intent::CyclePreampMode(input) | Intent::PickPreampMode { input, .. } => {
            input_supports(state, *input, InputControl::Mode)
        }
        Intent::TogglePreampPhase(input) => input_supports(state, *input, InputControl::Phase),
        Intent::TogglePreampPhantom(input) => input_supports(state, *input, InputControl::Phantom),
        Intent::AdjustInputGainAt { address, .. } | Intent::SetInputGainAt { address, .. } => state
            .ui_profile
            .supports_input(*address, InputControl::Gain),
        Intent::AdjustInputParameterAt {
            address,
            parameter_id,
            ..
        }
        | Intent::SetInputParameterAt {
            address,
            parameter_id,
            ..
        } => state
            .ui_profile
            .supports_input(*address, InputControl::Parameter(*parameter_id)),
        Intent::CycleInputModeAt { address } | Intent::SetInputModeAt { address, .. } => state
            .ui_profile
            .supports_input(*address, InputControl::Mode),
        Intent::ToggleInputPhaseAt { address } => state
            .ui_profile
            .supports_input(*address, InputControl::Phase),
        Intent::ToggleInputPhantomAt { address } => state
            .ui_profile
            .supports_input(*address, InputControl::Phantom),
        Intent::PickSampleRate(_) => state.ui_profile.supports_global(GlobalControl::SampleRate),
        Intent::PickClockSource(_) => state.ui_profile.supports_global(GlobalControl::ClockSource),
        Intent::SelectSurface(_) => state.ui_profile.supports_global(GlobalControl::Surface),
        Intent::AdjustFocused(_) | Intent::ToggleFocusedMute | Intent::ToggleFocusedDim => {
            state.ui_profile.actionable
        }
        _ => state.ui_profile.actionable,
    }
}

fn active_surface_supports(state: &AppState, control: MixerControl) -> bool {
    state
        .active_mixer_surface()
        .and_then(|index| state.mixers().get(index))
        .is_some_and(|surface| state.ui_profile.supports_mixer(surface.surface, control))
}

fn input_supports(state: &AppState, input: u8, control: InputControl) -> bool {
    state
        .input_spaces
        .iter()
        .flat_map(|space| space.inputs.iter())
        .find(|slot| slot.address.space == 0 && slot.address.index == u16::from(input))
        .is_some_and(|slot| state.ui_profile.supports_input(slot.address, control))
}

pub fn mouse_action(area: Rect, state: &AppState, x: u16, y: u16) -> Option<Intent> {
    mouse_action_unchecked(area, state, x, y).filter(|intent| intent_is_available(state, intent))
}

fn mouse_action_unchecked(area: Rect, state: &AppState, x: u16, y: u16) -> Option<Intent> {
    let point = (x, y);
    let chunks = root_chunks(area);

    if state.popup.hotkeys_open {
        return Some(Intent::ToggleHotkeysPopup);
    }

    if let Some(action) = device_header_mouse_action(area, state, point) {
        return Some(action);
    }

    if state.popup.raw_view_open {
        return raw_mouse_action(area, state, point);
    }

    if contains_point(titlebar_layout(chunks[0])[1], point) {
        return system_panel_mouse_action(titlebar_layout(chunks[0])[1], state, point);
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
        return assignment_picker_mouse_action(
            area,
            state,
            picker,
            state.popup.assignment_picker_address,
            point,
        );
    }

    if let Some(picker) = state.popup.routing_source_picker {
        return routing_source_picker_mouse_action(area, state, picker, point);
    }

    if state.popup.routing_open {
        return routing_popup_mouse_action(area, state, point);
    }

    if state.popup.options_open {
        return options_popup_mouse_action(area, state, point);
    }

    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout_for_state(page[0], state);
    let mixer_sections = mixer_layout(main[1]);

    if let Some(action) = output_list_mouse_action(page[1], state, point) {
        return Some(action);
    }

    if let Some(action) = mixer_tab_mouse_action(mixer_sections[0], state, point) {
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
    slider_mouse_action_unchecked(area, state, x, y)
        .filter(|intent| intent_is_available(state, intent))
}

fn slider_mouse_action_unchecked(area: Rect, state: &AppState, x: u16, y: u16) -> Option<Intent> {
    if any_modal_popup_open(state) || state.popup.raw_view_open {
        return None;
    }

    let point = (x, y);
    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout_for_state(page[0], state);
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
    slider_wheel_action_unchecked(area, state, x, y, increase)
        .filter(|intent| intent_is_available(state, intent))
}

fn slider_wheel_action_unchecked(
    area: Rect,
    state: &AppState,
    x: u16,
    y: u16,
    increase: bool,
) -> Option<Intent> {
    if any_modal_popup_open(state) {
        return None;
    }

    let point = (x, y);

    if state.popup.raw_view_open {
        if state.raw_view.selected_tab == RawPacketTab::Query75 {
            if let Some(action) = query_reply_wheel_action(area, state, point, increase) {
                return Some(action);
            }
        }
        return raw_dump_wheel_action(area, state, point, increase);
    }

    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout_for_state(page[0], state);
    let mixer_sections = mixer_layout(main[1]);

    output_list_slider_wheel_action(page[1], state, point, increase)
        .or_else(|| mixer_list_slider_wheel_action(mixer_sections[1], state, point, increase))
        .or_else(|| preamp_slider_wheel_action(main[0], state, point, increase))
}

fn routing_popup_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    let popup = dynamic_routing_popup_area(area, state);
    if !contains_point(popup, point) {
        return Some(Intent::CloseRoutingPopup);
    }
    if state.ui_profile.driver_kind == antelope_protocol::RuntimeDriverKind::ZenGo {
        return if state.routing_capabilities.is_empty() && state.ui_profile.supports_any_routing() {
            // Compatibility-only AFX layout for legacy state with no profile topology.
            afx_routing_mouse_action(popup, state, point)
        } else {
            None
        };
    }

    let inner = routing_editor_inner_area(popup);
    if point.1 <= inner.y {
        return None;
    }
    let viewport = routing_destination_viewport(popup, state);
    let visible_row = usize::from(point.1.saturating_sub(inner.y).saturating_sub(1));
    let capability_index = viewport.start.checked_add(visible_row)?;
    if capability_index >= viewport.end {
        return None;
    }
    let capability = state.routing_capabilities.get(capability_index)?;
    let row = Rect::new(
        inner.x,
        inner.y.saturating_add(1).saturating_add(visible_row as u16),
        inner.width,
        1,
    );
    if !contains_point(row, point) {
        return None;
    }
    let Some(editor) = state
        .popup
        .routing_editor
        .filter(|editor| editor.destination == capability.destination)
    else {
        return Some(Intent::SelectRoutingDestination {
            destination: capability.destination,
        });
    };
    let rects = routing_editor_row_rects(row);
    if contains_point(rects.previous_channel, point) {
        return Some(Intent::SelectRoutingChannel {
            destination: capability.destination,
            channel: editor.channel.saturating_sub(1),
        });
    }
    if contains_point(rects.next_channel, point) {
        let channel = editor
            .channel
            .saturating_add(1)
            .min(capability.channel_count.saturating_sub(1));
        return Some(Intent::SelectRoutingChannel {
            destination: capability.destination,
            channel,
        });
    }
    if contains_point(rects.source, point)
        && state.general_routing_channel_available(capability.destination, editor.channel)
    {
        return Some(Intent::OpenRoutingSourcePicker {
            destination: capability.destination,
            channel: editor.channel,
        });
    }
    Some(Intent::SelectRoutingDestination {
        destination: capability.destination,
    })
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
        let start_x = refresh_row.x + prefix.chars().count() as u16;
        let labels: Vec<String> = refresh_rates
            .iter()
            .map(|r| {
                if *r == state.ui.settings.refresh_rate {
                    format!("* {}", r.label())
                } else {
                    r.label().to_string()
                }
            })
            .collect();
        let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let rects = inline_chip_rects(start_x, refresh_row.y, &label_refs);
        for (i, rect) in rects.into_iter().enumerate() {
            if contains_point(rect, point) {
                return Some(Intent::SetRefreshRate(refresh_rates[i]));
            }
        }
    }

    if contains_point(peak_threshold_row, point) {
        let status_text = if state.ui.settings.peak_enabled {
            format!("ON ({} dB)", state.ui.settings.peak_threshold_db())
        } else {
            "OFF".to_string()
        };
        let prefix = "Peaks:  ";
        let start_x = peak_threshold_row.x + prefix.chars().count() as u16;
        let rects = inline_chip_rects(start_x, peak_threshold_row.y, &[&status_text, "↓", "↑"]);
        if contains_point(rects[1], point) {
            return Some(Intent::CyclePeakThreshold(false));
        }
        if contains_point(rects[2], point) {
            return Some(Intent::CyclePeakThreshold(true));
        }
    }

    if contains_point(peak_toggle_row, point) {
        let label = if state.ui.settings.peak_enabled {
            "Disable"
        } else {
            "Enable"
        };
        let prefix = "Toggle: ";
        let start_x = peak_toggle_row.x + prefix.chars().count() as u16;
        let rects = inline_chip_rects(start_x, peak_toggle_row.y, &[label]);
        if contains_point(rects[0], point) {
            return Some(Intent::TogglePeakEnabled);
        }
    }

    if contains_point(peak_hold_row, point) {
        let hold_durations = crate::app::PeakHoldDuration::all();
        let prefix = "Hold:   ";
        let start_x = peak_hold_row.x + prefix.chars().count() as u16;
        let labels: Vec<String> = hold_durations
            .iter()
            .map(|h| {
                if *h == state.ui.settings.peak_hold_duration {
                    format!("* {}", h.label())
                } else {
                    h.label().to_string()
                }
            })
            .collect();
        let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let rects = inline_chip_rects(start_x, peak_hold_row.y, &label_refs);
        for (i, rect) in rects.into_iter().enumerate() {
            if contains_point(rect, point) {
                return Some(Intent::CyclePeakHoldDuration(hold_durations[i]));
            }
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
    if contains_point(raw_back_button_hit_area(header[1]), point) {
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
    let scopes = raw_scope_hit_areas(layout[2], state.raw_view.selected_tab);
    for (scope, scope_area) in RawMapScope::options_for(state.raw_view.selected_tab)
        .iter()
        .zip(scopes)
    {
        if contains_point(scope_area, point) {
            return Some(Intent::SelectRawMapScope(*scope));
        }
    }

    if state.raw_view.selected_tab == RawPacketTab::Query75 {
        let content = raw_content_layout(layout[3], true);
        let history = content.history()?;
        if !contains_point(history, point) {
            return None;
        }
        let inner = section_block("Recent 0x75 Replies", true).inner(history);
        if !contains_point(inner, point) {
            return None;
        }
        let total = state.raw_view.recent_query_reply_entries.len();
        let list_visible = QUERY_REPLY_VISIBLE_COUNT.min(total);
        let start = state
            .raw_view
            .query_reply_scroll
            .min(total.saturating_sub(list_visible));
        let visible = list_visible.min(usize::from(inner.height));
        if visible == 0 {
            return None;
        }
        let row = point.1.saturating_sub(inner.y) as usize;
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
    let content = raw_content_layout(layout[3], true);
    let history = content.history()?;
    if !contains_point(history, point) {
        return None;
    }
    let total = state.raw_view.recent_query_reply_entries.len();
    if total <= QUERY_REPLY_VISIBLE_COUNT {
        return None;
    }
    Some(Intent::ScrollQueryReplyList { increase })
}

pub(crate) fn raw_dump_wheel_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
    increase: bool,
) -> Option<Intent> {
    if !state.popup.raw_view_open {
        return None;
    }
    let content = raw_content_layout(
        raw_page_layout(area)[3],
        state.raw_view.selected_tab == RawPacketTab::Query75,
    );
    (contains_point(content.map(), point) || contains_point(content.dump(), point)).then_some(
        Intent::ScrollRawDump {
            increase,
            page: false,
        },
    )
}

fn assignment_picker_mouse_action(
    area: Rect,
    state: &AppState,
    picker: AssignmentPickerState,
    address: Option<MixerAddress>,
    point: (u16, u16),
) -> Option<Intent> {
    let popup = assignment_picker_area(area);
    if !contains_point(popup, point) {
        return Some(Intent::CloseAssignmentPicker);
    }

    let title = format!("Assign CH {:02}", picker.strip);
    let inner = popup_list_inner_area(popup, &title);
    if !contains_point(inner, point) {
        return None;
    }
    let profile_choices = address
        .map(|address| state.routing_source_choices(address.surface))
        .unwrap_or_default();
    let item_count = if profile_choices.is_empty() {
        MixerAssignment::grounded_choices().len()
    } else {
        profile_choices.len()
    };
    let viewport = popup_list_viewport(popup, &title, item_count, state.popup.selected_index);
    let index = viewport
        .start
        .saturating_add(point.1.saturating_sub(inner.y) as usize);
    if !profile_choices.is_empty() {
        let choice = profile_choices.get(index)?;
        return Some(Intent::PickRoutingSourceAt {
            address: address?,
            source: choice.source,
        });
    }
    let assignment = *MixerAssignment::grounded_choices().get(index)?;
    address.map_or(
        Some(Intent::PickAssignment {
            strip: picker.strip,
            assignment,
        }),
        |address| {
            Some(Intent::PickAssignmentAt {
                address,
                assignment,
            })
        },
    )
}

fn routing_source_picker_mouse_action(
    area: Rect,
    state: &AppState,
    picker: crate::app::RoutingSourcePickerState,
    point: (u16, u16),
) -> Option<Intent> {
    let popup = assignment_picker_area(area);
    if !contains_point(popup, point) {
        return Some(Intent::CloseRoutingSourcePicker);
    }
    let title = routing_source_picker_title(state, picker.destination, picker.channel);
    let inner = popup_list_inner_area(popup, &title);
    if !contains_point(inner, point) {
        return None;
    }
    let choices = state.routing_source_choices_for_destination(picker.destination);
    let viewport = popup_list_viewport(popup, &title, choices.len(), state.popup.selected_index);
    let index = viewport
        .start
        .saturating_add(usize::from(point.1.saturating_sub(inner.y)));
    let choice = choices.get(index)?;
    Some(Intent::PickRoutingSource {
        destination: picker.destination,
        channel: picker.channel,
        source: choice.source,
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
    // panel_block has 1-cell borders; chips render at x+1, y+1.
    let x = area.x.saturating_add(1);
    let y = area.y.saturating_add(1);
    let labels = ["RAW", "OPTNS", "X"];
    let rects = inline_chip_rects(x, y, &labels);
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

pub fn device_header_name_hit(area: Rect, state: &AppState, x: u16, y: u16) -> bool {
    contains_point(
        device_header_name_hit_area(device_header_area(area), state),
        (x, y),
    )
}

fn device_header_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    let area = device_header_area(area);
    if !contains_point(area, point) {
        return None;
    }
    let geometry = device_header_geometry(area, state);
    if contains_point(geometry.sample_rate, point) {
        if state.device.status.clock_source == Some(ClockSource::Internal) {
            Some(Intent::OpenSampleRateSelector)
        } else {
            None
        }
    } else if contains_point(geometry.clock_source, point) {
        Some(Intent::OpenClockSourceSelector)
    } else {
        None
    }
}

fn mixer_tab_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
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
    let tabs = dynamic_surface_tab_hit_areas(area, state);
    state
        .mixers()
        .iter()
        .zip(tabs)
        .find(|(_, rect)| contains_point(*rect, point))
        .map(|(surface, _)| Intent::SelectMixerSurface {
            surface: surface.surface,
        })
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

fn dynamic_mixer_control_action(
    controls: DynamicMixerControlRects,
    address: antelope_protocol::MixerAddress,
    point: (u16, u16),
    wheel: Option<bool>,
    semantics: Option<antelope_protocol::FaderSemantics>,
    pan_range: Option<(i32, i32)>,
    send_range: Option<(i32, i32)>,
) -> Option<Intent> {
    if let Some(increase) = wheel {
        if controls
            .pan
            .is_some_and(|rect| contains_point(wheel_hitbox(rect), point))
        {
            return Some(Intent::AdjustMixerPanAt {
                address,
                right: increase,
            });
        }
        if controls
            .fader
            .is_some_and(|rect| contains_point(wheel_hitbox(rect), point))
        {
            return Some(Intent::AdjustMixerLevelAt { address, increase });
        }
        return None;
    }
    if let Some(rect) = controls.pan {
        if let Some(ratio) = slider_ratio_for_horizontal_point(rect, point) {
            return Some(Intent::SetMixerPanAt {
                address,
                pan: pan_from_ratio(ratio, pan_range?),
            });
        }
    }
    if let Some(rect) = controls.fader {
        if let Some(ratio) = slider_ratio_for_vertical_point(rect, point) {
            return Some(Intent::SetMixerLevelAt {
                address,
                level: mixer_level_from_ratio(ratio, semantics?),
            });
        }
    }
    if let Some(rect) = controls.send {
        if let Some(ratio) = slider_ratio_for_horizontal_point(rect, point) {
            return Some(Intent::SetMixerSendAt {
                address,
                send: value_from_ratio(ratio, send_range?),
            });
        }
    }
    if controls
        .link
        .is_some_and(|rect| contains_point(rect, point))
    {
        return Some(Intent::ToggleMixerLinkAt { address });
    }
    if controls
        .solo
        .is_some_and(|rect| contains_point(rect, point))
    {
        return Some(Intent::ToggleMixerSoloAt { address });
    }
    if controls
        .mute
        .is_some_and(|rect| contains_point(rect, point))
    {
        return Some(Intent::ToggleMixerMuteAt { address });
    }
    None
}

fn dynamic_mixer_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
    wheel: Option<bool>,
) -> Option<Intent> {
    let inner = mixer_strip_panel_layout_for_meter_lanes(area, mix_meter_lane_count(state))[0];
    let surface = state
        .active_mixer_surface()
        .and_then(|index| state.mixers().get(index))?;
    if let (Some(master), Some(card)) = (surface.master.as_ref(), mixer_master_area(inner, state)) {
        let address = antelope_protocol::MixerAddress {
            surface: surface.surface,
            strip: master.strip,
        };
        let controls = dynamic_mixer_control_rects(card, state, address)?;
        if contains_point(card, point) {
            if wheel.is_none()
                && controls
                    .source
                    .is_some_and(|rect| contains_point(rect, point))
            {
                return Some(Intent::OpenAssignmentPickerAt { address });
            }
            return dynamic_mixer_control_action(
                controls,
                address,
                point,
                wheel,
                state.mixer_fader(address.surface),
                state.mixer_range(address.surface, MixerControl::Pan),
                state.mixer_range(address.surface, MixerControl::Send),
            );
        }
    }
    let strip_area = mixer_input_strip_area(inner, state);
    let (start, end) = mixer_strip_visible_bounds(inner, state);
    for (slot, index) in (start..end).enumerate() {
        let strip = surface.strips.get(index)?;
        let card =
            dynamic_mixer_strip_card_area(strip_area, state, slot, end.saturating_sub(start));
        if !contains_point(card, point) {
            continue;
        }
        let address = antelope_protocol::MixerAddress {
            surface: surface.surface,
            strip: strip.strip,
        };
        let controls = dynamic_mixer_control_rects(card, state, address)?;
        if wheel.is_none()
            && controls
                .source
                .is_some_and(|rect| contains_point(rect, point))
        {
            return Some(Intent::OpenAssignmentPickerAt { address });
        }
        return dynamic_mixer_control_action(
            controls,
            address,
            point,
            wheel,
            state.mixer_fader(address.surface),
            state.mixer_range(address.surface, MixerControl::Pan),
            state.mixer_range(address.surface, MixerControl::Send),
        )
        .or_else(|| wheel.is_none().then_some(Intent::SelectMixerChannel(index)));
    }
    None
}

pub(crate) fn mixer_list_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    dynamic_mixer_mouse_action(area, state, point, None)
}

pub(crate) fn mixer_list_slider_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    dynamic_mixer_mouse_action(area, state, point, None).filter(|intent| {
        matches!(
            intent,
            Intent::SetMixerLevelAt { .. }
                | Intent::SetMixerPanAt { .. }
                | Intent::SetMixerSendAt { .. }
        )
    })
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
    dynamic_mixer_mouse_action(area, state, point, Some(increase))
}

fn dynamic_input_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    for (space_index, input_index, row) in dynamic_input_rows(area, state) {
        if !contains_point(row, point) {
            continue;
        }
        let input = state
            .input_spaces
            .get(space_index)?
            .inputs
            .get(input_index)?;
        let controls = dynamic_input_control_rects(row, state, space_index, input_index)?;
        if row.height >= 3 {
            if let Some(gain) = controls.gain {
                if let Some(ratio) = slider_ratio_for_horizontal_point(gain, point) {
                    let range = state.input_range(input.address, input.mode)?;
                    return Some(Intent::SetInputGainAt {
                        address: input.address,
                        raw: preamp_gain_from_ratio(range, ratio)?,
                    });
                }
            }
            let buttons = dynamic_preamp_button_rects(row, state, input);
            if buttons
                .first()
                .is_some_and(|(_, rect)| contains_point(*rect, point))
                && state.input_range(input.address, input.mode).is_some()
            {
                return Some(Intent::AdjustInputGainAt {
                    address: input.address,
                    increase: false,
                });
            }
            if buttons
                .get(1)
                .is_some_and(|(_, rect)| contains_point(*rect, point))
                && state.input_range(input.address, input.mode).is_some()
            {
                return Some(Intent::AdjustInputGainAt {
                    address: input.address,
                    increase: true,
                });
            }
            if controls
                .mode
                .is_some_and(|rect| contains_point(rect, point))
            {
                return Some(Intent::CycleInputModeAt {
                    address: input.address,
                });
            }
            if controls
                .phantom
                .is_some_and(|rect| contains_point(rect, point))
            {
                return Some(Intent::ToggleInputPhantomAt {
                    address: input.address,
                });
            }
            if controls
                .phase
                .is_some_and(|rect| contains_point(rect, point))
            {
                return Some(Intent::ToggleInputPhaseAt {
                    address: input.address,
                });
            }
            return Some(Intent::SelectPreampInput(input_index));
        }
        if let Some(gain) = controls.gain {
            if let Some(ratio) = slider_ratio_for_horizontal_point(gain, point) {
                let value =
                    preamp_gain_from_ratio(state.input_range(input.address, input.mode)?, ratio)?;
                let control = state
                    .ui_profile
                    .input_capabilities(input.address)
                    .iter()
                    .find(|capability| {
                        capability.kind == antelope_protocol::RuntimeInputControlKind::Gain
                    })
                    .and_then(|capability| capability.control)?;
                return Some(match control {
                    InputControl::Gain => Intent::SetInputGainAt {
                        address: input.address,
                        raw: value,
                    },
                    InputControl::Parameter(parameter_id) => Intent::SetInputParameterAt {
                        address: input.address,
                        parameter_id,
                        value,
                    },
                    _ => return None,
                });
            }
        }
        if controls
            .mode
            .is_some_and(|rect| contains_point(rect, point))
        {
            return Some(Intent::CycleInputModeAt {
                address: input.address,
            });
        }
        if controls
            .phantom
            .is_some_and(|rect| contains_point(rect, point))
        {
            return Some(Intent::ToggleInputPhantomAt {
                address: input.address,
            });
        }
        if controls
            .phase
            .is_some_and(|rect| contains_point(rect, point))
        {
            return Some(Intent::ToggleInputPhaseAt {
                address: input.address,
            });
        }
    }
    None
}

fn preamp_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    dynamic_input_mouse_action(area, state, point)
}

fn preamp_slider_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    dynamic_input_mouse_action(area, state, point).filter(|intent| {
        matches!(
            intent,
            Intent::SetInputGainAt { .. } | Intent::SetInputParameterAt { .. }
        )
    })
}

fn preamp_slider_wheel_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
    increase: bool,
) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    for (space_index, input_index, row) in dynamic_input_rows(area, state) {
        let input = state
            .input_spaces
            .get(space_index)?
            .inputs
            .get(input_index)?;
        let controls = dynamic_input_control_rects(row, state, space_index, input_index)?;
        if row.height >= 3 {
            if controls
                .gain
                .is_some_and(|rect| contains_point(wheel_hitbox(rect), point))
                && state.input_range(input.address, input.mode).is_some()
            {
                return Some(Intent::AdjustInputGainAt {
                    address: input.address,
                    increase,
                });
            }
            continue;
        }
        if controls
            .gain
            .is_some_and(|rect| contains_point(wheel_hitbox(rect), point))
        {
            let control = state
                .ui_profile
                .input_capabilities(input.address)
                .iter()
                .find(|capability| {
                    capability.kind == antelope_protocol::RuntimeInputControlKind::Gain
                })
                .and_then(|capability| capability.control)?;
            return Some(match control {
                InputControl::Gain => Intent::AdjustInputGainAt {
                    address: input.address,
                    increase,
                },
                InputControl::Parameter(parameter_id) => Intent::AdjustInputParameterAt {
                    address: input.address,
                    parameter_id,
                    increase,
                },
                _ => return None,
            });
        }
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
    for (index, row) in dynamic_output_card_areas(inner, state.outputs().len())
        .into_iter()
        .enumerate()
    {
        let controls = dynamic_output_control_rects(row, state, index)?;
        if let Some(level) = controls.level {
            if let Some(ratio) = slider_ratio_for_horizontal_point(level, point) {
                return Some(Intent::SetOutputLevel {
                    index,
                    step: output_step_from_ratio(
                        ratio,
                        state.output_semantics(OutputControl::Level)?,
                    ),
                });
            }
        }
        if controls.dim.is_some_and(|rect| contains_point(rect, point)) {
            return Some(Intent::ToggleOutputDim(index));
        }
        if controls
            .mute
            .is_some_and(|rect| contains_point(rect, point))
        {
            return Some(Intent::ToggleOutputMute(index));
        }
        if contains_point(row, point) {
            return Some(Intent::SelectOutput(index));
        }
    }
    None
}

fn output_list_slider_mouse_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let inner = inner_area(area);
    for (index, row) in dynamic_output_card_areas(inner, state.outputs().len())
        .into_iter()
        .enumerate()
    {
        let controls = dynamic_output_control_rects(row, state, index)?;
        if let Some(level) = controls.level {
            if let Some(ratio) = slider_ratio_for_horizontal_point(level, point) {
                return Some(Intent::SetOutputLevel {
                    index,
                    step: output_step_from_ratio(
                        ratio,
                        state.output_semantics(OutputControl::Level)?,
                    ),
                });
            }
        }
    }
    None
}

fn output_list_slider_wheel_action(
    area: Rect,
    state: &AppState,
    point: (u16, u16),
    increase: bool,
) -> Option<Intent> {
    if !contains_point(area, point) {
        return None;
    }
    let inner = inner_area(area);
    for (index, row) in dynamic_output_card_areas(inner, state.outputs().len())
        .into_iter()
        .enumerate()
    {
        let controls = dynamic_output_control_rects(row, state, index)?;
        if controls
            .level
            .is_some_and(|rect| contains_point(wheel_hitbox(rect), point))
        {
            return Some(Intent::AdjustOutputLevel { index, increase });
        }
    }
    None
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

pub fn mixer_strip_viewport_capacity(area: Rect, state: &AppState) -> usize {
    // Replicate the layout chain to get the actual inner width of the mixer strip panel.
    // root_chunks -> mixer_page_layout -> mixer_main_layout -> mixer_layout -> mixer_strip_panel_layout
    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout_for_state(page[0], state);
    let mixer = mixer_layout(main[1]);
    let inner = mixer_strip_inner_area(mixer[1]);
    mixer_strip_viewport_capacity_for_state(inner, state)
}

pub fn mixer_strip_panel_contains(area: Rect, state: &AppState, x: u16, y: u16) -> bool {
    let chunks = root_chunks(area);
    let page = mixer_page_layout(chunks[1]);
    let main = mixer_main_layout_for_state(page[0], state);
    let mixer = mixer_layout(main[1]);
    let list = mixer_strip_panel_layout_for_meter_lanes(mixer[1], mix_meter_lane_count(state));
    contains_point(list[0], (x, y))
}

fn afx_routing_mouse_action(area: Rect, state: &AppState, point: (u16, u16)) -> Option<Intent> {
    if state.active_mixer_channels().len() < 8 || !contains_point(area, point) {
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
            return Some(Intent::OpenAssignmentPickerAt {
                address: MixerAddress {
                    surface: state.active_mixer_surface()? as u8,
                    strip: (left_index + 1) as u16,
                },
            });
        }
        if contains_point(rects[4], point) {
            return Some(Intent::OpenAssignmentPickerAt {
                address: MixerAddress {
                    surface: state.active_mixer_surface()? as u8,
                    strip: (right_index + 1) as u16,
                },
            });
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MixMeterState {
    pub(crate) name: String,
    pub(crate) lanes: Vec<DynamicMeterState>,
}

impl MixMeterState {
    pub(crate) fn lane_label(&self, lane: u8) -> String {
        let is_stereo_pair =
            self.lanes.len() == 2 && self.lanes.iter().map(|meter| meter.lane).eq([0, 1]);
        if is_stereo_pair {
            match lane {
                0 => "L".to_string(),
                1 => "R".to_string(),
                _ => format!("Lane {}", lane.saturating_add(1)),
            }
        } else {
            format!("Lane {}", lane.saturating_add(1))
        }
    }
}

pub(crate) fn mix_meter(state: &AppState) -> Option<MixMeterState> {
    let surface = state
        .active_mixer_surface()
        .and_then(|index| state.mixers().get(index))?;
    let mut lanes: Vec<_> = state
        .meters
        .iter()
        .filter(|meter| {
            meter.target == RuntimeMeterTarget::MixMaster
                && meter.target_index == u16::from(surface.surface)
        })
        .cloned()
        .collect();
    lanes.sort_unstable_by_key(|meter| meter.lane);
    (!lanes.is_empty()).then(|| MixMeterState {
        name: surface.name.clone(),
        lanes,
    })
}

pub(crate) fn mix_meter_lane_count(state: &AppState) -> usize {
    mix_meter(state).map_or(0, |meter| meter.lanes.len())
}
