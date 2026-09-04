use antelope_protocol::{FaderDirection, FaderSemantics, MixerAddress, RuntimeEntry};

use crate::{app::AppState, device::ProfileCatalog};

use super::{draw, layouts, mouse_action};
use ratatui::{backend::TestBackend, layout::Rect, Terminal};

fn zen_go_entry() -> RuntimeEntry {
    ProfileCatalog::builtin()
        .entries()
        .iter()
        .find(|entry| entry.id == "zen_go_sc")
        .expect("Zen Go profile")
        .clone()
}

fn synthetic_topology_entry() -> RuntimeEntry {
    let mut entry = zen_go_entry();
    let mut input = entry.profile.inputs[1].clone();
    input.index = 2;
    input.id = "physical_input_3".into();
    input.name = "Input 3".into();
    entry.profile.inputs.push(input);

    let mut output = entry.profile.outputs[2].clone();
    output.id = 3;
    output.name = "Output 4".into();
    entry.profile.outputs.push(output);

    entry.profile.mixers.truncate(1);
    entry.profile.mixers[0].strip_count = 7;
    entry
        .profile
        .address_spaces
        .iter_mut()
        .find(|space| space.id == "physical_inputs")
        .expect("physical input space")
        .count = Some(3);
    entry
}

fn render_text(state: &AppState) -> String {
    let backend = TestBackend::new(160, 50);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|frame| draw(frame, state)).expect("draw");
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn fader_helpers_preserve_profile_attenuation_direction() {
    let semantics = FaderSemantics {
        min: 0,
        max: 90,
        direction: FaderDirection::Attenuation,
        unity: 0,
    };
    assert_eq!(layouts::fader_ratio(0, semantics), 1.0);
    assert_eq!(layouts::fader_ratio(90, semantics), 0.0);
    assert_eq!(layouts::fader_display_db(18, semantics), -18);
    assert_eq!(layouts::mixer_level_from_ratio(1.0, semantics), 0);
    assert_eq!(layouts::mixer_level_from_ratio(0.0, semantics), 90);
    let state = AppState::from_entry(&zen_go_entry());
    let pan_range = state
        .mixer_range(0, antelope_protocol::MixerControl::Pan)
        .expect("profile pan range");
    assert_eq!(pan_range, (-30, 30));
    assert_eq!(layouts::pan_from_ratio(0.0, pan_range), -30);
    assert_eq!(layouts::pan_from_ratio(0.5, pan_range), 0);
    assert_eq!(layouts::pan_from_ratio(1.0, pan_range), 30);
}

#[test]
fn profile_vectors_drive_dynamic_labels_and_counts() {
    let entry = synthetic_topology_entry();
    let state = AppState::from_entry(&entry);
    let text = render_text(&state);
    assert_eq!(
        state
            .input_spaces
            .iter()
            .find(|space| space.id == "physical_inputs")
            .expect("physical input space")
            .inputs
            .len(),
        3
    );
    assert_eq!(state.outputs().len(), 4);
    assert_eq!(state.mixers().len(), 1);
    assert_eq!(state.mixers()[0].strips.len(), 7);
    assert!(text.contains("Input 3"));
    assert!(text.contains("Output 4"));
    assert!(text.contains("CH 07"));
    assert!(!text.contains("CH 08"));
}

#[test]
fn profile_vectors_render_second_mixer_surface_label() {
    let state = AppState::from_entry(&zen_go_entry());
    let text = render_text(&state);
    assert_eq!(state.mixers().len(), 2);
    assert!(text.contains("MIX 2 / HP2"));
}

#[test]
fn output_render_and_mouse_use_profile_range() {
    let mut entry = synthetic_topology_entry();
    let parameter = entry
        .profile
        .params
        .iter_mut()
        .find(|param| param.name == "bus_level")
        .expect("output level parameter");
    parameter.range = Some((0, 42));
    parameter.direction = Some(FaderDirection::Attenuation);
    parameter.unity = Some(0);
    let semantics = FaderSemantics {
        min: 0,
        max: 42,
        direction: FaderDirection::Attenuation,
        unity: 0,
    };
    let mut state = AppState::from_entry(&entry);
    state.output.dynamic[3].level = Some(10);
    let text = render_text(&state);
    assert!(text.contains("LVL -10 dB"));
    assert_eq!(layouts::output_ratio(0, semantics), 1.0);
    assert_eq!(layouts::output_ratio(42, semantics), 0.0);
    assert_eq!(layouts::output_step_from_ratio(0.0, semantics), 42);
    assert_eq!(layouts::output_step_from_ratio(1.0, semantics), 0);
    assert_eq!(layouts::output_ratio(21, semantics), 0.5);
    assert_eq!(layouts::output_step_from_ratio(0.5, semantics), 21);
}

