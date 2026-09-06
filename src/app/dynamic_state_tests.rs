use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use antelope_protocol::{
    load_profile_pack, Action, ControlValue, DeviceEvent, DynamicDeviceState, DynamicGlobalState,
    DynamicInputState, DynamicMixerStrip, DynamicMixerSurface, DynamicOutputState,
    DynamicRoutingGroup, DynamicStatePatch, GlobalControl, InputAddress, MixerAddress,
    OutputAddress, OutputControl, OutputMode, ProfileDriver, RoutingSource, RuntimeDriverKind,
    RuntimeEntry, RuntimeProfile, RuntimeReadiness, Surface,
};

use ratatui::layout::Rect;

use crate::device::ProfileCatalog;
use crate::transport::{MockTransport, Transport};

use super::{AppState, Controller, Intent, PendingMutation};

fn orion_entry() -> RuntimeEntry {
    let mut entry = load_profile_pack(include_bytes!(
        "../../antelope-protocol/tests/fixtures/orion/profile_driver_pack.json"
    ))
    .expect("Orion fixture pack")
    .profiles
    .into_iter()
    .next()
    .expect("Orion fixture entry");
    entry.readiness = RuntimeReadiness::Supported;
    entry.driver_kind = RuntimeDriverKind::Profile;
    // Synthetic test assumption: profile-derived codec tests use unnumbered
    // payload framing; canonical Orion framing remains unknown and disabled.
    entry.profile.transport.uses_numbered_reports = Some(false);
    entry
}

fn orion_profile() -> RuntimeProfile {
    orion_entry().profile
}

fn zen_go_profile() -> RuntimeProfile {
    ProfileCatalog::builtin()
        .find(0x23e5, 0xa015)
        .expect("built-in Zen Go profile")
        .profile
        .clone()
}

fn supported_profile_without_optional_control() -> RuntimeEntry {
    let mut entry = orion_entry();
    entry
        .profile
        .params
        .retain(|parameter| parameter.name != "bus_dim");
    entry
}

fn controller_for_profile(entry: RuntimeEntry) -> Controller {
    Controller::new(
        Box::new(MockTransport::default()),
        Box::new(ProfileDriver::new(entry).expect("supported profile driver")),
    )
    .expect("profile controller")
}

fn unsupported_input_action() -> Action {
    Action::SetOutput {
        address: OutputAddress { id: 0 },
        control: OutputControl::Dim,
        value: ControlValue::Bool(true),
    }
}

#[test]
fn hardware_surface_global_does_not_reset_ui_mixer_surface() {
    let mut state = AppState::from_profile(&zen_go_profile());
    state.mixer.surface_index = 0;
    state.globals = vec![DynamicGlobalState {
        control: GlobalControl::Surface,
        value: ControlValue::Enum(0x0c),
    }];

    state.apply_dynamic_globals_to_status();

    assert_eq!(state.mixer.surface_index, 0);
    assert_eq!(state.mixer.surface, Surface::Hp2);
}

#[test]
fn dynamic_state_orion_allocates_profile_geometry() {
    let state = AppState::from_profile(&orion_profile());
    assert_eq!(state.inputs_for_space("physical_inputs").len(), 12);
    assert_eq!(state.inputs_for_space("adat_inputs").len(), 16);
    assert_eq!(state.inputs_for_space("spdif_inputs").len(), 2);
    assert_eq!(state.outputs().len(), 6);
    assert_eq!(state.mixers().len(), 4);
    assert!(state.mixers().iter().all(|mixer| mixer.master.is_some()));
    assert!(state.mixers().iter().all(|mixer| mixer.strips.len() == 32));
}

#[test]
fn dynamic_state_zen_go_preserves_geometry_and_labels() {
    let state = AppState::from_profile(&zen_go_profile());
    assert_eq!(
        state
            .input_spaces
            .iter()
            .map(|space| space.id.as_str())
            .collect::<Vec<_>>(),
        vec!["physical_inputs"]
    );
    assert_eq!(state.inputs_for_space("physical_inputs").len(), 2);
    assert_eq!(state.outputs().len(), 3);
    assert_eq!(state.mixers().len(), 2);
    assert!(state.mixers().iter().all(|mixer| mixer.strips.len() == 16));
    assert_eq!(state.outputs()[0].name, "Monitor");
    assert_eq!(state.mixers()[0].name, "MIX 1 / Monitor-HP1");
}

