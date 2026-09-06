use std::collections::HashSet;

use antelope_protocol::{
    DynamicMeterState, DynamicRoutingGroup, InputAddress, InputControl, MixerAddress,
    MixerAssignment, MixerControl, OutputControl, RoutingSource, RuntimeDriverKind, RuntimeEntry,
    RuntimeInputControlKind, RuntimeMeterTarget, RuntimeReadiness,
};
use ratatui::{
    backend::TestBackend,
    layout::Rect,
    style::Color,
    widgets::{Block, Borders},
    Terminal,
};

use crate::{
    app::{AppState, Intent},
    device::{DeviceCandidate, DevicePickerState, ProfileCatalog},
};

use super::{
    device_picker_activation_row, draw, draw_device_picker, mouse_action, slider_mouse_action,
    slider_wheel_action,
};

fn entry(id: &str) -> RuntimeEntry {
    ProfileCatalog::builtin()
        .entries()
        .iter()
        .find(|entry| entry.id == id)
        .unwrap_or_else(|| panic!("missing built-in profile {id}"))
        .clone()
}

fn orion_ui_state() -> AppState {
    AppState::from_entry(&entry("orion_studio_3"))
}

fn discrete_4_ui_state() -> AppState {
    AppState::from_entry(&entry("discrete_4"))
}

fn zen_go_ui_state() -> AppState {
    AppState::from_entry(&entry("zen_go_sc"))
}

fn supported_dynamic_state() -> AppState {
    let mut entry = entry("zen_go_sc");
    entry.driver_kind = RuntimeDriverKind::Profile;
    AppState::from_entry(&entry)
}

fn supported_dynamic_state_without(param: &str) -> AppState {
    let mut entry = entry("zen_go_sc");
    entry.driver_kind = RuntimeDriverKind::Profile;
    entry
        .profile
        .params
        .retain(|candidate| candidate.name != param);
    for space in &mut entry.profile.address_spaces {
        space
            .input_capabilities
            .retain(|capability| capability.parameter != param);
    }
    AppState::from_entry(&entry)
}

fn test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(width, height)).expect("test terminal")
}

fn draw_page(terminal: &mut Terminal<TestBackend>, state: &AppState) {
    terminal
        .draw(|frame| draw(frame, state))
        .expect("draw page");
}

fn terminal_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

#[test]
fn typed_mix_master_meter_renders_as_mixer_state_not_physical_output() {
    let mut state = zen_go_ui_state();
    state.meters = vec![DynamicMeterState {
        target: RuntimeMeterTarget::MixMaster,
        target_index: 0,
        lane: 0,
        value: 0x21,
    }];
    let mut terminal = test_terminal(220, 48);
    draw_page(&mut terminal, &state);
    let text = terminal_text(&terminal);

    assert!(text.contains("MIX MASTER"));
    assert!(!text.contains("OUTPUT METER"));
}

#[test]
fn selected_orion_mix_meter_uses_each_profile_mapping_without_stereo_inference() {
    let mut state = orion_ui_state();
    state.meters = (0_u16..4)
        .map(|target_index| DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index,
            lane: 0,
            value: 0x12 + target_index as u8,
        })
        .collect();

    for surface_index in 0..4 {
        state.mixer.surface_index = surface_index as usize;
        let meter = super::mouse::mix_meter(&state).expect("selected Orion meter");
        assert_eq!(meter.lanes.len(), 1);
        assert_eq!(meter.name, state.mixers()[surface_index].name);

        let line = super::render::render_mix_meter_state_line(&state);
        let lane_label = meter.lane_label(0);
        let db = antelope_protocol::meter_display_db(meter.lanes[0].value)
            .expect("fixture meter has a display value");
        assert!(line.contains(&lane_label));
        assert!(line.contains(&format!("{db} dB")));
        assert!(!line.contains(" R "));

        let mut terminal = test_terminal(80, 1);
        terminal
            .draw(|frame| {
                super::render::render_mix_meter_widget(frame.area(), frame.buffer_mut(), &meter)
            })
            .expect("render selected Orion meter");
        let graphical = terminal_text(&terminal);
        assert!(graphical.contains(&lane_label));
        assert!(graphical.contains(&format!("{db} dB")));
        assert!(!graphical.contains(" R "));
    }
}

#[test]
fn physical_output_meter_does_not_render_as_selected_mix_meter() {
    let mut state = zen_go_ui_state();
    state.meters = vec![DynamicMeterState {
        target: RuntimeMeterTarget::PhysicalOutput,
        target_index: 0,
        lane: 0,
        value: 0x12,
    }];

    assert!(super::mouse::mix_meter(&state).is_none());
    assert_eq!(
        super::render::render_mix_meter_state_line(&state),
        "Mix meter: unavailable for selected mixer"
    );
}

#[test]
fn selected_device_title_uses_runtime_profile_name() {
    let state = orion_ui_state();
    let title = super::render::render_device_header(&state).to_string();
    assert!(title.contains("Antelope Orion Studio Synergy Core"));
}

#[test]
fn device_header_name_hit_area_covers_only_device_name() {
    let state = orion_ui_state();
    let area = Rect::new(0, 0, 140, 48);
    let header = super::layouts::device_header_area(area);
    let hit = super::layouts::device_header_name_hit_area(header, &state);

    assert!(super::device_header_name_hit(area, &state, hit.x, hit.y));
    assert!(!super::device_header_name_hit(
        area,
        &state,
        hit.x.saturating_add(hit.width),
        hit.y
    ));
}

