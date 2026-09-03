use antelope_protocol::{
    load_profile_pack, DeviceEvent, DynamicDeviceState, DynamicInputState, DynamicMixerStrip,
    DynamicMixerSurface, DynamicOutputState, DynamicStatePatch,
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
