use antelope_protocol::{
    load_profile_pack, Action, DeviceDriver, DeviceEvent, DynamicStatePatch, MixerAddress,
    QueryRequest, ZenGoDriver,
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
            pan: 32,
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
            pan: 32,
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
        assert_eq!(surface.strips[0].pan, Some(0x20));
        assert_eq!(surface.strips[0].muted, Some(false));
        assert_eq!(surface.strips[0].soloed, Some(false));
    }
}

#[test]
fn zen_go_q18_readback_only_emits_declared_solo_field() {
    let driver = ZenGoDriver::new(test_support::zen_go_profile()).expect("driver");
    let event = driver
        .decode(&test_support::hex_fixture(include_str!(
            "fixtures/zen_go/q18_reply.hex"
        )))
        .expect("decode")
        .expect("event");
    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Mixer(surface)),
        ..
    } = event
    else {
        panic!("expected mixer patch");
    };
    assert_eq!(surface.strips[0].fader, None);
    assert_eq!(surface.strips[0].pan, None);
    assert_eq!(surface.strips[0].muted, None);
    assert_eq!(surface.strips[0].meter, None);
    assert_eq!(surface.strips[0].soloed, Some(true));
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
fn zen_go_driver_rejects_wrong_identity_and_incomplete_topology() {
    let mut wrong = test_support::zen_go_profile();
    wrong.identity.pid = 0xa221;
    assert!(ZenGoDriver::new(wrong).is_err());
    let mut incomplete = test_support::zen_go_profile();
    incomplete.outputs.pop();
    assert!(ZenGoDriver::new(incomplete).is_err());
}