#[test]
fn dynamic_strip_renders_profile_meter_value_without_mtr_prefix() {
    let mut state = AppState::from_entry(&zen_go_entry());
    state.mixers_mut()[0].strips[0].fader = Some(18);
    state.mixers_mut()[0].strips[0].meter = Some(21);
    let text = render_text(&state);
    assert!(text.contains("LVL -18 dB"));
    assert!(text.contains("-21 dB"));
    assert!(!text.contains("MTR"));
}

#[test]
fn dynamic_mouse_maps_attenuation_top_to_unity_and_bottom_to_maximum() {
    let state = AppState::from_entry(&zen_go_entry());
    let area = Rect::new(0, 0, 160, 50);
    let controls = layouts::dynamic_mixer_control_rects_for_test(
        &state,
        MixerAddress {
            surface: 0,
            strip: 1,
        },
    )
    .expect("mixer controls");
    let fader = controls.fader.expect("fader");
    let top = super::slider_mouse_action(area, &state, fader.x, fader.y);
    let bottom =
        super::slider_mouse_action(area, &state, fader.x, fader.y.saturating_add(fader.height));
    assert!(matches!(
        top,
        Some(crate::app::Intent::SetMixerLevelAt { level: 0, .. })
    ));
    assert!(
        matches!(bottom, Some(crate::app::Intent::SetMixerLevelAt { .. })),
        "bottom={bottom:?} fader={fader:?}"
    );
    assert_eq!(
        layouts::mixer_level_from_ratio(0.0, state.mixer_fader(0).unwrap()),
        90
    );
}

#[test]
fn dynamic_mouse_rejects_out_of_profile_addresses() {
    let entry = synthetic_topology_entry();
    let state = AppState::from_entry(&entry);
    let area = Rect::new(0, 0, 160, 50);
    assert!(layouts::dynamic_input_control_rects_for_test(&state, 0, 3).is_none());
    assert!(layouts::dynamic_output_control_rects_for_test(&state, 4).is_none());
    assert!(layouts::dynamic_mixer_control_rects_for_test(
        &state,
        MixerAddress {
            surface: 0,
            strip: 8,
        }
    )
    .is_none());
    assert!(layouts::dynamic_mixer_control_rects_for_test(
        &state,
        MixerAddress {
            surface: 0,
            strip: 1,
        }
    )
    .expect("first mixer strip")
    .source
    .is_some());
    assert!(mouse_action(area, &state, 159, 49).is_none());
}

#[test]
fn input_range_does_not_fallback_to_another_mode() {
    let mut entry = synthetic_topology_entry();
    let gain = entry
        .profile
        .params
        .iter_mut()
        .find(|param| param.name == "gain")
        .expect("gain parameter");
    gain.range = None;
    gain.range_by_mode = vec![("mic".into(), (0, 65)), ("hiz".into(), (0, 45))];
    let state = AppState::from_entry(&entry);
    let address = state.input_spaces[0].inputs[0].address;
    assert_eq!(state.input_range(address, Some(0)), Some((0, 65)));
    assert_eq!(state.input_range(address, Some(1)), None);
    assert_eq!(state.input_range(address, Some(2)), Some((0, 45)));
}

#[test]
fn dynamic_controls_follow_optional_profile_capabilities() {
    let mut entry = synthetic_topology_entry();
    entry
        .profile
        .params
        .retain(|param| param.name != "bus_mute");
    let state = AppState::from_entry(&entry);
    let output = state.outputs()[0].address;
    assert!(state
        .ui_profile
        .supports_output(output, antelope_protocol::OutputControl::Level));
    assert!(!state
        .ui_profile
        .supports_output(output, antelope_protocol::OutputControl::Mute));
}