#[test]
fn dynamic_state_reconfiguration_clamps_selection_and_viewport() {
    let mut state = AppState::from_profile(&orion_profile());
    state.output.selected = 5;
    state.preamp.selected_input = 11;
    state.mixer.surface_index = 3;
    state.mixer.selected_channel = 31;
    state.mixer.strip_scroll = 29;

    state.reconfigure_for_profile(&zen_go_profile());

    assert_eq!(state.output.selected, 2);
    assert_eq!(state.preamp.selected_input, 1);
    assert_eq!(state.mixer.surface_index, 1);
    assert_eq!(state.mixer.selected_channel, 15);
    assert!(state.mixer.strip_scroll <= 15);
}

#[test]
fn dynamic_state_snapshot_keeps_owned_raw_bytes() {
    let mut state = AppState::from_profile(&orion_profile());
    let raw = vec![0x73; 777];
    let snapshot = DynamicDeviceState {
        globals: vec![],
        inputs: vec![],
        outputs: vec![],
        mixers: vec![],
        meters: vec![],
        routing: vec![],
        zen_go_compatibility: None,
    };

    assert!(!state.apply_dynamic_state(snapshot, raw.clone()));
    assert_eq!(state.raw_view.latest_raw_73, Some(raw));
}

#[test]
fn dynamic_state_query_patches_use_stable_addresses() {
    let mut state = AppState::from_profile(&orion_profile());
    let mixer = DynamicMixerSurface {
        surface: 2,
        name: "patched".into(),
        master: state.mixers()[2].master.clone(),
        strips: (1..=32)
            .map(|strip| DynamicMixerStrip {
                strip,
                name: format!("CH {strip:02}"),
                fader: Some(i32::from(strip)),
                pan: Some(0),
                send: Some(96),
                muted: Some(false),
                soloed: Some(false),
                linked: Some(false),
                meter: None,
                parameters: vec![],
            })
            .collect(),
    };
    assert!(state.observe_event(DeviceEvent::QueryReply {
        query_id: 4,
        sub_id: 2,
        body: vec![1, 2, 3],
        patch: Some(DynamicStatePatch::Mixer(mixer)),
        raw: vec![0x75; 320],
    }));
    assert_eq!(state.mixers()[2].name, "patched");
    assert_eq!(state.mixers()[2].strips[31].fader, Some(32));

    let mut zen_go = AppState::from_profile(&zen_go_profile());
    let original_name = zen_go.mixers()[0].strips[0].name.clone();
    let original = &mut zen_go.mixers_mut()[0].strips[0];
    original.fader = Some(30);
    original.pan = Some(17);
    original.muted = Some(true);
    original.soloed = Some(false);
    original.linked = Some(true);
    original.meter = Some(42);
    let mut partial = zen_go.mixers()[0].clone();
    partial.name.clear();
    for strip in &mut partial.strips {
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
    partial.strips[0].fader = Some(7);
    zen_go.observe_event(DeviceEvent::QueryReply {
        query_id: 4,
        sub_id: 0,
        body: vec![],
        patch: Some(DynamicStatePatch::Mixer(partial)),
        raw: vec![0x75; 320],
    });
    let merged = &zen_go.mixers()[0].strips[0];
    assert_eq!(zen_go.mixers()[0].name, "MIX 1 / Monitor-HP1");
    assert_eq!(merged.name, original_name);
    assert_eq!(merged.fader, Some(7));
    assert_eq!(merged.pan, Some(17));
    assert_eq!(merged.muted, Some(true));
    assert_eq!(merged.soloed, Some(false));
    assert_eq!(merged.linked, Some(true));
    assert_eq!(merged.meter, Some(42));

    let before = state.mixers().to_vec();
    let unknown = DynamicMixerSurface {
        surface: 99,
        name: "unknown".into(),
        master: None,
        strips: vec![],
    };
    assert!(state.observe_event(DeviceEvent::QueryReply {
        query_id: 4,
        sub_id: 99,
        body: vec![],
        patch: Some(DynamicStatePatch::Mixer(unknown)),
        raw: vec![0x75; 320],
    }));
    assert_eq!(state.mixers(), before);
}

#[test]
fn dynamic_state_malformed_mixer_patches_preserve_declared_topology_and_values() {
    let observe = |state: &mut AppState, mixer| {
        state.observe_event(DeviceEvent::QueryReply {
            query_id: 4,
            sub_id: 0,
            body: vec![],
            patch: Some(DynamicStatePatch::Mixer(mixer)),
            raw: vec![0x75; 320],
        });
    };

    let mut orion = AppState::from_profile(&orion_profile());
    orion.mixers_mut()[0].master.as_mut().expect("master").fader = Some(70);
    orion.mixers_mut()[0].strips[0].fader = Some(31);
    let declared_orion = orion.mixers()[0].clone();

    let mut wrong_count = declared_orion.clone();
    wrong_count.strips.pop();
    observe(&mut orion, wrong_count);
    assert_eq!(orion.mixers()[0], declared_orion);

    let mut reordered = declared_orion.clone();
    reordered.strips.swap(0, 1);
    observe(&mut orion, reordered);
    assert_eq!(orion.mixers()[0], declared_orion);

    let mut unknown_address = declared_orion.clone();
    unknown_address.strips[0].strip = u16::MAX;
    observe(&mut orion, unknown_address);
    assert_eq!(orion.mixers()[0], declared_orion);

    let mut missing_master = declared_orion.clone();
    missing_master.master = None;
    observe(&mut orion, missing_master);
    assert_eq!(orion.mixers()[0], declared_orion);

    let mut zen_go = AppState::from_profile(&zen_go_profile());
    zen_go.mixers_mut()[0].strips[0].fader = Some(41);
    let declared_zen_go = zen_go.mixers()[0].clone();
    let mut unexpected_master = declared_zen_go.clone();
    unexpected_master.master = Some(DynamicMixerStrip {
        strip: 0,
        name: "Unexpected".into(),
        fader: Some(99),
        pan: None,
        send: None,
        muted: None,
        soloed: None,
        linked: None,
        meter: None,
        parameters: vec![],
    });
    observe(&mut zen_go, unexpected_master);
    assert_eq!(zen_go.mixers()[0], declared_zen_go);
}

#[test]
fn dynamic_state_declared_empty_input_space_survives_reconfigure_and_unknown_patch() {
    let mut profile = zen_go_profile();
    let mut empty_space = profile.address_spaces[0].clone();
    empty_space.id = "empty_inputs".into();
    empty_space.space_id = 77;
    empty_space.name = "Empty Inputs".into();
    empty_space.count = Some(0);
    profile.address_spaces.push(empty_space);

    let mut state = AppState::from_profile(&profile);
    let assert_empty_space = |state: &AppState| {
        let space = state
            .input_spaces
            .iter()
            .find(|space| space.space_id == 77)
            .expect("declared empty input space");
        assert_eq!(space.id, "empty_inputs");
        assert_eq!(space.name, "Empty Inputs");
        assert!(space.inputs.is_empty());
    };
    assert_empty_space(&state);

    state.reconfigure_for_profile(&profile);
    assert_empty_space(&state);

    state.observe_event(DeviceEvent::QueryReply {
        query_id: 0,
        sub_id: 0,
        body: vec![],
        patch: Some(DynamicStatePatch::Inputs(vec![DynamicInputState {
            address: InputAddress {
                space: 77,
                index: 0,
            },
            name: "Unknown".into(),
            mode: None,
            gain: Some(10),
            phantom: None,
            phase: None,
            meter: None,
            parameters: vec![],
        }])),
        raw: vec![0x75; 320],
    });
    assert_empty_space(&state);
}

#[test]
fn dynamic_state_unknown_patches_never_create_profile_topology() {
    let mut state = AppState::from_profile(&zen_go_profile());
    let input_spaces = state.input_spaces.len();
    let input_count = state.inputs_for_space("physical_inputs").len();
    let output_count = state.outputs().len();
    let mixer_count = state.mixers().len();
    let global_count = state.globals.len();
    let routing_count = state.routing.len();

    let patches = [
        DynamicStatePatch::Inputs(vec![DynamicInputState {
            address: InputAddress {
                space: 99,
                index: 0,
            },
            name: "unknown".into(),
            mode: None,
            gain: None,
            phantom: None,
            phase: None,
            meter: None,
            parameters: vec![],
        }]),
        DynamicStatePatch::Outputs(vec![DynamicOutputState {
            address: OutputAddress { id: 99 },
            name: "unknown".into(),
            level: None,
            muted: None,
            dimmed: None,
            parameters: vec![],
        }]),
        DynamicStatePatch::Mixer(DynamicMixerSurface {
            surface: 99,
            name: "unknown".into(),
            master: None,
            strips: vec![],
        }),
        DynamicStatePatch::Globals(vec![DynamicGlobalState {
            control: GlobalControl::Parameter(u16::MAX),
            value: ControlValue::Int(1),
        }]),
        DynamicStatePatch::Routing(DynamicRoutingGroup {
            destination: u16::MAX,
            name: "unknown".into(),
            sources: vec![],
        }),
    ];

    for (sub_id, patch) in patches.into_iter().enumerate() {
        state.observe_event(DeviceEvent::QueryReply {
            query_id: 0,
            sub_id: sub_id as u8,
            body: vec![],
            patch: Some(patch),
            raw: vec![0x75; 320],
        });
    }

    assert_eq!(state.input_spaces.len(), input_spaces);
    assert_eq!(state.inputs_for_space("physical_inputs").len(), input_count);
    assert_eq!(state.outputs().len(), output_count);
    assert_eq!(state.mixers().len(), mixer_count);
    assert_eq!(state.globals.len(), global_count);
    assert_eq!(state.routing.len(), routing_count);
}

#[test]
fn dynamic_state_empty_input_topology_is_not_created_by_patch() {
    let mut state = AppState::from_profile(&zen_go_profile());
    state.input_spaces.clear();
    state.observe_event(DeviceEvent::QueryReply {
        query_id: 0,
        sub_id: 0,
        body: vec![],
        patch: Some(DynamicStatePatch::Inputs(vec![DynamicInputState {
            address: InputAddress { space: 0, index: 0 },
            name: "Input 1".into(),
            mode: None,
            gain: Some(10),
            phantom: None,
            phase: None,
            meter: None,
            parameters: vec![],
        }])),
        raw: vec![0x75; 320],
    });
    assert!(state.input_spaces.is_empty());
}

#[test]
fn dynamic_state_routing_patch_replaces_complete_group() {
    let mut state = AppState::from_profile(&orion_profile());
    state.routing.push(DynamicRoutingGroup {
        destination: 7,
        name: "route".into(),
        sources: vec![RoutingSource { bank: 1, index: 1 }; 16],
    });
    let replacement = DynamicRoutingGroup {
        destination: 7,
        name: "route updated".into(),
        sources: vec![RoutingSource { bank: 2, index: 3 }; 16],
    };
    state.observe_event(DeviceEvent::QueryReply {
        query_id: 3,
        sub_id: 7,
        body: vec![9],
        patch: Some(DynamicStatePatch::Routing(replacement.clone())),
        raw: vec![0x75; 320],
    });
    assert_eq!(state.routing_group(7), Some(&replacement));
}

#[test]
fn dynamic_state_routing_patch_requires_declared_destination_and_count() {
    let mut state = AppState::from_profile(&orion_profile());
    let observe = |state: &mut AppState, group| {
        state.observe_event(DeviceEvent::QueryReply {
            query_id: 3,
            sub_id: 7,
            body: vec![],
            patch: Some(DynamicStatePatch::Routing(group)),
            raw: vec![0x75; 320],
        });
    };
    observe(
        &mut state,
        DynamicRoutingGroup {
            destination: 99,
            name: "unknown".into(),
            sources: vec![RoutingSource { bank: 1, index: 0 }],
        },
    );
    observe(
        &mut state,
        DynamicRoutingGroup {
            destination: 7,
            name: "wrong size".into(),
            sources: vec![RoutingSource { bank: 1, index: 0 }],
        },
    );
    assert!(state.routing.is_empty());

    let valid = DynamicRoutingGroup {
        destination: 7,
        name: "adat".into(),
        sources: vec![RoutingSource { bank: 1, index: 0 }; 16],
    };
    observe(&mut state, valid.clone());
    assert_eq!(state.routing_group(7), Some(&valid));
}

#[test]
fn dynamic_state_missing_optional_control_does_not_write() {
    let entry = supported_profile_without_optional_control();
    let transport = MockTransport::default();
    let driver = ProfileDriver::new(entry).expect("supported profile without optional control");
    let mut controller =
        Controller::new(Box::new(transport.clone()), Box::new(driver)).expect("controller");
    let error = controller
        .send(unsupported_input_action(), None)
        .expect_err("missing optional control must fail");
    assert!(error.to_string().contains("unsupported"));
    assert!(transport.take_writes().is_empty());
}

#[test]
fn dynamic_state_complete_mixer_mutation_preserves_companions() {
    let mut state = AppState::from_profile(&orion_profile());
    let strip = state.mixers_mut()[0].strips.get_mut(0).expect("strip");
    strip.fader = Some(10);
    strip.pan = Some(31);
    strip.send = Some(90);
    strip.muted = Some(true);
    strip.soloed = Some(true);
    let action = state
        .complete_mixer_action(
            MixerAddress {
                surface: 0,
                strip: 1,
            },
            |strip| strip.fader = Some(11),
        )
        .expect("complete mixer action");
    assert!(matches!(
        action,
        Action::SetMixerStripState {
            fader: 11,
            pan: 31,
            muted: true,
            soloed: true,
            send: Some(90),
            ..
        }
    ));
}

fn zen_snapshot_frame(output_modes: [u8; 3]) -> Vec<u8> {
    let mut frame = vec![0u8; antelope_protocol::HID_REPORT_SIZE];
    frame[..4].copy_from_slice(&0x73u32.to_le_bytes());
    let payload = antelope_protocol::SNAPSHOT_PAYLOAD_OFFSET;
    frame[payload + antelope_protocol::OFFSET_SAMPLE_RATE_CODE] = 0x02;
    frame[payload + antelope_protocol::OFFSET_CLOCK_SOURCE] = 0x01;
    frame[payload + antelope_protocol::OFFSET_SAMPLE_RATE_HZ_START
        ..payload + antelope_protocol::OFFSET_SAMPLE_RATE_HZ_END]
        .copy_from_slice(&48_000u32.to_be_bytes());
    frame[payload + antelope_protocol::OFFSET_STATUS_FLAGS_0] = 0x08;
    frame[payload + antelope_protocol::OFFSET_MONITOR_VOLUME] = 0x20;
    frame[payload + antelope_protocol::OFFSET_MONITOR_MODE] = output_modes[0];
    frame[payload + antelope_protocol::OFFSET_HP1_VOLUME] = 0x20;
    frame[payload + antelope_protocol::OFFSET_HP1_MODE] = output_modes[1];
    frame[payload + antelope_protocol::OFFSET_HP2_VOLUME] = 0x20;
    frame[payload + antelope_protocol::OFFSET_HP2_MODE] = output_modes[2];
    frame[payload + antelope_protocol::OFFSET_PREAMP1_MODE] = 0x00;
    frame[payload + antelope_protocol::OFFSET_PREAMP2_MODE] = 0x00;
    frame[payload + antelope_protocol::OFFSET_SURFACE_SELECTOR] = 0x0f;
    frame
}

fn zen_controller_with_transport() -> (Controller, MockTransport) {
    let transport = MockTransport::default();
    let controller = Controller::new(
        Box::new(transport.clone()),
        Box::new(crate::device::builtin_zen_go_driver().expect("Zen Go driver")),
    )
    .expect("Zen Go controller");
    (controller, transport)
}

#[derive(Clone)]
struct FailAtWriteTransport {
    inner: MockTransport,
    fail_at: usize,
    attempts: Arc<AtomicUsize>,
}

impl Transport for FailAtWriteTransport {
    fn write(&self, data: &[u8]) -> anyhow::Result<()> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        if attempt == self.fail_at {
            return Err(anyhow::anyhow!(
                "deterministic write failure at attempt {attempt}"
            ));
        }
        self.inner.write(data)
    }

    fn read(&self, timeout: Duration) -> anyhow::Result<Option<Vec<u8>>> {
        self.inner.read(timeout)
    }

    fn is_available(&self) -> anyhow::Result<bool> {
        self.inner.is_available()
    }
}