#[test]
fn selectable_picker_rows_share_rendered_list_geometry() {
    let catalog = ProfileCatalog::builtin();
    let first = DeviceCandidate::new(
        "zen-a",
        0x23e5,
        0xa015,
        Some("ZEN-A".into()),
        Some("Zen Go".into()),
        0,
        0,
        3,
    );
    let second = DeviceCandidate::new(
        "zen-b",
        0x23e5,
        0xa015,
        Some("ZEN-B".into()),
        Some("Zen Go".into()),
        0,
        0,
        3,
    );
    let picker = DevicePickerState::new(vec![first, second], &catalog);
    let area = Rect::new(0, 0, 100, 30);
    let popup = super::layouts::device_picker_area(area);
    let content = Block::default().borders(Borders::ALL).inner(popup);

    assert_eq!(
        device_picker_activation_row(area, &picker, content.x, content.y),
        Some(0)
    );
    assert_eq!(
        device_picker_activation_row(area, &picker, content.x, content.y + 1),
        Some(1)
    );
    assert!(device_picker_activation_row(area, &picker, popup.x, content.y).is_none());
    assert!(device_picker_activation_row(area, &picker, content.x, content.y + 2).is_none());
}

#[test]
fn device_picker_renders_active_serial_and_path_fallback() {
    let catalog = ProfileCatalog::builtin();
    let active = DeviceCandidate::new(
        "hid-active",
        0x23e5,
        0xa015,
        Some("ZEN-A".into()),
        Some("Zen Go".into()),
        0,
        0,
        3,
    );
    let no_serial = DeviceCandidate::new(
        "hid-no-serial",
        0x23e5,
        0xa015,
        None,
        Some("Zen Go".into()),
        0,
        0,
        3,
    );
    let mut picker = DevicePickerState::new(vec![active.clone(), no_serial], &catalog);
    picker.set_active_candidate(Some(active));
    let mut terminal = test_terminal(100, 30);
    terminal
        .draw(|frame| draw_device_picker(frame, &picker))
        .expect("draw selector");
    let text = terminal_text(&terminal);
    assert!(text.contains("serial ZEN-A"));
    assert!(text.contains("ACTIVE"));
    assert!(text.contains("path hid-no-serial"));
}

#[test]
fn unsupported_picker_row_renders_but_has_no_mouse_activation() {
    let catalog = ProfileCatalog::builtin();
    let candidate = DeviceCandidate::new(
        "orion",
        0x23e5,
        0xbeef,
        Some("UNKNOWN-1".into()),
        Some("Unknown Antelope".into()),
        0,
        0,
        3,
    );
    let picker = DevicePickerState::new(vec![candidate], &catalog);
    let mut terminal = test_terminal(100, 30);
    terminal
        .draw(|frame| draw_device_picker(frame, &picker))
        .expect("draw unsupported picker");
    let text = terminal_text(&terminal);
    assert!(text.contains("Unknown Antelope"));
    assert!(text.contains("unsupported"));
    assert!(device_picker_activation_row(Rect::new(0, 0, 100, 30), &picker, 3, 4,).is_none());
}

fn render_orion_screen() -> String {
    render_to_string(&orion_ui_state())
}

fn available_intents(state: &AppState) -> Vec<Intent> {
    let area = Rect::new(0, 0, 140, 48);
    let mut intents = Vec::new();
    for y in 0..area.height {
        for x in 0..area.width {
            for intent in [
                mouse_action(area, state, x, y),
                slider_mouse_action(area, state, x, y),
                slider_wheel_action(area, state, x, y, true),
                slider_wheel_action(area, state, x, y, false),
            ]
            .into_iter()
            .flatten()
            {
                if !intents.contains(&intent) {
                    intents.push(intent);
                }
            }
        }
    }
    intents
}

fn render_to_string(state: &AppState) -> String {
    let mut terminal = test_terminal(140, 48);
    draw_page(&mut terminal, state);
    terminal_text(&terminal)
}

#[test]
fn orion_header_includes_device_name_and_supported_readiness() {
    let text = render_orion_screen();
    assert!(text.contains("Antelope Orion Studio Synergy Core"));
    assert!(text.to_lowercase().contains("supported"));
    assert!(!text.to_lowercase().contains("disabled"));
}

#[test]
fn orion_outputs_and_input_spaces_use_complete_profile_topology() {
    let state = orion_ui_state();
    let text = render_to_string(&state);
    for output in state.outputs() {
        assert!(
            text.contains(&output.name),
            "missing output {}",
            output.name
        );
    }
    for space in &state.input_spaces {
        assert!(text.contains(&space.name), "missing space {}", space.name);
        for input in &space.inputs {
            assert!(text.contains(&input.name), "missing input {}", input.name);
        }
    }
    assert_eq!(state.inputs_for_space("physical_inputs").len(), 12);
    assert_eq!(state.inputs_for_space("adat_inputs").len(), 16);
    assert_eq!(state.inputs_for_space("spdif_inputs").len(), 2);
}

