use antelope_protocol::{
    encode_command, encode_mixer_assignment_frames_with_table, encode_query, load_profile_pack,
    Action, Command, ControlValue, DeviceDriver, DeviceEvent, DriverError, DynamicMixerSurface,
    DynamicStatePatch, FrameEndian, FrameOperation, GlobalControl, InputAddress, InputControl,
    MixerAddress, MixerControl, OutputAddress, OutputControl, ProfileDriver, QueryRequest,
    RoutingSource, RuntimeDriverKind, RuntimeEntry, RuntimeReadiness, WholeStateField, ZenGoDriver,
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
    let mut entry = stored_orion_entry();
    entry.readiness = RuntimeReadiness::Supported;
    entry.driver_kind = RuntimeDriverKind::Profile;
    // Synthetic test assumption: generic codec fixtures model unnumbered HID
    // payloads only. Canonical Orion framing remains unknown and disabled.
    entry.profile.transport.uses_numbered_reports = Some(false);
    entry
}

fn disabled_orion_entry() -> RuntimeEntry {
    stored_orion_entry()
}

fn profile_driver_from_fixture() -> ProfileDriver {
    ProfileDriver::new(fixture_entry()).expect("profile driver")
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
fn synthetic_unnumbered_fixture_constructs_for_generic_codec_tests() {
    ProfileDriver::new(fixture_entry()).expect("synthetic unnumbered fixture");
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
            pan: 12,
            muted: false,
            soloed: false,
            send: Some(30),
        })
        .expect("mixer frame");
    assert_eq!(&mixer.frames[0][16..23], &[0xd4, 0x05, 3, 32, 60, 12, 30]);

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
    expected.extend([(0x0a, 0), (0x15, 0), (0x16, 0)]);
    expected.extend((0..64).map(|index| (0x19, index)));
    expected.extend([(0x0b, 0), (0x0b, 4)]);

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
    let encoded: Vec<u8> = driver
        .startup_requests()
        .iter()
        .flat_map(|request| {
            driver
                .encode(Action::Query(*request))
                .expect("bounded startup request")
                .frames
                .remove(0)
        })
        .collect();
    assert_eq!(encoded, fixture);
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
        vec![(3, 16)]
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
            control: OutputControl::Parameter(0x69),
            value: ControlValue::Bool(true),
        },
        Action::SetGlobal {
            control: GlobalControl::SampleRate,
            value: ControlValue::Enum(6),
        },
        Action::SetGlobal {
            control: GlobalControl::Parameter(0x0e),
            value: ControlValue::Int(73),
        },
        Action::SetMixerStripState {
            address: MixerAddress {
                surface: 3,
                strip: 32,
            },
            fader: 44,
            pan: 32,
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
                control: OutputControl::Parameter(0x69),
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
                control: GlobalControl::Parameter(0x0e),
                value: ControlValue::Int(73),
            },
            0x12,
            vec![0x0e, 73],
        ),
    ] {
        assert_complete_parameter_frame(action, opcode, &payload);
    }
}

#[test]
fn profile_driver_encodes_every_confirmed_finite_orion_family() {
    let driver = profile_driver_from_fixture();
    for action in required_orion_actions() {
        let batch = driver.encode(action).expect("confirmed Orion action");
        assert_eq!(batch.frames.len(), 1);
        assert_eq!(batch.frames[0].len(), 320);
    }
}

#[test]
fn profile_derived_auraverb_whole_state_matches_complete_confirmed_frame() {
    let action = required_orion_actions()
        .into_iter()
        .find(|action| matches!(action, Action::SetWholeState { .. }))
        .expect("AuraVerb action");
    let actual = profile_driver_from_fixture()
        .encode(action)
        .unwrap()
        .frames
        .remove(0);
    let mut expected = vec![0; 320];
    expected[0] = 0x70;
    expected[4] = 0x1d;
    expected[16..29].copy_from_slice(&[0xda, 0x0b, 0, 81, 100, 0, 100, 11, 13, 24, 66, 50, 1]);
    assert_eq!(actual, expected);
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
    let event = driver.decode(&meter).expect("meter decode").expect("event");
    assert!(matches!(event, DeviceEvent::Meter { raw, .. } if raw == meter));
}

