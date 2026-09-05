use antelope_protocol::load_profile_pack;

use crate::device::{
    select_candidate, select_reconnect_candidate, CandidateStatus, DeviceCandidate,
    DevicePickerState, DeviceSelection, DeviceSession, ProfileCatalog, SelectionMatch,
};

fn candidate(path: &[u8], pid: u16, serial: Option<&str>) -> DeviceCandidate {
    DeviceCandidate::new_with_path_bytes(
        path.to_vec(),
        0x23e5,
        pid,
        serial.map(str::to_owned),
        None,
        0,
        0,
        3,
    )
}

#[test]
fn picker_sorts_promoted_supported_orion_before_zen_go_by_profile_name() {
    let catalog = ProfileCatalog::builtin();
    let picker = DevicePickerState::new(
        vec![
            candidate(b"orion", 0xa221, Some("ORION-1")),
            candidate(b"zen-go", 0xa015, Some("ZEN-1")),
        ],
        &catalog,
    );
    assert_eq!(picker.entries()[0].status, CandidateStatus::Supported);
    assert_eq!(picker.entries()[0].candidate.path_bytes, b"orion");
    assert_eq!(picker.entries()[1].status, CandidateStatus::Supported);
    assert_eq!(picker.entries()[1].candidate.path_bytes, b"zen-go");
}

#[test]
fn non_control_peer_does_not_make_control_candidate_ambiguous() {
    let catalog = ProfileCatalog::builtin();
    let control = candidate(b"zen-control", 0xa015, Some("ZEN-1"));
    let mut non_control = candidate(b"zen-peer", 0xa015, Some("ZEN-1"));
    non_control.interface_number = 2;

    let picker = DevicePickerState::new(vec![non_control, control], &catalog);

    assert_eq!(picker.entries()[0].candidate.path_bytes, b"zen-control");
    assert!(picker.entries()[0].is_selectable());
}

#[test]
fn active_picker_marks_same_model_serial_and_selects_it() {
    let catalog = ProfileCatalog::builtin();
    let active = candidate(b"zen-active", 0xa015, Some("ZEN-A"));
    let other = candidate(b"zen-other", 0xa015, Some("ZEN-B"));
    let same_serial_other_model = candidate(b"orion", 0xa221, Some("ZEN-A"));
    let mut picker = DevicePickerState::new(
        vec![other, same_serial_other_model, active.clone()],
        &catalog,
    );

    picker.set_active_candidate(Some(active.clone()));

    assert_eq!(picker.selected_candidate(), Some(&active));
    assert!(picker.is_active(&active));
    assert_eq!(
        picker
            .entries()
            .iter()
            .filter(|entry| picker.is_active(&entry.candidate))
            .count(),
        1
    );
    assert!(picker
        .entries()
        .iter()
        .any(|entry| entry.candidate.serial() == Some("ZEN-B")
            && !picker.is_active(&entry.candidate)));
}

#[test]
fn missing_serial_uses_exact_hid_path_for_active_identity() {
    let catalog = ProfileCatalog::builtin();
    let active = candidate(b"hid-active", 0xa015, None);
    let same_path = candidate(b"hid-active", 0xa015, Some("now-known"));
    let different_path = candidate(b"hid-other", 0xa015, None);
    let mut picker = DevicePickerState::new(vec![same_path, different_path], &catalog);

    picker.set_active_candidate(Some(active));

    assert!(picker.is_active(&picker.entries()[0].candidate));
    assert!(!picker.is_active(&picker.entries()[1].candidate));
}

#[test]
fn promoted_orion_picker_entry_supports_activation() {
    let catalog = ProfileCatalog::builtin();
    let picker =
        DevicePickerState::new(vec![candidate(b"orion", 0xa221, Some("ORION-1"))], &catalog);
    assert_eq!(picker.entries()[0].status, CandidateStatus::Supported);
    assert_eq!(
        picker.activate_selected().map(|c| c.path_bytes.as_slice()),
        Some(b"orion".as_slice())
    );
    assert_eq!(
        picker.activate_row(0).map(|c| c.path_bytes.as_slice()),
        Some(b"orion".as_slice())
    );
}