#[test]
fn orion_mixer_surfaces_and_pages_cover_all_input_strips() {
    let mut state = orion_ui_state();
    assert_eq!(state.mixers().len(), 4);
    for surface in 0..4 {
        state.mixer.surface_index = surface;
        for page in 0..4 {
            state.mixer.strip_scroll = page * 8;
            let text = render_to_string(&state);
            assert!(text.contains(&state.mixers()[surface].name));
            assert!(text.contains(&format!("CH {:02}", page * 8 + 1)));
            assert!(text.contains(&format!("CH {:02}", page * 8 + 8)));
        }
    }
}

#[test]
fn non_zen_mixer_title_and_page_right_use_authoritative_strips() {
    let mut state = orion_ui_state();
    state.mixer.channels[0].truncate(1);
    let text = render_to_string(&state);
    assert!(text.contains("Mixer Strips 1-8 / 32"));
    assert!(state.visible_mixer_strip_bounds().end == 8);
    state.mixer.strip_scroll = 24;
    let text = render_to_string(&state);
    assert!(text.contains("Mixer Strips 25-32 / 32"));
}

#[test]
fn orion_input_spaces_use_only_canonical_typed_controls_and_expose_intents() {
    let state = orion_ui_state();
    let expected = [
        (
            0,
            vec![
                RuntimeInputControlKind::Gain,
                RuntimeInputControlKind::Mode,
                RuntimeInputControlKind::Phantom,
                RuntimeInputControlKind::Phase,
            ],
        ),
        (1, vec![RuntimeInputControlKind::Gain]),
        (2, vec![RuntimeInputControlKind::Gain]),
    ];
    for (space_index, kinds) in expected {
        let input = &state.input_spaces[space_index].inputs[0];
        let declared: Vec<_> = state
            .ui_profile
            .input_capabilities(input.address)
            .iter()
            .map(|capability| capability.kind)
            .collect();
        assert_eq!(declared, kinds);
        let geometry = super::layouts::dynamic_input_control_rects_for_test(&state, space_index, 0)
            .expect("input geometry");
        assert_eq!(
            geometry.gain.is_some(),
            kinds.contains(&RuntimeInputControlKind::Gain)
        );
        assert_eq!(
            geometry.mode.is_some(),
            kinds.contains(&RuntimeInputControlKind::Mode)
        );
        assert_eq!(
            geometry.phantom.is_some(),
            kinds.contains(&RuntimeInputControlKind::Phantom)
        );
        assert_eq!(
            geometry.phase.is_some(),
            kinds.contains(&RuntimeInputControlKind::Phase)
        );
        assert!(geometry.link.is_none());
    }
    for space_index in [1, 2] {
        let geometry = super::layouts::dynamic_input_control_rects_for_test(&state, space_index, 0)
            .expect("input control geometry");
        assert!(geometry.mode.is_none() && geometry.phantom.is_none() && geometry.phase.is_none());
        assert!(geometry.gain.is_some() && geometry.link.is_none());
    }
    assert!(available_intents(&state)
        .iter()
        .any(Intent::writes_hardware));
}

#[test]
fn orion_input_links_have_no_capability_rectangle_hit_area() {
    let state = orion_ui_state();
    for space_index in 0..3 {
        let input = &state.input_spaces[space_index].inputs[0];
        assert!(state
            .ui_profile
            .input_capabilities(input.address)
            .iter()
            .all(|capability| capability.kind != RuntimeInputControlKind::Link));
        assert!(
            super::layouts::dynamic_input_control_rects_for_test(&state, space_index, 0)
                .unwrap()
                .link
                .is_none()
        );
    }
    assert!(available_intents(&state).iter().any(|intent| matches!(
        intent,
        Intent::SetInputParameterAt { .. } | Intent::AdjustInputParameterAt { .. }
    )));
}

#[test]
fn orion_master_is_separate_from_input_strip_pages() {
    let mut state = orion_ui_state();
    state.mixer.strip_scroll = 24;
    let text = render_to_string(&state);
    assert!(text.contains("Master"));
    assert!(text.contains("CH 25"));
    assert!(text.contains("CH 32"));
    assert!(!state.visible_mixer_strip_bounds().contains(&32));
}

#[test]
fn dynamic_input_mode_uses_profile_value_label() {
    let mut state = zen_go_ui_state();
    state.input_spaces[0].inputs[0].mode = Some(0);

    let text = render_to_string(&state);

    assert!(text.contains("Mic"));
    assert!(!text.contains("M0"));
}

#[test]
fn zen_go_mixer_uses_full_200_column_viewport() {
    let state = zen_go_ui_state();
    let mut terminal = test_terminal(200, 55);

    draw_page(&mut terminal, &state);
    let text = terminal_text(&terminal);

    assert!(text.contains("Mixer Strips 1-10 / 16"), "{text}");
    assert!(text.contains("CH 10"), "{text}");
}

#[test]
fn zen_go_mixer_source_chip_uses_compact_assignment_label() {
    let mut state = zen_go_ui_state();
    state.mixer.channels[0][0].assignment = Some(MixerAssignment::Preamp(1));

    let text = render_to_string(&state);

    assert!(text.contains("P1"));
    assert_eq!(text.matches("Preamp 1").count(), 1);
}

#[test]
fn zen_go_routing_popup_shows_recording_assignments() {
    let mut state = zen_go_ui_state();
    state.mixer.channels[0][0].assignment = Some(MixerAssignment::Preamp(1));
    state.popup.routing_open = true;

    let text = render_to_string(&state);

    assert!(text.contains("Zen Go USB recordings mirror mixer strip assignments"));
    assert!(text.contains("USB 1/2"), "{text}");
    assert!(text.contains("REC 1"));
    assert!(text.contains("P1"));
    assert!(!text.contains("destination_6"));
}

