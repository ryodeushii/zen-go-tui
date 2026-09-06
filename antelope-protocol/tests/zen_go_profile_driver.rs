use antelope_protocol::{
    load_profile_pack, Action, DeviceDriver, DeviceEvent, DynamicStatePatch, MixerAddress,
    PanState, QueryRequest, ZenGoDriver,
};

mod test_support {
    use super::*;

    pub fn zen_go_profile() -> antelope_protocol::RuntimeProfile {
        load_profile_pack(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/device/generated_profiles.json"
        )))
        .expect("checked-in generated profile pack")
        .profiles
        .into_iter()
        .find(|entry| entry.profile.identity.pid == 0xa015)
        .expect("Zen Go profile")
        .profile
    }

    pub fn hex_fixture(hex: &str) -> Vec<u8> {
        hex.split_whitespace()
            .flat_map(|word| {
                (0..word.len())
                    .step_by(2)
                    .map(move |index| u8::from_str_radix(&word[index..index + 2], 16).expect("hex"))
            })
            .collect()
    }
}

#[test]
fn zen_go_driver_startup_requests_come_from_profile_safe_pairs() {
    let driver = ZenGoDriver::new(test_support::zen_go_profile()).expect("driver");
    assert_eq!(driver.startup_requests().len(), 47);
    assert_eq!(
        driver.startup_requests()[0],
        QueryRequest {
            query_id: 0x01,
            sub_id: 0
        }
    );
    assert!(driver.startup_requests().contains(&QueryRequest {
        query_id: 0x04,
        sub_id: 3
    }));
}

#[test]
fn zen_go_fader_encoding_preserves_attenuation_domain() {
    let driver = ZenGoDriver::new(test_support::zen_go_profile()).expect("driver");
    let batch = driver
        .encode(Action::SetMixerStripState {
            address: MixerAddress {
                surface: 0,
                strip: 1,
            },
            fader: 0,
            pan: 0,
            muted: false,
            soloed: false,
            send: None,
        })
        .expect("unity fader");
    assert_eq!(batch.frames[0][20], 0);
    assert!(driver
        .encode(Action::SetMixerStripState {
            address: MixerAddress {
                surface: 0,
                strip: 1
            },
            fader: 91,
            pan: 0,
            muted: false,
            soloed: false,
            send: None,
        })
        .is_err());
}

#[test]
fn zen_go_q04_readback_emits_profile_surface_patch() {
    let driver = ZenGoDriver::new(test_support::zen_go_profile()).expect("driver");
    for (fixture, expected_surface) in [("q04_mix1_reply.hex", 0), ("q04_mix2_reply.hex", 1)] {
        let bytes = test_support::hex_fixture(include_str!(concat!(
            "fixtures/zen_go/",
            "q04_mix1_reply.hex"
        )));
        let bytes = if fixture == "q04_mix2_reply.hex" {
            test_support::hex_fixture(include_str!("fixtures/zen_go/q04_mix2_reply.hex"))
        } else {
            bytes
        };
        let event = driver.decode(&bytes).expect("decode").expect("event");
        let DeviceEvent::QueryReply {
            patch: Some(DynamicStatePatch::Mixer(surface)),
            ..
        } = event
        else {
            panic!("expected mixer patch");
        };
        assert_eq!(surface.surface, expected_surface);
        assert_eq!(surface.strips[0].fader, Some(0));
        assert_eq!(surface.strips[1].fader, Some(0x12));
        assert_eq!(surface.strips[2].fader, Some(0x1e));
        assert_eq!(surface.strips[3].fader, Some(0x5a));
        assert_eq!(surface.strips[0].pan, Some(0));
        assert_eq!(surface.strips[0].muted, Some(false));
        assert_eq!(surface.strips[0].soloed, Some(false));
    }
}

#[test]
fn zen_go_q04_readback_converts_pan_to_semantic_values() {
    let driver = ZenGoDriver::new(test_support::zen_go_profile()).expect("driver");
    let mut bytes = test_support::hex_fixture(include_str!("fixtures/zen_go/q04_mix1_reply.hex"));
    bytes[19] = PanState::left().raw();
    bytes[21] = PanState::right().raw();

    let event = driver.decode(&bytes).expect("decode").expect("event");
    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Mixer(surface)),
        ..
    } = event
    else {
        panic!("expected mixer patch");
    };
    assert_eq!(surface.strips[0].pan, Some(-30));
    assert_eq!(surface.strips[1].pan, Some(30));
}

