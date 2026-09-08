use antelope_protocol::{
    encode_command, encode_mixer_assignment_frames_with_table, encode_query, load_profile_pack,
    Action, Command, ControlValue, DeviceDriver, DeviceEvent, DriverError, DynamicMixerSurface,
    DynamicStatePatch, FrameEndian, FrameOperation, GlobalControl, InputAddress, InputControl,
    MixerAddress, MixerControl, OutputAddress, OutputControl, OutputTrimAddress,
    ParamReadbackField, ProfileDriver, QueryRequest, RoutingSource, RuntimeConstraint,
    RuntimeDriverKind, RuntimeEntry, RuntimeFrame, RuntimeMeterMapping, RuntimeMeterTarget,
    RuntimeReadiness, RuntimeRoutingReadbackSourceDomain, WholeStateField, ZenGoDriver,
};

fn stored_orion_entry() -> RuntimeEntry {
    load_profile_pack(include_bytes!("fixtures/orion/profile_driver_pack.json"))
        .expect("fixture pack")
        .profiles
        .into_iter()
        .next()
        .expect("fixture entry")
}

fn fixture_entry() -> RuntimeEntry {
    stored_orion_entry()
}

fn observed_readback_fixture_entry() -> RuntimeEntry {
    let mut entry = fixture_entry();
    for group in &mut entry.profile.routing_groups {
        group.source_domains.retain(|domain| domain.bank != 2);
        group
            .readback_source_domains
            .push(RuntimeRoutingReadbackSourceDomain {
                bank: 2,
                indices: (0..24).collect(),
                status: "observed".into(),
                evidence: "synthetic host-dependent readback fixture".into(),
            });
    }
    entry
}

fn canonical_orion_entry() -> RuntimeEntry {
    load_profile_pack(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/device/generated_profiles.json"
    )))
    .expect("generated profile pack")
    .profiles
    .into_iter()
    .find(|entry| entry.profile.identity.pid == 0xa221)
    .expect("canonical Orion profile")
}

fn state_meter_fixture_entry() -> RuntimeEntry {
    fixture_entry()
}

fn confirmed_meter_fixture_entry() -> RuntimeEntry {
    let mut entry = fixture_entry();
    let meter_frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "meter_report")
        .expect("meter report");
    meter_frame.status = "confirmed".into();
    meter_frame.operations.extend([
        FrameOperation::Scalar {
            offset: 32,
            width: 1,
            field: "channel_meter_base".into(),
            endian: FrameEndian::NotApplicable,
        },
        FrameOperation::FixedByte {
            offset: 1,
            value: 0x1f,
        },
    ]);
    entry
        .profile
        .decoders
        .iter_mut()
        .find(|decoder| decoder.frame_id == "meter_report")
        .expect("meter decoder")
        .status = "confirmed".into();
    entry
}

fn disabled_orion_entry() -> RuntimeEntry {
    let mut entry = stored_orion_entry();
    entry.readiness = RuntimeReadiness::Disabled;
    entry.driver_kind = RuntimeDriverKind::None;
    entry.profile.transport.uses_numbered_reports = None;
    entry
}

fn profile_driver_from_fixture() -> ProfileDriver {
    ProfileDriver::new(fixture_entry()).expect("profile driver")
}

fn zen_go_driver() -> ZenGoDriver {
    let profile = load_profile_pack(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/device/generated_profiles.json"
    )))
    .expect("generated profile pack")
    .profiles
    .into_iter()
    .find(|entry| entry.profile.identity.pid == 0xa015)
    .expect("Zen Go profile")
    .profile;
    ZenGoDriver::new(profile).expect("Zen Go driver")
}

fn non_orion_fixture_entry() -> RuntimeEntry {
    let mut entry = fixture_entry();
    entry.id = "synthetic_other_profile".into();
    entry.profile.identity.vid = 0x1234;
    entry.profile.identity.pid = 0x5678;
    // This synthetic profile changes bus parameters to generic output scope,
    // so it cannot retain Orion's strictly bus-scoped MONO capability.
    entry
        .profile
        .constraints
        .retain(|constraint| constraint.name != "output_mono_targets");
    for parameter in &mut entry.profile.params {
        let applies_to = match parameter.name.as_str() {
            name if name.starts_with("bus_") => "outputs",
            "adat_gain" => "adat_inputs",
            "spdif_gain" => "spdif_inputs",
            "gain" => "physical_inputs",
            name if name.starts_with("mix_") => "mixers",
            _ => parameter.applies_to.as_str(),
        }
        .to_string();
        parameter.applies_to = applies_to;
        // Replace Orion's legacy-only offset aliases so non-Orion tests can
        // exercise unrelated behavior under strict reference validation.
        if parameter
            .frame
            .offsets
            .iter()
            .all(|(field, _)| field.starts_with("offset_"))
        {
            parameter.frame.offsets = if parameter.applies_to == "globals" {
                vec![("value".into(), 17)]
            } else {
                vec![
                    ("param_id".into(), 16),
                    ("channel".into(), 17),
                    ("value".into(), 18),
                ]
            };
        }
    }
    entry
}

fn add_topology_constraints(entry: &mut RuntimeEntry) {
    for (name, scalar) in [
        ("mixer_readback_category", 0x04),
        ("routing_readback_category", 0x03),
        ("routing_source_count", 32),
        ("routing_destination_count", 15),
    ] {
        entry.profile.constraints.push(RuntimeConstraint {
            name: name.into(),
            status: "confirmed".into(),
            range: None,
            values: Vec::new(),
            scalar: Some(scalar),
            text: String::new(),
            metadata: String::new(),
        });
    }
}

fn hex_fixture(text: &str) -> Vec<u8> {
    text.lines()
        .flat_map(|line| {
            line.split('#')
                .next()
                .unwrap_or_default()
                .split_ascii_whitespace()
        })
        .map(|byte| u8::from_str_radix(byte, 16).expect("hex byte"))
        .collect()
}

fn decode_orion_mixer_record() -> DynamicMixerSurface {
    let frame = hex_fixture(include_str!("fixtures/orion/readback_75.hex"));
    let event = profile_driver_from_fixture()
        .decode(&frame)
        .expect("decode")
        .expect("event");
    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Mixer(surface)),
        ..
    } = event
    else {
        panic!("expected mixer patch")
    };
    surface
}

#[test]
fn profile_derived_fixture_preserves_disabled_runtime_policy() {
    let entry = disabled_orion_entry();
    assert_eq!(entry.readiness, RuntimeReadiness::Disabled);
    assert_eq!(entry.driver_kind, RuntimeDriverKind::None);
    assert!(entry.profile.transport.uses_numbered_reports.is_none());
}

#[test]
fn profile_driver_rejects_disabled_profile_before_encoding() {
    let error = ProfileDriver::new(disabled_orion_entry()).expect_err("disabled must fail");
    assert!(matches!(error, DriverError::UnsupportedAction(_)));
}

#[test]
fn promoted_canonical_orion_rejects_unconfirmed_report_framing() {
    let mut entry = disabled_orion_entry();
    entry.readiness = RuntimeReadiness::Supported;
    entry.driver_kind = RuntimeDriverKind::Profile;

    let error = ProfileDriver::new(entry).expect_err("unknown report framing must fail");
    assert!(error.to_string().contains("unconfirmed report framing"));
}

#[test]
fn canonical_orion_fixture_constructs_for_generic_codec_tests() {
    ProfileDriver::new(fixture_entry()).expect("canonical Orion fixture");
}

#[test]
fn observation_only_decoder_with_overlapping_fields_does_not_block_driver() {
    let mut entry = fixture_entry();
    entry.profile.frames.push(RuntimeFrame {
        id: "observation_only".into(),
        kind: "decoder".into(),
        status: "observed".into(),
        report_size: Some(320),
        operations: vec![
            FrameOperation::FixedByte {
                offset: 16,
                value: 0xd7,
            },
            FrameOperation::FixedByte {
                offset: 16,
                value: 0x98,
            },
        ],
        metadata: String::new(),
    });

    ProfileDriver::new(entry).expect("observation-only decoder must not block driver");
}

#[test]
fn synthetic_numbered_fixture_rejects_unrepresentable_generic_framing() {
    let mut entry = fixture_entry();
    // Synthetic test assumption: exercise known-numbered framing without
    // making any canonical Orion transport claim.
    entry.profile.transport.uses_numbered_reports = Some(true);

    let error = ProfileDriver::new(entry).expect_err("numbered framing is not representable");
    assert!(error.to_string().contains("numbered report framing"));
}

#[test]
fn profile_driver_rejects_supported_non_profile_entry() {
    let mut entry = fixture_entry();
    entry.driver_kind = RuntimeDriverKind::ZenGo;
    let error = ProfileDriver::new(entry).expect_err("non-profile must fail");
    assert!(matches!(error, DriverError::UnsupportedAction(_)));
}

#[test]
fn non_orion_rejects_alias_like_parameter_with_mismatched_target() {
    let mut entry = non_orion_fixture_entry();
    add_topology_constraints(&mut entry);
    entry
        .profile
        .params
        .iter_mut()
        .find(|parameter| parameter.name == "bus_level")
        .expect("bus_level fixture parameter")
        .applies_to = "physical_inputs".into();
    let driver = ProfileDriver::new(entry).expect("current behavior constructs driver");
    let error = driver
        .encode(Action::SetOutput {
            address: OutputAddress { id: 5 },
            control: OutputControl::Parameter(71),
            value: ControlValue::Int(1),
        })
        .expect_err("mismatched target must be rejected");
    assert!(matches!(error, DriverError::InvalidAction(_)));
}

#[test]
fn non_orion_rejects_unknown_offset_semantic() {
    let mut entry = non_orion_fixture_entry();
    entry
        .profile
        .params
        .iter_mut()
        .find(|parameter| parameter.name == "gain")
        .expect("gain fixture parameter")
        .frame
        .offsets
        .push(("offset_unknown".into(), 16));
    let error = ProfileDriver::new(entry).expect_err("unknown offset semantic must be rejected");
    assert!(matches!(error, DriverError::InvalidAction(_)));
}

#[test]
fn non_orion_rejects_parameter_with_only_unknown_offset_semantics() {
    let mut entry = non_orion_fixture_entry();
    add_topology_constraints(&mut entry);
    entry
        .profile
        .params
        .iter_mut()
        .find(|parameter| parameter.name == "gain")
        .expect("gain fixture parameter")
        .frame
        .offsets = vec![("offset_unknown".into(), 16)];

    let error = ProfileDriver::new(entry)
        .expect_err("non-Orion unknown-only offset parameter must be rejected");
    assert!(matches!(error, DriverError::InvalidAction(_)));
}

#[test]
fn non_orion_rejects_topology_inference_without_scalar_constraints() {
    let entry = non_orion_fixture_entry();
    let error = ProfileDriver::new(entry)
        .expect_err("missing routing/category scalars must reject non-Orion profile");
    assert!(matches!(error, DriverError::InvalidAction(_)));
}

#[test]
fn profile_driver_rejects_unconfirmed_or_absent_parameter() {
    let driver = profile_driver_from_fixture();
    let error = driver
        .encode(Action::SetOutput {
            address: OutputAddress { id: 5 },
            control: OutputControl::Parameter(99),
            value: ControlValue::Int(1),
        })
        .expect_err("absent parameter must fail");
    assert!(matches!(error, DriverError::UnsupportedAction(_)));

    let error = driver
        .encode(Action::SetInput {
            address: InputAddress { space: 0, index: 0 },
            control: InputControl::Parameter(3),
            value: ControlValue::Int(1),
        })
        .expect_err("global parameter id must not map into input frame");
    assert!(matches!(error, DriverError::InvalidAction(_)));
}

#[test]
fn profile_driver_encodes_full_frames_and_checks_bounds() {
    let driver = profile_driver_from_fixture();
    let input = driver
        .encode(Action::SetInput {
            address: InputAddress {
                space: 0,
                index: 11,
            },
            control: InputControl::Gain,
            value: ControlValue::Int(12),
        })
        .expect("input frame");
    assert_eq!(input.frames[0].len(), 320);
    assert_eq!(&input.frames[0][0..5], &[0x70, 0, 0, 0, 0x13]);
    assert_eq!(&input.frames[0][16..19], &[0x50, 11, 12]);

    let output = driver
        .encode(Action::SetOutput {
            address: OutputAddress { id: 5 },
            control: OutputControl::Level,
            value: ControlValue::Int(12),
        })
        .expect("output frame");
    assert_eq!(&output.frames[0][16..19], &[0x47, 5, 12]);

    let mixer = driver
        .encode(Action::SetMixerStripState {
            address: MixerAddress {
                surface: 3,
                strip: 32,
            },
            fader: 60,
            pan: 30,
            muted: false,
            soloed: false,
            send: Some(30),
        })
        .expect("mixer frame");
    assert_eq!(&mixer.frames[0][16..23], &[0xd4, 0x05, 3, 32, 60, 62, 30]);

    for action in [
        Action::SetInput {
            address: InputAddress {
                space: 0,
                index: 12,
            },
            control: InputControl::Gain,
            value: ControlValue::Int(1),
        },
        Action::SetOutput {
            address: OutputAddress { id: 6 },
            control: OutputControl::Level,
            value: ControlValue::Int(1),
        },
        Action::SetMixer {
            address: MixerAddress {
                surface: 4,
                strip: 1,
            },
            control: MixerControl::Fader,
            value: ControlValue::Int(1),
        },
        Action::SetRouting {
            destination: 99,
            channel: 0,
            source: RoutingSource { bank: 0, index: 0 },
        },
    ] {
        assert!(driver.encode(action).is_err());
    }
}