#[test]
fn empty_picker_navigation_and_activation_are_safe() {
    let catalog = ProfileCatalog::builtin();
    let mut picker = DevicePickerState::new(Vec::new(), &catalog);
    picker.select_next();
    picker.select_previous();
    assert!(picker.activate_selected().is_none());
    assert!(picker.entries().is_empty());
}

#[test]
fn partial_profile_is_rejected_before_hid_open() {
    let catalog = ProfileCatalog::builtin();
    let candidate = candidate(b"this-path-must-never-open", 0xa2b5, Some("DISCRETE-8-1"));

    let error = match DeviceSession::open_candidate(&candidate, &catalog) {
        Err(error) => error,
        Ok(_) => panic!("partial candidate must fail before HIDAPI construction"),
    };

    assert!(error.to_string().contains("partial"), "{error:#}");
}

#[test]
fn mock_session_constructs_zen_go_driver_without_discovery() {
    let session = DeviceSession::open_mock().expect("mock session");
    assert_eq!(session.driver_definition().pid, 0xa015);
    assert_eq!(session.device_name(), "Antelope Zen Go Synergy Core");
}

#[test]
fn replacement_session_starts_with_isolated_device_state() {
    let mut old = DeviceSession::open_mock().expect("old mock session");
    old.controller_mut().state.ui.last_message = "stale old-device state".into();

    let replacement =
        crate::device::replace_session(old, DeviceSession::open_mock).expect("replacement session");

    assert_ne!(
        replacement.controller().state.ui.last_message,
        "stale old-device state"
    );
    assert!(replacement
        .controller()
        .state
        .device
        .status
        .metadata
        .is_none());
}

#[test]
fn reconnect_rejects_reused_exact_path_with_changed_unit_identity() {
    let catalog = ProfileCatalog::builtin();
    let previous = candidate(b"reused-path", 0xa015, Some("ZEN-OLD"));
    let replacement = candidate(b"reused-path", 0xa015, Some("ZEN-NEW"));
    let picker = DevicePickerState::new(vec![replacement], &catalog);

    assert!(select_reconnect_candidate(&previous, picker.entries()).is_none());
}

#[test]
fn reconnect_rejects_exact_path_when_both_serials_are_empty() {
    let catalog = ProfileCatalog::builtin();
    let previous = candidate(b"empty-serial-path", 0xa015, Some(""));
    let replacement = candidate(b"empty-serial-path", 0xa015, Some(""));
    let picker = DevicePickerState::new(vec![replacement], &catalog);

    assert!(select_reconnect_candidate(&previous, picker.entries()).is_none());
}

#[test]
fn reconnect_rejects_changed_path_when_both_serials_are_empty() {
    let catalog = ProfileCatalog::builtin();
    let previous = candidate(b"old-empty-path", 0xa015, Some(""));
    let replacement = candidate(b"new-empty-path", 0xa015, Some(""));
    let picker = DevicePickerState::new(vec![replacement], &catalog);

    assert!(select_reconnect_candidate(&previous, picker.entries()).is_none());
}

#[test]
fn reconnect_rejects_exact_path_when_serial_is_absent() {
    let catalog = ProfileCatalog::builtin();
    let previous = candidate(b"absent-serial-path", 0xa015, None);
    let replacement = candidate(b"absent-serial-path", 0xa015, None);
    let picker = DevicePickerState::new(vec![replacement], &catalog);

    assert!(select_reconnect_candidate(&previous, picker.entries()).is_none());
}

#[test]
fn reconnect_accepts_unique_changed_path_with_matching_nonempty_identity() {
    let catalog = ProfileCatalog::builtin();
    let previous = candidate(b"old-stable-path", 0xa015, Some("ZEN-1"));
    let replacement = candidate(b"new-stable-path", 0xa015, Some("ZEN-1"));
    let picker = DevicePickerState::new(vec![replacement], &catalog);

    let replacement = select_reconnect_candidate(&previous, picker.entries())
        .expect("unique changed path with matching serial");
    assert_eq!(replacement.path_bytes, b"new-stable-path");
}

