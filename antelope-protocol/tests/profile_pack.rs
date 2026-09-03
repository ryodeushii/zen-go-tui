use antelope_protocol::{
    load_profile_pack, FrameOperation, ProfileDriver, ProfileLoadError, ProfilePack,
    RuntimeDriverKind, RuntimeEntry, RuntimeInputCapability, RuntimeInputControlKind,
    RuntimeReadiness,
};

fn fixture_pack() -> ProfilePack {
    load_profile_pack(include_bytes!("fixtures/profile_pack_v1.json")).expect("fixture must load")
}

fn valid_pack_with_two_same_id() -> ProfilePack {
    let mut pack = fixture_pack();
    let duplicate: RuntimeEntry = pack.profiles()[0].clone();
    pack.profiles.push(duplicate);
    pack
}

fn pack_with_readback_index_outside_count() -> ProfilePack {
    let mut pack = fixture_pack();
    pack.profiles[0].profile.startup_queries[0].sub_id = 1;
    pack
}

#[test]
fn loads_version_one_profile_pack() {
    let pack = load_profile_pack(include_bytes!("fixtures/profile_pack_v1.json"))
        .expect("fixture must load");
    assert_eq!(pack.schema_version(), 1);
    assert_eq!(pack.profiles().len(), 1);
    assert_eq!(pack.profiles()[0].profile().identity().pid, 0xa221);
}

#[test]
fn validates_all_canonical_input_capability_kind_key_pairs() {
    let legal_pairs = [
        (RuntimeInputControlKind::Gain, "gain", Some(0x50)),
        (RuntimeInputControlKind::Gain, "adat_gain", Some(0x5b)),
        (RuntimeInputControlKind::Gain, "spdif_gain", Some(0x5c)),
        (RuntimeInputControlKind::Mode, "input_mode", Some(0x4f)),
        (RuntimeInputControlKind::Phantom, "phantom", Some(0x51)),
        (RuntimeInputControlKind::Phase, "phase_invert", Some(0x52)),
        (RuntimeInputControlKind::Link, "channel_link", None),
        (RuntimeInputControlKind::Link, "adat_channel_link", None),
        (RuntimeInputControlKind::Link, "spdif_channel_link", None),
    ];
    for (kind, parameter_key, parameter_id) in legal_pairs {
        let mut pack = fixture_pack();
        let mut param = pack.profiles[0].profile.params[0].clone();
        param.name = parameter_key.into();
        param.id = parameter_id;
        pack.profiles[0].profile.params = vec![param];
        pack.profiles[0].profile.address_spaces[0].input_capabilities =
            vec![RuntimeInputCapability {
                kind,
                parameter: parameter_key.into(),
                parameter_id,
                label: "CONTROL".into(),
            }];
        ProfilePack::validate(pack).unwrap_or_else(|error| {
            panic!("canonical pair {kind:?}/{parameter_key} must validate: {error}")
        });
    }
}

#[test]
fn rejects_external_input_capability_kind_key_mismatch_with_context() {
    let mut pack = fixture_pack();
    let mut param = pack.profiles[0].profile.params[0].clone();
    param.name = "adat_gain".into();
    param.id = Some(0x5b);
    pack.profiles[0].profile.params = vec![param];
    pack.profiles[0].profile.address_spaces[0].input_capabilities = vec![RuntimeInputCapability {
        kind: RuntimeInputControlKind::Phantom,
        parameter: "adat_gain".into(),
        parameter_id: Some(0x5b),
        label: "48V".into(),
    }];

    let error = ProfilePack::validate(pack).expect_err("mismatched kind/key must fail");
    assert!(matches!(
        &error,
        ProfileLoadError::InvalidReportGeometry { .. }
    ));
    let message = error.to_string();
    assert!(message.contains("physical_inputs"));
    assert!(message.contains("phantom"));
    assert!(message.contains("adat_gain"));
}

