use std::collections::HashSet;

use antelope_protocol::{
    InputAddress, InputControl, MixerAddress, MixerControl, OutputControl, RuntimeDriverKind,
    RuntimeEntry, RuntimeInputControlKind, RuntimeReadiness,
};
use ratatui::{
    backend::TestBackend,
    layout::Rect,
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
    AppState::from_entry(&entry("discrete_4_synergy_core"))
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
fn selected_device_title_uses_runtime_profile_name() {
    let state = orion_ui_state();
    let title = super::render::render_device_header(&state).to_string();
    assert!(title.contains("Antelope Orion Studio III"));
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
    assert!(text.contains("Antelope Orion Studio III"));
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
        let geometry =
            super::layouts::dynamic_input_control_rects_for_test(&state, space_index, 0).unwrap();
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
    for name in ["Monitor", "HP1", "HP2", "Preamp 1", "Preamp 2"] {
        assert!(text.contains(name), "missing Zen Go label {name}");
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
    let mut entry = entry("discrete_4_synergy_core");
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
    let state = supported_dynamic_state_without("phantom");
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
    let state = AppState::from_entry(&entry);
    let address = InputAddress { space: 9, index: 0 };
    assert!(available_intents(&state).iter().any(|intent| matches!(
        intent,
        Intent::SetInputGainAt { address: found, .. } if *found == address
    )));
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
        text.contains("LVL 17"),
        "dynamic fader must drive rendered value"
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
fn dynamic_master_controls_are_separate_and_addressed_as_strip_zero() {
    let mut state = supported_dynamic_state();
    state.mixer.surfaces[0].master = Some(state.mixer.surfaces[0].strips[0].clone());
    state.mixer.surfaces[0].master.as_mut().unwrap().strip = 0;
    state.mixer.surfaces[0].master.as_mut().unwrap().name = "Master".into();
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