#[test]
fn zen_go_geometry_and_names_remain_unchanged() {
    let state = zen_go_ui_state();
    assert_eq!(state.mixers().len(), 2);
    assert!(state
        .mixers()
        .iter()
        .all(|surface| surface.strips.len() == 16));
    assert!(state
        .mixers()
        .iter()
        .all(|surface| surface.master.is_none()));
    assert_eq!(state.outputs().len(), 3);
    assert_eq!(state.inputs_for_space("physical_inputs").len(), 2);
    assert_eq!(
        super::layouts::output_card_areas(Rect::new(0, 0, 90, 3)).len(),
        3
    );
    let text = render_to_string(&state);
    for output in state.outputs() {
        assert!(
            text.contains(&output.name),
            "missing profile output label {}",
            output.name
        );
    }
    for input in state
        .input_spaces
        .iter()
        .flat_map(|space| space.inputs.iter())
    {
        assert!(
            text.contains(&input.name),
            "missing profile input label {}",
            input.name
        );
    }
}

#[test]
fn routing_lists_every_destination_and_declared_finite_row_count() {
    let mut state = orion_ui_state();
    state.popup.routing_open = true;
    let text = render_to_string(&state);
    for group in &state.routing_capabilities {
        assert!(text.contains(&group.name), "missing route {}", group.name);
        assert!(text.contains(&format!("{} ch", group.channel_count)));
    }
}

#[test]
fn disabled_profiles_are_visible_but_produce_no_hardware_intents() {
    let mut entry = entry("discrete_4");
    entry.readiness = RuntimeReadiness::Disabled;
    entry.driver_kind = RuntimeDriverKind::None;
    let state = AppState::from_entry(&entry);
    let text = render_to_string(&state).to_lowercase();
    assert!(text.contains(state.ui_profile.readiness_label()));
    assert!(available_intents(&state)
        .iter()
        .all(|intent| !intent.writes_hardware()));
}

#[test]
fn supported_zen_go_metadata_is_actionable() {
    let state = zen_go_ui_state();
    assert_eq!(
        state.ui_profile.readiness,
        Some(RuntimeReadiness::Supported)
    );
    assert!(state.ui_profile.actionable);
    assert_eq!(state.ui_profile.device_name, "Antelope Zen Go Synergy Core");
}

#[test]
fn capabilities_come_from_confirmed_params_not_observed_values() {
    let mut entry = entry("zen_go_sc");
    let state = AppState::from_entry(&entry);
    let output = state.outputs()[0].address;
    assert!(state
        .ui_profile
        .supports_output(output, OutputControl::Level));
    assert_eq!(state.outputs()[0].level, None);

    entry
        .profile
        .params
        .retain(|param| param.name != "mix_solo");
    let state = AppState::from_entry(&entry);
    let mixer = state.mixers()[0].surface;
    assert!(!state.ui_profile.supports_mixer(mixer, MixerControl::Solo));
    assert!(state.ui_profile.supports_mixer(mixer, MixerControl::Fader));
    let input = state.input_spaces[0].inputs[0].address;
    assert!(state.ui_profile.supports_input(input, InputControl::Gain));
}

#[test]
fn dynamic_output_capability_geometry_matches_renderer_and_mouse() {
    let state = supported_dynamic_state();
    assert_eq!(state.outputs()[0].level, None);
    let geometry = super::layouts::dynamic_output_control_rects_for_test(&state, 0)
        .expect("first output geometry");
    assert!(geometry.level.is_some());
    assert!(available_intents(&state).iter().any(|intent| matches!(
        intent,
        Intent::SetOutputLevel { index: 0, .. } | Intent::AdjustOutputLevel { index: 0, .. }
    )));

    let no_mute = supported_dynamic_state_without("bus_mute");
    let geometry = super::layouts::dynamic_output_control_rects_for_test(&no_mute, 0)
        .expect("first output geometry");
    assert!(geometry.mute.is_none());
    assert!(!available_intents(&no_mute)
        .iter()
        .any(|intent| matches!(intent, Intent::ToggleOutputMute(0))));

    let no_dim = supported_dynamic_state_without("bus_dim");
    let geometry = super::layouts::dynamic_output_control_rects_for_test(&no_dim, 0)
        .expect("first output geometry");
    assert!(geometry.dim.is_none());
}

#[test]
fn dynamic_input_optional_control_geometry_and_addresses_match_mouse() {
    let mut state = supported_dynamic_state_without("phantom");
    state.input_spaces[0].inputs[0].mode = Some(0);
    let input = state.input_spaces[0].inputs[0].address;
    let geometry = super::layouts::dynamic_input_control_rects_for_test(&state, 0, 0)
        .expect("first input geometry");
    assert!(geometry.gain.is_some());
    assert!(geometry.phantom.is_none());
    assert!(available_intents(&state).iter().any(|intent| matches!(
        intent,
        Intent::SetInputGainAt { address, .. } if *address == input
    )));
    assert!(!available_intents(&state).iter().any(|intent| matches!(
        intent,
        Intent::ToggleInputPhantomAt { address } if *address == input
    )));
}