#[test]
fn rejects_external_input_capability_optional_id_mismatch_with_context() {
    let mut pack = fixture_pack();
    let mut param = pack.profiles[0].profile.params[0].clone();
    param.name = "channel_link".into();
    param.id = None;
    pack.profiles[0].profile.params = vec![param];
    pack.profiles[0].profile.address_spaces[0].input_capabilities = vec![RuntimeInputCapability {
        kind: RuntimeInputControlKind::Link,
        parameter: "channel_link".into(),
        parameter_id: Some(0xa2),
        label: "LINK".into(),
    }];

    let error = ProfilePack::validate(pack).expect_err("Some versus None must fail");
    assert!(matches!(
        &error,
        ProfileLoadError::InvalidReportGeometry { .. }
    ));
    let message = error.to_string();
    assert!(message.contains("physical_inputs"));
    assert!(message.contains("link"));
    assert!(message.contains("channel_link"));
}

#[test]
fn rejects_external_pack_with_uppercase_source_sha256() {
    let mut pack = fixture_pack();
    pack.profiles[0].profile.provenance.source_sha256 = "A".repeat(64);
    let bytes = serde_json::to_vec(&pack).expect("serialize uppercase SHA-256 pack");

    let error = load_profile_pack(&bytes).expect_err("uppercase SHA-256 must fail at load time");
    assert!(matches!(error, ProfileLoadError::InvalidProvenance { .. }));
    assert!(error.to_string().contains("lowercase"));
}

#[test]
fn rejects_unknown_schema_version() {
    let error = load_profile_pack(br#"{"schema_version":99,"profiles":[]}"#)
        .expect_err("unknown schema must fail");
    assert!(matches!(
        error,
        ProfileLoadError::UnsupportedSchemaVersion { .. }
    ));
}

#[test]
fn rejects_duplicate_identity_and_profile_id() {
    let pack = valid_pack_with_two_same_id();
    let error = ProfilePack::validate(pack).expect_err("duplicate identity must fail");
    assert!(matches!(error, ProfileLoadError::DuplicateProfileId { .. }));
}

#[test]
fn rejects_unsafe_readback_bounds_and_unconfirmed_commands() {
    let pack = pack_with_readback_index_outside_count();
    let error = ProfilePack::validate(pack).expect_err("unsafe query must fail");
    assert!(matches!(
        error,
        ProfileLoadError::InvalidReadbackBounds { .. }
    ));
}

#[test]
fn file_loader_reads_owned_profile_pack() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/profile_pack_v1.json");
    let pack = antelope_protocol::load_profile_pack_file(&path).expect("fixture file");
    assert_eq!(pack.profiles()[0].id, "orion_studio_3");
}

#[test]
fn promoted_orion_fixture_constructs_profile_driver() {
    let pack = antelope_protocol::load_profile_pack(include_bytes!(
        "fixtures/orion/profile_driver_pack.json"
    ))
    .expect("promoted Orion fixture must load");
    let entry = pack
        .profiles
        .into_iter()
        .find(|entry| entry.id == "orion_studio_3")
        .expect("promoted Orion profile");
    assert_eq!(entry.readiness, RuntimeReadiness::Supported);
    assert_eq!(entry.driver_kind, RuntimeDriverKind::Profile);
    ProfileDriver::new(entry).expect("promoted Orion profile driver");
}

#[test]
fn rejects_indexed_operation_when_final_reachable_index_exceeds_report() {
    let mut pack = fixture_pack();
    pack.profiles[0].profile.frames[0]
        .operations
        .push(FrameOperation::Indexed {
            base: 319,
            stride: 1,
            index_field: "physical_inputs".into(),
            width: 1,
            max_index: Some(1),
        });
    let error = ProfilePack::validate(pack).expect_err("reachable span must fit");
    assert!(matches!(
        error,
        ProfileLoadError::InvalidReportGeometry { .. }
    ));
}

#[test]
fn rejects_indexed_operation_without_proven_finite_domain() {
    let mut pack = fixture_pack();
    pack.profiles[0].profile.frames[0]
        .operations
        .push(FrameOperation::Indexed {
            base: 16,
            stride: 1,
            index_field: "unknown_targets".into(),
            width: 1,
            max_index: None,
        });
    let error = ProfilePack::validate(pack).expect_err("missing domain must fail");
    assert!(matches!(
        error,
        ProfileLoadError::MissingOperationDomain { .. }
    ));
}

