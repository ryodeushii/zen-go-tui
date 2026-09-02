use antelope_protocol::load_profile_pack;

use crate::device::{
    select_candidate, select_reconnect_candidate, DeviceCandidate, DevicePickerState,
    DeviceSelection, DeviceSession, ProfileCatalog, SelectionMatch,
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
fn picker_sorts_supported_candidates_before_disabled_candidates() {
    let catalog = ProfileCatalog::builtin();
    let picker = DevicePickerState::new(
        vec![
            candidate(b"orion", 0xa221, Some("ORION-1")),
            candidate(b"zen-go", 0xa015, Some("ZEN-1")),
        ],
        &catalog,
    );
    assert!(picker.entries()[0].status.is_selectable());
    assert_eq!(picker.entries()[0].candidate.path_bytes, b"zen-go");
    assert!(!picker.entries()[1].status.is_selectable());
    assert!(picker.entries()[1].diagnostic.contains("disabled"));
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
fn disabled_picker_entry_has_no_activation_action() {
    let catalog = ProfileCatalog::builtin();
    let picker =
        DevicePickerState::new(vec![candidate(b"orion", 0xa221, Some("ORION-1"))], &catalog);
    assert!(picker.activate_selected().is_none());
    assert!(picker.activate_row(0).is_none());
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
fn disabled_orion_is_rejected_before_hid_open() {
    let catalog = ProfileCatalog::builtin();
    let orion = candidate(b"this-path-must-never-open", 0xa221, Some("ORION-1"));

    let error = match DeviceSession::open_candidate(&orion, &catalog) {
        Err(error) => error,
        Ok(_) => panic!("disabled candidate must fail before HIDAPI construction"),
    };

    assert!(error.to_string().contains("disabled"));
}

#[test]
fn mock_session_constructs_zen_go_driver_without_discovery() {
    let session = DeviceSession::open_mock().expect("mock session");
    assert_eq!(session.driver_definition().pid, 0xa015);
    assert_eq!(session.device_name(), "Antelope Zen Go Synergy Core");
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