#[test]
fn rich_preamp_gain_slider_uses_mode_specific_profile_endpoints() {
    let area = Rect::new(0, 0, 200, 55);
    let mut state = zen_go_ui_state();
    let page = super::layouts::mixer_page_layout(super::layouts::root_chunks(area)[1]);
    let main = super::layouts::mixer_main_layout_for_state(page[0], &state);
    let row = super::layouts::dynamic_input_rows(main[0], &state)[0].2;
    let gain = super::layouts::dynamic_input_control_rects(row, &state, 0, 0)
        .and_then(|controls| controls.gain)
        .expect("rich preamp gain slider");
    let address = state.input_spaces[0].inputs[0].address;

    for (mode, min, max) in [(0, 0, 65), (2, 0, 45)] {
        state.input_spaces[0].inputs[0].mode = Some(mode);
        assert_eq!(
            slider_mouse_action(area, &state, gain.x, gain.y),
            Some(Intent::SetInputGainAt { address, raw: min })
        );
        assert_eq!(
            slider_mouse_action(
                area,
                &state,
                gain.x.saturating_add(gain.width.saturating_sub(1)),
                gain.y,
            ),
            Some(Intent::SetInputGainAt { address, raw: max })
        );
    }

    state.input_spaces[0].inputs[0].mode = Some(1);
    assert_eq!(slider_mouse_action(area, &state, gain.x, gain.y), None);

    state.input_spaces[0].inputs[0].mode = None;
    assert_eq!(slider_mouse_action(area, &state, gain.x, gain.y), None);
    let gain_down =
        super::layouts::dynamic_preamp_button_rects(row, &state, &state.input_spaces[0].inputs[0])
            [0]
        .1;
    assert!(!matches!(
        mouse_action(area, &state, gain_down.x, gain_down.y),
        Some(Intent::AdjustInputGainAt { .. })
    ));
    assert_eq!(
        slider_wheel_action(area, &state, gain.x, gain.y, true),
        None
    );
}

#[test]
fn rich_preamp_gain_slider_requires_a_grounded_mode_range() {
    let area = Rect::new(0, 0, 200, 55);
    let state = zen_go_ui_state();
    let page = super::layouts::mixer_page_layout(super::layouts::root_chunks(area)[1]);
    let main = super::layouts::mixer_main_layout_for_state(page[0], &state);
    let row = super::layouts::dynamic_input_rows(main[0], &state)[0].2;
    let gain = super::layouts::dynamic_input_control_rects(row, &state, 0, 0)
        .and_then(|controls| controls.gain)
        .expect("rich preamp gain slider");
    let address = state.input_spaces[0].inputs[0].address;

    assert_eq!(state.input_range(address, None), None);
    assert_eq!(slider_mouse_action(area, &state, gain.x, gain.y), None);
}

#[test]
fn orion_rich_preamp_controls_stay_inside_each_card() {
    let area = Rect::new(0, 0, 200, 55);
    let state = orion_ui_state();
    let page = super::layouts::mixer_page_layout(super::layouts::root_chunks(area)[1]);
    let main = super::layouts::mixer_main_layout_for_state(page[0], &state);

    for (space_index, input_index, card) in super::layouts::dynamic_input_rows(main[0], &state) {
        if state.input_spaces[space_index].kind != "physical_inputs" {
            continue;
        }
        let input = &state.input_spaces[space_index].inputs[input_index];
        let controls =
            super::layouts::dynamic_input_control_rects(card, &state, space_index, input_index)
                .expect("physical preamp controls");
        let mut rects = vec![
            controls.gain,
            controls.mode,
            controls.phantom,
            controls.phase,
            controls.link,
        ];
        rects.extend(
            super::layouts::dynamic_preamp_button_rects(card, &state, input)
                .into_iter()
                .map(|(_, rect)| Some(rect)),
        );
        for rect in rects.into_iter().flatten() {
            assert!(
                rect.x >= card.x
                    && rect.y >= card.y
                    && rect.right() <= card.right()
                    && rect.bottom() <= card.bottom(),
                "control {rect:?} escaped card {card:?}",
            );
        }
    }
}

#[test]
fn non_first_input_space_uses_stable_address_intent() {
    let mut entry = entry("zen_go_sc");
    entry.driver_kind = RuntimeDriverKind::Profile;
    let mut space = entry.profile.address_spaces[0].clone();
    space.id = "second".into();
    space.name = "Second".into();
    space.space_id = 9;
    space.count = Some(1);
    entry.profile.address_spaces.push(space);
    let mut input = entry.profile.inputs[0].clone();
    input.id = "second_1".into();
    input.space = "second".into();
    input.space_id = 9;
    input.index = 0;
    input.name = "Second 1".into();
    entry.profile.inputs.push(input);
    entry
        .profile
        .params
        .iter_mut()
        .find(|param| param.name == "gain")
        .expect("gain parameter")
        .range = Some((0, 65));
    let state = AppState::from_entry(&entry);
    let address = InputAddress { space: 9, index: 0 };
    let area = Rect::new(0, 0, 140, 48);
    let page = super::layouts::mixer_page_layout(super::layouts::root_chunks(area)[1]);
    let main = super::layouts::mixer_main_layout_for_state(page[0], &state);
    let row = super::layouts::dynamic_input_rows(main[0], &state)
        .into_iter()
        .find(|(space_index, input_index, _)| *space_index == 1 && *input_index == 0)
        .expect("second input-space row")
        .2;
    let gain = super::layouts::dynamic_input_control_rects(row, &state, 1, 0)
        .and_then(|controls| controls.gain)
        .expect("second input-space gain control");

    assert!(matches!(
        slider_mouse_action(area, &state, gain.x, gain.y),
        Some(Intent::SetInputGainAt { address: found, .. }) if found == address
    ));
}