#[test]
fn orion_physical_line_gain_negative_boundary_encodes_and_decodes_as_int8() {
    let driver = profile_driver_from_fixture();
    let frame = driver
        .encode(Action::SetInput {
            address: InputAddress { space: 0, index: 0 },
            control: InputControl::Gain,
            value: ControlValue::Int(-6),
        })
        .expect("Orion line minimum")
        .frames
        .remove(0);
    let mut expected = vec![0; 320];
    expected[0] = 0x70;
    expected[4] = 0x13;
    expected[16..19].copy_from_slice(&[0x50, 0, 0xfa]);
    assert_eq!(frame, expected);

    for value in [-7, 76] {
        assert!(matches!(
            driver.encode(Action::SetInput {
                address: InputAddress { space: 0, index: 0 },
                control: InputControl::Gain,
                value: ControlValue::Int(value),
            }),
            Err(DriverError::InvalidAction(_))
        ));
    }

    let mut snapshot = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    snapshot[49] = 0xfa;
    snapshot[61] = 0x01;
    let DeviceEvent::Snapshot { state, .. } = driver
        .decode(&snapshot)
        .expect("signed gain readback")
        .expect("state event")
    else {
        panic!("snapshot")
    };
    let input = state
        .inputs
        .iter()
        .find(|input| input.address == InputAddress { space: 0, index: 0 })
        .expect("physical input 1");
    assert_eq!(input.mode, Some(1));
    assert_eq!(input.gain, Some(-6));
}

#[test]
fn query_bounds_and_layout_are_profile_driven() {
    let driver = profile_driver_from_fixture();
    for index in 0..4 {
        let frame = driver
            .encode(Action::Query(QueryRequest::new(0x04, index)))
            .expect("bounded query")
            .frames
            .remove(0);
        assert_eq!(frame[0], 0x74);
        assert_eq!(&frame[4..8], &0x10_u32.to_le_bytes());
        assert_eq!(frame[8], 0x04);
        assert_eq!(frame[12], index);
    }
    for unsafe_index in [4, 5] {
        assert!(driver
            .encode(Action::Query(QueryRequest::new(0x04, unsafe_index)))
            .is_err());
    }
}

#[test]
fn profile_derived_startup_walk_is_exactly_113_bounded_requests() {
    let driver = profile_driver_from_fixture();
    let mut expected = vec![(0x11, 0), (0x11, 1), (0x0b, 1), (0x0b, 2), (0x1b, 0)];
    expected.extend((0..16).map(|index| (0x1a, index)));
    expected.extend((0..15).map(|index| (0x03, index)));
    for index in 0..4 {
        expected.extend([(0x04, index), (0x0b, 3)]);
    }
    expected.extend([(0x0a, 0), (0x15, 0), (0x0b, 0), (0x16, 0)]);
    expected.extend((0..64).map(|index| (0x19, index)));
    expected.push((0x0b, 4));

    let actual: Vec<_> = driver
        .startup_requests()
        .iter()
        .map(|request| (request.query_id, request.sub_id))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 113);
    for request in driver.startup_requests() {
        let frame = driver
            .encode(Action::Query(*request))
            .expect("startup request must stay inside finite bound")
            .frames
            .remove(0);
        assert_eq!(frame.len(), 320);
        assert_eq!(frame[8], request.query_id);
        assert_eq!(frame[12], request.sub_id);
    }
}

#[test]
fn profile_derived_startup_fixture_matches_every_complete_frame() {
    let fixture = hex_fixture(include_str!("fixtures/orion/startup_requests.txt"));
    assert_eq!(fixture.len(), 113 * 320);

    let driver = profile_driver_from_fixture();
    for (index, expected) in fixture.chunks(320).enumerate() {
        let request = QueryRequest::new(expected[8], expected[12]);
        let actual = driver
            .encode(Action::Query(request))
            .expect("fixture startup request must be bounded")
            .frames
            .into_iter()
            .next()
            .expect("query frame");
        assert_eq!(actual, expected, "startup frame {index}");
    }
}

#[test]
fn profile_derived_orion_geometry_is_complete() {
    let profile = &fixture_entry().profile;
    assert_eq!(profile.inputs_in("physical_inputs"), 12);
    assert_eq!(profile.inputs_in("adat_inputs"), 16);
    assert_eq!(profile.inputs_in("spdif_inputs"), 2);
    assert_eq!(profile.outputs.len(), 6);
    assert_eq!(profile.mixers.len(), 4);
    assert!(profile
        .mixers
        .iter()
        .all(|mixer| mixer.has_master && mixer.strip_count == 32));
    assert_eq!(
        profile
            .link_domains
            .iter()
            .map(|domain| (domain.protocol_space, domain.pair_count))
            .collect::<Vec<_>>(),
        vec![(1, 1), (3, 16)]
    );
    assert_eq!(
        profile
            .routing_groups
            .iter()
            .map(|group| (group.destination, group.channel_count))
            .collect::<Vec<_>>(),
        vec![
            (0, 16),
            (1, 2),
            (2, 2),
            (3, 2),
            (4, 2),
            (5, 2),
            (6, 32),
            (7, 16),
            (8, 2),
            (9, 32),
            (10, 32),
            (11, 32),
            (12, 32),
            (13, 32),
            (14, 16),
        ]
    );
}

fn required_orion_actions() -> Vec<Action> {
    vec![
        Action::SetInput {
            address: InputAddress { space: 0, index: 3 },
            control: InputControl::Mode,
            value: ControlValue::Enum(2),
        },
        Action::SetInput {
            address: InputAddress {
                space: 0,
                index: 11,
            },
            control: InputControl::Gain,
            value: ControlValue::Int(12),
        },
        Action::SetInput {
            address: InputAddress { space: 0, index: 2 },
            control: InputControl::Phantom,
            value: ControlValue::Bool(true),
        },
        Action::SetInput {
            address: InputAddress { space: 0, index: 4 },
            control: InputControl::Phase,
            value: ControlValue::Bool(true),
        },
        Action::SetInput {
            address: InputAddress {
                space: 1,
                index: 15,
            },
            control: InputControl::Gain,
            value: ControlValue::Int(-6),
        },
        Action::SetInput {
            address: InputAddress { space: 2, index: 1 },
            control: InputControl::Gain,
            value: ControlValue::Int(12),
        },
        Action::SetOutput {
            address: OutputAddress { id: 5 },
            control: OutputControl::Level,
            value: ControlValue::Int(48),
        },
        Action::SetOutput {
            address: OutputAddress { id: 3 },
            control: OutputControl::Mute,
            value: ControlValue::Bool(true),
        },
        Action::SetOutput {
            address: OutputAddress { id: 2 },
            control: OutputControl::Dim,
            value: ControlValue::Bool(true),
        },
        Action::SetOutput {
            address: OutputAddress { id: 2 },
            control: OutputControl::Mono,
            value: ControlValue::Bool(true),
        },
        Action::SetGlobal {
            control: GlobalControl::SampleRate,
            value: ControlValue::Enum(6),
        },
        Action::SetGlobal {
            control: GlobalControl::Brightness,
            value: ControlValue::Int(73),
        },
        Action::SetOutputTrim {
            address: OutputTrimAddress { target: 2 },
            value: 6,
        },
        Action::SetMixerStripState {
            address: MixerAddress {
                surface: 3,
                strip: 32,
            },
            fader: 44,
            pan: 30,
            muted: true,
            soloed: false,
            send: Some(55),
        },
        Action::SetLink {
            surface: 3,
            pair: 15,
            enabled: true,
        },
        Action::SetRoutingGroup {
            destination: 14,
            changed_channel: None,
            sources: vec![
                RoutingSource {
                    bank: 0x03,
                    index: 15
                };
                16
            ],
        },
        Action::SetWholeState {
            operation: 0xda,
            target: 0,
            enabled: true,
            fields: vec![
                WholeStateField { id: 0, value: 81 },
                WholeStateField { id: 1, value: 100 },
                WholeStateField { id: 2, value: 0 },
                WholeStateField { id: 3, value: 11 },
                WholeStateField { id: 4, value: 13 },
                WholeStateField { id: 5, value: 24 },
                WholeStateField { id: 6, value: 66 },
                WholeStateField { id: 7, value: 50 },
            ],
        },
    ]
}

fn assert_complete_parameter_frame(action: Action, opcode: u8, payload: &[u8]) {
    let actual = profile_driver_from_fixture()
        .encode(action)
        .expect("confirmed parameter frame")
        .frames
        .remove(0);
    let mut expected = vec![0; 320];
    expected[0] = 0x70;
    expected[4] = opcode;
    expected[16..16 + payload.len()].copy_from_slice(payload);
    assert_eq!(actual, expected);
}

#[test]
fn profile_derived_confirmed_parameter_families_match_complete_frames() {
    for (action, opcode, payload) in [
        (
            Action::SetInput {
                address: InputAddress { space: 0, index: 3 },
                control: InputControl::Mode,
                value: ControlValue::Enum(2),
            },
            0x13,
            vec![0x4f, 3, 2],
        ),
        (
            Action::SetInput {
                address: InputAddress {
                    space: 0,
                    index: 11,
                },
                control: InputControl::Gain,
                value: ControlValue::Int(12),
            },
            0x13,
            vec![0x50, 11, 12],
        ),
        (
            Action::SetInput {
                address: InputAddress { space: 0, index: 2 },
                control: InputControl::Phantom,
                value: ControlValue::Bool(true),
            },
            0x13,
            vec![0x51, 2, 1],
        ),
        (
            Action::SetInput {
                address: InputAddress { space: 0, index: 4 },
                control: InputControl::Phase,
                value: ControlValue::Bool(true),
            },
            0x13,
            vec![0x52, 4, 1],
        ),
        (
            Action::SetInput {
                address: InputAddress {
                    space: 1,
                    index: 15,
                },
                control: InputControl::Gain,
                value: ControlValue::Int(-6),
            },
            0x13,
            vec![0x5b, 15, 0xfa],
        ),
        (
            Action::SetInput {
                address: InputAddress { space: 2, index: 1 },
                control: InputControl::Gain,
                value: ControlValue::Int(12),
            },
            0x13,
            vec![0x5c, 1, 12],
        ),
        (
            Action::SetOutput {
                address: OutputAddress { id: 5 },
                control: OutputControl::Level,
                value: ControlValue::Int(48),
            },
            0x13,
            vec![0x47, 5, 48],
        ),
        (
            Action::SetOutput {
                address: OutputAddress { id: 3 },
                control: OutputControl::Mute,
                value: ControlValue::Bool(true),
            },
            0x13,
            vec![0x48, 3, 1],
        ),
        (
            Action::SetOutput {
                address: OutputAddress { id: 2 },
                control: OutputControl::Dim,
                value: ControlValue::Bool(true),
            },
            0x13,
            vec![0x68, 2, 1],
        ),
        (
            Action::SetOutput {
                address: OutputAddress { id: 2 },
                control: OutputControl::Mono,
                value: ControlValue::Bool(true),
            },
            0x13,
            vec![0x69, 2, 1],
        ),
        (
            Action::SetGlobal {
                control: GlobalControl::SampleRate,
                value: ControlValue::Enum(6),
            },
            0x12,
            vec![0x03, 6],
        ),
        (
            Action::SetGlobal {
                control: GlobalControl::Brightness,
                value: ControlValue::Int(73),
            },
            0x12,
            vec![0x0e, 73],
        ),
        (
            Action::SetGlobal {
                control: GlobalControl::TalkbackButton,
                value: ControlValue::Bool(true),
            },
            0x12,
            vec![0x1f, 1],
        ),
        (
            Action::SetGlobal {
                control: GlobalControl::TalkbackButton,
                value: ControlValue::Bool(false),
            },
            0x12,
            vec![0x1f, 0],
        ),
        (
            Action::SetGlobal {
                control: GlobalControl::TalkbackSource,
                value: ControlValue::Enum(12),
            },
            0x12,
            vec![0x27, 12],
        ),
        (
            Action::SetGlobal {
                control: GlobalControl::TalkbackGain,
                value: ControlValue::Int(96),
            },
            0x12,
            vec![0x20, 96],
        ),
    ] {
        assert_complete_parameter_frame(action, opcode, &payload);
    }
}