fn zen_controller_with_failure(fail_at: usize) -> (Controller, MockTransport) {
    let transport = MockTransport::default();
    let failing = FailAtWriteTransport {
        inner: transport.clone(),
        fail_at,
        attempts: Arc::new(AtomicUsize::new(0)),
    };
    let controller = Controller::new(
        Box::new(failing),
        Box::new(crate::device::builtin_zen_go_driver().expect("Zen Go driver")),
    )
    .expect("Zen Go controller");
    (controller, transport)
}

fn observe_zen_snapshot(
    controller: &mut Controller,
    transport: &MockTransport,
    output_modes: [u8; 3],
) {
    transport.push_read(zen_snapshot_frame(output_modes));
    controller
        .poll_device_without_writes(Duration::ZERO)
        .expect("observe Zen Go snapshot");
}

fn flush_zen_mode_command(controller: &mut Controller, transport: &MockTransport) {
    controller.flush_commands().expect("flush Zen Go command");
    assert_eq!(transport.take_writes().len(), 1);
}

#[test]
fn zen_go_unknown_output_mode_is_unavailable_and_does_not_write() {
    let (mut controller, transport) = zen_controller_with_transport();
    observe_zen_snapshot(&mut controller, &transport, [0x00, 0x00, 0x00]);
    observe_zen_snapshot(&mut controller, &transport, [0x03, 0x00, 0x00]);

    assert_eq!(
        controller.state.output.states[0].mode,
        OutputMode::Unknown(0x03)
    );
    assert_eq!(controller.state.outputs()[0].muted, None);
    assert_eq!(controller.state.outputs()[0].dimmed, None);

    let error = controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect_err("unknown mode must block output mode actions");
    assert!(error.to_string().contains("unknown"));
    assert!(transport.take_writes().is_empty());
}

