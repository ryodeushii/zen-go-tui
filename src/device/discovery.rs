//! Safe, read-only HID discovery for Antelope hardware.
//!
//! Discovery owns only copied metadata from hidapi.  Classification and
//! sorting stay pure so callers can test device selection without opening a
//! HID handle or requiring hardware.

use std::fmt;

use hidapi::{DeviceInfo, HidApi};

use super::{DeviceEntry, Readiness};

/// USB vendor ID registered to Antelope Audio.
pub const ANTELOPE_VID: u16 = 0x23e5;

/// Render HID path bytes without replacing invalid UTF-8.
pub(crate) fn path_context(path_bytes: &[u8]) -> String {
    path_bytes.escape_ascii().to_string()
}

/// Metadata copied from one HID interface during enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCandidate {
    /// HIDAPI device path rendered for UI and diagnostics.
    pub path: String,
    /// Exact HIDAPI path bytes.  Never reconstruct an open path from `path`.
    pub path_bytes: Vec<u8>,
    pub vid: u16,
    pub pid: u16,
    pub serial_number: Option<String>,
    pub product_string: Option<String>,
    pub usage_page: u16,
    pub usage: u16,
    pub interface_number: i32,
}

impl DeviceCandidate {
    /// Construct a candidate from copied HID metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        path: impl Into<String>,
        vid: u16,
        pid: u16,
        serial_number: Option<String>,
        product_string: Option<String>,
        usage_page: u16,
        usage: u16,
        interface_number: i32,
    ) -> Self {
        let path = path.into();
        Self::new_with_path_bytes(
            path.into_bytes(),
            vid,
            pid,
            serial_number,
            product_string,
            usage_page,
            usage,
            interface_number,
        )
    }

    /// Construct a candidate while retaining HIDAPI's exact path bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_path_bytes(
        path_bytes: Vec<u8>,
        vid: u16,
        pid: u16,
        serial_number: Option<String>,
        product_string: Option<String>,
        usage_page: u16,
        usage: u16,
        interface_number: i32,
    ) -> Self {
        Self {
            path: String::from_utf8_lossy(&path_bytes).into_owned(),
            path_bytes,
            vid,
            pid,
            serial_number,
            product_string,
            usage_page,
            usage,
            interface_number,
        }
    }

    /// Serial-number alias matching hidapi's terminology in display code.
    pub fn serial(&self) -> Option<&str> {
        self.serial_number.as_deref()
    }

    /// Product-string alias matching hidapi's terminology in display code.
    pub fn product(&self) -> Option<&str> {
        self.product_string.as_deref()
    }

    /// Classify this metadata record against a generated catalog.
    pub fn classification<'a>(&self, catalog: &'a [DeviceEntry]) -> CandidateClassification<'a> {
        classify_candidate_details(self, catalog)
    }

    pub(crate) fn from_device_info(device: &DeviceInfo) -> Self {
        Self::new_with_path_bytes(
            device.path().to_bytes().to_vec(),
            device.vendor_id(),
            device.product_id(),
            device.serial_number().map(str::to_owned),
            device.product_string().map(str::to_owned),
            device.usage_page(),
            device.usage(),
            device.interface_number(),
        )
    }
}

/// Readiness classification for one candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateStatus {
    /// Profile has validated runtime support and may be opened.
    Supported,
    /// Profile is known but only partially safe/implemented.
    Partial,
    /// Profile exists but has not been verified for normal control.
    Unverified,
    /// Profile is known and deliberately disabled.
    Disabled,
    /// Candidate identity is ambiguous and must not be opened.
    Ambiguous,
    /// No known Antelope profile matches this VID/PID.
    Unsupported,
}

impl From<Readiness> for CandidateStatus {
    fn from(readiness: Readiness) -> Self {
        match readiness {
            Readiness::Supported => Self::Supported,
            Readiness::Partial => Self::Partial,
            Readiness::Unverified => Self::Unverified,
            Readiness::Disabled => Self::Disabled,
        }
    }
}

impl CandidateStatus {
    /// Whether selecting this candidate may create a normal transport.
    pub const fn is_selectable(self) -> bool {
        matches!(self, Self::Supported)
    }

    /// Catalog readiness represented by this status, when identity is not
    /// ambiguous or unknown.
    pub const fn readiness(self) -> Option<Readiness> {
        match self {
            Self::Supported => Some(Readiness::Supported),
            Self::Partial => Some(Readiness::Partial),
            Self::Unverified => Some(Readiness::Unverified),
            Self::Disabled => Some(Readiness::Disabled),
            Self::Ambiguous | Self::Unsupported => None,
        }
    }