#[test]
fn profile_driver_rejects_talkback_types_ranges_and_readback_only_residue() {
    let driver = profile_driver_from_fixture();
    for value in [-1, 13] {
        assert!(driver
            .encode(Action::SetGlobal {
                control: GlobalControl::TalkbackSource,
                value: ControlValue::Enum(value),
            })
            .is_err());
    }
    for value in [-1, 97] {
        assert!(driver
            .encode(Action::SetGlobal {
                control: GlobalControl::TalkbackGain,
                value: ControlValue::Int(value),
            })
            .is_err());
    }
    assert!(driver
        .encode(Action::SetGlobal {
            control: GlobalControl::TalkbackButton,
            value: ControlValue::Int(1),
        })
        .is_err());
    assert!(driver
        .encode(Action::SetGlobal {
            control: GlobalControl::TalkbackSourceResidue,
            value: ControlValue::Enum(1),
        })
        .is_err());
}

#[test]
fn profile_driver_encodes_all_bounded_output_trim_targets() {
    let driver = profile_driver_from_fixture();
    for value in [-1, 101] {
        assert!(driver
            .encode(Action::SetGlobal {
                control: GlobalControl::Brightness,
                value: ControlValue::Int(value),
            })
            .is_err());
    }
    for target in 0..=2 {
        let batch = driver
            .encode(Action::SetOutputTrim {
                address: OutputTrimAddress { target },
                value: 6,
            })
            .expect("confirmed trim target");
        assert_eq!(batch.frames[0][4], 0x13);
        assert_eq!(&batch.frames[0][16..19], &[0x4b, target, 6]);
    }
    for (target, value) in [(3, 0), (0, -1), (0, 7)] {
        assert!(driver
            .encode(Action::SetOutputTrim {
                address: OutputTrimAddress { target },
                value,
            })
            .is_err());
    }
}

#[test]
fn profile_driver_encodes_every_confirmed_finite_orion_family() {
    let driver = profile_driver_from_fixture();
    for action in required_orion_actions() {
        let is_unconfirmed = matches!(action, Action::SetWholeState { .. });
        let result = driver.encode(action);
        if is_unconfirmed {
            assert!(
                result.is_err(),
                "unconfirmed AuraVerb action must fail closed"
            );
        } else {
            let batch = result.expect("confirmed Orion action");
            assert_eq!(batch.frames.len(), 1);
            assert_eq!(batch.frames[0].len(), 320);
        }
    }
}

#[test]
fn profile_derived_auraverb_whole_state_rejects_unconfirmed_frame() {
    let action = required_orion_actions()
        .into_iter()
        .find(|action| matches!(action, Action::SetWholeState { .. }))
        .expect("AuraVerb action");
    let error = profile_driver_from_fixture()
        .encode(action)
        .expect_err("unconfirmed AuraVerb frame must fail closed");
    assert!(error
        .to_string()
        .contains("confirmed whole-state operation"));
}

#[test]
fn whole_state_is_fail_closed_for_partial_duplicate_or_out_of_range_fields() {
    let driver = profile_driver_from_fixture();
    for fields in [
        vec![WholeStateField { id: 0, value: 1 }],
        vec![
            WholeStateField { id: 0, value: 1 },
            WholeStateField { id: 0, value: 2 },
        ],
        (0..8)
            .map(|id| WholeStateField {
                id,
                value: if id == 7 { 101 } else { 1 },
            })
            .collect(),
    ] {
        assert!(driver
            .encode(Action::SetWholeState {
                operation: 0xda,
                target: 0,
                enabled: true,
                fields,
            })
            .is_err());
    }
}

#[test]
fn superseded_meter_fallback_requires_canonical_orion_identity() {
    let mut entry = non_orion_fixture_entry();
    add_topology_constraints(&mut entry);
    entry.id = "noncanonical_orion".into();
    let driver = ProfileDriver::new(entry).expect("driver with changed runtime identity");
    let mut frame = vec![0; 320];
    frame[0] = 0x75;
    frame[1] = 0x1f;
    assert!(matches!(
        driver.decode(&frame),
        Err(DriverError::InvalidAction(_))
    ));
}

#[test]
fn readback_and_meter_discriminators_are_distinct_and_raw_is_owned() {
    let driver = profile_driver_from_fixture();
    let readback = hex_fixture(include_str!("fixtures/orion/readback_75.hex"));
    let event = driver
        .decode(&readback)
        .expect("readback decode")
        .expect("event");
    let DeviceEvent::QueryReply { body, raw, .. } = event else {
        panic!("query reply")
    };
    assert_eq!(raw, readback);
    assert_eq!(body, readback[16..]);

    let mut meter = vec![0; 320];
    meter[0] = 0x75;
    meter[1] = 0x1f;
    let event = driver.decode(&meter).expect("superseded meter decode");
    assert!(
        event.is_none(),
        "state source must ignore superseded meter response"
    );
}

#[test]
fn state_only_readback_with_unproven_discriminator_is_rejected() {
    let driver = profile_driver_from_fixture();
    let mut frame = vec![0; 320];
    frame[0] = 0x75;
    frame[1] = 0x20;
    assert!(matches!(
        driver.decode(&frame),
        Err(DriverError::InvalidAction(_))
    ));
}

#[test]
fn malformed_known_reports_fail_instead_of_being_ignored() {
    let driver = profile_driver_from_fixture();
    assert!(driver.decode(&[0x75; 12]).is_err());

    let mut unknown_readback = vec![0; 320];
    unknown_readback[0] = 0x75;
    unknown_readback[1] = 0x7f;
    assert!(matches!(
        driver.decode(&unknown_readback),
        Err(DriverError::InvalidAction(_))
    ));

    let unknown = vec![0x99; 320];
    assert!(driver.decode(&unknown).expect("unknown policy").is_none());
}

#[test]
fn runtime_rejects_output_mono_declaration_with_wrong_parameter_id() {
    let mut entry = canonical_orion_entry();
    entry
        .profile
        .params
        .iter_mut()
        .find(|parameter| parameter.name == "bus_mono")
        .expect("bus_mono parameter")
        .id = Some(0x68);

    assert!(matches!(
        ProfileDriver::new(entry),
        Err(DriverError::InvalidAction(message)) if message.contains("output mono")
    ));
}

#[test]
fn runtime_rejects_output_mono_declaration_with_non_bus_scope() {
    let mut entry = canonical_orion_entry();
    entry
        .profile
        .params
        .iter_mut()
        .find(|parameter| parameter.name == "bus_mono")
        .expect("bus_mono parameter")
        .applies_to = "outputs".into();

    assert!(matches!(
        ProfileDriver::new(entry),
        Err(DriverError::InvalidAction(message)) if message.contains("output mono")
    ));
}

#[test]
fn canonical_orion_output_mono_encodes_param_69_only_for_captured_targets() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    for bus in [0, 1, 2, 5] {
        for enabled in [false, true] {
            let frame = driver
                .encode(Action::SetOutput {
                    address: OutputAddress { id: bus },
                    control: OutputControl::Mono,
                    value: ControlValue::Bool(enabled),
                })
                .expect("captured mono target")
                .frames
                .remove(0);
            let mut expected = vec![0; 320];
            expected[0] = 0x70;
            expected[4] = 0x13;
            expected[16] = 0x69;
            expected[17] = bus as u8;
            expected[18] = u8::from(enabled);
            assert_eq!(frame, expected);
        }
    }

    for bus in [3, 4] {
        assert!(matches!(
            driver.encode(Action::SetOutput {
                address: OutputAddress { id: bus },
                control: OutputControl::Mono,
                value: ControlValue::Bool(true),
            }),
            Err(DriverError::UnsupportedAction(_))
        ));
    }
    assert!(matches!(
        driver.encode(Action::SetOutput {
            address: OutputAddress { id: 0 },
            control: OutputControl::Parameter(0x69),
            value: ControlValue::Bool(true),
        }),
        Err(DriverError::InvalidAction(_))
    ));
}

#[test]
fn canonical_orion_snapshot_decodes_mono_bit_only_for_captured_targets() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    let mut frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    for bus in [0usize, 2, 3] {
        frame[29 + 3 * bus] |= 0x10;
    }
    let DeviceEvent::Snapshot { state, .. } = driver.decode(&frame).unwrap().unwrap() else {
        panic!("snapshot")
    };

    assert_eq!(
        state
            .outputs
            .iter()
            .map(|output| (output.address.id, output.mono))
            .collect::<Vec<_>>(),
        vec![
            (0, Some(true)),
            (1, Some(false)),
            (2, Some(true)),
            (3, None),
            (4, None),
            (5, Some(false)),
        ]
    );
    assert_eq!(state.outputs[1].level, Some(20), "sibling state changed");
    assert_eq!(state.outputs[1].muted, None, "sibling state changed");
    assert_eq!(state.outputs[1].dimmed, None, "sibling state changed");
}

#[test]
fn dynamic_mixer_state_keeps_master_outside_input_strip_vector() {
    let surface = decode_orion_mixer_record();
    assert_eq!(surface.surface, 0);
    assert!(surface.master.is_some());
    assert_eq!(surface.strips.len(), 32);
}