#[test]
fn reconnect_accepts_exact_path_with_unchanged_unit_identity() {
    let catalog = ProfileCatalog::builtin();
    let previous = candidate(b"stable-path", 0xa015, Some("ZEN-1"));
    let picker = DevicePickerState::new(vec![previous.clone()], &catalog);

    let replacement =
        select_reconnect_candidate(&previous, picker.entries()).expect("unchanged exact identity");
    assert_eq!(replacement.path_bytes, b"stable-path");
    assert_eq!(replacement.serial(), Some("ZEN-1"));
}

#[test]
fn explicit_serial_selector_handles_hid_and_hex_shaped_values() {
    let hid_serial = candidate(b"path-a", 0xa015, Some("hid-42"));
    let hex_serial = candidate(b"path-b", 0xa015, Some("23e5:a015"));

    let hid_selection = DeviceSelection::parse("serial:hid-42").expect("explicit serial");
    let hex_selection = DeviceSelection::parse("serial:23e5:a015").expect("hex-shaped serial");

    assert_eq!(
        hid_selection.match_candidate(&hid_serial),
        Some(SelectionMatch::Serial)
    );
    assert_eq!(
        hex_selection.match_candidate(&hex_serial),
        Some(SelectionMatch::Serial)
    );
}

#[test]
fn explicit_path_selector_and_empty_prefix_validation_are_unambiguous() {
    let path_candidate = candidate(b"hid-42", 0xa015, Some("different-serial"));
    let selection = DeviceSelection::parse("path:hid-42").expect("explicit path");

    assert_eq!(
        selection.match_candidate(&path_candidate),
        Some(SelectionMatch::ExactPath)
    );
    assert!(DeviceSelection::parse("serial:").is_err());
    assert!(DeviceSelection::parse("path:").is_err());
}

#[test]
fn runtime_device_selection_prefers_exact_raw_path() {
    let candidate = candidate(&[b'h', b'i', b'd', 0xff], 0xa015, Some("ZEN-1"));
    let selection = DeviceSelection::path_bytes(vec![b'h', b'i', b'd', 0xff]);
    assert_eq!(
        selection.match_candidate(&candidate),
        Some(SelectionMatch::ExactPath)
    );
}

#[test]
fn runtime_device_selection_rejects_duplicate_serial_with_paths() {
    let candidates = vec![
        candidate(b"hid-a", 0xa015, Some("DUPLICATE")),
        candidate(b"hid-b", 0xa015, Some("DUPLICATE")),
    ];
    let selection = DeviceSelection::parse("DUPLICATE").expect("serial selection");
    let message = select_candidate(&candidates, &selection)
        .expect_err("must be ambiguous")
        .to_string();
    assert!(
        message.contains("ambiguous") && message.contains("hid-a") && message.contains("hid-b")
    );
}

#[test]
fn runtime_device_selection_rejects_duplicate_identity_with_paths() {
    let candidates = vec![
        candidate(b"hid-a", 0xa015, Some("ZEN-1")),
        candidate(b"hid-b", 0xa015, Some("ZEN-2")),
    ];
    let selection = DeviceSelection::parse("23e5:a015").expect("identity selection");
    let message = select_candidate(&candidates, &selection)
        .expect_err("must be ambiguous")
        .to_string();
    assert!(
        message.contains("ambiguous") && message.contains("hid-a") && message.contains("hid-b")
    );
}

#[test]
fn external_runtime_catalog_entry_is_classified_without_static_catalog() {
    let mut catalog = ProfileCatalog::builtin();
    let mut pack = load_profile_pack(include_bytes!(
        "../../antelope-protocol/tests/fixtures/profile_pack_v1.json"
    ))
    .expect("valid generic fixture");
    let entry = pack.profiles.first_mut().expect("fixture entry");
    entry.id = "external-safe-profile".into();
    entry.profile.identity.name = "External Safe Profile".into();
    entry.profile.identity.pid = 0xe001;
    catalog.add_external(pack).expect("valid external profile");
    let picker = DevicePickerState::new(
        vec![candidate(b"external", 0xe001, Some("EXT-1"))],
        &catalog,
    );
    assert!(picker.entries()[0].status.is_selectable());
    assert_eq!(picker.entries()[0].profile_name, "External Safe Profile");
}