    /// Stable sort priority.  Lower values appear first.
    pub const fn sort_rank(self) -> u8 {
        match self {
            Self::Supported => 0,
            Self::Partial => 1,
            Self::Unverified => 2,
            Self::Disabled => 3,
            Self::Ambiguous => 4,
            Self::Unsupported => 5,
        }
    }
}

impl fmt::Display for CandidateStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Supported => "supported",
            Self::Partial => "partial",
            Self::Unverified => "unverified",
            Self::Disabled => "disabled",
            Self::Ambiguous => "ambiguous",
            Self::Unsupported => "unsupported",
        };
        f.write_str(label)
    }
}

/// Classification result with profile association and human-readable safety
/// diagnostics.  `classify_candidate` is the compact status-only API; this
/// result is used when a picker needs to explain why a candidate is disabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateClassification<'a> {
    pub status: CandidateStatus,
    pub profile: Option<&'a DeviceEntry>,
    pub diagnostics: Vec<String>,
}

impl<'a> CandidateClassification<'a> {
    pub const fn is_selectable(&self) -> bool {
        self.status.is_selectable()
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostics.first().map(String::as_str)
    }
}

/// Enumerate all Antelope HID interfaces without opening any device.
///
/// `HidApi::device_list` is intentionally filtered only by vendor ID here:
/// unknown product IDs remain visible to the caller as unsupported candidates.
/// Results are sorted for presentation, while retaining every interface.
pub fn enumerate_antelope_devices(api: &HidApi, catalog: &[DeviceEntry]) -> Vec<DeviceCandidate> {
    let mut candidates: Vec<_> = api
        .device_list()
        .filter(|device| device.vendor_id() == ANTELOPE_VID)
        .map(DeviceCandidate::from_device_info)
        .collect();

    sort_candidates(&mut candidates, catalog);
    candidates
}

/// Classify one candidate against generated profile data.
///
/// This function does not inspect HID internals and never opens a device.
pub fn classify_candidate(candidate: &DeviceCandidate, catalog: &[DeviceEntry]) -> CandidateStatus {
    classify_candidate_details(candidate, catalog).status
}

/// Return status, matching profile, and diagnostics for one candidate.
pub fn classify_candidate_details<'a>(
    candidate: &DeviceCandidate,
    catalog: &'a [DeviceEntry],
) -> CandidateClassification<'a> {
    let Some(profile) = catalog
        .iter()
        .find(|entry| entry.vid() == candidate.vid && entry.pid() == candidate.pid)
    else {
        return CandidateClassification {
            status: CandidateStatus::Unsupported,
            profile: None,
            diagnostics: vec![format!(
                "unsupported Antelope HID product {:04x}:{:04x}",
                candidate.vid, candidate.pid
            )],
        };
    };

    let (status, diagnostic) = match profile.readiness {
        Readiness::Supported => (CandidateStatus::Supported, None),
        Readiness::Partial => (
            CandidateStatus::Partial,
            Some(format!(
                "{} is partially supported and cannot be controlled safely",
                profile.definition.identity.name
            )),
        ),
        Readiness::Unverified => (
            CandidateStatus::Unverified,
            Some(format!(
                "{} is unverified and cannot be controlled safely",
                profile.definition.identity.name
            )),
        ),
        Readiness::Disabled => (
            CandidateStatus::Disabled,
            Some(format!(
                "{} is disabled until runtime support is validated",
                profile.definition.identity.name
            )),
        ),
    };

    let mut diagnostics = diagnostic.into_iter().collect::<Vec<_>>();
    if candidate.interface_number < 0 {
        diagnostics.push("ambiguous HID interface: interface number unavailable".to_string());
        return CandidateClassification {
            status: CandidateStatus::Ambiguous,
            profile: Some(profile),
            diagnostics,
        };
    }

    let transport = &profile.definition.transport;
    if let Some(expected) = transport.expected_interface_number {
        if candidate.interface_number != expected {
            diagnostics.push(format!(
                "expected control interface {}, found {}",
                expected, candidate.interface_number
            ));
            return CandidateClassification {
                status: CandidateStatus::Ambiguous,
                profile: Some(profile),
                diagnostics,
            };
        }
    }
    if let Some(expected) = transport.expected_usage_page {
        if candidate.usage_page != expected {
            diagnostics.push(format!(
                "expected control usage page 0x{:04x}, found 0x{:04x}",
                expected, candidate.usage_page
            ));
            return CandidateClassification {
                status: CandidateStatus::Ambiguous,
                profile: Some(profile),
                diagnostics,
            };
        }
    }
    if let Some(expected) = transport.expected_usage {
        if candidate.usage != expected {
            diagnostics.push(format!(
                "expected control usage 0x{:04x}, found 0x{:04x}",
                expected, candidate.usage
            ));
            return CandidateClassification {
                status: CandidateStatus::Ambiguous,
                profile: Some(profile),
                diagnostics,
            };
        }
    }

    CandidateClassification {
        status,
        profile: Some(profile),
        diagnostics,
    }
}