#[test]
fn profile_derived_state_report_decodes_every_confirmed_address_and_value() {
    let driver = profile_driver_from_fixture();
    let frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    let event = driver.decode(&frame).expect("state decode").expect("event");
    let DeviceEvent::Snapshot { state, raw } = event else {
        panic!("snapshot")
    };
    assert_eq!(raw, frame);
    assert_eq!(
        state
            .inputs
            .iter()
            .filter(|input| input.address.space == 0)
            .map(|input| (
                input.address.index,
                input.gain,
                input.mode,
                input.phantom,
                input.phase
            ))
            .collect::<Vec<_>>(),
        (0..12)
            .map(|index| {
                (
                    index,
                    Some(i32::from(index) + 1),
                    Some(i32::from(index % 4)),
                    Some(index % 2 == 0),
                    Some(index % 3 == 0),
                )
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(
        state
            .inputs
            .iter()
            .filter(|input| input.address.space == 1)
            .map(|input| (input.address.index, input.gain))
            .collect::<Vec<_>>(),
        (-6..=9)
            .enumerate()
            .map(|(index, gain)| (index as u16, Some(gain)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        state
            .inputs
            .iter()
            .filter(|input| input.address.space == 2)
            .map(|input| (input.address.index, input.gain))
            .collect::<Vec<_>>(),
        vec![(0, Some(-6)), (1, Some(12))]
    );
    assert_eq!(
        state
            .outputs
            .iter()
            .map(|output| {
                (
                    output.address.id,
                    output.level,
                    output.muted,
                    output.dimmed,
                    output.mono,
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (0, Some(10), None, None, Some(false)),
            (1, Some(20), None, None, Some(false)),
            (2, Some(30), None, None, Some(false)),
            (3, Some(40), None, None, None),
            (4, Some(50), None, None, None),
            (5, Some(60), None, None, Some(false)),
        ]
    );
    assert_eq!(
        state.globals,
        vec![
            antelope_protocol::DynamicGlobalState {
                control: GlobalControl::SampleRate,
                value: ControlValue::Enum(4),
            },
            antelope_protocol::DynamicGlobalState {
                control: GlobalControl::ClockSource,
                value: ControlValue::Enum(0),
            },
            antelope_protocol::DynamicGlobalState {
                control: GlobalControl::Brightness,
                value: ControlValue::Int(73),
            },
            antelope_protocol::DynamicGlobalState {
                control: GlobalControl::OutputTrim(OutputTrimAddress { target: 0 }),
                value: ControlValue::Enum(0),
            },
            antelope_protocol::DynamicGlobalState {
                control: GlobalControl::OutputTrim(OutputTrimAddress { target: 1 }),
                value: ControlValue::Enum(0),
            },
            antelope_protocol::DynamicGlobalState {
                control: GlobalControl::OutputTrim(OutputTrimAddress { target: 2 }),
                value: ControlValue::Enum(0),
            },
            antelope_protocol::DynamicGlobalState {
                control: GlobalControl::TalkbackButton,
                value: ControlValue::Bool(false),
            },
            antelope_protocol::DynamicGlobalState {
                control: GlobalControl::TalkbackSourceResidue,
                value: ControlValue::Enum(0),
            },
            antelope_protocol::DynamicGlobalState {
                control: GlobalControl::TalkbackGain,
                value: ControlValue::Int(0),
            },
        ]
    );
    assert_eq!(state.mixers.len(), 4);
    assert!(state
        .mixers
        .iter()
        .all(|surface| surface.master.is_some() && surface.strips.len() == 32));
}

#[test]
fn orion_talkback_readback_keeps_source_residue_partial_and_gain_owner_unknown() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    let mut frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    frame[73] = 0x7d; // button + destination/high-source residue; only low 2 bits are source truth.
    frame[74] = 42;
    let DeviceEvent::Snapshot { state, .. } = driver.decode(&frame).unwrap().unwrap() else {
        panic!("snapshot")
    };
    assert!(state
        .globals
        .contains(&antelope_protocol::DynamicGlobalState {
            control: GlobalControl::TalkbackButton,
            value: ControlValue::Bool(true),
        }));
    assert!(state
        .globals
        .contains(&antelope_protocol::DynamicGlobalState {
            control: GlobalControl::TalkbackSourceResidue,
            value: ControlValue::Enum(1),
        }));
    assert!(!state
        .globals
        .iter()
        .any(|global| { global.control == GlobalControl::TalkbackSource }));
    assert!(state
        .globals
        .contains(&antelope_protocol::DynamicGlobalState {
            control: GlobalControl::TalkbackGain,
            value: ControlValue::Int(42),
        }));

    frame[74] = 97;
    assert!(driver.decode(&frame).is_err());
}

#[test]
fn orion_output_trim_readback_extracts_each_packed_field_independently() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    let cases = [
        (0, 24, 0x6f, [6, 0, 0]),
        (1, 25, 0x1c, [0, 7, 0]),
        (2, 25, 0xc3, [0, 0, 6]),
    ];
    for (_changed_target, offset, packed, expected) in cases {
        let mut frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
        frame[24] = 0;
        frame[25] = 0;
        frame[offset] = packed;
        let DeviceEvent::Snapshot { state, .. } = driver.decode(&frame).unwrap().unwrap() else {
            panic!("snapshot")
        };
        let values = state
            .globals
            .iter()
            .filter_map(|global| match global.control {
                GlobalControl::OutputTrim(address) => match global.value {
                    ControlValue::Enum(value) => Some((address.target, value)),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            vec![(0, expected[0]), (1, expected[1]), (2, expected[2])]
        );
    }
}

#[test]
fn orion_clock_source_decodes_profile_values_zero_and_six() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    for value in [0, 6] {
        let mut frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
        frame[19] = value;
        let DeviceEvent::Snapshot { state, .. } = driver.decode(&frame).unwrap().unwrap() else {
            panic!("snapshot")
        };
        assert!(state
            .globals
            .contains(&antelope_protocol::DynamicGlobalState {
                control: GlobalControl::ClockSource,
                value: ControlValue::Enum(i32::from(value)),
            }));
    }
}

#[test]
fn unknown_clock_readback_is_preserved_as_raw_enum_value() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    let mut frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    frame[19] = 9;
    let DeviceEvent::Snapshot { state, .. } = driver.decode(&frame).unwrap().unwrap() else {
        panic!("snapshot")
    };
    assert!(state
        .globals
        .contains(&antelope_protocol::DynamicGlobalState {
            control: GlobalControl::ClockSource,
            value: ControlValue::Enum(9),
        }));
}

#[test]
fn orion_usb_clock_source_uses_exact_confirmed_global_frame() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    let batch = driver
        .encode(Action::SetGlobal {
            control: GlobalControl::ClockSource,
            value: ControlValue::Enum(6),
        })
        .expect("USB clock source");
    let mut expected = vec![0; 320];
    expected[0] = 0x70;
    expected[4..8].copy_from_slice(&0x12_u32.to_le_bytes());
    expected[16] = 0x04;
    expected[17] = 0x06;
    assert_eq!(batch.frames, vec![expected]);
}

#[test]
fn clock_source_write_rejects_value_not_declared_by_profile() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    assert!(matches!(
        driver.encode(Action::SetGlobal {
            control: GlobalControl::ClockSource,
            value: ControlValue::Enum(7),
        }),
        Err(DriverError::InvalidAction(_))
    ));
}

#[test]
fn zen_clock_source_keeps_only_profile_declared_raw_choices() {
    let driver = zen_go_driver();
    assert!(driver
        .encode(Action::SetGlobal {
            control: GlobalControl::ClockSource,
            value: ControlValue::Enum(2),
        })
        .is_ok());
    assert!(matches!(
        driver.encode(Action::SetGlobal {
            control: GlobalControl::ClockSource,
            value: ControlValue::Enum(3),
        }),
        Err(DriverError::InvalidAction(_))
    ));
}

#[test]
fn profile_driver_decodes_explicit_mapped_meter_lanes_from_full_report_offsets() {
    let mut entry = state_meter_fixture_entry();
    entry.profile.meter_mappings.clear();
    entry.profile.meter_mappings.push(RuntimeMeterMapping {
        frame_id: "state_report".into(),
        target: RuntimeMeterTarget::MixMaster,
        target_index: 0,
        lane: 0,
        offset: 0xea,
        raw_min: 0,
        raw_max: 96,
        status: "observed".into(),
        status_text: "observed".into(),
        evidence: "synthetic full-report lane".into(),
    });
    let driver = ProfileDriver::new(entry).expect("profile driver");
    let mut frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    frame[0xea] = 0x3c;
    let DeviceEvent::Snapshot { state, .. } = driver.decode(&frame).unwrap().unwrap() else {
        panic!("snapshot")
    };
    assert_eq!(state.meters.len(), 1);
    assert_eq!(state.meters[0].target, RuntimeMeterTarget::MixMaster);
    assert_eq!(state.meters[0].target_index, 0);
    assert_eq!(state.meters[0].lane, 0);
    assert_eq!(state.meters[0].value, 0x3c);
}

#[test]
fn profile_driver_rejects_competing_explicit_meter_lane_across_frames() {
    let mut entry = fixture_entry();
    entry.profile.meter_mappings = vec![
        RuntimeMeterMapping {
            frame_id: "state_report".into(),
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 0,
            offset: 0xea,
            raw_min: 0,
            raw_max: 96,
            status: "observed".into(),
            status_text: "observed".into(),
            evidence: "first lane".into(),
        },
        RuntimeMeterMapping {
            frame_id: "meter_report".into(),
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 0,
            offset: 0xea,
            raw_min: 0,
            raw_max: 96,
            status: "observed".into(),
            status_text: "observed".into(),
            evidence: "competing lane".into(),
        },
    ];
    let error = ProfileDriver::new(entry).expect_err("duplicate target lane must fail closed");
    assert!(error.to_string().contains("declared more than once"));
}

#[test]
fn state_only_profile_populates_physical_meters_from_state_report() {
    let driver = ProfileDriver::new(state_meter_fixture_entry()).expect("state meter fixture");
    let frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    let DeviceEvent::Snapshot { state, .. } = driver.decode(&frame).unwrap().unwrap() else {
        panic!("snapshot")
    };
    assert_eq!(
        state
            .inputs
            .iter()
            .filter(|input| input.address.space == 0)
            .map(|input| input.meter)
            .collect::<Vec<_>>(),
        vec![Some(0); 12]
    );
}

#[test]
fn state_only_profile_ignores_superseded_meter_report_bytes_after_monitor_level() {
    let driver = profile_driver_from_fixture();
    let mut frame = vec![0; 320];
    frame[0] = 0x75;
    frame[1] = 0x1f;
    frame[32] = 9;
    frame[33..45].copy_from_slice(&[1, 4, 7, 10, 13, 16, 19, 22, 25, 28, 31, 34]);
    assert!(driver.decode(&frame).unwrap().is_none());
}

#[test]
fn confirmed_meter_report_emits_meter_only_for_confirmed_discriminator() {
    let driver =
        ProfileDriver::new(confirmed_meter_fixture_entry()).expect("confirmed meter source");
    let mut frame = vec![0; 320];
    frame[0] = 0x75;
    frame[1] = 0x73;
    frame[33..45].fill(0xff);
    let error = driver
        .decode(&frame)
        .expect_err("unconfirmed discriminator must not decode as meter");
    assert!(error.to_string().contains("discriminator"));
}

#[test]
fn confirmed_meter_report_path_still_decodes_all_physical_meters() {
    let driver =
        ProfileDriver::new(confirmed_meter_fixture_entry()).expect("confirmed meter source");
    let mut frame = vec![0; 320];
    frame[0] = 0x75;
    frame[1] = 0x1f;
    frame[32..44].copy_from_slice(&[1, 4, 7, 10, 13, 16, 19, 22, 25, 28, 31, 34]);
    let DeviceEvent::Meter {
        inputs,
        meters,
        raw,
    } = driver.decode(&frame).unwrap().unwrap()
    else {
        panic!("typed meter event")
    };
    assert_eq!(raw, frame);
    assert!(meters.is_empty());
    assert_eq!(inputs.len(), 12);
    assert_eq!(
        inputs
            .iter()
            .map(|input| (input.address, input.meter))
            .collect::<Vec<_>>(),
        (0..12)
            .map(|index| (
                InputAddress { space: 0, index },
                Some(1 + u8::try_from(index).unwrap() * 3),
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn canonical_orion_profile_driver_decodes_all_physical_and_provisional_output_meters() {
    let entry = canonical_orion_entry();
    assert_eq!(
        entry
            .profile
            .meter_mappings
            .iter()
            .map(|mapping| (
                mapping.target,
                mapping.target_index,
                mapping.lane,
                mapping.offset
            ))
            .collect::<Vec<_>>(),
        vec![
            (RuntimeMeterTarget::PhysicalOutput, 0, 0, 157),
            (RuntimeMeterTarget::PhysicalOutput, 1, 0, 158),
            (RuntimeMeterTarget::PhysicalOutput, 2, 0, 159),
            (RuntimeMeterTarget::PhysicalOutput, 3, 0, 160),
            (RuntimeMeterTarget::PhysicalOutput, 4, 0, 177),
            (RuntimeMeterTarget::PhysicalOutput, 5, 0, 178),
        ]
    );
    assert_eq!(entry.readiness, RuntimeReadiness::Supported);
    assert_eq!(entry.driver_kind, RuntimeDriverKind::Profile);
    assert_eq!(entry.profile.startup_queries.len(), 113);
    assert!(!entry
        .profile
        .decoders
        .iter()
        .any(|decoder| decoder.frame_id == "meter_report"
            && decoder.status.eq_ignore_ascii_case("confirmed")));

    let driver = ProfileDriver::new(entry).expect("canonical Orion profile driver");
    let mut state_frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    for (offset, value) in [157, 158, 159, 160, 177, 178].into_iter().zip(1_u8..=6) {
        state_frame[offset] = value;
    }
    let physical_values = [0, 9, 18, 27, 36, 45, 54, 63, 72, 81, 90, 96];
    state_frame[221..233].copy_from_slice(&physical_values);
    let DeviceEvent::Snapshot { state, .. } = driver
        .decode(&state_frame)
        .unwrap()
        .expect("state snapshot")
    else {
        panic!("snapshot")
    };
    assert_eq!(
        state
            .inputs
            .iter()
            .filter(|input| input.address.space == 0)
            .map(|input| (input.address.index, input.meter))
            .collect::<Vec<_>>(),
        physical_values
            .into_iter()
            .enumerate()
            .map(|(index, value)| (u16::try_from(index).unwrap(), Some(value)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        state
            .meters
            .iter()
            .map(|meter| (meter.target_index, meter.lane, meter.value))
            .collect::<Vec<_>>(),
        vec![
            (0, 0, 1),
            (1, 0, 2),
            (2, 0, 3),
            (3, 0, 4),
            (4, 0, 5),
            (5, 0, 6)
        ]
    );
}

#[test]
fn canonical_orion_old_157_mirror_does_not_drive_physical_meters() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    let mut frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    frame[157..169].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    frame[221..233].fill(96);

    let DeviceEvent::Snapshot { state, .. } = driver.decode(&frame).unwrap().unwrap() else {
        panic!("snapshot")
    };
    assert_eq!(
        state
            .inputs
            .iter()
            .filter(|input| input.address.space == 0)
            .map(|input| input.meter)
            .collect::<Vec<_>>(),
        vec![Some(96); 12]
    );
}

#[test]
fn confirmed_meter_report_takes_precedence_over_state_physical_meter_layout() {
    let driver =
        ProfileDriver::new(confirmed_meter_fixture_entry()).expect("confirmed meter-report source");
    let state = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    let DeviceEvent::Snapshot { state, .. } = driver.decode(&state).unwrap().unwrap() else {
        panic!("snapshot")
    };
    assert!(state
        .inputs
        .iter()
        .filter(|input| input.address.space == 0)
        .all(|input| input.meter.is_none()));
}

#[test]
fn typed_routing_readback_layout_is_validated_when_present() {
    let mut entry = fixture_entry();
    entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "readback")
        .unwrap()
        .operations
        .push(FrameOperation::Indexed {
            base: 17,
            stride: 2,
            index_field: "routing_source_pair".into(),
            width: 1,
            max_index: Some(31),
        });
    assert!(ProfileDriver::new(entry).is_err());
}

#[test]
fn no_send_implicit_mixer_readback_decodes_three_byte_records_without_send() {
    let mut entry = fixture_entry();
    entry
        .profile
        .params
        .retain(|parameter| parameter.name != "mix_send");
    entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "mix_command")
        .unwrap()
        .operations
        .retain(|operation| {
            !matches!(operation, FrameOperation::Scalar { field, .. } if field == "send")
        });
    let driver = ProfileDriver::new(entry).expect("no-send mixer profile");
    let frame = hex_fixture(include_str!("fixtures/orion/readback_75.hex"));
    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Mixer(surface)),
        ..
    } = driver.decode(&frame).unwrap().unwrap()
    else {
        panic!("mixer patch")
    };
    assert!(surface.master.as_ref().expect("master").send.is_none());
    assert!(surface.strips.iter().all(|strip| strip.send.is_none()));
}

#[test]
fn canonical_orion_state_meter_requires_exact_confirmed_geometry() {
    for mutation in 0..5 {
        let mut entry = canonical_orion_entry();
        let state = entry
            .profile
            .frames
            .iter_mut()
            .find(|frame| frame.id == "state_report")
            .unwrap();
        let FrameOperation::Indexed {
            base,
            stride,
            index_field,
            width,
            max_index,
        } = state
            .operations
            .iter_mut()
            .find(|operation| {
                matches!(
                    operation,
                    FrameOperation::Indexed { index_field, .. } if index_field == "physical_meter"
                )
            })
            .unwrap()
        else {
            unreachable!()
        };
        match mutation {
            0 => *width = 2,
            1 => *max_index = Some(10),
            2 => *base = 220,
            3 => *stride = 2,
            4 => *index_field = "channel_meter".into(),
            _ => unreachable!(),
        }
        assert!(
            ProfileDriver::new(entry).is_err(),
            "canonical Orion state meter mutation {mutation} must fail construction"
        );
    }
}

#[test]
fn canonical_orion_state_meter_rejects_truncated_channel_twelve() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    let mut frame = hex_fixture(include_str!("fixtures/orion/state_report_73.hex"));
    frame.truncate(232);

    assert!(driver.decode(&frame).is_err());
}

#[test]
fn canonical_orion_state_meter_rejects_duplicate_physical_operation() {
    let mut entry = canonical_orion_entry();
    entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "state_report")
        .unwrap()
        .operations
        .push(FrameOperation::Indexed {
            base: 221,
            stride: 1,
            index_field: "physical_meter".into(),
            width: 1,
            max_index: Some(11),
        });

    assert!(ProfileDriver::new(entry).is_err());
}

#[test]
fn malformed_declared_state_meter_is_rejected_even_with_meter_report_source() {
    let mut entry = confirmed_meter_fixture_entry();
    entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "state_report")
        .expect("state report")
        .operations
        .push(FrameOperation::Indexed {
            base: 157,
            stride: 1,
            index_field: "physical_meter".into(),
            width: 1,
            max_index: Some(10),
        });
    let error =
        ProfileDriver::new(entry).expect_err("malformed declared state meter must fail closed");
    assert!(matches!(error, DriverError::InvalidAction(_)));
}

#[test]
fn valid_bounded_non_patch_readbacks_return_owned_none_patch() {
    let driver = profile_driver_from_fixture();
    for (category, index) in [(0x0a, 0), (0x0b, 4), (0x11, 1), (0x19, 63), (0x1a, 15)] {
        let mut frame = vec![0; 320];
        frame[0] = 0x75;
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[8] = category;
        frame[12] = index;
        frame[16] = category ^ index;
        let DeviceEvent::QueryReply {
            patch, body, raw, ..
        } = driver.decode(&frame).unwrap().unwrap()
        else {
            panic!("bounded query reply")
        };
        assert!(patch.is_none());
        assert_eq!(body, frame[16..]);
        assert_eq!(raw, frame);
    }
}

#[test]
fn globals_and_routing_group_validate_before_writing() {
    let driver = profile_driver_from_fixture();
    let global = driver
        .encode(Action::SetGlobal {
            control: GlobalControl::SampleRate,
            value: ControlValue::Enum(2),
        })
        .expect("global");
    assert_eq!(&global.frames[0][16..18], &[0x03, 0x02]);

    let sources = (0..16)
        .map(|index| RoutingSource { bank: 3, index })
        .collect();
    let routing = driver
        .encode(Action::SetRoutingGroup {
            destination: 0,
            changed_channel: None,
            sources,
        })
        .expect("routing");
    assert_eq!(routing.frames.len(), 1);
    assert_eq!(routing.frames[0][16], 0xd3);
}

#[test]
fn zen_go_normalized_actions_preserve_representative_bytes() {
    let driver = zen_go_driver();
    let output = driver
        .encode(Action::SetOutput {
            address: OutputAddress { id: 0 },
            control: OutputControl::Level,
            value: ControlValue::Int(0x12),
        })
        .expect("output");
    assert_eq!(&output.frames[0][0x10..0x13], &[0x47, 0x00, 0x12]);
    assert!(matches!(
        driver.encode(Action::SetOutput {
            address: OutputAddress { id: 0 },
            control: OutputControl::Mono,
            value: ControlValue::Bool(true),
        }),
        Err(DriverError::UnsupportedAction(_))
    ));

    let preamp = driver
        .encode(Action::SetInput {
            address: InputAddress { space: 0, index: 1 },
            control: InputControl::Mode,
            value: ControlValue::Enum(1),
        })
        .expect("preamp");
    assert_eq!(&preamp.frames[0][0x10..0x13], &[0x4f, 0x01, 0x01]);

    let mixer = driver
        .encode(Action::SetMixerStripState {
            address: MixerAddress {
                surface: 1,
                strip: 7,
            },
            fader: 0x22,
            pan: 30,
            muted: true,
            soloed: false,
            send: None,
        })
        .expect("mixer");
    assert_eq!(
        &mixer.frames[0][0x10..0x16],
        &[0xd4, 0x04, 0x01, 0x07, 0x22, 0x7e]
    );

    let query = driver
        .encode(Action::Query(QueryRequest::new(0x04, 0x03)))
        .expect("query");
    assert_eq!(&query.frames[0][0..8], &[0x74, 0, 0, 0, 0x10, 0, 0, 0]);
    assert_eq!(query.frames[0][8], 0x04);
    assert_eq!(query.frames[0][12], 0x03);
}

#[test]
fn zen_go_routing_group_preserves_complete_assignment_table_bytes() {
    let driver = zen_go_driver();
    let mut sources = vec![
        RoutingSource {
            bank: 0x08,
            index: 0
        };
        16
    ];
    for (index, source) in sources.iter_mut().take(8).enumerate() {
        *source = RoutingSource {
            bank: 0x01,
            index: index as u16,
        };
    }
    sources[10] = RoutingSource {
        bank: 0x01,
        index: 0,
    };
    let batch = driver
        .encode(Action::SetRoutingGroup {
            destination: 0,
            changed_channel: Some(10),
            sources,
        })
        .expect("complete routing group");
    let bank06 = batch
        .frames
        .iter()
        .find(|frame| frame[0x10..0x13] == [0xd3, 0x41, 0x06])
        .expect("bank 06 frame");
    assert_eq!(
        &bank06[0x10 + 0x03..0x10 + 0x0d],
        &[0x03, 0x00, 0x03, 0x01, 0x03, 0x02, 0x03, 0x03, 0x01, 0x04]
    );
    assert_eq!(&bank06[0x10 + 0x17..0x10 + 0x19], &[0x01, 0x00]);
}

fn constructor_error_without_frame(frame_id: &str) -> DriverError {
    let mut entry = fixture_entry();
    entry.profile.frames.retain(|frame| frame.id != frame_id);
    ProfileDriver::new(entry).expect_err("missing required frame must fail")
}

#[test]
fn constructor_rejects_missing_command_frame() {
    assert!(constructor_error_without_frame("command")
        .to_string()
        .contains("command"));
}

#[test]
fn constructor_rejects_missing_global_frame() {
    assert!(constructor_error_without_frame("global_command")
        .to_string()
        .contains("global"));
}

#[test]
fn constructor_rejects_missing_mixer_frame() {
    assert!(constructor_error_without_frame("mix_command")
        .to_string()
        .contains("mix"));
}

#[test]
fn constructor_rejects_missing_link_frame() {
    assert!(constructor_error_without_frame("link_command")
        .to_string()
        .contains("link"));
}

#[test]
fn constructor_rejects_missing_routing_frame() {
    assert!(constructor_error_without_frame("routing_command")
        .to_string()
        .contains("routing"));
}

#[test]
fn constructor_rejects_missing_state_frame() {
    assert!(constructor_error_without_frame("state_report")
        .to_string()
        .contains("state"));
}

#[test]
fn constructor_allows_state_only_profile_without_meter_frame() {
    let mut entry = fixture_entry();
    entry
        .profile
        .frames
        .retain(|frame| frame.id != "meter_report");
    assert!(ProfileDriver::new(entry).is_ok());
}

#[test]
fn state_only_profile_decodes_readback_without_meter_frame() {
    let mut entry = fixture_entry();
    entry
        .profile
        .frames
        .retain(|frame| frame.id != "meter_report");
    let driver = ProfileDriver::new(entry).expect("state-only profile driver");
    let mut frame = vec![0; 320];
    frame[0] = 0x75;
    frame[8] = 0x0a;
    assert!(matches!(
        driver.decode(&frame),
        Ok(Some(DeviceEvent::QueryReply { .. }))
    ));
}

#[test]
fn constructor_rejects_missing_readback_frame() {
    assert!(constructor_error_without_frame("readback")
        .to_string()
        .contains("readback"));
}

#[test]
fn constructor_rejects_missing_or_unsafe_startup_walk_before_io() {
    let mut entry = fixture_entry();
    entry.profile.startup_queries.clear();
    assert!(ProfileDriver::new(entry)
        .expect_err("missing startup walk")
        .to_string()
        .contains("startup"));

    let mut entry = fixture_entry();
    entry.profile.startup_queries[0] = QueryRequest::new(0x04, 4);
    assert!(ProfileDriver::new(entry)
        .expect_err("unsafe startup request")
        .to_string()
        .contains("outside count"));
}

#[test]
fn constructor_rejects_missing_state_semantics_and_canonical_orion_meter() {
    let mut entry = fixture_entry();
    entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "state_report")
        .unwrap()
        .operations
        .retain(|operation| {
            !matches!(operation, FrameOperation::Scalar { field, .. } if field == "gain_base")
        });
    assert!(ProfileDriver::new(entry).is_err(), "state_report/gain_base");

    let mut entry = fixture_entry();
    entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "state_report")
        .unwrap()
        .operations
        .retain(|operation| {
            !matches!(
                operation,
                FrameOperation::Indexed { index_field, .. } if index_field == "physical_meter"
            )
        });
    assert!(
        ProfileDriver::new(entry).is_err(),
        "canonical Orion must retain its confirmed physical meter mapping"
    );
}