#[test]
fn zen_go_q18_readback_emits_both_surfaces_with_declared_solo_field() {
    let driver = ZenGoDriver::new(test_support::zen_go_profile()).expect("driver");
    let event = driver
        .decode(&test_support::hex_fixture(include_str!(
            "fixtures/zen_go/q18_reply.hex"
        )))
        .expect("decode")
        .expect("event");
    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Mixers(surfaces)),
        ..
    } = event
    else {
        panic!("expected mixer surfaces patch");
    };
    assert_eq!(surfaces.len(), 2);
    assert_eq!(surfaces[0].surface, 0);
    assert_eq!(surfaces[1].surface, 1);
    assert_eq!(surfaces[0].strips.len(), 16);
    assert_eq!(surfaces[1].strips.len(), 16);
    assert_eq!(surfaces[0].strips[0].fader, None);
    assert_eq!(surfaces[0].strips[0].pan, None);
    assert_eq!(surfaces[0].strips[0].muted, None);
    assert_eq!(surfaces[0].strips[0].meter, None);
    assert_eq!(surfaces[0].strips[0].soloed, Some(true));
    assert_eq!(surfaces[1].strips[0].fader, None);
    assert_eq!(surfaces[1].strips[0].pan, None);
    assert_eq!(surfaces[1].strips[0].muted, None);
    assert_eq!(surfaces[1].strips[0].meter, None);
    assert_eq!(surfaces[1].strips[0].soloed, Some(false));
}

#[test]
fn zen_go_profile_meter_offsets_convert_payload_to_full_report_and_decode_mix_master_lanes() {
    let profile = test_support::zen_go_profile();
    assert_eq!(
        profile
            .meter_mappings
            .iter()
            .map(|mapping| (mapping.target_index, mapping.lane, mapping.offset))
            .collect::<Vec<_>>(),
        vec![(0, 0, 0xea), (0, 1, 0xeb), (1, 0, 0xee), (1, 1, 0xef)]
    );
    assert!(profile.meter_mappings.iter().all(|mapping| matches!(
        mapping.target,
        antelope_protocol::RuntimeMeterTarget::MixMaster
    )));

    let driver = ZenGoDriver::new(profile).expect("driver");
    let mut frame = test_support::hex_fixture(include_str!(
        "fixtures/zen_go/state_with_candidate_meters.hex"
    ));
    frame[0xea..0xf0].copy_from_slice(&[0x11, 0x22, 0, 0, 0x33, 0x44]);
    let DeviceEvent::Snapshot { state, .. } = driver.decode(&frame).unwrap().unwrap() else {
        panic!("expected snapshot");
    };
    assert_eq!(
        state
            .meters
            .iter()
            .map(|meter| (meter.target_index, meter.lane, meter.value))
            .collect::<Vec<_>>(),
        vec![(0, 0, 0x11), (0, 1, 0x22), (1, 0, 0x33), (1, 1, 0x44)]
    );
}

#[test]
fn zen_go_snapshot_uses_profile_candidate_preamp_meters() {
    let driver = ZenGoDriver::new(test_support::zen_go_profile()).expect("driver");
    let event = driver
        .decode(&test_support::hex_fixture(include_str!(
            "fixtures/zen_go/state_with_candidate_meters.hex"
        )))
        .expect("decode")
        .expect("snapshot");
    let DeviceEvent::Snapshot { state, .. } = event else {
        panic!("expected snapshot");
    };
    assert_eq!(
        state
            .inputs
            .iter()
            .find(|input| input.address.index == 0)
            .unwrap()
            .meter,
        Some(0x41)
    );
    assert_eq!(
        state
            .inputs
            .iter()
            .find(|input| input.address.index == 1)
            .unwrap()
            .meter,
        Some(0x52)
    );
    assert!(state
        .mixers
        .iter()
        .all(|mixer| mixer.strips.iter().all(|strip| strip.fader.is_none())));
}

#[test]
fn zen_go_candidate_preamp_meter_filter_preserves_range_exception_and_rejection() {
    let profile = test_support::zen_go_profile();
    let driver = ZenGoDriver::new(profile.clone()).expect("driver");
    let template = test_support::hex_fixture(include_str!(
        "fixtures/zen_go/state_with_candidate_meters.hex"
    ));

    for (raw, expected) in [
        (0x00, None),
        (0x01, Some(0x01)),
        (0x49, Some(0x49)),
        (0x4a, None),
        (0x51, None),
        (0x52, Some(0x52)),
        (0x53, None),
    ] {
        let mut frame = template.clone();
        for meter in profile.candidate_preamp_meters() {
            frame[antelope_protocol::SNAPSHOT_PAYLOAD_OFFSET + meter.offset] = raw;
        }
        let DeviceEvent::Snapshot { state, .. } =
            driver.decode(&frame).expect("decode").expect("snapshot")
        else {
            panic!("expected snapshot");
        };
        assert_eq!(
            state
                .inputs
                .iter()
                .find(|input| input.address.index == 0)
                .expect("input 1")
                .meter,
            expected,
            "raw candidate value {raw:#x}"
        );
        assert_eq!(
            state
                .inputs
                .iter()
                .find(|input| input.address.index == 1)
                .expect("input 2")
                .meter,
            expected,
            "raw candidate value {raw:#x}"
        );
    }
}