#[test]
fn zen_go_mute_to_dim_is_rejected_without_writes() {
    let (mut controller, transport) = zen_controller_with_transport();
    observe_zen_snapshot(&mut controller, &transport, [0x01, 0x00, 0x00]);

    let error = controller
        .apply_intent(Intent::ToggleOutputDim(0), Rect::default())
        .expect_err("direct Mute to Dim must be rejected");
    assert!(error.to_string().contains("unverified"));
    assert!(transport.take_writes().is_empty());
}

#[test]
fn zen_go_dim_to_mute_is_rejected_without_writes() {
    let (mut controller, transport) = zen_controller_with_transport();
    observe_zen_snapshot(&mut controller, &transport, [0x02, 0x00, 0x00]);

    let error = controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect_err("direct Dim to Mute must be rejected");
    assert!(error.to_string().contains("unverified"));
    assert!(transport.take_writes().is_empty());
}

#[test]
fn zen_go_pending_return_to_normal_does_not_allow_dim_bypass() {
    let (mut controller, transport) = zen_controller_with_transport();
    observe_zen_snapshot(&mut controller, &transport, [0x01, 0x00, 0x00]);

    controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect("queue return to Normal");
    assert!(transport.take_writes().is_empty());
    assert_eq!(controller.state.outputs()[0].muted, Some(false));

    let error = controller
        .apply_intent(Intent::ToggleOutputDim(0), Rect::default())
        .expect_err("pending Normal must not be bypassed");
    assert!(error.to_string().contains("awaiting state readback"));
    assert!(transport.take_writes().is_empty());

    flush_zen_mode_command(&mut controller, &transport);
    // The first snapshot can still carry the previous mode. It must not
    // release the guard merely because a snapshot arrived.
    observe_zen_snapshot(&mut controller, &transport, [0x01, 0x00, 0x00]);
    let error = controller
        .apply_intent(Intent::ToggleOutputDim(0), Rect::default())
        .expect_err("stale Mute readback must keep the guard active");
    assert!(error.to_string().contains("awaiting state readback"));
    assert!(transport.take_writes().is_empty());

    observe_zen_snapshot(&mut controller, &transport, [0x00, 0x00, 0x00]);
    controller
        .apply_intent(Intent::ToggleOutputDim(0), Rect::default())
        .expect("observed Normal permits isolated Dim");
    flush_zen_mode_command(&mut controller, &transport);
}

