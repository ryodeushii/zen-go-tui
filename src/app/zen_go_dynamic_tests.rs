use antelope_protocol::{
    load_profile_pack, DeviceEvent, DeviceSnapshot, DynamicDeviceState, DynamicInputState,
    DynamicMixerStrip, DynamicMixerSurface, DynamicOutputState, DynamicStatePatch, QueryResponse,
};

use super::AppState;

mod test_support {
    use super::*;

    pub fn zen_go_profile() -> antelope_protocol::RuntimeProfile {
        let pack = load_profile_pack(include_bytes!("../device/generated_profiles.json"))
            .expect("generated profile pack");
        pack.profiles
            .into_iter()
            .find(|entry| entry.profile.identity.pid == 0xa015)
            .expect("Zen Go profile")
            .profile
    }

    pub fn q04_surface_patch(surface: u8, fader: i32) -> DynamicMixerSurface {
        DynamicMixerSurface {
            surface,
            name: String::new(),
            master: None,
            strips: (1..=16)
                .map(|strip| DynamicMixerStrip {
                    strip,
                    name: String::new(),
                    fader: (strip == 1).then_some(fader),
                    pan: None,
                    send: None,
                    muted: None,
                    soloed: None,
                    linked: None,
                    meter: None,
                    parameters: Vec::new(),
                })
                .collect(),
        }
    }
}

#[test]
fn profile_topology_controls_all_zen_go_collections() {
    let state = AppState::from_profile(&test_support::zen_go_profile());
    assert_eq!(state.inputs_for_space("physical_inputs").len(), 2);
    assert_eq!(state.outputs().len(), 3);
    assert_eq!(state.mixers().len(), 2);
    assert!(state.mixers().iter().all(|mixer| mixer.strips.len() == 16));
}

#[test]
fn mixer_patch_updates_fader_without_erasing_existing_meter() {
    let mut state = AppState::from_profile(&test_support::zen_go_profile());
    state.mixers_mut()[0].strips[0].meter = Some(0x55);
    let changed = state.observe_event(DeviceEvent::QueryReply {
        query_id: 0x04,
        sub_id: 0,
        body: vec![0; 34],
        patch: Some(DynamicStatePatch::Mixer(test_support::q04_surface_patch(
            0, 0x12,
        ))),
        raw: vec![0; 320],
    });

    assert!(changed);
    assert_eq!(state.mixers()[0].strips[0].fader, Some(0x12));
    assert_eq!(state.mixers()[0].strips[0].meter, Some(0x55));
}

#[test]
fn topology_changing_patch_is_rejected_without_mutating_state() {
    let mut state = AppState::from_profile(&test_support::zen_go_profile());
    let before = state.mixers()[0].strips.len();
    let mut patch = test_support::q04_surface_patch(0, 0x12);
    patch.strips.pop();

    assert!(!state.apply_dynamic_patch(DynamicStatePatch::Mixer(patch)));
    assert_eq!(state.mixers()[0].strips.len(), before);
}

#[test]
fn normalized_snapshot_preserves_candidate_preamp_meters() {
    let profile = test_support::zen_go_profile();
    let mut state = AppState::from_profile(&profile);
    let mut inputs: Vec<DynamicInputState> = state
        .input_spaces
        .iter()
        .flat_map(|space| space.inputs.iter().cloned())
        .collect();
    inputs[0].meter = Some(0x21);
    inputs[1].meter = Some(0x22);
    let snapshot = DynamicDeviceState {
        globals: state.globals.clone(),
        inputs,
        outputs: state.outputs().to_vec(),
        mixers: state.mixers().to_vec(),
        routing: Vec::new(),
        zen_go_compatibility: None,
    };

    assert!(state.observe_event(DeviceEvent::Snapshot {
        state: snapshot,
        raw: vec![0x73; 320],
    }));
    assert_eq!(
        state.inputs_for_space("physical_inputs")[0].meter,
        Some(0x21)
    );
    assert_eq!(
        state.inputs_for_space("physical_inputs")[1].meter,
        Some(0x22)
    );
    assert_eq!(state.preamp.state.input1.observed_meter, Some(0x21));
    assert_eq!(state.preamp.state.input2.observed_meter, Some(0x22));
}

#[test]
fn invalid_patch_address_is_rejected_without_mutation() {
    let mut state = AppState::from_profile(&test_support::zen_go_profile());
    let before = state.outputs().to_vec();
    let invalid = DynamicOutputState {
        address: antelope_protocol::OutputAddress { id: 99 },
        name: "invalid".into(),
        level: Some(1),
        muted: None,
        dimmed: None,
        parameters: Vec::new(),
    };
    assert!(!state.apply_dynamic_patch(DynamicStatePatch::Outputs(vec![invalid])));
    assert_eq!(state.outputs(), before);
}

#[test]
fn legacy_readback_ignores_indexes_outside_profile_geometry() {
    let mut state = AppState::from_profile(&test_support::zen_go_profile());
    state.mixer.channels[0].truncate(1);
    state.mixer.channels[1].truncate(1);
    let body = vec![
        0x06, 0x03, 0x00, 0x03, 0x01, 0x03, 0x02, 0x03, 0x03, 0x01, 0x02, 0x01, 0x03, 0x01, 0x04,
        0x01, 0x05, 0x01, 0x06, 0x01, 0x07, 0x08, 0x00, 0x08, 0x00, 0x08, 0x00, 0x08, 0x00, 0x08,
        0x00, 0x08, 0x00,
    ];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state.observe_frame(
            DeviceSnapshot::QueryReply(QueryResponse {
                query_id: 0x03,
                sub_id: 0x06,
                body,
            }),
            vec![0x75; 320],
        )
    }));
    assert!(result.is_ok());
}

#[test]
fn q18_patch_preserves_omitted_surface_fields_and_applies_supported_fields() {
    let mut state = AppState::from_profile(&test_support::zen_go_profile());
    for (surface_index, surface) in state.mixers_mut().iter_mut().enumerate() {
        let strip = &mut surface.strips[0];
        strip.name = format!("Existing {surface_index}");
        strip.fader = Some(0x21 + surface_index as i32);
        strip.pan = Some(0x22 + surface_index as i32);
        strip.muted = Some(true);
        strip.meter = Some(0x23 + surface_index as u8);
        strip.linked = Some(true);
    }
    let before = state.mixers().to_vec();
    let mut q18 = before.clone();
    for (surface_index, surface) in q18.iter_mut().enumerate() {
        surface.name.clear();
        for strip in &mut surface.strips {
            strip.name.clear();
            strip.fader = None;
            strip.pan = None;
            strip.send = None;
            strip.muted = None;
            strip.soloed = None;
            strip.linked = None;
            strip.meter = None;
            strip.parameters.clear();
        }
        surface.strips[0].soloed = Some(surface_index == 0);
        surface.strips[1].fader = Some(0x30 + surface_index as i32);
    }

    assert!(state.apply_dynamic_patch(DynamicStatePatch::Mixers(q18)));
    for (surface_index, surface) in state.mixers().iter().enumerate() {
        let existing = &before[surface_index].strips[0];
        let merged = &surface.strips[0];
        assert_eq!(merged.name, existing.name);
        assert_eq!(merged.fader, existing.fader);
        assert_eq!(merged.pan, existing.pan);
        assert_eq!(merged.muted, existing.muted);
        assert_eq!(merged.meter, existing.meter);
        assert_eq!(merged.linked, existing.linked);
        assert_eq!(merged.soloed, Some(surface_index == 0));
        assert_eq!(surface.strips[1].fader, Some(0x30 + surface_index as i32));
    }
}