#[test]
fn dynamic_mixer_uses_authoritative_strip_and_optional_capabilities() {
    let mut state = supported_dynamic_state_without("mix_solo");
    state.mixer.surfaces[0].strips[0].fader = Some(17);
    state.mixer.channels[0][0].level = Some(88);
    let address = MixerAddress {
        surface: 0,
        strip: 1,
    };
    let text = render_to_string(&state);
    assert!(
        text.contains("LVL -17 dB"),
        "dynamic fader must drive rendered attenuation value"
    );
    assert!(!text.contains("LVL 88"));
    let geometry = super::layouts::dynamic_mixer_control_rects_for_test(&state, address)
        .expect("strip geometry");
    assert!(geometry.fader.is_some());
    assert!(geometry.solo.is_none());
    assert!(available_intents(&state).iter().any(|intent| matches!(
        intent,
        Intent::SetMixerLevelAt { address: found, .. } if *found == address
    )));
    assert!(!available_intents(&state).iter().any(|intent| matches!(
        intent,
        Intent::ToggleMixerSoloAt { address: found } if *found == address
    )));
}

#[test]
fn assignment_picker_viewport_tracks_first_deep_and_keyboard_selected_rows() {
    let popup = Rect::new(0, 0, 42, 22);

    assert_eq!(
        super::layouts::popup_list_viewport(popup, "Assign CH 01", 119, 0),
        0..20
    );
    assert_eq!(
        super::layouts::popup_list_viewport(popup, "Assign CH 01", 119, 117),
        98..118
    );
    assert_eq!(
        super::layouts::popup_list_viewport(popup, "Assign CH 01", 119, 118),
        99..119
    );
}

#[test]
fn assignment_picker_viewport_handles_short_empty_and_zero_height_lists() {
    let popup = Rect::new(0, 0, 42, 22);
    assert_eq!(
        super::layouts::popup_list_viewport(popup, "Assign CH 01", 3, 2),
        0..3
    );
    assert_eq!(
        super::layouts::popup_list_viewport(popup, "Assign CH 01", 0, 12),
        0..0
    );
    assert_eq!(
        super::layouts::popup_list_viewport(Rect::new(0, 0, 42, 2), "Assign CH 01", 119, 118),
        119..119
    );
}

#[test]
fn orion_assignment_picker_mouse_rows_follow_the_rendered_viewport() {
    let area = Rect::new(0, 0, 140, 48);
    let popup = super::layouts::assignment_picker_area(area);
    let inner = super::layouts::popup_list_inner_area(popup, "Assign CH 01");
    let address = MixerAddress {
        surface: 0,
        strip: 1,
    };
    let mut state = orion_ui_state();
    state.routing.push(DynamicRoutingGroup {
        destination: 10,
        name: "mix_ch1".into(),
        sources: vec![RoutingSource { bank: 11, index: 0 }; 32],
    });
    state.popup.assignment_picker = Some(crate::app::AssignmentPickerState { strip: 1 });
    state.popup.assignment_picker_address = Some(address);

    state.popup.selected_index = 0;
    assert_eq!(
        mouse_action(area, &state, inner.x, inner.y),
        Some(Intent::PickRoutingSourceAt {
            address,
            source: RoutingSource { bank: 0, index: 0 },
        })
    );

    state.popup.selected_index = 118;
    let text = render_to_string(&state);
    assert!(
        text.contains("MUTE"),
        "deep selected choice must be rendered"
    );
    assert_eq!(
        mouse_action(area, &state, inner.x, inner.y + inner.height - 1),
        Some(Intent::PickRoutingSourceAt {
            address,
            source: RoutingSource { bank: 11, index: 0 },
        })
    );
    assert_eq!(
        mouse_action(area, &state, inner.x, inner.y + inner.height),
        None,
        "popup border is not a selectable row"
    );
}

#[test]
fn dynamic_mixer_source_label_uses_profile_routing_readback_for_each_surface() {
    let mut state = orion_ui_state();
    for (destination, source) in [
        (10, RoutingSource { bank: 2, index: 23 }),
        (13, RoutingSource { bank: 4, index: 1 }),
    ] {
        state.routing.push(DynamicRoutingGroup {
            destination,
            name: format!("destination_{destination}"),
            sources: vec![source; 32],
        });
    }

    for (address, expected) in [
        (
            MixerAddress {
                surface: 0,
                strip: 1,
            },
            "Computer Playback 24",
        ),
        (
            MixerAddress {
                surface: 3,
                strip: 1,
            },
            "S/PDIF In R",
        ),
    ] {
        let label = super::layouts::mixer_source_label(&state, address);
        assert_eq!(label, expected);
        let geometry = super::layouts::dynamic_mixer_control_rects_for_test(&state, address)
            .expect("strip geometry");
        assert_eq!(
            geometry.source,
            Some(super::layouts::mixer_header_chip_rects(geometry.card, &label).1)
        );
        state.mixer.surface_index = usize::from(address.surface);
        let rendered = render_to_string(&state);
        assert!(!rendered.contains("SOURCE ?"));
    }
}

