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
}

#[test]
fn profile_vectors_drive_dynamic_labels_and_counts() {
    let mut entry = zen_go_entry();
    entry.profile.outputs.truncate(2);
    entry.profile.mixers.truncate(1);
    entry.profile.mixers[0].strip_count = 7;
    entry
        .profile
        .inputs
        .retain(|input| input.space == "physical_inputs");
    entry
        .profile
        .address_spaces
        .retain(|space| space.id == "physical_inputs");
    let state = AppState::from_entry(&entry);
    let text = render_text(&state);
    assert_eq!(state.outputs().len(), 2);
    assert_eq!(state.mixers().len(), 1);
    assert_eq!(state.mixers()[0].strips.len(), 7);
    assert!(text.contains("CH 07"));
    assert!(!text.contains("CH 08"));
}

#[test]
fn dynamic_strip_renders_profile_meter_and_attenuation_label() {
    let mut state = AppState::from_entry(&zen_go_entry());
    state.mixers_mut()[0].strips[0].fader = Some(18);
    state.mixers_mut()[0].strips[0].meter = Some(0x52);
    let text = render_text(&state);
    assert!(text.contains("LVL -18 dB"));
    assert!(text.contains("MTR"));
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
fn dynamic_mouse_rejects_out_of_profile_mixer_addresses() {
    let state = AppState::from_entry(&zen_go_entry());
    let area = Rect::new(0, 0, 160, 50);
    assert!(state
        .mixers()
        .iter()
        .all(|surface| surface.strips.iter().all(|strip| strip.strip <= 16)));
    assert!(layouts::dynamic_mixer_control_rects_for_test(
        &state,
        MixerAddress {
            surface: 0,
            strip: 99
        }
    )
    .is_none());
    assert!(mouse_action(area, &state, 159, 49).is_none());
}