#[test]
fn constructor_rejects_missing_confirmed_decoder_mapping() {
    let mut entry = fixture_entry();
    entry
        .profile
        .decoders
        .retain(|decoder| decoder.frame_id != "readback");
    assert!(ProfileDriver::new(entry)
        .expect_err("missing decoder mapping")
        .to_string()
        .contains("decoder"));
}

#[test]
fn constructor_rejects_malformed_or_incomplete_settings_contracts() {
    for mutation in 0..18 {
        let mut entry = fixture_entry();
        match mutation {
            0 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "screen_brightness")
                    .unwrap()
                    .status = "observed".into()
            }
            1 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "screen_brightness")
                    .unwrap()
                    .range = Some((0, 101))
            }
            2 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "output_trim")
                    .unwrap()
                    .range = Some((0, 7))
            }
            3 => {
                entry
                    .profile
                    .constraints
                    .iter_mut()
                    .find(|constraint| constraint.name == "parameter_target.output_trim")
                    .unwrap()
                    .values = vec![0, 1]
            }
            4 => entry
                .profile
                .frames
                .iter_mut()
                .find(|frame| frame.id == "global_command")
                .unwrap()
                .operations
                .push(FrameOperation::FixedByte {
                    offset: 5,
                    value: 0,
                }),
            5 => {
                let frame = entry
                    .profile
                    .frames
                    .iter_mut()
                    .find(|frame| frame.id == "command")
                    .unwrap();
                frame.operations.push(frame.operations[0].clone());
            }
            6 => entry
                .profile
                .constraints
                .retain(|constraint| constraint.name != "output_trim_target.2"),
            7 => entry
                .profile
                .frames
                .iter_mut()
                .find(|frame| frame.id == "state_report")
                .unwrap()
                .operations
                .retain(|operation| {
                    !matches!(
                        operation,
                        FrameOperation::BitField {
                            offset: 25,
                            mask: 0xe0,
                            shift: 5,
                            ..
                        }
                    )
                }),
            8 => {
                let frame = entry
                    .profile
                    .frames
                    .iter_mut()
                    .find(|frame| frame.id == "state_report")
                    .unwrap();
                let duplicate = frame
                    .operations
                    .iter()
                    .find(|operation| {
                        matches!(operation,
                            FrameOperation::Scalar { field, offset: 26, .. }
                                if field == "screen_brightness_byte_offset"
                        )
                    })
                    .unwrap()
                    .clone();
                frame.operations.push(duplicate);
            }
            9 => entry
                .profile
                .params
                .iter_mut()
                .find(|parameter| parameter.name == "screen_brightness")
                .unwrap()
                .readback
                .fields
                .clear(),
            10 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "screen_brightness")
                    .unwrap()
                    .readback
                    .frame = "global_command".into()
            }
            11 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "screen_brightness")
                    .unwrap()
                    .readback
                    .fields[0] = ParamReadbackField::Scalar {
                    offset: 99,
                    width: 1,
                }
            }
            12 => {
                let reference = &mut entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "screen_brightness")
                    .unwrap()
                    .readback;
                reference.fields.push(reference.fields[0].clone());
            }
            13 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "output_trim")
                    .unwrap()
                    .readback
                    .fields
                    .pop();
            }
            14 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "output_trim")
                    .unwrap()
                    .readback
                    .fields[2] = ParamReadbackField::BitField {
                    target: 1,
                    offset: 25,
                    mask: 0xe0,
                    shift: 5,
                }
            }
            15 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "output_trim")
                    .unwrap()
                    .readback
                    .semantic = "brightness".into()
            }
            16 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "output_trim")
                    .unwrap()
                    .readback
                    .fields[1] = ParamReadbackField::BitField {
                    target: 1,
                    offset: 26,
                    mask: 0x1c,
                    shift: 2,
                }
            }
            _ => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "output_trim")
                    .unwrap()
                    .readback
                    .offsets[0]
                    .1 = 99
            }
        }
        assert!(
            ProfileDriver::new(entry).is_err(),
            "mutation {mutation} must fail closed"
        );
    }
}