#[test]
fn dynamic_mixer_source_label_falls_back_without_readback_and_retains_zen_legacy() {
    let orion = orion_ui_state();
    assert_eq!(
        super::layouts::mixer_source_label(
            &orion,
            MixerAddress {
                surface: 0,
                strip: 1,
            }
        ),
        "SOURCE ?"
    );

    let mut zen = zen_go_ui_state();
    zen.mixer.channels[0][0].assignment = Some(MixerAssignment::Preamp(1));
    let address = MixerAddress {
        surface: 0,
        strip: 1,
    };
    let label = super::layouts::mixer_source_label(&zen, address);
    assert_eq!(label, "P1");
    let geometry = super::layouts::dynamic_mixer_control_rects_for_test(&zen, address)
        .expect("Zen strip geometry");
    assert_eq!(
        geometry.source,
        Some(super::layouts::mixer_header_chip_rects(geometry.card, &label).1)
    );
}

#[test]
fn dynamic_mixer_source_chip_stays_inside_strip_border() {
    let mut orion = orion_ui_state();
    orion.routing.push(DynamicRoutingGroup {
        destination: 10,
        name: "mix_ch1".into(),
        sources: vec![RoutingSource { bank: 11, index: 0 }; 32],
    });
    for state in [zen_go_ui_state(), orion] {
        let address = MixerAddress {
            surface: 0,
            strip: 1,
        };
        let geometry = super::layouts::dynamic_mixer_control_rects_for_test(&state, address)
            .expect("strip geometry");
        let source = geometry.source.expect("source control");
        let inner_right = geometry
            .card
            .x
            .saturating_add(geometry.card.width)
            .saturating_sub(1);
        let channel = super::layouts::mixer_header_chip_rects(geometry.card, "").0;

        assert!(source.x > geometry.card.x);
        assert!(source.x >= channel.x.saturating_add(channel.width));
        assert!(source.x.saturating_add(source.width) <= inner_right);
    }
}

#[test]
fn dynamic_mixer_renders_meter_value_without_mtr_prefix() {
    assert_eq!(super::render::dynamic_meter_value_label(Some(21)), "-21 dB");
    assert_eq!(super::render::dynamic_meter_value_label(Some(127)), "-∞ dB");
    assert_eq!(super::render::dynamic_meter_value_label(None), "?");

    let mut state = supported_dynamic_state_without("mix_solo");
    state.mixer.surfaces[0].strips[0].meter = Some(21);
    let active_text = render_to_string(&state);

    state.mixer.surfaces[0].strips[0].meter = None;
    let absent_text = render_to_string(&state);

    assert!(!active_text.contains("MTR"));
    assert!(active_text.contains("-21 dB"));
    assert_ne!(active_text, absent_text, "meter state must remain rendered");
}

#[test]
fn dynamic_master_controls_are_separate_and_addressed_as_strip_zero() {
    let mut state = supported_dynamic_state();
    state.mixer.surfaces[0].master = Some(state.mixer.surfaces[0].strips[0].clone());
    let master = state.mixer.surfaces[0]
        .master
        .as_mut()
        .expect("master strip");
    master.strip = 0;
    master.name = "Master".into();
    let address = MixerAddress {
        surface: 0,
        strip: 0,
    };
    let geometry = super::layouts::dynamic_mixer_control_rects_for_test(&state, address)
        .expect("master geometry");
    assert!(geometry.fader.is_some());
    assert!(available_intents(&state).iter().any(|intent| matches!(
        intent,
        Intent::SetMixerLevelAt { address: found, .. } if *found == address
    )));
}

#[test]
fn supported_orion_has_all_dynamic_rows_and_control_intents() {
    let state = orion_ui_state();
    assert_eq!(super::layouts::dynamic_output_row_count_for_test(&state), 6);
    assert_eq!(super::layouts::dynamic_input_row_count_for_test(&state), 30);
    assert!(available_intents(&state)
        .iter()
        .any(Intent::writes_hardware));
}

#[test]
fn physical_inputs_use_rich_cards_while_digital_inputs_stay_compact() {
    let area = Rect::new(0, 0, 140, 48);
    let state = orion_ui_state();
    assert_eq!(state.input_spaces[0].kind, "physical_inputs");

    let page = super::layouts::mixer_page_layout(super::layouts::root_chunks(area)[1]);
    let main = super::layouts::mixer_main_layout_for_state(page[0], &state);
    let rows = super::layouts::dynamic_input_rows(main[0], &state);
    let physical = rows
        .iter()
        .filter(|(space, _, _)| *space == 0)
        .collect::<Vec<_>>();
    assert_eq!(physical.len(), 12);
    assert!(physical.iter().all(|(_, _, rect)| rect.height == 5));
    for (space_index, input_index, card) in physical {
        let input = &state.input_spaces[*space_index].inputs[*input_index];
        for (_, control) in super::layouts::dynamic_preamp_button_rects(*card, &state, input) {
            assert!(
                control.x.saturating_add(control.width) <= card.x.saturating_add(card.width),
                "control {control:?} overruns card {card:?}",
            );
        }
    }
    assert!(rows
        .iter()
        .filter(|(space, _, _)| *space != 0)
        .all(|(_, _, rect)| rect.height == 1));
}