#[test]
fn zen_go_isolated_output_mode_transitions_remain_available() {
    let (mut controller, transport) = zen_controller_with_transport();
    observe_zen_snapshot(&mut controller, &transport, [0x00, 0x00, 0x00]);

    controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect("Normal to Mute remains available");
    flush_zen_mode_command(&mut controller, &transport);
    observe_zen_snapshot(&mut controller, &transport, [0x01, 0x00, 0x00]);

    controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect("Mute to observed Normal remains available");
    flush_zen_mode_command(&mut controller, &transport);
    observe_zen_snapshot(&mut controller, &transport, [0x00, 0x00, 0x00]);

    controller
        .apply_intent(Intent::ToggleOutputDim(0), Rect::default())
        .expect("Normal to Dim remains available");
    flush_zen_mode_command(&mut controller, &transport);
    observe_zen_snapshot(&mut controller, &transport, [0x02, 0x00, 0x00]);

    controller
        .apply_intent(Intent::ToggleOutputDim(0), Rect::default())
        .expect("Dim to observed Normal remains available");
    flush_zen_mode_command(&mut controller, &transport);
}

#[test]
fn zen_go_failed_flush_removes_only_provably_unsent_mode_guard() {
    let (mut controller, transport) = zen_controller_with_failure(0);
    observe_zen_snapshot(&mut controller, &transport, [0x00, 0x00, 0x00]);

    controller
        .send(
            Action::SetOutput {
                address: OutputAddress { id: 0 },
                control: OutputControl::Level,
                value: ControlValue::Int(19),
            },
            None,
        )
        .expect("queue preceding output write");
    controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect("queue mode write after preceding output write");
    assert_eq!(controller.state.outputs()[0].muted, Some(true));

    assert!(controller.flush_commands().is_err());
    assert!(transport.take_writes().is_empty());
    assert_eq!(controller.state.outputs()[0].muted, None);
    assert_eq!(controller.state.outputs()[0].dimmed, None);
    assert_eq!(
        controller.state.output.states[0].mode,
        OutputMode::Unknown(u8::MAX)
    );

    let error = controller
        .apply_intent(Intent::ToggleOutputDim(0), Rect::default())
        .expect_err("failed recovery must require a known snapshot");
    assert!(error.to_string().contains("unknown"));

    observe_zen_snapshot(&mut controller, &transport, [0x00, 0x00, 0x00]);
    controller
        .apply_intent(Intent::ToggleOutputDim(0), Rect::default())
        .expect("fresh Normal readback permits isolated mode retry");
    controller
        .flush_commands()
        .expect("retry after unsent mode recovery");
    assert_eq!(transport.take_writes().len(), 1);
}