#[test]
fn constructor_rejects_ambiguous_semantic_operation() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "command")
        .unwrap();
    let duplicate = frame.operations.iter().find(|operation| matches!(operation, FrameOperation::Scalar { field, .. } if field == "param_id")).unwrap().clone();
    frame.operations.push(duplicate);
    assert!(ProfileDriver::new(entry)
        .expect_err("ambiguous mapping")
        .to_string()
        .contains("ambiguous"));
}

#[test]
fn width_one_not_applicable_scalar_is_accepted() {
    let entry = fixture_entry();
    assert!(entry
        .profile
        .frames
        .iter()
        .flat_map(|frame| &frame.operations)
        .any(|operation| {
            matches!(
                operation,
                FrameOperation::Scalar {
                    width: 1,
                    endian: FrameEndian::NotApplicable,
                    ..
                }
            )
        }));
    ProfileDriver::new(entry).expect("width-one NotApplicable is valid");
}

#[test]
fn scalar_schema_requires_semantics_and_declared_endianness() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "command")
        .unwrap();
    let FrameOperation::Scalar { field, .. } = frame.operations.iter_mut().find(|operation| matches!(operation, FrameOperation::Scalar { field, .. } if field == "param_id")).unwrap() else { unreachable!() };
    field.clear();
    assert!(ProfileDriver::new(entry).is_err());

    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "command")
        .unwrap();
    let FrameOperation::Scalar { width, endian, .. } = frame.operations.iter_mut().find(|operation| matches!(operation, FrameOperation::Scalar { field, .. } if field == "value")).unwrap() else { unreachable!() };
    *width = 2;
    *endian = FrameEndian::NotApplicable;
    assert!(ProfileDriver::new(entry).is_err());
}

#[test]
fn constructor_rejects_scalar_width_zero_before_encode() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "command")
        .unwrap();
    let FrameOperation::Scalar { width, .. } = frame
        .operations
        .iter_mut()
        .find(|operation| matches!(operation, FrameOperation::Scalar { field, .. } if field == "value"))
        .unwrap()
    else {
        unreachable!()
    };
    *width = 0;

    let error = ProfileDriver::new(entry).expect_err("zero-width scalar must fail construction");
    assert!(error
        .to_string()
        .contains("command scalar \"value\" width 0"));
}

#[test]
fn constructor_rejects_scalar_width_five_before_encode() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "command")
        .unwrap();
    let FrameOperation::Scalar { width, .. } = frame
        .operations
        .iter_mut()
        .find(|operation| matches!(operation, FrameOperation::Scalar { field, .. } if field == "value"))
        .unwrap()
    else {
        unreachable!()
    };
    *width = 5;

    let error = ProfileDriver::new(entry).expect_err("width-five scalar must fail construction");
    assert!(error
        .to_string()
        .contains("command scalar \"value\" width 5"));
}

#[test]
fn constructor_rejects_bit_field_mask_zero_before_encode() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "state_report")
        .unwrap();
    let FrameOperation::BitField { mask, .. } = frame
        .operations
        .iter_mut()
        .find(|operation| matches!(operation, FrameOperation::BitField { field, .. } if field == "mask"))
        .unwrap()
    else {
        unreachable!()
    };
    *mask = 0;

    let error = ProfileDriver::new(entry).expect_err("zero bit-field mask must fail construction");
    assert!(error
        .to_string()
        .contains("state_report bit field \"mask\" has zero mask"));
}

#[test]
fn constructor_rejects_bit_field_shift_eight_before_encode() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "state_report")
        .unwrap();
    let FrameOperation::BitField { shift, .. } = frame
        .operations
        .iter_mut()
        .find(|operation| matches!(operation, FrameOperation::BitField { field, .. } if field == "mask"))
        .unwrap()
    else {
        unreachable!()
    };
    *shift = 8;

    let error =
        ProfileDriver::new(entry).expect_err("shift-eight bit field must fail construction");
    assert!(error
        .to_string()
        .contains("state_report bit field \"mask\" shift 8"));
}

#[test]
fn constructor_rejects_bit_field_mask_below_shift_before_encode() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "state_report")
        .unwrap();
    let FrameOperation::BitField { mask, shift, .. } = frame
        .operations
        .iter_mut()
        .find(|operation| matches!(operation, FrameOperation::BitField { field, .. } if field == "mask"))
        .unwrap()
    else {
        unreachable!()
    };
    *mask = 0b0000_0011;
    *shift = 2;

    let error = ProfileDriver::new(entry)
        .expect_err("bit-field mask fully below shift must fail construction");
    assert!(error
        .to_string()
        .contains("state_report bit field \"mask\" mask 0x03 is below shift 2"));
}

#[test]
fn shifted_semantic_offsets_and_big_endian_width_are_profile_driven() {
    let remove_fixed_settings = |entry: &mut RuntimeEntry| {
        entry.profile.constraints.retain(|constraint| {
            !matches!(
                constraint.name.as_str(),
                "output_mono_targets" | "parameter_target.output_trim"
            )
        });
        entry.profile.params.retain(|parameter| {
            !matches!(parameter.name.as_str(), "screen_brightness" | "output_trim")
        });
    };
    let mut entry = fixture_entry();
    remove_fixed_settings(&mut entry);
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "command")
        .unwrap();
    for operation in &mut frame.operations {
        if let FrameOperation::Scalar { field, offset, .. } = operation {
            *offset = match field.as_str() {
                "param_id" => 30,
                "channel" => 31,
                "value" => 32,
                _ => *offset,
            };
        }
    }
    for parameter in
        entry.profile.params.iter_mut().filter(|parameter| {
            parameter.applies_to != "globals" && parameter.applies_to != "mixers"
        })
    {
        for (field, offset) in &mut parameter.frame.offsets {
            *offset = match field.as_str() {
                "param_id" => 30,
                "channel" => 31,
                "value" => 32,
                _ => *offset,
            };
        }
    }
    let driver = ProfileDriver::new(entry).expect("shifted driver");
    let frame = driver
        .encode(Action::SetInput {
            address: InputAddress { space: 0, index: 2 },
            control: InputControl::Gain,
            value: ControlValue::Int(12),
        })
        .unwrap()
        .frames
        .remove(0);
    assert_eq!(&frame[30..33], &[0x50, 2, 12]);
    assert_eq!(&frame[16..19], &[0, 0, 0]);

    let mut entry = fixture_entry();
    remove_fixed_settings(&mut entry);
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "command")
        .unwrap();
    let FrameOperation::Scalar { width, endian, .. } = frame.operations.iter_mut().find(|operation| matches!(operation, FrameOperation::Scalar { field, .. } if field == "value")).unwrap() else { unreachable!() };
    *width = 2;
    *endian = FrameEndian::Big;
    let gain = entry
        .profile
        .params
        .iter_mut()
        .find(|parameter| parameter.name == "gain")
        .unwrap();
    gain.range = Some((0, 1024));
    let driver = ProfileDriver::new(entry).expect("big endian driver");
    let frame = driver
        .encode(Action::SetInput {
            address: InputAddress { space: 0, index: 2 },
            control: InputControl::Gain,
            value: ControlValue::Int(0x0102),
        })
        .unwrap()
        .frames
        .remove(0);
    let mut expected = vec![0; 320];
    expected[0] = 0x70;
    expected[4] = 0x13;
    expected[16] = 0x50;
    expected[17] = 2;
    expected[18..20].copy_from_slice(&[0x01, 0x02]);
    assert_eq!(frame, expected);
}

fn complete_mixer_frame(fader: i32, pan: i32, muted: bool, soloed: bool, send: i32) -> Vec<u8> {
    profile_driver_from_fixture()
        .encode(Action::SetMixerStripState {
            address: MixerAddress {
                surface: 2,
                strip: 17,
            },
            fader,
            pan,
            muted,
            soloed,
            send: Some(send),
        })
        .expect("complete mixer")
        .frames
        .remove(0)
}

#[test]
fn generic_mixer_pan_uses_profile_center() {
    let mut entry = fixture_entry();
    entry
        .profile
        .params
        .iter_mut()
        .find(|parameter| parameter.name == "mix_pan")
        .expect("mix_pan parameter")
        .range = Some((-30, 22));
    for mixer in &mut entry.profile.mixers {
        mixer.pan_range = Some((-30, 22));
        mixer.pan_center = Some(40);
    }
    let driver = ProfileDriver::new(entry).expect("profile driver");
    let frame = driver
        .encode(Action::SetMixerStripState {
            address: MixerAddress {
                surface: 2,
                strip: 17,
            },
            fader: 44,
            pan: -10,
            muted: false,
            soloed: false,
            send: Some(55),
        })
        .expect("semantic pan")
        .frames
        .remove(0);

    assert_eq!(frame[21] & 0x3f, 30);

    let mut readback = hex_fixture(include_str!("fixtures/orion/readback_75.hex"));
    readback[17] = 30;
    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Mixer(surface)),
        ..
    } = driver
        .decode(&readback)
        .expect("mixer readback")
        .expect("mixer event")
    else {
        panic!("mixer patch")
    };
    assert_eq!(surface.master.expect("master").pan, Some(-10));
}

#[test]
fn complete_mixer_mutations_preserve_all_companion_fields() {
    let cases = [
        (44, 12, true, true, 55),
        (45, 12, true, true, 55),
        (44, 13, true, true, 55),
        (44, 12, false, true, 55),
        (44, 12, true, false, 55),
        (44, 12, true, true, 56),
    ];
    for (fader, pan, muted, soloed, send) in cases {
        let frame = complete_mixer_frame(fader, pan, muted, soloed, send);
        let mut expected = vec![0; 320];
        expected[0] = 0x70;
        expected[4] = 0x17;
        expected[16] = 0xd4;
        expected[17] = 0x05;
        expected[18] = 2;
        expected[19] = 17;
        expected[20] = fader as u8;
        expected[21] =
            (pan + 32) as u8 | if muted { 0x40 } else { 0 } | if soloed { 0x80 } else { 0 };
        expected[22] = send as u8;
        assert_eq!(frame, expected);
    }
}

#[test]
fn mixer_profile_without_send_accepts_none_and_rejects_some() {
    let mut entry = fixture_entry();
    entry
        .profile
        .params
        .retain(|parameter| parameter.name != "mix_send");
    let mix = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "mix_command")
        .unwrap();
    mix.operations.retain(
        |operation| !matches!(operation, FrameOperation::Scalar { field, .. } if field == "send"),
    );
    let driver = ProfileDriver::new(entry).expect("no-send mixer profile");
    let complete = Action::SetMixerStripState {
        address: MixerAddress {
            surface: 0,
            strip: 1,
        },
        fader: 1,
        pan: 2,
        muted: false,
        soloed: false,
        send: None,
    };
    assert!(driver.encode(complete).is_ok());
    let invalid = Action::SetMixerStripState {
        address: MixerAddress {
            surface: 0,
            strip: 1,
        },
        fader: 1,
        pan: 2,
        muted: false,
        soloed: false,
        send: Some(1),
    };
    assert!(driver.encode(invalid).is_err());
}