/// Classify all candidates and add duplicate/ambiguous-interface diagnostics.
///
/// Candidates sharing a VID/PID and serial are treated as interfaces of one
/// physical unit.  When serial metadata is unavailable, all matching
/// VID/PIDs are grouped conservatively: selecting one would otherwise risk
/// reconnecting to a different identical unit.
pub fn classify_candidates<'a>(
    candidates: &[DeviceCandidate],
    catalog: &'a [DeviceEntry],
) -> Vec<CandidateClassification<'a>> {
    let mut classifications: Vec<_> = candidates
        .iter()
        .map(|candidate| classify_candidate_details(candidate, catalog))
        .collect();

    for (index, candidate) in candidates.iter().enumerate() {
        if candidate.interface_number < 0 {
            classifications[index].status = CandidateStatus::Ambiguous;
        }
    }

    for index in 0..candidates.len() {
        let Some(profile) = classifications[index].profile else {
            continue;
        };
        let same_product = candidates
            .iter()
            .filter(|peer| {
                peer.vid == candidates[index].vid
                    && peer.pid == candidates[index].pid
                    && matches_profile_transport(peer, profile)
            })
            .collect::<Vec<_>>();
        let mixed_serials = same_product.len() > 1
            && same_product.iter().any(|peer| peer.serial_number.is_none())
            && same_product.iter().any(|peer| peer.serial_number.is_some());
        if mixed_serials && matches_profile_transport(&candidates[index], profile) {
            classifications[index].status = CandidateStatus::Ambiguous;
            classifications[index].diagnostics.push(format!(
                "ambiguous HID identity for {:04x}:{:04x}: mixed serial metadata; identity fallback is disabled",
                candidates[index].vid, candidates[index].pid
            ));
            continue;
        }

        let peers: Vec<_> = candidates
            .iter()
            .enumerate()
            .filter(|(peer_index, peer)| {
                *peer_index != index
                    && matches_profile_transport(peer, profile)
                    && same_identity(&candidates[index], peer)
            })
            .collect();

        if peers.is_empty() {
            continue;
        }

        classifications[index].status = CandidateStatus::Ambiguous;
        let paths = peers
            .iter()
            .map(|(_, peer)| {
                format!(
                    "{} (interface {})",
                    path_context(&peer.path_bytes),
                    peer.interface_number
                )
            })
            .chain(std::iter::once(format!(
                "{} (interface {})",
                path_context(&candidates[index].path_bytes),
                candidates[index].interface_number
            )))
            .collect::<Vec<_>>();
        classifications[index].diagnostics.push(format!(
            "ambiguous HID interfaces for {:04x}:{:04x}; matching paths: {}",
            candidates[index].vid,
            candidates[index].pid,
            paths.join(", ")
        ));
    }

    classifications
}

/// Stable supported-first ordering for candidate presentation.
pub fn sort_candidates(candidates: &mut [DeviceCandidate], catalog: &[DeviceEntry]) {
    let statuses = classify_candidates(candidates, catalog)
        .into_iter()
        .map(|classification| classification.status)
        .collect::<Vec<_>>();
    let mut ordered = candidates.iter().cloned().enumerate().collect::<Vec<_>>();

    ordered.sort_by(|(left_index, left), (right_index, right)| {
        statuses[*left_index]
            .sort_rank()
            .cmp(&statuses[*right_index].sort_rank())
            .then_with(|| candidate_name(left, catalog).cmp(&candidate_name(right, catalog)))
            .then_with(|| left.path_bytes.cmp(&right.path_bytes))
    });

    for (slot, (_, candidate)) in ordered.into_iter().enumerate() {
        candidates[slot] = candidate;
    }
}