#[test]
fn zen_go_uncertain_mode_write_stays_locked_through_all_snapshot_modes() {
    let (mut controller, transport) = zen_controller_with_failure(1);
    observe_zen_snapshot(&mut controller, &transport, [0x00, 0x00, 0x00]);

    controller
        .send(
            Action::SetOutput {
                address: OutputAddress { id: 0 },
                control: OutputControl::Level,
                value: ControlValue::Int(19),
            },
            None,
        )
        .expect("queue preceding output write");
    controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect("queue uncertain Mute write after preceding output write");
    assert!(controller.flush_commands().is_err());
    assert_eq!(transport.take_writes().len(), 1);
    assert_eq!(controller.state.outputs()[0].muted, None);
    assert_eq!(controller.state.outputs()[0].dimmed, None);
    assert_eq!(
        controller.state.output.states[0].mode,
        OutputMode::Unknown(u8::MAX)
    );

    for output_modes in [[0x00, 0x00, 0x00], [0x01, 0x00, 0x00], [0x03, 0x00, 0x00]] {
        observe_zen_snapshot(&mut controller, &transport, output_modes);
        let error = controller
            .apply_intent(Intent::ToggleOutputDim(0), Rect::default())
            .expect_err("uncertain mode must remain locked for this session");
        assert!(error.to_string().contains("delivery is uncertain"));
        assert!(error.to_string().contains("disabled for this session"));
        assert!(transport.take_writes().is_empty());
    }
}