#[test]
fn zen_go_uses_one_rich_card_per_physical_preamp() {
    let area = Rect::new(0, 0, 200, 55);
    let state = zen_go_ui_state();
    let page = super::layouts::mixer_page_layout(super::layouts::root_chunks(area)[1]);
    let main = super::layouts::mixer_main_layout_for_state(page[0], &state);
    let rows = super::layouts::dynamic_input_rows(main[0], &state);

    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|(_, _, rect)| rect.height == 5));
    assert_eq!(rows[0].2.y, rows[1].2.y);
}

#[test]
fn zen_go_physical_inputs_render_rich_preamp_cards() {
    let mut state = zen_go_ui_state();
    state.input_spaces[0].inputs[0].gain = Some(43);
    state.input_spaces[0].inputs[0].meter = Some(127);
    let mut terminal = test_terminal(200, 55);
    draw_page(&mut terminal, &state);
    let text = terminal_text(&terminal);

    assert!(text.contains("Preamp 1"));
    assert!(text.contains("Preamp 2"));
    assert_eq!(text.matches("OBS").count(), 2);
    assert!(text.contains("GAIN 43 dB"));
    assert!(text.contains("-∞ dB"));
}

#[test]
fn dynamic_preamp_phase_chip_reflects_active_state() {
    let mut state = zen_go_ui_state();
    for (active, expected) in [(false, Color::Green), (true, Color::Yellow)] {
        state.input_spaces[0].inputs[0].phase = Some(active);
        assert_eq!(
            super::render::dynamic_input_control_color(
                &state,
                &state.input_spaces[0].inputs[0],
                RuntimeInputControlKind::Phase,
            ),
            expected,
        );
    }

    let unsupported = supported_dynamic_state_without("phase_invert");
    assert_eq!(
        super::render::dynamic_input_control_color(
            &unsupported,
            &unsupported.input_spaces[0].inputs[0],
            RuntimeInputControlKind::Phase,
        ),
        Color::DarkGray,
    );
}

#[test]
fn orion_renders_a_rich_card_for_each_physical_preamp_only() {
    let state = orion_ui_state();
    let mut terminal = test_terminal(200, 55);
    draw_page(&mut terminal, &state);
    let text = terminal_text(&terminal);

    assert_eq!(text.matches("OBS").count(), 12);
    assert_eq!(text.matches("OBS ?").count(), 12);
    assert!(!text.contains("OBS -∞ dB"));
    assert!(text.matches("GAIN").count() >= 12);
    assert!(text.contains("ADAT 1"));
    assert!(text.contains("S/PDIF L"));
}

#[test]
fn orion_physical_meter_source_is_unavailable_without_fabricated_values() {
    let state = orion_ui_state();
    assert!(state.input_spaces[0]
        .inputs
        .iter()
        .all(|input| input.meter.is_none()));
    assert_eq!(super::layouts::meter_slider_label("OBS", None), "OBS ?");
}

#[test]
fn empty_and_oversized_dynamic_geometry_never_panics_or_writes() {
    let mut state = discrete_4_ui_state();
    state.output.dynamic.clear();
    state.input_spaces.clear();
    state.mixer.surfaces.clear();
    state.mixer.channels.clear();
    state.routing_capabilities.clear();
    state.output.selected = usize::MAX;
    state.preamp.selected_input = usize::MAX;
    state.mixer.surface_index = usize::MAX;
    state.mixer.selected_channel = usize::MAX;
    state.mixer.strip_scroll = usize::MAX;
    for (width, height) in [(1, 1), (20, 8), (140, 48)] {
        let mut terminal = test_terminal(width, height);
        draw_page(&mut terminal, &state);
        let area = Rect::new(0, 0, width, height);
        assert!(mouse_action(
            area,
            &state,
            width.saturating_sub(1),
            height.saturating_sub(1)
        )
        .is_none_or(|intent| !intent.writes_hardware()));
    }
}

#[test]
fn visible_mouse_hardware_intents_resolve_to_profile_bounds() {
    let state = zen_go_ui_state();
    let intents = available_intents(&state);
    let mut kinds = HashSet::new();
    for intent in intents.into_iter().filter(Intent::writes_hardware) {
        match intent {
            Intent::AdjustOutputLevel { index, .. }
            | Intent::SetOutputLevel { index, .. }
            | Intent::ToggleOutputMute(index)
            | Intent::ToggleOutputDim(index) => assert!(index < state.outputs().len()),
            Intent::AdjustMixerLevel { index, .. }
            | Intent::SetMixerLevel { index, .. }
            | Intent::AdjustMixerPan { index, .. }
            | Intent::SetMixerPan { index, .. } => {
                assert!(index < state.mixers()[0].strips.len())
            }
            Intent::AdjustPreampGain { input, .. }
            | Intent::SetPreampGain { input, .. }
            | Intent::OpenPreampModeSelector(input)
            | Intent::CyclePreampMode(input)
            | Intent::TogglePreampPhase(input)
            | Intent::TogglePreampPhantom(input) => {
                assert!(usize::from(input) < state.input_spaces[0].inputs.len())
            }
            _ => {}
        }
        kinds.insert(std::mem::discriminant(&intent));
    }
    assert!(!kinds.is_empty());
}

#[test]
fn orion_readiness_is_promoted_supported() {
    assert_eq!(
        entry("orion_studio_3").readiness,
        RuntimeReadiness::Supported
    );
    assert_eq!(
        entry("orion_studio_3").driver_kind,
        RuntimeDriverKind::Profile
    );
}