#[test]
fn malformed_known_reports_fail_instead_of_being_ignored() {
    let driver = profile_driver_from_fixture();
    assert!(driver.decode(&[0x75; 12]).is_err());

    let mut invalid_discriminator = vec![0; 320];
    invalid_discriminator[0] = 0x75;
    invalid_discriminator[1] = 0x7f;
    assert!(driver.decode(&invalid_discriminator).is_err());

    let unknown = vec![0x99; 320];
    assert!(driver.decode(&unknown).expect("unknown policy").is_none());
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
            .map(|output| (output.address.id, output.level, output.muted, output.dimmed,))
            .collect::<Vec<_>>(),
        vec![
            (0, Some(10), Some(true), Some(false)),
            (1, Some(20), Some(false), Some(true)),
            (2, Some(30), Some(false), Some(false)),
            (3, Some(40), Some(true), None),
            (4, Some(50), None, None),
            (5, Some(60), Some(false), Some(true)),
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
                control: GlobalControl::Parameter(0x0e),
                value: ControlValue::Int(73),
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
fn profile_derived_meter_report_decodes_all_confirmed_physical_meters() {
    let driver = profile_driver_from_fixture();
    let mut frame = vec![0; 320];
    frame[0] = 0x75;
    frame[1] = 0x1f;
    frame[32..44].copy_from_slice(&[1, 4, 7, 10, 13, 16, 19, 22, 25, 28, 31, 34]);
    let DeviceEvent::Meter { inputs, raw } = driver.decode(&frame).unwrap().unwrap() else {
        panic!("typed meter event")
    };
    assert_eq!(raw, frame);
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
fn valid_bounded_non_patch_readbacks_return_owned_none_patch() {
    let driver = profile_driver_from_fixture();
    for (category, index) in [(0x0a, 0), (0x0b, 7), (0x11, 1), (0x19, 63), (0x1a, 15)] {
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
        .map(|index| RoutingSource { bank: 2, index })
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
    let driver = ZenGoDriver::new();
    let output = driver
        .encode(Action::SetOutput {
            address: OutputAddress { id: 0 },
            control: OutputControl::Level,
            value: ControlValue::Int(0x12),
        })
        .expect("output");
    assert_eq!(&output.frames[0][0x10..0x13], &[0x47, 0x00, 0x12]);

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
            pan: 0x3e,
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
    let driver = ZenGoDriver::new();
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
fn constructor_rejects_missing_meter_frame() {
    assert!(constructor_error_without_frame("meter_report")
        .to_string()
        .contains("meter"));
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
fn constructor_rejects_missing_state_or_meter_semantics_before_io() {
    for (frame_id, semantic) in [
        ("state_report", "physical_gain"),
        ("meter_report", "physical_meter"),
    ] {
        let mut entry = fixture_entry();
        entry
            .profile
            .frames
            .iter_mut()
            .find(|frame| frame.id == frame_id)
            .unwrap()
            .operations
            .retain(|operation| {
                !matches!(operation, FrameOperation::Indexed { index_field, .. } if index_field == semantic)
            });
        assert!(ProfileDriver::new(entry).is_err(), "{frame_id}/{semantic}");
    }
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
fn constructor_rejects_ambiguous_semantic_operation() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "command")
        .unwrap();
    let duplicate = frame.operations.iter().find(|operation| matches!(operation, FrameOperation::Scalar { field, .. } if field == "parameter")).unwrap().clone();
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
    let FrameOperation::Scalar { field, .. } = frame.operations.iter_mut().find(|operation| matches!(operation, FrameOperation::Scalar { field, .. } if field == "parameter")).unwrap() else { unreachable!() };
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
        .find(|frame| frame.id == "mix_command")
        .unwrap();
    let FrameOperation::BitField { mask, .. } = frame
        .operations
        .iter_mut()
        .find(|operation| matches!(operation, FrameOperation::BitField { field, .. } if field == "pan"))
        .unwrap()
    else {
        unreachable!()
    };
    *mask = 0;

    let error = ProfileDriver::new(entry).expect_err("zero bit-field mask must fail construction");
    assert!(error
        .to_string()
        .contains("mix_command bit field \"pan\" has zero mask"));
}

#[test]
fn constructor_rejects_bit_field_shift_eight_before_encode() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "mix_command")
        .unwrap();
    let FrameOperation::BitField { shift, .. } = frame
        .operations
        .iter_mut()
        .find(|operation| matches!(operation, FrameOperation::BitField { field, .. } if field == "pan"))
        .unwrap()
    else {
        unreachable!()
    };
    *shift = 8;

    let error =
        ProfileDriver::new(entry).expect_err("shift-eight bit field must fail construction");
    assert!(error
        .to_string()
        .contains("mix_command bit field \"pan\" shift 8"));
}

#[test]
fn constructor_rejects_bit_field_mask_below_shift_before_encode() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "mix_command")
        .unwrap();
    let FrameOperation::BitField { mask, shift, .. } = frame
        .operations
        .iter_mut()
        .find(|operation| matches!(operation, FrameOperation::BitField { field, .. } if field == "pan"))
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
        .contains("mix_command bit field \"pan\" mask 0x03 is below shift 2"));
}