#[test]
fn partial_mixer_and_missing_atomic_send_are_rejected() {
    let driver = profile_driver_from_fixture();
    assert!(driver
        .encode(Action::SetMixer {
            address: MixerAddress {
                surface: 0,
                strip: 1
            },
            control: MixerControl::Fader,
            value: ControlValue::Int(1)
        })
        .is_err());
    assert!(driver
        .encode(Action::SetMixerStripState {
            address: MixerAddress {
                surface: 0,
                strip: 1
            },
            fader: 1,
            pan: 2,
            muted: false,
            soloed: false,
            send: None
        })
        .is_err());
}

#[test]
fn typed_pair_index_resolution_uses_only_confirmed_link_domains() {
    let driver = profile_driver_from_fixture();
    for (surface, pair, enabled) in [(1, 0, true), (1, 0, false), (3, 15, true)] {
        let frame = driver
            .encode(Action::SetLink {
                surface,
                pair,
                enabled,
            })
            .expect("confirmed link domain")
            .frames
            .remove(0);
        let mut expected = vec![0; 320];
        expected[0] = 0x70;
        expected[4] = 0x14;
        expected[16] = 0xa2;
        expected[17] = surface;
        expected[18] = pair as u8;
        expected[19] = u8::from(enabled);
        assert_eq!(frame, expected);
    }

    for undeclared_space in [0, 2, 4] {
        let error = driver
            .encode(Action::SetLink {
                surface: undeclared_space,
                pair: 0,
                enabled: true,
            })
            .expect_err("undeclared link space must reject before frame emission");
        assert!(error.to_string().contains("link domain"));
    }
    for (surface, pair) in [(1, 1), (3, 16)] {
        assert!(driver
            .encode(Action::SetLink {
                surface,
                pair,
                enabled: true
            })
            .is_err());
    }
}

#[test]
fn constructor_rejects_noncanonical_set_link_frame_contract() {
    for mutation in 0..18 {
        let mut entry = fixture_entry();
        let frame = entry
            .profile
            .frames
            .iter_mut()
            .find(|frame| frame.id == "link_command")
            .expect("link frame");
        match mutation {
            0 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::FixedByte { offset: 0, value } = operation {
                    *value = 0x71;
                }
            }),
            1 => frame.operations.retain(
                |operation| !matches!(operation, FrameOperation::FixedByte { offset: 0, .. }),
            ),
            2 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::FixedByte { offset: 4, value } = operation {
                    *value = 0x15;
                }
            }),
            3 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::FixedByte { offset: 4, .. } = operation {
                    if let FrameOperation::FixedByte { offset, .. } = operation {
                        *offset = 5;
                    }
                }
            }),
            4 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::FixedByte { offset: 16, value } = operation {
                    *value = 0xa3;
                }
            }),
            5 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::FixedByte { offset: 16, .. } = operation {
                    if let FrameOperation::FixedByte { offset, .. } = operation {
                        *offset = 15;
                    }
                }
            }),
            6 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::Scalar { field, offset, .. } = operation {
                    if field == "space" {
                        *offset = 20;
                    }
                }
            }),
            7 => frame.operations.retain(
                |operation| !matches!(operation, FrameOperation::Scalar { field, .. } if field == "space"),
            ),
            8 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::PairIndex { base, .. } = operation {
                    *base = 19;
                }
            }),
            9 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::PairIndex { pair_field, .. } = operation {
                    *pair_field = "pair".into();
                }
            }),
            10 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::Scalar { field, offset, .. } = operation {
                    if field == "enabled" {
                        *offset = 20;
                    }
                }
            }),
            11 => frame.operations.retain(
                |operation| !matches!(operation, FrameOperation::Scalar { field, .. } if field == "enabled"),
            ),
            12 => frame.operations.push(FrameOperation::FixedByte {
                offset: 5,
                value: 0xff,
            }),
            13 => frame.operations.push(FrameOperation::FixedByte {
                offset: 0,
                value: 0x70,
            }),
            14 => frame.operations.push(FrameOperation::FixedByte {
                offset: 17,
                value: 0xff,
            }),
            15 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::Scalar { field, width, .. } = operation {
                    if field == "space" {
                        *width = 2;
                    }
                }
            }),
            16 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::Scalar { field, endian, .. } = operation {
                    if field == "enabled" {
                        *endian = FrameEndian::Big;
                    }
                }
            }),
            17 => frame.operations.iter_mut().for_each(|operation| {
                if let FrameOperation::PairIndex { stride, .. } = operation {
                    *stride = 2;
                }
            }),
            _ => unreachable!(),
        }
        let error = ProfileDriver::new(entry).expect_err("invalid SET_LINK contract");
        assert!(
            error.to_string().contains("link"),
            "mutation {mutation}: {error}"
        );
    }
}

#[test]
fn spdif_link_request_does_not_fabricate_state_or_software_mirror_gain() {
    let driver = profile_driver_from_fixture();
    driver
        .encode(Action::SetLink {
            surface: 1,
            pair: 0,
            enabled: true,
        })
        .expect("S/PDIF link request");

    let batch = driver
        .encode(Action::SetInput {
            address: InputAddress { space: 2, index: 0 },
            control: InputControl::Gain,
            value: ControlValue::Int(7),
        })
        .expect("single S/PDIF gain write");
    assert_eq!(
        batch.frames.len(),
        1,
        "unknown link state must not mirror gain"
    );
    let mut expected = vec![0; 320];
    expected[0] = 0x70;
    expected[4] = 0x13;
    expected[16] = 0x5c;
    expected[17] = 0;
    expected[18] = 7;
    assert_eq!(batch.frames[0], expected);
}

#[test]
fn destination_specific_routing_domains_control_outbound_and_inbound_validation() {
    let mut entry = fixture_entry();
    entry.profile.routing_groups[1]
        .source_domains
        .retain(|domain| domain.bank != 3);
    let driver = ProfileDriver::new(entry).expect("destination-specific routing fixture");

    driver
        .encode(Action::SetRoutingGroup {
            destination: 0,
            changed_channel: None,
            sources: vec![RoutingSource { bank: 3, index: 15 }; 16],
        })
        .expect("bank 2 is valid for destination A");
    let outbound_error = driver
        .encode(Action::SetRoutingGroup {
            destination: 1,
            changed_channel: None,
            sources: vec![RoutingSource { bank: 3, index: 0 }; 2],
        })
        .expect_err("bank 2 is unavailable for destination B");
    assert!(outbound_error.to_string().contains("destination 1"));

    let mut inbound = vec![0; 320];
    inbound[0] = 0x75;
    inbound[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    inbound[8] = 0x03;
    inbound[12] = 0;
    inbound[16] = 0;
    for channel in 0..16 {
        inbound[17 + channel * 2] = 3;
        inbound[18 + channel * 2] = channel as u8;
    }
    driver
        .decode(&inbound)
        .expect("destination A inbound domain")
        .expect("destination A event");
    inbound[12] = 1;
    inbound[16] = 1;
    let inbound_error = driver
        .decode(&inbound)
        .expect_err("destination B inbound domain must reject");
    assert!(inbound_error.to_string().contains("source bank 3"));
}

#[test]
fn constructor_rejects_malformed_or_ambiguous_talkback_contracts_before_io() {
    for mutation in 0..8 {
        let mut entry = fixture_entry();
        match mutation {
            0 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "talkback_button")
                    .unwrap()
                    .id = Some(0x21)
            }
            1 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "talkback_source")
                    .unwrap()
                    .readback
                    .truth = "complete".into()
            }
            2 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "talkback_source")
                    .unwrap()
                    .readback
                    .modulus = None
            }
            3 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "talkback_source")
                    .unwrap()
                    .readback
                    .fields[0] = ParamReadbackField::MaskedScalar {
                    offset: 73,
                    mask: 0xff,
                    shift: 0,
                }
            }
            4 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "talkback_gain")
                    .unwrap()
                    .range = Some((0, 97))
            }
            5 => {
                entry
                    .profile
                    .params
                    .iter_mut()
                    .find(|parameter| parameter.name == "talkback_gain")
                    .unwrap()
                    .readback
                    .fields[0] = ParamReadbackField::Scalar {
                    offset: 75,
                    width: 1,
                }
            }
            6 => entry
                .profile
                .params
                .retain(|parameter| parameter.name != "talkback_button"),
            _ => entry.profile.params.push(
                entry
                    .profile
                    .params
                    .iter()
                    .find(|parameter| parameter.name == "talkback_source")
                    .unwrap()
                    .clone(),
            ),
        }
        assert!(
            ProfileDriver::new(entry).is_err(),
            "talkback mutation {mutation} must fail closed"
        );
    }
}

#[test]
fn constructor_rejects_invalid_link_domains_before_io() {
    let mut missing = fixture_entry();
    missing.profile.link_domains.clear();
    assert!(ProfileDriver::new(missing).is_err());

    for mutate in 0..4 {
        let mut entry = fixture_entry();
        match mutate {
            0 => entry
                .profile
                .link_domains
                .push(entry.profile.link_domains[0].clone()),
            1 => entry.profile.link_domains[0].pair_count = 0,
            2 => entry.profile.link_domains[0].status = "unconfirmed".into(),
            3 => entry.profile.link_domains[0].evidence.clear(),
            _ => unreachable!(),
        }
        assert!(ProfileDriver::new(entry).is_err(), "link mutation {mutate}");
    }

    let mut mismatched = fixture_entry();
    mismatched.profile.link_domains[1].pair_count = 15;
    let error = ProfileDriver::new(mismatched).expect_err("semantic pair mapping mismatch");
    assert!(error.to_string().contains("pair mapping"));

    let mut bad_spdif_scope = fixture_entry();
    bad_spdif_scope.profile.link_domains[0].protocol_space = 2;
    assert!(ProfileDriver::new(bad_spdif_scope).is_err());

    let mut missing_spdif_capability = fixture_entry();
    missing_spdif_capability.profile.address_spaces[2]
        .input_capabilities
        .retain(|capability| capability.kind != antelope_protocol::RuntimeInputControlKind::Link);
    assert!(ProfileDriver::new(missing_spdif_capability).is_err());
}

#[test]
fn constructor_rejects_invalid_destination_source_domains_before_io() {
    for mutate in 0..5 {
        let mut entry = fixture_entry();
        match mutate {
            0 => entry.profile.routing_groups[0].source_domains.clear(),
            1 => {
                let duplicate = entry.profile.routing_groups[0].source_domains[0].clone();
                entry.profile.routing_groups[0]
                    .source_domains
                    .push(duplicate);
            }
            2 => entry.profile.routing_groups[0].source_domains[0].index_count = 0,
            3 => entry.profile.routing_groups[0].source_domains[0].status = "unconfirmed".into(),
            4 => entry.profile.routing_groups[0].source_domains[0]
                .evidence
                .clear(),
            _ => unreachable!(),
        }
        assert!(
            ProfileDriver::new(entry).is_err(),
            "routing mutation {mutate}"
        );
    }
}

#[test]
fn constructor_rejects_invalid_observed_readback_domains_before_io() {
    for mutate in 0..6 {
        let mut entry = observed_readback_fixture_entry();
        let domain = &mut entry.profile.routing_groups[0].readback_source_domains[0];
        match mutate {
            0 => domain.indices.clear(),
            1 => domain.indices = vec![0, 0],
            2 => domain.indices = vec![1, 0],
            3 => domain.bank = 0,
            4 => domain.status = "confirmed".into(),
            5 => domain.evidence.clear(),
            _ => unreachable!(),
        }
        assert!(
            ProfileDriver::new(entry).is_err(),
            "readback domain mutation {mutate}"
        );
    }
}

#[test]
fn canonical_orion_computer_playback_write_bound_is_conservative_24_channels() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    let mut sources = vec![RoutingSource { bank: 11, index: 0 }; 32];
    sources[0] = RoutingSource { bank: 2, index: 23 };
    driver
        .encode(Action::SetRoutingGroup {
            destination: 10,
            changed_channel: None,
            sources: sources.clone(),
        })
        .expect("Computer Playback 24 is within the host-independent write bound");

    sources[0] = RoutingSource { bank: 2, index: 24 };
    let error = driver
        .encode(Action::SetRoutingGroup {
            destination: 10,
            changed_channel: None,
            sources,
        })
        .expect_err("Computer Playback 25 requires a future active-host capability");
    assert!(error.to_string().contains("outside 0..23"));
}