#[test]
fn zen_go_candidate_preamp_meter_uses_profile_offset_and_missing_bytes_stay_unknown() {
    let mut profile = test_support::zen_go_profile();
    profile
        .state_report
        .as_mut()
        .expect("state report")
        .candidate_preamp_meters[0]
        .offset = 0xd0;
    let driver = ZenGoDriver::new(profile.clone()).expect("driver");
    let mut frame = test_support::hex_fixture(include_str!(
        "fixtures/zen_go/state_with_candidate_meters.hex"
    ));
    frame[antelope_protocol::SNAPSHOT_PAYLOAD_OFFSET + 0xce] = 0x41;
    frame[antelope_protocol::SNAPSHOT_PAYLOAD_OFFSET + 0xd0] = 0x2a;
    let DeviceEvent::Snapshot { state, .. } =
        driver.decode(&frame).expect("decode").expect("snapshot")
    else {
        panic!("expected snapshot");
    };
    assert_eq!(
        state
            .inputs
            .iter()
            .find(|input| input.address.index == 0)
            .expect("input 1")
            .meter,
        Some(0x2a)
    );

    let mut truncated = frame;
    truncated.truncate(antelope_protocol::SNAPSHOT_PAYLOAD_OFFSET + 0xce);
    let DeviceEvent::Snapshot { state, .. } = driver
        .decode(&truncated)
        .expect("decode truncated snapshot")
        .expect("snapshot")
    else {
        panic!("expected snapshot");
    };
    assert!(state.inputs.iter().all(|input| input.meter.is_none()));
    assert!(driver.decode(&truncated[..5]).is_err());
}

#[test]
fn zen_go_driver_rejects_reindexed_physical_inputs_and_candidates() {
    let mut profile = test_support::zen_go_profile();
    for input in profile
        .inputs
        .iter_mut()
        .filter(|input| input.space == "physical_inputs")
    {
        input.index += 1;
    }
    for meter in &mut profile
        .state_report
        .as_mut()
        .expect("state report")
        .candidate_preamp_meters
    {
        meter.input_index += 1;
    }

    let error = ZenGoDriver::new(profile).expect_err("Zen Go physical inputs must stay 0 and 1");
    assert!(error.to_string().contains("indices 0 and 1"));
}

#[test]
fn zen_go_driver_rejects_candidate_meter_outside_payload_geometry() {
    let mut profile = test_support::zen_go_profile();
    profile
        .state_report
        .as_mut()
        .expect("state report")
        .candidate_preamp_meters[0]
        .offset = 0x130;

    let error = ZenGoDriver::new(profile).expect_err("candidate offset must be bounded");
    assert!(error.to_string().contains("payload offset"));
}

#[test]
fn zen_go_83_report_remains_auxiliary() {
    let driver = ZenGoDriver::new(test_support::zen_go_profile()).expect("driver");
    let event = driver
        .decode(&test_support::hex_fixture(include_str!(
            "fixtures/zen_go/meter_83_auxiliary.hex"
        )))
        .expect("decode")
        .expect("event");
    assert!(matches!(event, DeviceEvent::Auxiliary { .. }));
}

#[test]
fn zen_go_driver_rejects_mapped_meters_without_finite_report_size() {
    let mut profile = test_support::zen_go_profile();
    assert!(!profile.meter_mappings.is_empty());
    profile.transport.report_size = None;

    let error = ZenGoDriver::new(profile).expect_err("mapped meters require report geometry");
    assert!(error.to_string().contains("finite report size"));
}

#[test]
fn zen_go_driver_preserves_empty_map_compatibility_without_report_size() {
    let mut profile = test_support::zen_go_profile();
    profile.meter_mappings.clear();
    profile.state_report = None;
    profile.transport.report_size = None;

    ZenGoDriver::new(profile).expect("empty meter maps retain old-profile compatibility");
}

#[test]
fn zen_go_driver_rejects_wrong_identity_and_incomplete_topology() {
    let mut wrong = test_support::zen_go_profile();
    wrong.identity.pid = 0xa221;
    assert!(ZenGoDriver::new(wrong).is_err());
    let mut incomplete = test_support::zen_go_profile();
    incomplete.outputs.pop();
    assert!(ZenGoDriver::new(incomplete).is_err());
}