#[test]
fn shifted_semantic_offsets_and_big_endian_width_are_profile_driven() {
    let mut entry = fixture_entry();
    let frame = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "command")
        .unwrap();
    for operation in &mut frame.operations {
        if let FrameOperation::Scalar { field, offset, .. } = operation {
            *offset = match field.as_str() {
                "parameter" => 30,
                "target" => 31,
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
                "parameter" => 30,
                "target" => 31,
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
        expected[21] = pan as u8 | if muted { 0x40 } else { 0 } | if soloed { 0x80 } else { 0 };
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
    let readback = entry
        .profile
        .frames
        .iter_mut()
        .find(|frame| frame.id == "readback")
        .unwrap();
    let FrameOperation::Indexed { stride, width, .. } = readback.operations.iter_mut().find(|operation| matches!(operation, FrameOperation::Indexed { index_field, .. } if index_field == "mixer_slot")).unwrap() else { unreachable!() };
    *stride = 2;
    *width = 2;
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
fn typed_pair_index_resolution_uses_only_confirmed_link_domain() {
    let driver = profile_driver_from_fixture();
    let frame = driver
        .encode(Action::SetLink {
            surface: 3,
            pair: 15,
            enabled: true,
        })
        .expect("confirmed mixer link domain")
        .frames
        .remove(0);
    let mut expected = vec![0; 320];
    expected[0] = 0x70;
    expected[4] = 0x14;
    expected[16] = 0xa2;
    expected[17] = 3;
    expected[18] = 15;
    expected[19] = 1;
    assert_eq!(frame, expected);

    for undeclared_space in [0, 1] {
        let error = driver
            .encode(Action::SetLink {
                surface: undeclared_space,
                pair: 0,
                enabled: true,
            })
            .expect_err("undeclared link space must reject before frame emission");
        assert!(error.to_string().contains("link domain"));
    }
    assert!(driver
        .encode(Action::SetLink {
            surface: 3,
            pair: 16,
            enabled: true
        })
        .is_err());
}

#[test]
fn destination_specific_routing_domains_control_outbound_and_inbound_validation() {
    let mut entry = fixture_entry();
    entry.profile.routing_groups[1]
        .source_domains
        .retain(|domain| domain.bank != 2);
    let driver = ProfileDriver::new(entry).expect("destination-specific routing fixture");

    driver
        .encode(Action::SetRoutingGroup {
            destination: 0,
            changed_channel: None,
            sources: vec![RoutingSource { bank: 2, index: 15 }; 16],
        })
        .expect("bank 2 is valid for destination A");
    let outbound_error = driver
        .encode(Action::SetRoutingGroup {
            destination: 1,
            changed_channel: None,
            sources: vec![RoutingSource { bank: 2, index: 0 }; 2],
        })
        .expect_err("bank 2 is unavailable for destination B");
    assert!(outbound_error.to_string().contains("destination 1"));

    let mut inbound = vec![0; 320];
    inbound[0] = 0x75;
    inbound[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    inbound[8] = 0x03;
    inbound[12] = 0;
    for channel in 0..16 {
        inbound[16 + channel * 2] = 2;
        inbound[17 + channel * 2] = channel as u8;
    }
    driver
        .decode(&inbound)
        .expect("destination A inbound domain")
        .expect("destination A event");
    inbound[12] = 1;
    let inbound_error = driver
        .decode(&inbound)
        .expect_err("destination B inbound domain must reject");
    assert!(inbound_error.to_string().contains("index 1"));
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
    mismatched.profile.link_domains[0].pair_count = 15;
    let error = ProfileDriver::new(mismatched).expect_err("semantic pair mapping mismatch");
    assert!(error.to_string().contains("pair mapping"));
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
        .map(|index| RoutingSource { bank: 2, index })
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
        (Some(10), Some(2), Some(true), Some(false), Some(20))
    );
    assert_eq!(surface.strips.len(), 32);
    assert_eq!(
        (surface.strips[31].fader, surface.strips[31].send),
        (Some(42), Some(52))
    );

    let mut routing = mixer_bytes.clone();
    routing[8] = 0x03;
    routing[12] = 0;
    for index in 0..16 {
        routing[16 + index * 2] = 2;
        routing[17 + index * 2] = index as u8;
    }
    let DeviceEvent::QueryReply {
        patch: Some(DynamicStatePatch::Routing(group)),
        ..
    } = driver.decode(&routing).unwrap().unwrap()
    else {
        panic!("routing patch")
    };
    assert_eq!(group.sources.len(), 16);
    assert_eq!(group.sources[15], RoutingSource { bank: 2, index: 15 });

    let mut unknown = mixer_bytes.clone();
    unknown[8] = 0x7e;
    assert!(driver.decode(&unknown).is_err());
    let mut outside = mixer_bytes.clone();
    outside[12] = 15;
    assert!(driver.decode(&outside).is_err());
    assert!(driver.decode(&mixer_bytes[..40]).is_err());
}

#[test]
fn zen_go_normalized_actions_equal_existing_full_frames() {
    let driver = ZenGoDriver::new();
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
        pan: 0x3e,
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