#[test]
fn zen_go_successful_mode_readback_releases_with_unrelated_queue() {
    let (mut controller, transport) = zen_controller_with_transport();
    observe_zen_snapshot(&mut controller, &transport, [0x00, 0x00, 0x00]);

    controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect("queue Mute mode write");
    flush_zen_mode_command(&mut controller, &transport);

    controller
        .send(
            Action::SetOutput {
                address: OutputAddress { id: 1 },
                control: OutputControl::Level,
                value: ControlValue::Int(19),
            },
            None,
        )
        .expect("queue unrelated level write");
    observe_zen_snapshot(&mut controller, &transport, [0x01, 0x00, 0x00]);

    controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect("expected Mute readback releases the successful mode guard");
    controller
        .flush_commands()
        .expect("flush unrelated level and mode writes");
    assert_eq!(transport.take_writes().len(), 2);
}

#[test]
fn zen_go_same_output_queued_mode_stays_guarded_on_snapshot() {
    let (mut controller, transport) = zen_controller_with_transport();
    observe_zen_snapshot(&mut controller, &transport, [0x00, 0x00, 0x00]);

    controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect("queue Mute mode write");
    observe_zen_snapshot(&mut controller, &transport, [0x01, 0x00, 0x00]);

    let error = controller
        .apply_intent(Intent::ToggleOutputMute(0), Rect::default())
        .expect_err("same-output queued mode must not be released by readback");
    assert!(error.to_string().contains("awaiting state readback"));
    assert!(transport.take_writes().is_empty());
}