#[test]
fn rejects_pair_index_when_final_reachable_pair_exceeds_report() {
    let mut pack = fixture_pack();
    pack.profiles[0].profile.frames[0]
        .operations
        .push(FrameOperation::PairIndex {
            base: 319,
            stride: 1,
            pair_field: "input_pairs".into(),
            width: 2,
            max_index: Some(0),
        });
    let error = ProfilePack::validate(pack).expect_err("pair write width must fit");
    assert!(matches!(
        error,
        ProfileLoadError::InvalidReportGeometry { .. }
    ));
}

#[test]
fn rejects_supported_entry_without_driver() {
    let mut pack = fixture_pack();
    pack.profiles[0].driver_kind = RuntimeDriverKind::None;
    let error = ProfilePack::validate(pack).expect_err("supported + none must fail");
    assert!(matches!(
        error,
        ProfileLoadError::InvalidDriverReadiness { .. }
    ));
}

#[test]
fn rejects_supported_zen_go_driver_for_non_zen_identity() {
    let mut pack = fixture_pack();
    pack.profiles[0].driver_kind = RuntimeDriverKind::ZenGo;
    let error = ProfilePack::validate(pack).expect_err("Zen Go driver identity must be exact");
    assert!(matches!(
        error,
        ProfileLoadError::InvalidDriverReadiness { .. }
    ));
}

#[test]
fn rejects_supported_non_zen_entry_that_is_not_profile_driven() {
    let mut pack = fixture_pack();
    pack.profiles[0].profile.identity.pid = 0xff01;
    pack.profiles[0].driver_kind = RuntimeDriverKind::None;
    let error = ProfilePack::validate(pack).expect_err("non-Zen supported entry must use profile");
    assert!(matches!(
        error,
        ProfileLoadError::InvalidDriverReadiness { .. }
    ));
}

#[test]
fn allows_disabled_entry_to_preserve_absent_generic_data() {
    let mut pack = fixture_pack();
    pack.profiles[0].readiness = RuntimeReadiness::Disabled;
    pack.profiles[0].driver_kind = RuntimeDriverKind::None;
    pack.profiles[0].profile.readback = None;
    pack.profiles[0].profile.startup_queries.clear();
    ProfilePack::validate(pack).expect("disabled profile preserves absence");
}

#[test]
fn load_rejects_selectable_profile_without_link_domains() {
    let mut pack = fixture_pack();
    pack.profiles[0].profile.link_domains.clear();
    let bytes = serde_json::to_vec(&pack).expect("serialize mutated external pack");

    let error = load_profile_pack(&bytes).expect_err("missing link domains must fail at load time");
    assert!(matches!(
        error,
        ProfileLoadError::MissingRequiredField { .. }
    ));
    let message = error.to_string();
    assert!(message.contains("orion_studio_3"));
    assert!(message.contains("link_domains"));
}

#[test]
fn rejects_selectable_routing_groups_without_per_destination_source_domains() {
    let mut pack = fixture_pack();
    pack.profiles[0].profile.routing_groups[0]
        .source_domains
        .clear();
    let error = ProfilePack::validate(pack).expect_err("missing source domain must fail");
    assert!(matches!(
        error,
        ProfileLoadError::MissingRequiredField { .. }
    ));
}

#[test]
fn rejects_duplicate_empty_overflowing_or_unconfirmed_routing_source_domains() {
    for mutate in 0..4 {
        let mut pack = fixture_pack();
        let domains = &mut pack.profiles[0].profile.routing_groups[0].source_domains;
        match mutate {
            0 => domains.push(domains[0].clone()),
            1 => domains[0].index_count = 0,
            2 => domains[0].index_count = 257,
            3 => domains[0].status = "unconfirmed".into(),
            _ => unreachable!(),
        }
        let error = ProfilePack::validate(pack).expect_err("invalid source domain must fail");
        assert!(matches!(
            error,
            ProfileLoadError::InvalidReportGeometry { .. }
        ));
    }
}

#[test]
fn rejects_unconfirmed_selectable_parameter() {
    let mut pack = fixture_pack();
    pack.profiles[0].profile.params[0].status = "unconfirmed".into();
    let error = ProfilePack::validate(pack).expect_err("command parameter must be confirmed");
    assert!(matches!(error, ProfileLoadError::UnconfirmedCommand { .. }));
}
