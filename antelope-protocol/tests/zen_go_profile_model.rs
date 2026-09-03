use antelope_protocol::{encode_profile_query, QueryRequest, RuntimeProfile};

mod test_support {
    use antelope_protocol::{load_profile_pack, RuntimeProfile};

    pub fn zen_go_profile() -> RuntimeProfile {
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
}

fn zen_go_profile() -> RuntimeProfile {
    test_support::zen_go_profile()
}

#[test]
fn zen_go_profile_keeps_all_safe_queries_without_category_bounds() {
    let profile = zen_go_profile();
    let readback = profile.readback.as_ref().expect("readback metadata");
    assert!(readback.category_counts.is_empty());
    assert_eq!(readback.safe_queries.len(), 47);
    assert!(readback.allows(QueryRequest {
        query_id: 0x04,
        sub_id: 0
    }));
    assert!(readback.allows(QueryRequest {
        query_id: 0x04,
        sub_id: 3
    }));
    assert!(!readback.allows(QueryRequest {
        query_id: 0x04,
        sub_id: 4
    }));
}

#[test]
fn zen_go_profile_exposes_only_capture_scoped_mixer_layouts() {
    let profile = zen_go_profile();
    let readback = profile.readback.as_ref().expect("readback metadata");
    let q040 = readback
        .layout_for(QueryRequest {
            query_id: 0x04,
            sub_id: 0,
        })
        .expect("q04/0 layout");
    let q180 = readback
        .layout_for(QueryRequest {
            query_id: 0x18,
            sub_id: 0,
        })
        .expect("q18/0 layout");

    assert_eq!(
        (q040.body_size, q040.record_count, q040.record_stride),
        (34, 16, 2)
    );
    assert_eq!(q040.surface, Some(0));
    assert_eq!(q180.supported_fields, vec!["solo".to_owned()]);
}

#[test]
fn zen_go_profile_exposes_candidate_preamp_meters_and_attenuation_fader() {
    let profile = zen_go_profile();
    assert_eq!(profile.candidate_preamp_meter(0), Some(0xce));
    assert_eq!(profile.candidate_preamp_meter(1), Some(0xcf));
    assert_eq!(profile.mixer_fader(0).expect("fader").unity, 0);
    assert_eq!(profile.mixer_fader(0).expect("fader").max, 90);
}

#[test]
fn zen_go_profile_codec_encodes_explicit_q04_3_and_rejects_q04_4() {
    let profile = zen_go_profile();
    let readback = profile.readback.as_ref().expect("readback metadata");
    assert!(readback.category_counts.is_empty());
    let frame = encode_profile_query(
        &profile,
        QueryRequest {
            query_id: 0x04,
            sub_id: 3,
        },
    )
    .expect("explicitly safe q04/3");
    assert_eq!(&frame[0..8], &[0x74, 0, 0, 0, 0x10, 0, 0, 0]);
    assert_eq!(frame[8], 0x04);
    assert_eq!(frame[12], 3);
    assert!(encode_profile_query(
        &profile,
        QueryRequest {
            query_id: 0x04,
            sub_id: 4,
        },
    )
    .is_err());
}