fn candidate_name<'a>(candidate: &'a DeviceCandidate, catalog: &'a [DeviceEntry]) -> &'a str {
    candidate.product_string.as_deref().unwrap_or_else(|| {
        catalog
            .iter()
            .find(|entry| entry.vid() == candidate.vid && entry.pid() == candidate.pid)
            .map(|entry| entry.definition.identity.name)
            .unwrap_or("")
    })
}

fn matches_profile_transport(candidate: &DeviceCandidate, profile: &DeviceEntry) -> bool {
    if candidate.interface_number < 0 {
        return false;
    }

    let transport = &profile.definition.transport;
    transport
        .expected_interface_number
        .map_or(true, |expected| candidate.interface_number == expected)
        && transport
            .expected_usage_page
            .map_or(true, |expected| candidate.usage_page == expected)
        && transport
            .expected_usage
            .map_or(true, |expected| candidate.usage == expected)
}

fn same_identity(left: &DeviceCandidate, right: &DeviceCandidate) -> bool {
    if left.vid != right.vid || left.pid != right.pid {
        return false;
    }

    match (&left.serial_number, &right.serial_number) {
        (Some(left), Some(right)) => left == right,
        // Without serial metadata there is no safe way to distinguish two
        // identical units or two HID interfaces of one unit.
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_candidate, classify_candidates, sort_candidates, CandidateStatus, DeviceCandidate,
    };
    use crate::device::DEVICE_CATALOG;

    fn candidate(path: &str, pid: u16) -> DeviceCandidate {
        candidate_with_serial(path, pid, Some("unit-1"))
    }

    fn candidate_with_serial(path: &str, pid: u16, serial: Option<&str>) -> DeviceCandidate {
        DeviceCandidate::new(
            path,
            0x23e5,
            pid,
            serial.map(str::to_owned),
            Some("Antelope".to_string()),
            0xffa0,
            0x0003,
            3,
        )
    }

    #[test]
    fn candidate_keeps_lossy_display_path_and_raw_path_bytes() {
        let raw_path = b"/dev/hidraw-\xff".to_vec();
        let candidate = DeviceCandidate::new_with_path_bytes(
            raw_path.clone(),
            0x23e5,
            0xa015,
            Some("unit-1".to_string()),
            None,
            0xffa0,
            0x0003,
            3,
        );

        assert_eq!(candidate.path, "/dev/hidraw-\u{fffd}");
        assert_eq!(candidate.path_bytes, raw_path);
    }

    #[test]
    fn candidate_preserves_hid_metadata() {
        let candidate = candidate("/dev/hidraw-antelope", 0xa015);

        assert_eq!(candidate.path, "/dev/hidraw-antelope");
        assert_eq!(candidate.vid, 0x23e5);
        assert_eq!(candidate.pid, 0xa015);
        assert_eq!(candidate.serial_number.as_deref(), Some("unit-1"));
        assert_eq!(candidate.product_string.as_deref(), Some("Antelope"));
        assert_eq!(candidate.usage_page, 0xffa0);
        assert_eq!(candidate.usage, 0x0003);
        assert_eq!(candidate.interface_number, 3);
    }

    #[test]
    fn known_supported_profile_is_classified_supported() {
        let status = classify_candidate(&candidate("/dev/hidraw0", 0xa015), DEVICE_CATALOG);

        assert_eq!(status, CandidateStatus::Supported);
    }

    #[test]
    fn partial_profile_is_classified_partial() {
        let status = classify_candidate(&candidate("/dev/hidraw1", 0xa2b5), DEVICE_CATALOG);

        assert_eq!(status, CandidateStatus::Partial);
    }

    #[test]
    fn unverified_profile_is_classified_unverified() {
        let status = classify_candidate(&candidate("/dev/hidraw2", 0xa2be), DEVICE_CATALOG);

        assert_eq!(status, CandidateStatus::Unverified);
    }

    #[test]
    fn disabled_profile_is_classified_disabled_with_reason() {
        let classification =
            super::classify_candidate_details(&candidate("/dev/hidraw3", 0xa221), DEVICE_CATALOG);

        assert_eq!(classification.status, CandidateStatus::Disabled);
        assert!(classification
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("disabled")));
    }

    #[test]
    fn unknown_antelope_pid_is_classified_unsupported() {
        let status = classify_candidate(&candidate("/dev/hidraw2", 0xbeef), DEVICE_CATALOG);

        assert_eq!(status, CandidateStatus::Unsupported);
    }

    #[test]
    fn missing_interface_number_is_classified_ambiguous() {
        let candidate = DeviceCandidate::new(
            "/dev/hidraw-missing-interface",
            0x23e5,
            0xa015,
            None,
            None,
            0xffa0,
            3,
            -1,
        );

        assert_eq!(
            classify_candidate(&candidate, DEVICE_CATALOG),
            CandidateStatus::Ambiguous
        );
    }

    #[test]
    fn sorting_keeps_supported_candidates_first_and_is_stable() {
        let mut candidates = vec![
            candidate("/dev/z-disabled", 0xa221),
            candidate_with_serial("/dev/b-supported", 0xa015, Some("unit-b")),
            candidate_with_serial("/dev/a-supported", 0xa015, Some("unit-a")),
            candidate_with_serial("/dev/duplicate", 0xa015, Some("unit-c")),
            candidate_with_serial("/dev/duplicate", 0xa015, Some("unit-d")),
        ];

        sort_candidates(&mut candidates, DEVICE_CATALOG);

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "/dev/a-supported",
                "/dev/b-supported",
                "/dev/duplicate",
                "/dev/duplicate",
                "/dev/z-disabled",
            ]
        );
    }

    #[test]
    fn duplicate_interfaces_are_reported_ambiguous() {
        let candidates = vec![
            candidate("/dev/hidraw3", 0xa015),
            candidate("/dev/hidraw4", 0xa015),
        ];
        let classifications = classify_candidates(&candidates, DEVICE_CATALOG);

        assert_eq!(classifications.len(), 2);
        assert!(classifications
            .iter()
            .all(|classification| classification.status == CandidateStatus::Ambiguous));
        assert!(classifications.iter().all(|classification| classification
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("ambiguous"))));
    }

    #[test]
    fn missing_serial_also_keeps_same_pid_candidates_ambiguous() {
        let candidates = vec![
            candidate_with_serial("/dev/hidraw5", 0xa015, None),
            candidate_with_serial("/dev/hidraw6", 0xa015, None),
        ];
        let classifications = classify_candidates(&candidates, DEVICE_CATALOG);

        assert!(classifications
            .iter()
            .all(|classification| classification.status == CandidateStatus::Ambiguous));
    }

    #[test]
    fn mixed_serial_availability_is_conservatively_ambiguous() {
        let candidates = vec![
            candidate_with_serial("/dev/hidraw-serial", 0xa015, Some("unit-1")),
            candidate_with_serial("/dev/hidraw-missing", 0xa015, None),
        ];
        let classifications = classify_candidates(&candidates, DEVICE_CATALOG);

        assert!(classifications
            .iter()
            .all(|classification| classification.status == CandidateStatus::Ambiguous));
        assert!(classifications.iter().all(|classification| classification
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("mixed serial"))));
    }

    #[test]
    fn distinct_serial_units_remain_selectable() {
        let candidates = vec![
            candidate_with_serial("/dev/hidraw-unit-a", 0xa015, Some("unit-a")),
            candidate_with_serial("/dev/hidraw-unit-b", 0xa015, Some("unit-b")),
        ];
        let classifications = classify_candidates(&candidates, DEVICE_CATALOG);

        assert!(classifications
            .iter()
            .all(|classification| classification.status == CandidateStatus::Supported));
    }

    #[test]
    fn expected_control_interface_is_not_blocked_by_non_control_peer() {
        let candidates = vec![
            candidate("/dev/hidraw-control", 0xa015),
            DeviceCandidate::new(
                "/dev/hidraw-non-control",
                0x23e5,
                0xa015,
                Some("unit-1".to_string()),
                Some("Antelope".to_string()),
                0xffa0,
                0x0003,
                4,
            ),
        ];

        let classifications = classify_candidates(&candidates, DEVICE_CATALOG);

        assert_eq!(classifications[0].status, CandidateStatus::Supported);
        assert_eq!(classifications[1].status, CandidateStatus::Ambiguous);
        assert!(classifications[1]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("expected control interface")));
    }

    #[test]
    fn expected_control_interface_mismatch_is_rejected_with_diagnostic() {
        let mut candidate = candidate("/dev/hidraw-wrong-interface", 0xa015);
        candidate.interface_number = 2;

        let classification = super::classify_candidate_details(&candidate, DEVICE_CATALOG);

        assert_eq!(classification.status, CandidateStatus::Ambiguous);
        assert!(classification
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("expected control interface")));
    }
}