#[test]
fn dynamic_state_visible_bounds_are_safe() {
    let mut empty = AppState::from_profile(&zen_go_profile());
    empty.mixer.surfaces.clear();
    assert_eq!(empty.active_mixer_surface(), None);
    assert_eq!(empty.visible_mixer_strip_bounds(), 0..0);

    let mut state = AppState::from_profile(&zen_go_profile());
    state.mixer.strip_scroll = usize::MAX;
    state.mixer.visible_strip_count = usize::MAX;
    assert_eq!(state.visible_mixer_strip_bounds(), 15..16);
}

#[test]
fn dynamic_state_malformed_addresses_and_pending_mutations_do_not_panic() {
    let mut state = AppState::from_profile(&zen_go_profile());
    assert!(catch_unwind(AssertUnwindSafe(|| {
        assert!(state
            .complete_mixer_action(
                MixerAddress {
                    surface: u8::MAX,
                    strip: u16::MAX,
                },
                |_| {}
            )
            .is_none());
        let mut controller = controller_for_profile(orion_entry());
        controller.pending_mutation = Some(PendingMutation::MixerLevel {
            mixer: antelope_protocol::MixerSurface::Mix1,
            channel: u8::MAX,
            level: 1,
            pan: antelope_protocol::PanState::center(),
            muted: false,
        });
        controller.confirm_pending_write();
    }))
    .is_ok());
}

#[allow(dead_code)]
fn _normalized_records_compile(
    _globals: Vec<DynamicGlobalState>,
    _inputs: Vec<DynamicInputState>,
    _outputs: Vec<DynamicOutputState>,
    _address: InputAddress,
    _control: GlobalControl,
) {
    let _ = controller_for_profile;
}