#[test]
fn canonical_orion_accepts_only_observed_oscillator_routing_readback_indices() {
    let driver = ProfileDriver::new(canonical_orion_entry()).expect("canonical Orion driver");
    let mut routing = vec![0; 320];
    routing[0] = 0x75;
    routing[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    routing[8] = 0x03;
    routing[12] = 6;
    routing[16] = 6;
    for channel in 0..32 {
        routing[17 + channel * 2] = 0x0b;
    }
    routing[19] = 0x0c;

    for oscillator_index in [0, 1] {
        routing[20] = oscillator_index;
        let DeviceEvent::QueryReply {
            patch: Some(DynamicStatePatch::Routing(group)),
            ..
        } = driver
            .decode(&routing)
            .expect("observed oscillator routing readback")
            .expect("routing event")
        else {
            panic!("routing patch")
        };
        assert_eq!(group.destination, 6);
        assert_eq!(group.sources.len(), 32);
        assert_eq!(
            group.sources[1],
            RoutingSource {
                bank: 0x0c,
                index: u16::from(oscillator_index),
            }
        );
        assert!(group
            .sources
            .iter()
            .enumerate()
            .all(|(channel, source)| channel == 1
                || *source
                    == RoutingSource {
                        bank: 0x0b,
                        index: 0
                    }));
    }

    routing[20] = 2;
    let out_of_range = driver
        .decode(&routing)
        .expect_err("unobserved oscillator index must reject");
    assert!(out_of_range
        .to_string()
        .contains("not in the profile readback domain"));

    routing[19] = 0x0d;
    routing[20] = 0;
    let unknown_bank = driver
        .decode(&routing)
        .expect_err("unknown source bank must reject");
    assert!(unknown_bank.to_string().contains("source bank 13"));

    let outbound = driver
        .encode(Action::SetRoutingGroup {
            destination: 6,
            changed_channel: None,
            sources: vec![
                RoutingSource {
                    bank: 0x0c,
                    index: 0
                };
                32
            ],
        })
        .expect_err("readback-only oscillator domain must not authorize writes");
    assert!(outbound.to_string().contains("source bank 12"));
}

#[test]
fn zen_go_exposes_no_talkback_writes() {
    let driver = zen_go_driver();
    for (control, value) in [
        (GlobalControl::TalkbackButton, ControlValue::Bool(false)),
        (GlobalControl::TalkbackSource, ControlValue::Enum(0)),
        (GlobalControl::TalkbackGain, ControlValue::Int(0)),
        (GlobalControl::TalkbackSourceResidue, ControlValue::Enum(0)),
    ] {
        assert!(driver.encode(Action::SetGlobal { control, value }).is_err());
    }
}

#[test]
fn complete_routing_group_preserves_every_ordered_source_pair() {
    let driver = profile_driver_from_fixture();
    assert!(driver
        .encode(Action::SetRouting {
            destination: 0,
            channel: 3,
            source: RoutingSource { bank: 1, index: 2 }
        })
        .is_err());
    let sources: Vec<_> = (0..16)
        .map(|index| RoutingSource { bank: 3, index })
        .collect();
    let frame = driver
        .encode(Action::SetRoutingGroup {
            destination: 0,
            changed_channel: None,
            sources: sources.clone(),
        })
        .unwrap()
        .frames
        .remove(0);
    let mut expected = vec![0; 320];
    expected[0] = 0x70;
    expected[4] = 0x53;
    expected[16] = 0xd3;
    expected[17] = 0x41;
    expected[18] = 0;
    for (channel, source) in sources.iter().enumerate() {
        expected[19 + channel * 2] = source.bank;
        expected[20 + channel * 2] = source.index as u8;
    }
    assert_eq!(frame, expected);
    assert!(driver
        .encode(Action::SetRoutingGroup {
            destination: 0,
            changed_channel: None,
            sources: vec![RoutingSource { bank: 0, index: 0 }; 15]
        })
        .is_err());
    assert!(driver
        .encode(Action::SetRoutingGroup {
            destination: 0,
            changed_channel: None,
            sources: vec![RoutingSource { bank: 12, index: 0 }; 16]
        })
        .is_err());
}

#[test]
fn inbound_readback_bounds_and_complete_patches_are_enforced() {
    let driver = profile_driver_from_fixture();
    let mixer_bytes = hex_fixture(include_str!("fixtures/orion/readback_75.hex"));
    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Mixer(surface)),
        ..
    } = driver.decode(&mixer_bytes).unwrap().unwrap()
    else {
        panic!("mixer patch")
    };
    let master = surface.master.unwrap();
    assert_eq!(
        (
            master.fader,
            master.pan,
            master.muted,
            master.soloed,
            master.send
        ),
        (Some(10), Some(-30), Some(true), Some(false), Some(20))
    );
    assert_eq!(surface.strips.len(), 32);
    assert_eq!(
        (surface.strips[31].fader, surface.strips[31].send),
        (Some(42), Some(52))
    );

    let mut routing = mixer_bytes.clone();
    routing[8] = 0x03;
    routing[12] = 0;
    routing[16] = 0;
    for index in 0..16 {
        routing[17 + index * 2] = 3;
        routing[18 + index * 2] = index as u8;
    }
    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Routing(group)),
        ..
    } = driver.decode(&routing).unwrap().unwrap()
    else {
        panic!("routing patch")
    };
    assert_eq!(group.sources.len(), 16);
    assert_eq!(group.sources[15], RoutingSource { bank: 3, index: 15 });

    let mut unknown = mixer_bytes.clone();
    unknown[8] = 0x7e;
    assert!(driver.decode(&unknown).is_err());
    let mut outside = mixer_bytes.clone();
    outside[12] = 15;
    assert!(driver.decode(&outside).is_err());
    assert!(driver.decode(&mixer_bytes[..40]).is_err());
}

#[test]
fn observed_readback_domains_accept_complete_host_dependent_bank_without_authorizing_writes() {
    let driver = ProfileDriver::new(observed_readback_fixture_entry())
        .expect("observed readback fixture driver");
    let mut routing = vec![0; 320];
    routing[0] = 0x75;
    routing[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    routing[8] = 0x03;
    routing[12] = 13;
    routing[16] = 13;
    for index in 0..24 {
        routing[17 + index * 2] = 0x02;
        routing[18 + index * 2] = index as u8;
    }
    for index in 0..8 {
        let channel = 24 + index;
        routing[17 + channel * 2] = 0;
        routing[18 + channel * 2] = index as u8;
    }

    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Routing(group)),
        ..
    } = driver
        .decode(&routing)
        .expect("complete observed bank-2 routing readback")
        .expect("routing event")
    else {
        panic!("routing patch")
    };
    assert_eq!(group.destination, 13);
    assert_eq!(group.sources.len(), 32);
    for (index, source) in group.sources.iter().take(24).enumerate() {
        assert_eq!(source.bank, 2);
        assert_eq!(source.index, index as u16);
    }
    assert_eq!(group.sources[24], RoutingSource { bank: 0, index: 0 });

    let outbound_error = driver
        .encode(Action::SetRoutingGroup {
            destination: 13,
            changed_channel: None,
            sources: vec![RoutingSource { bank: 2, index: 0 }; 32],
        })
        .expect_err("observed readback domain must not authorize writes");
    assert!(outbound_error.to_string().contains("source bank 2"));

    let mut destination_two = vec![0; 320];
    destination_two[0] = 0x75;
    destination_two[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    destination_two[8] = 0x03;
    destination_two[12] = 2;
    destination_two[16] = 2;
    destination_two[17..21].copy_from_slice(&[0x02, 0x02, 0x02, 0x03]);
    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Routing(group)),
        ..
    } = driver
        .decode(&destination_two)
        .expect("destination 2 observed bank-2 routing readback")
        .expect("destination 2 routing event")
    else {
        panic!("destination 2 routing patch")
    };
    assert_eq!(
        group.sources,
        vec![
            RoutingSource { bank: 2, index: 2 },
            RoutingSource { bank: 2, index: 3 },
        ]
    );

    let mut unlisted = routing.clone();
    unlisted[18 + 23 * 2] = 24;
    let unlisted_error = driver
        .decode(&unlisted)
        .expect_err("unlisted observed source index must reject");
    assert!(unlisted_error
        .to_string()
        .contains("not in the profile readback domain"));

    let mut unknown = routing;
    unknown[17] = 0x0d;
    let unknown_error = driver
        .decode(&unknown)
        .expect_err("unknown bank 0x0d must reject");
    assert!(unknown_error.to_string().contains("source bank 13"));
}

#[test]
fn zen_go_normalized_actions_equal_existing_full_frames() {
    let driver = zen_go_driver();
    let output_action = Action::SetOutput {
        address: OutputAddress { id: 0 },
        control: OutputControl::Level,
        value: ControlValue::Int(0x12),
    };
    assert_eq!(
        driver.encode(output_action).unwrap().frames,
        vec![match encode_command(Command::SetOutputVolume {
            target: antelope_protocol::OutputTarget::Monitor,
            step: 0x12
        }) {
            antelope_protocol::EncodeResult::Single(frame) => frame.to_vec(),
            _ => panic!(),
        }]
    );
    let preamp_action = Action::SetInput {
        address: InputAddress { space: 0, index: 1 },
        control: InputControl::Mode,
        value: ControlValue::Enum(1),
    };
    assert_eq!(
        driver.encode(preamp_action).unwrap().frames,
        vec![match encode_command(Command::SetPreampMode {
            input: 1,
            mode: antelope_protocol::PreampMode::Line
        }) {
            antelope_protocol::EncodeResult::Single(frame) => frame.to_vec(),
            _ => panic!(),
        }]
    );
    let mixer_action = Action::SetMixerStripState {
        address: MixerAddress {
            surface: 1,
            strip: 7,
        },
        fader: 0x22,
        pan: 30,
        muted: true,
        soloed: false,
        send: None,
    };
    assert_eq!(
        driver.encode(mixer_action).unwrap().frames,
        vec![match encode_command(Command::SetMixerLevel {
            mixer: antelope_protocol::MixerSurface::Mix2,
            channel: 7,
            level: 0x22,
            pan_state: antelope_protocol::PanState::right(),
            muted: true,
            soloed: false
        }) {
            antelope_protocol::EncodeResult::Single(frame) => frame.to_vec(),
            _ => panic!(),
        }]
    );
    for (control, value, command) in [
        (
            GlobalControl::SampleRate,
            ControlValue::Enum(4),
            Command::SetSampleRate(antelope_protocol::SampleRate::Hz96000),
        ),
        (
            GlobalControl::ClockSource,
            ControlValue::Enum(2),
            Command::SetClockSource(antelope_protocol::ClockSource::Usb),
        ),
        (
            GlobalControl::Surface,
            ControlValue::Enum(0x0c),
            Command::SelectSurface(antelope_protocol::Surface::Hp2),
        ),
    ] {
        assert_eq!(
            driver
                .encode(Action::SetGlobal { control, value })
                .unwrap()
                .frames,
            vec![match encode_command(command) {
                antelope_protocol::EncodeResult::Single(frame) => frame.to_vec(),
                antelope_protocol::EncodeResult::WithRefresh(frame) => frame.to_vec(),
                _ => panic!(),
            }]
        );
    }
    let link = Action::SetLink {
        surface: 0,
        pair: 0,
        enabled: true,
    };
    let expected = match encode_command(Command::SetLinkState {
        selector: 0,
        enabled: true,
        companion_bank: Some(0),
    }) {
        antelope_protocol::EncodeResult::WithCompanion { companion, main } => {
            vec![companion.to_vec(), main.to_vec()]
        }
        _ => panic!(),
    };
    assert_eq!(driver.encode(link).unwrap().frames, expected);
    let query = QueryRequest::new(4, 3);
    assert_eq!(
        driver.encode(Action::Query(query)).unwrap().frames,
        vec![encode_query(query).to_vec()]
    );

    let mut assignments = [antelope_protocol::MixerAssignment::Mute; 16];
    assignments[10] = antelope_protocol::MixerAssignment::ComputerPlay(1);
    let sources = assignments
        .into_iter()
        .map(|assignment| match assignment {
            antelope_protocol::MixerAssignment::Mute => RoutingSource { bank: 8, index: 0 },
            antelope_protocol::MixerAssignment::ComputerPlay(channel) => RoutingSource {
                bank: 1,
                index: u16::from(channel - 1),
            },
            _ => unreachable!(),
        })
        .collect();
    let expected: Vec<_> =
        encode_mixer_assignment_frames_with_table(11, assignments[10], &assignments)
            .into_iter()
            .map(|frame| frame.to_vec())
            .collect();
    assert_eq!(
        driver
            .encode(Action::SetRoutingGroup {
                destination: 0,
                changed_channel: Some(10),
                sources
            })
            .unwrap()
            .frames,
        expected
    );
}
