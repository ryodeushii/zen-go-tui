use std::collections::VecDeque;
use std::error::Error as StdError;
use std::ffi::CString;
use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use hidapi::{DeviceInfo, HidApi, HidDevice, HidError};

use crate::device::discovery::{
    classify_candidates, enumerate_antelope_devices, path_context, CandidateClassification,
    CandidateStatus, DeviceCandidate,
};
use crate::device::{DeviceEntry, DEVICE_CATALOG};
use antelope_protocol::RuntimeEntry;

pub trait Transport: Send {
    fn write(&self, data: &[u8]) -> Result<()>;
    fn read(&self, timeout: Duration) -> Result<Option<Vec<u8>>>;

    fn is_available(&self) -> Result<bool> {
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    DeviceUnavailable,
    DeviceDisconnected,
    UnsupportedDevice,
    AmbiguousDevice,
    InvalidReport,
    ShortWrite,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceUnavailable => write!(f, "device unavailable"),
            Self::DeviceDisconnected => write!(f, "device disconnected"),
            Self::UnsupportedDevice => write!(f, "unsupported device"),
            Self::AmbiguousDevice => write!(f, "ambiguous device identity"),
            Self::InvalidReport => write!(f, "invalid HID report"),
            Self::ShortWrite => write!(f, "short HID write"),
        }
    }
}

impl StdError for TransportError {}

/// Device-unavailable/disconnected errors are retryable.  Rejections caused
/// by unsupported or ambiguous identities must reach the caller immediately;
/// treating those as transient would make the CLI retry forever.
pub fn is_device_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<TransportError>(),
            Some(TransportError::DeviceUnavailable | TransportError::DeviceDisconnected)
        ) || (cause.to_string() == TransportError::DeviceDisconnected.to_string()
            && cause
                .source()
                .and_then(|source| source.downcast_ref::<HidError>())
                .is_some_and(hid_error_indicates_disconnect))
    })
}

fn hid_error_indicates_disconnect(error: &HidError) -> bool {
    let message_indicates_disconnect = |message: &str| {
        let message = message.to_ascii_lowercase();
        [
            "device disconnected",
            "no such device",
            "device removed",
            "broken pipe",
            "connection reset",
        ]
        .iter()
        .any(|fragment| message.contains(fragment))
    };

    match error {
        HidError::HidApiError { message } => message_indicates_disconnect(message),
        HidError::IoError { error } => {
            matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::NotFound
            ) || message_indicates_disconnect(&error.to_string())
        }
        _ => false,
    }
}

/// Keep original hidapi errors in the chain.  Only explicit disconnect
/// signals become retryable transport errors; permission, malformed, and
/// unrelated I/O failures remain non-retryable HID errors.
fn map_hid_error(operation: &str, path: &str, error: HidError) -> anyhow::Error {
    let context = format!("HID {operation} failed for path {path}");
    if hid_error_indicates_disconnect(&error) {
        Err::<(), _>(error)
            .context(TransportError::DeviceDisconnected)
            .context(context)
            .expect_err("error context cannot succeed")
    } else {
        Err::<(), _>(error)
            .context(context)
            .expect_err("error context cannot succeed")
    }
}

const HID_RECONNECT_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Default)]
struct HidTransportState {
    device: Option<HidDevice>,
    last_open_attempt: Option<Instant>,
    read_buffer: Vec<u8>,
}

pub struct HidTransport {
    candidate: DeviceCandidate,
    report_size: usize,
    numbered_reports: bool,
    api: Arc<Mutex<HidApi>>,
    state: Mutex<HidTransportState>,
}

impl HidTransport {
    /// Open exactly one matching path for compatibility with existing callers.
    /// Multiple matching interfaces/units are rejected instead of selecting an
    /// arbitrary first entry from hidapi's device list.
    pub fn open(vid: u16, pid: u16) -> Result<Self> {
        let api = Arc::new(Mutex::new(HidApi::new()?));
        let candidate = {
            let api_guard = api.lock().map_err(|_| anyhow!("hidapi lock poisoned"))?;
            let candidates: Vec<_> = api_guard
                .device_list()
                .map(DeviceCandidate::from_device_info)
                .collect();
            select_compatibility_candidate(&candidates, vid, pid)?
        };

        Self::open_candidate(api, &candidate)
    }

    /// Open selected candidate's exact HIDAPI path after validating its
    /// generated profile and current identity.
    pub fn open_path(candidate: &DeviceCandidate) -> Result<Self> {
        let api = Arc::new(Mutex::new(HidApi::new()?));
        Self::open_candidate(api, candidate)
    }

    /// Open an exact path using an owned runtime-catalog entry.
    ///
    /// Identity and report geometry are checked before HIDAPI is created, so
    /// disabled or incomplete external profiles cannot touch hardware.
    pub fn open_path_for_entry(candidate: &DeviceCandidate, entry: &RuntimeEntry) -> Result<Self> {
        let (report_size, numbered_reports) = validate_runtime_transport(candidate, entry)?;
        let api = Arc::new(Mutex::new(HidApi::new()?));
        {
            let api_guard = api.lock().map_err(|_| anyhow!("hidapi lock poisoned"))?;
            let current = api_guard
                .device_list()
                .filter(|info| {
                    info.vendor_id() == candidate.vid && info.product_id() == candidate.pid
                })
                .map(DeviceCandidate::from_device_info)
                .collect::<Vec<_>>();
            let exact = current
                .iter()
                .filter(|peer| peer.path_bytes == candidate.path_bytes)
                .collect::<Vec<_>>();
            if exact.len() != 1 || exact[0] != candidate {
                return Err(anyhow!(TransportError::DeviceUnavailable).context(format!(
                    "selected HID path {} disappeared or changed before open",
                    path_context(&candidate.path_bytes)
                )));
            }
            let identity_peers = current
                .iter()
                .filter(|peer| runtime_transport_matches(peer, entry))
                .filter(
                    |peer| match (&candidate.serial_number, &peer.serial_number) {
                        (Some(left), Some(right)) => left == right,
                        (None, _) | (_, None) => true,
                    },
                )
                .count();
            if identity_peers != 1 {
                return Err(anyhow!(TransportError::AmbiguousDevice).context(format!(
                    "runtime identity became ambiguous before opening path {}",
                    path_context(&candidate.path_bytes)
                )));
            }
        }
        let device = open_exact_hid_device(&api, candidate)?;
        Ok(Self {
            candidate: candidate.clone(),
            report_size,
            numbered_reports,
            api,
            state: Mutex::new(HidTransportState {
                device: Some(device),
                last_open_attempt: None,
                read_buffer: vec![0_u8; input_buffer_size(report_size, numbered_reports)],
            }),
        })
    }

    fn open_candidate(api: Arc<Mutex<HidApi>>, candidate: &DeviceCandidate) -> Result<Self> {
        let current_candidates = {
            let api_guard = api.lock().map_err(|_| anyhow!("hidapi lock poisoned"))?;
            enumerate_antelope_devices(&api_guard, DEVICE_CATALOG)
        };
        let classification =
            classify_selected_candidate(candidate, &current_candidates, DEVICE_CATALOG)?;
        if candidate.interface_number < 0 {
            return Err(anyhow!(TransportError::AmbiguousDevice).context(format!(
                "refusing HID candidate {} ({:04x}:{:04x}): interface number unavailable",
                path_context(&candidate.path_bytes),
                candidate.vid,
                candidate.pid
            )));
        }
        if !classification.is_selectable() {
            let diagnostic = classification
                .diagnostics
                .first()
                .map(String::as_str)
                .unwrap_or("candidate is not selectable");
            let error_kind = if classification.status == CandidateStatus::Ambiguous {
                TransportError::AmbiguousDevice
            } else {
                TransportError::UnsupportedDevice
            };
            return Err(anyhow!(error_kind).context(format!(
                "refusing HID candidate {} ({:04x}:{:04x}): {}",
                path_context(&candidate.path_bytes),
                candidate.vid,
                candidate.pid,
                diagnostic
            )));
        }

        let profile = classification
            .profile
            .expect("selectable candidate must have a catalog profile");
        let report_size = profile.definition.transport.report_size.ok_or_else(|| {
            anyhow!(TransportError::UnsupportedDevice).context(format!(
                "profile {} has no confirmed HID report size",
                profile.definition.identity.name
            ))
        })? as usize;
        if report_size == 0 {
            return Err(anyhow!(TransportError::UnsupportedDevice)
                .context("profile HID report size must be non-zero"));
        }
        let numbered_reports = profile
            .definition
            .transport
            .uses_numbered_reports
            .ok_or_else(|| {
                anyhow!(TransportError::UnsupportedDevice).context(format!(
                    "profile {} does not confirm HID report numbering",
                    profile.definition.identity.name
                ))
            })?;

        let device = open_exact_hid_device(&api, candidate)?;
        Ok(Self {
            candidate: candidate.clone(),
            report_size,
            numbered_reports,
            api,
            state: Mutex::new(HidTransportState {
                device: Some(device),
                last_open_attempt: None,
                read_buffer: vec![0_u8; input_buffer_size(report_size, numbered_reports)],
            }),
        })
    }

    fn ensure_device(
        api: &Mutex<HidApi>,
        state: &mut HidTransportState,
        candidate: &DeviceCandidate,
    ) -> Result<bool> {
        if state.device.is_some() {
            return Ok(true);
        }

        if state
            .last_open_attempt
            .is_some_and(|instant| instant.elapsed() < HID_RECONNECT_INTERVAL)
        {
            return Ok(false);
        }

        state.last_open_attempt = Some(Instant::now());

        match reconnect_hid_device(api, candidate) {
            Ok(device) => {
                state.device = Some(device);
                state.last_open_attempt = None;
                Ok(true)
            }
            Err(error) if is_device_error(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

impl Transport for HidTransport {
    fn write(&self, data: &[u8]) -> Result<()> {
        let report = prepare_output_report(data, self.report_size, self.numbered_reports)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("hid device lock poisoned"))?;

        if !Self::ensure_device(&self.api, &mut state, &self.candidate)? {
            return Err(TransportError::DeviceUnavailable.into());
        }

        let Some(device) = state.device.as_ref() else {
            return Err(TransportError::DeviceUnavailable.into());
        };

        match device.write(&report) {
            Ok(written) => validate_hid_write_length(written, report.len()),
            Err(error) => {
                let error =
                    map_hid_error("write", &path_context(&self.candidate.path_bytes), error);
                if is_device_error(&error) {
                    state.device = None;
                    state.last_open_attempt = Some(Instant::now());
                }
                Err(error)
            }
        }
    }

    fn read(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("hid device lock poisoned"))?;

        if !Self::ensure_device(&self.api, &mut state, &self.candidate)? {
            return Ok(None);
        }

        let mut buffer = std::mem::take(&mut state.read_buffer);
        let Some(device) = state.device.as_ref() else {
            state.read_buffer = buffer;
            return Ok(None);
        };

        let bytes = match device.read_timeout(
            &mut buffer,
            timeout.as_millis().clamp(0, i32::MAX as u128) as i32,
        ) {
            Ok(bytes) => bytes,
            Err(error) => {
                let error = map_hid_error("read", &path_context(&self.candidate.path_bytes), error);
                if is_device_error(&error) {
                    state.device = None;
                    state.last_open_attempt = Some(Instant::now());
                }
                state.read_buffer = buffer;
                return Err(error);
            }
        };
        let result = if bytes == 0 {
            Ok(None)
        } else {
            normalize_input_report(&buffer[..bytes], self.report_size, self.numbered_reports)
                .map(Some)
        };
        state.read_buffer = buffer;
        result
    }

    fn is_available(&self) -> Result<bool> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow!("hid device lock poisoned"))?;
        Self::ensure_device(&self.api, &mut state, &self.candidate)
    }
}

fn runtime_transport_matches(candidate: &DeviceCandidate, entry: &RuntimeEntry) -> bool {
    let transport = &entry.profile.transport;
    (candidate.vid, candidate.pid) == (entry.profile.identity.vid, entry.profile.identity.pid)
        && candidate.interface_number >= 0
        && transport
            .expected_interface_number
            .is_none_or(|expected| candidate.interface_number == expected)
        && transport
            .expected_usage_page
            .is_none_or(|expected| candidate.usage_page == expected)
        && transport
            .expected_usage
            .is_none_or(|expected| candidate.usage == expected)
}

fn validate_runtime_transport(
    candidate: &DeviceCandidate,
    entry: &RuntimeEntry,
) -> Result<(usize, bool)> {
    let profile = &entry.profile;
    if (candidate.vid, candidate.pid) != (profile.identity.vid, profile.identity.pid) {
        return Err(anyhow!(TransportError::UnsupportedDevice).context(format!(
            "candidate {:04x}:{:04x} does not match runtime profile {} ({:04x}:{:04x})",
            candidate.vid,
            candidate.pid,
            profile.identity.name,
            profile.identity.vid,
            profile.identity.pid
        )));
    }
    if candidate.interface_number < 0 {
        return Err(anyhow!(TransportError::AmbiguousDevice)
            .context("runtime candidate interface number is unavailable"));
    }
    let transport = &profile.transport;
    if transport
        .expected_interface_number
        .is_some_and(|expected| candidate.interface_number != expected)
    {
        return Err(anyhow!(TransportError::AmbiguousDevice).context(format!(
            "runtime profile {} expects interface {}, found {}",
            profile.identity.name,
            transport.expected_interface_number.unwrap(),
            candidate.interface_number
        )));
    }
    if transport
        .expected_usage_page
        .is_some_and(|expected| candidate.usage_page != expected)
        || transport
            .expected_usage
            .is_some_and(|expected| candidate.usage != expected)
    {
        return Err(anyhow!(TransportError::AmbiguousDevice).context(format!(
            "runtime profile {} HID usage does not match candidate",
            profile.identity.name
        )));
    }
    let report_size = usize::from(transport.report_size.ok_or_else(|| {
        anyhow!(TransportError::UnsupportedDevice).context(format!(
            "runtime profile {} has no confirmed HID report size",
            profile.identity.name
        ))
    })?);
    if report_size == 0 {
        return Err(anyhow!(TransportError::UnsupportedDevice)
            .context("runtime profile HID report size must be non-zero"));
    }
    let numbered_reports = transport.uses_numbered_reports.ok_or_else(|| {
        anyhow!(TransportError::UnsupportedDevice).context(format!(
            "runtime profile {} does not confirm HID report numbering",
            profile.identity.name
        ))
    })?;
    Ok((report_size, numbered_reports))
}

fn select_compatibility_candidate(
    candidates: &[DeviceCandidate],
    vid: u16,
    pid: u16,
) -> Result<DeviceCandidate> {
    let matching = candidates
        .iter()
        .filter(|candidate| candidate.vid == vid && candidate.pid == pid)
        .collect::<Vec<_>>();

    match matching.as_slice() {
        [] => Err(anyhow!(TransportError::DeviceUnavailable).context(format!(
            "no HID interfaces found for {:04x}:{:04x}",
            vid, pid
        ))),
        [candidate] => Ok((*candidate).clone()),
        matching => {
            let paths = matching
                .iter()
                .map(|candidate| path_context(&candidate.path_bytes))
                .collect::<Vec<_>>()
                .join(", ");
            Err(anyhow!(TransportError::AmbiguousDevice).context(format!(
                "found {} matching HID interface(s) for {:04x}:{:04x}; select an exact path: {}",
                matching.len(),
                vid,
                pid,
                paths
            )))
        }
    }
}

fn classify_selected_candidate<'a>(
    candidate: &DeviceCandidate,
    current_candidates: &[DeviceCandidate],
    catalog: &'a [DeviceEntry],
) -> Result<CandidateClassification<'a>> {
    let selected_index = current_candidates
        .iter()
        .position(|current| current.path_bytes == candidate.path_bytes)
        .ok_or_else(|| {
            anyhow!(TransportError::DeviceUnavailable).context(format!(
                "selected HID path is no longer present: {}",
                path_context(&candidate.path_bytes)
            ))
        })?;

    classify_candidates(current_candidates, catalog)
        .into_iter()
        .nth(selected_index)
        .ok_or_else(|| anyhow!("selected HID candidate classification disappeared"))
}

fn map_hid_refresh_error(error: HidError) -> anyhow::Error {
    map_hid_error(
        "refresh HID device list while reconnecting",
        "device list",
        error,
    )
}

fn open_exact_hid_device(api: &Mutex<HidApi>, candidate: &DeviceCandidate) -> Result<HidDevice> {
    let path = CString::new(candidate.path_bytes.as_slice()).map_err(|_| {
        anyhow!(TransportError::UnsupportedDevice).context("HID path contains an interior NUL")
    })?;
    let api = api.lock().map_err(|_| anyhow!("hidapi lock poisoned"))?;
    let Some(info) = api
        .device_list()
        .find(|info| info.path().to_bytes() == candidate.path_bytes.as_slice())
    else {
        return Err(anyhow!(TransportError::DeviceUnavailable).context(format!(
            "selected HID path is no longer present: {}",
            path_context(&candidate.path_bytes)
        )));
    };
    if !device_info_matches(info, candidate) {
        return Err(anyhow!(TransportError::AmbiguousDevice).context(format!(
            "selected HID path identity changed: {}",
            path_context(&candidate.path_bytes)
        )));
    }

    let same_unit_paths: Vec<_> = api
        .device_list()
        .filter(|info| same_unit_identity(info, candidate))
        .collect();
    if same_unit_paths.len() > 1 {
        let paths = same_unit_paths
            .iter()
            .map(|info| path_context(info.path().to_bytes()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(anyhow!(TransportError::AmbiguousDevice).context(format!(
            "multiple HID paths match selected identity {:04x}:{:04x}: {}",
            candidate.vid, candidate.pid, paths
        )));
    }

    api.open_path(path.as_c_str()).map_err(|error| {
        map_hid_error(
            "open selected HID path",
            &path_context(&candidate.path_bytes),
            error,
        )
    })
}

fn reconnect_hid_device(api: &Mutex<HidApi>, candidate: &DeviceCandidate) -> Result<HidDevice> {
    validate_saved_reconnect_serial(candidate)?;
    let path = CString::new(candidate.path_bytes.as_slice()).map_err(|_| {
        anyhow!(TransportError::UnsupportedDevice).context("HID path contains an interior NUL")
    })?;
    let mut api = api.lock().map_err(|_| anyhow!("hidapi lock poisoned"))?;
    api.refresh_devices().map_err(map_hid_refresh_error)?;

    let exact = api
        .device_list()
        .find(|info| info.path().to_bytes() == candidate.path_bytes.as_slice())
        .cloned();
    let mut exact_error = None;
    if let Some(info) = exact.as_ref() {
        if device_info_matches(info, candidate) {
            let same_unit_count = api
                .device_list()
                .filter(|info| same_unit_identity(info, candidate))
                .count();
            if same_unit_count > 1 {
                return Err(anyhow!(TransportError::AmbiguousDevice).context(format!(
                    "multiple HID paths match saved control identity {:04x}:{:04x}: {}",
                    candidate.vid, candidate.pid, same_unit_count
                )));
            }
            match api.open_path(path.as_c_str()) {
                Ok(device) => return Ok(device),
                Err(error) => {
                    let error = map_hid_error(
                        "reopen exact HID path",
                        &path_context(&candidate.path_bytes),
                        error,
                    );
                    exact_error = Some(error);
                }
            }
        }
    }

    let matches: Vec<_> = api
        .device_list()
        .filter(|info| reconnect_identity_matches(info, candidate))
        .cloned()
        .collect();

    match matches.as_slice() {
        [] => Err(exact_error.unwrap_or_else(|| {
            anyhow!(TransportError::DeviceUnavailable).context(format!(
                "HID identity is not currently present: {}",
                path_context(&candidate.path_bytes)
            ))
        })),
        [info] => api.open_path(info.path()).map_err(|error| {
            map_hid_error(
                "reopen validated HID path",
                &path_context(info.path().to_bytes()),
                error,
            )
        }),
        matches => {
            let paths = matches
                .iter()
                .map(|info| path_context(info.path().to_bytes()))
                .collect::<Vec<_>>()
                .join(", ");
            Err(anyhow!(TransportError::AmbiguousDevice).context(format!(
                "multiple HID paths match saved identity {:04x}:{:04x}: {}",
                candidate.vid, candidate.pid, paths
            )))
        }
    }
}

fn device_info_matches(info: &DeviceInfo, candidate: &DeviceCandidate) -> bool {
    info.vendor_id() == candidate.vid
        && info.product_id() == candidate.pid
        && info.usage_page() == candidate.usage_page
        && info.usage() == candidate.usage
        && info.interface_number() == candidate.interface_number
        && serial_metadata_matches(info.serial_number(), candidate.serial_number.as_deref())
        && optional_metadata_matches(info.product_string(), candidate.product_string.as_deref())
}

fn reconnect_identity_matches(info: &DeviceInfo, candidate: &DeviceCandidate) -> bool {
    reconnect_serial_matches(info.serial_number(), candidate.serial_number.as_deref())
        && device_info_matches(info, candidate)
        && info.path().to_bytes() != candidate.path_bytes.as_slice()
}

fn same_unit_identity(info: &DeviceInfo, candidate: &DeviceCandidate) -> bool {
    if info.vendor_id() != candidate.vid
        || info.product_id() != candidate.pid
        || info.usage_page() != candidate.usage_page
        || info.usage() != candidate.usage
        || info.interface_number() != candidate.interface_number
    {
        return false;
    }

    reconnect_serial_matches(info.serial_number(), candidate.serial_number.as_deref())
}

fn nonempty_serial(serial: Option<&str>) -> Option<&str> {
    serial.filter(|serial| !serial.is_empty())
}

fn reconnect_serial_matches(actual: Option<&str>, expected: Option<&str>) -> bool {
    matches!(
        (nonempty_serial(actual), nonempty_serial(expected)),
        (Some(actual), Some(expected)) if actual == expected
    )
}

fn validate_saved_reconnect_serial(candidate: &DeviceCandidate) -> Result<&str> {
    nonempty_serial(candidate.serial_number.as_deref()).ok_or_else(|| {
        anyhow!(TransportError::DeviceUnavailable).context(format!(
            "saved HID identity has no nonempty serial: {}",
            path_context(&candidate.path_bytes)
        ))
    })
}

fn serial_metadata_matches(actual: Option<&str>, expected: Option<&str>) -> bool {
    actual == expected
}

fn optional_metadata_matches(actual: Option<&str>, expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| actual == Some(expected))
}

fn input_buffer_size(report_size: usize, _numbered_reports: bool) -> usize {
    // hidapi exposes report IDs in its read buffer even for devices whose
    // single report uses ID 0.  Extra capacity lets normalization safely
    // accept either backend representation without truncating a report.
    report_size.saturating_add(1)
}

fn validate_hid_write_length(written: usize, expected: usize) -> Result<()> {
    if written == expected {
        return Ok(());
    }

    Err(anyhow!(TransportError::ShortWrite)
        .context(format!("HID write wrote {written} of {expected} bytes")))
}

fn prepare_output_report(
    data: &[u8],
    report_size: usize,
    numbered_reports: bool,
) -> Result<Vec<u8>> {
    if numbered_reports {
        if data.len() != report_size + 1 {
            return Err(anyhow!(TransportError::InvalidReport).context(format!(
                "numbered HID report requires {} bytes including report ID, got {}",
                report_size + 1,
                data.len()
            )));
        }
        return Ok(data.to_vec());
    }

    if data.len() == report_size + 1 {
        if data[0] != 0 {
            return Err(anyhow!(TransportError::InvalidReport)
                .context("non-numbered HID report must use report ID 0"));
        }
        return Ok(data.to_vec());
    }
    if data.len() != report_size {
        return Err(anyhow!(TransportError::InvalidReport).context(format!(
            "HID report requires {} payload bytes, got {}",
            report_size,
            data.len()
        )));
    }

    let mut report = Vec::with_capacity(report_size + 1);
    report.push(0);
    report.extend_from_slice(data);
    Ok(report)
}

fn normalize_input_report(
    data: &[u8],
    report_size: usize,
    numbered_reports: bool,
) -> Result<Vec<u8>> {
    if numbered_reports {
        if data.len() != report_size + 1 {
            return Err(anyhow!(TransportError::InvalidReport).context(format!(
                "numbered HID report requires {} bytes including report ID, got {}",
                report_size + 1,
                data.len()
            )));
        }
        return Ok(data[1..].to_vec());
    }

    if data.len() == report_size {
        return Ok(data.to_vec());
    }
    if data.len() == report_size + 1 && data[0] == 0 {
        return Ok(data[1..].to_vec());
    }
    Err(anyhow!(TransportError::InvalidReport).context(format!(
        "HID report requires {} payload bytes, got {}",
        report_size,
        data.len()
    )))
}

#[derive(Clone, Default)]
pub struct MockTransport {
    inner: Arc<Mutex<MockTransportInner>>,
}

#[derive(Default)]
struct MockTransportInner {
    reads: VecDeque<Vec<u8>>,
    writes: Vec<Vec<u8>>,
}

impl MockTransport {
    pub fn push_read(&self, data: Vec<u8>) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.reads.push_back(data);
        }
    }

    pub fn take_writes(&self) -> Vec<Vec<u8>> {
        if let Ok(mut inner) = self.inner.lock() {
            return std::mem::take(&mut inner.writes);
        }
        Vec::new()
    }
}

impl Transport for MockTransport {
    fn write(&self, data: &[u8]) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow!("mock transport lock poisoned"))?;
        inner.writes.push(data.to_vec());
        Ok(())
    }

    fn read(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow!("mock transport lock poisoned"))?;
        Ok(inner.reads.pop_front())
    }

    fn is_available(&self) -> Result<bool> {
        Ok(true)
    }
}

enum TransportMessage {
    Write {
        data: Vec<u8>,
        reply: Sender<Result<()>>,
    },
    IsAvailable {
        reply: Sender<Result<bool>>,
    },
    Shutdown,
}

struct ThreadState {
    available: bool,
    disconnect_reported: bool,
}

pub struct ThreadedTransport {
    msg_tx: Sender<TransportMessage>,
    frame_rx: Receiver<Result<Vec<u8>>>,
    #[allow(dead_code)]
    state: Arc<Mutex<ThreadState>>,
    thread: Option<JoinHandle<()>>,
}

impl ThreadedTransport {
    pub fn spawn(transport: HidTransport) -> Self {
        Self::spawn_with_transport(transport)
    }

    /// Spawn worker around any transport.  Kept public to make worker state
    /// deterministic to exercise without constructing a physical HID handle.
    pub fn spawn_with_transport<T>(transport: T) -> Self
    where
        T: Transport + 'static,
    {
        let (msg_tx, msg_rx) = mpsc::channel();
        let (frame_tx, frame_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(ThreadState {
            available: true,
            disconnect_reported: false,
        }));

        let state_clone = Arc::clone(&state);

        let thread = thread::Builder::new()
            .name("hid-transport".into())
            .spawn(move || {
                let poll_timeout = 50;

                'worker: loop {
                    let mut pause_after_poll = false;
                    match transport.read(Duration::from_millis(poll_timeout)) {
                        Ok(Some(frame)) => {
                            if let Ok(mut state) = state_clone.lock() {
                                state.available = true;
                                state.disconnect_reported = false;
                            }
                            if frame_tx.send(Ok(frame)).is_err() {
                                break 'worker;
                            }
                        }
                        Ok(None) => match transport.is_available() {
                            Ok(available) => {
                                pause_after_poll = !available;
                                if let Ok(mut state) = state_clone.lock() {
                                    if available {
                                        state.available = true;
                                        state.disconnect_reported = false;
                                    } else {
                                        state.available = false;
                                    }
                                }
                            }
                            Err(error) if is_device_error(&error) => {
                                pause_after_poll = true;
                                let report = state_clone.lock().is_ok_and(|mut state| {
                                    let report = state.available && !state.disconnect_reported;
                                    state.available = false;
                                    state.disconnect_reported = true;
                                    report
                                });
                                if report && frame_tx.send(Err(error)).is_err() {
                                    break 'worker;
                                }
                            }
                            Err(error) => {
                                if frame_tx.send(Err(error)).is_err() {
                                    break 'worker;
                                }
                                break 'worker;
                            }
                        },
                        Err(error) if is_device_error(&error) => {
                            pause_after_poll = true;
                            let report = state_clone.lock().is_ok_and(|mut state| {
                                let report = state.available && !state.disconnect_reported;
                                state.available = false;
                                state.disconnect_reported = true;
                                report
                            });
                            if report && frame_tx.send(Err(error)).is_err() {
                                break 'worker;
                            }
                        }
                        Err(error) => {
                            if frame_tx.send(Err(error)).is_err() {
                                break 'worker;
                            }
                            break 'worker;
                        }
                    }

                    match msg_rx.try_recv() {
                        Ok(TransportMessage::Write { data, reply }) => {
                            let result = transport.write(&data);
                            match &result {
                                Ok(()) => {
                                    if let Ok(mut state) = state_clone.lock() {
                                        state.available = true;
                                        state.disconnect_reported = false;
                                    }
                                }
                                Err(error) if is_device_error(error) => {
                                    if let Ok(mut state) = state_clone.lock() {
                                        state.available = false;
                                        state.disconnect_reported = true;
                                    }
                                }
                                Err(_) => {}
                            }
                            let _ = reply.send(result);
                        }
                        Ok(TransportMessage::IsAvailable { reply }) => {
                            let avail = state_clone
                                .lock()
                                .map(|state| state.available)
                                .unwrap_or(false);
                            let _ = reply.send(Ok(avail));
                        }
                        Ok(TransportMessage::Shutdown) | Err(TryRecvError::Disconnected) => {
                            break 'worker;
                        }
                        Err(TryRecvError::Empty) => {}
                    }

                    if pause_after_poll {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            })
            .expect("failed to spawn hid-transport thread");

        Self {
            msg_tx,
            frame_rx,
            state,
            thread: Some(thread),
        }
    }
}

impl Transport for ThreadedTransport {
    fn write(&self, data: &[u8]) -> Result<()> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.msg_tx
            .send(TransportMessage::Write {
                data: data.to_vec(),
                reply: reply_tx,
            })
            .map_err(|_| anyhow!("transport thread terminated"))?;
        reply_rx
            .recv()
            .map_err(|_| anyhow!("transport thread terminated"))?
    }

    fn read(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        match self.frame_rx.recv_timeout(timeout) {
            Ok(Ok(frame)) => Ok(Some(frame)),
            Ok(Err(error)) => Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(anyhow!("transport thread terminated"))
            }
        }
    }

    fn is_available(&self) -> Result<bool> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.msg_tx
            .send(TransportMessage::IsAvailable { reply: reply_tx })
            .map_err(|_| anyhow!("transport thread terminated"))?;
        reply_rx
            .recv()
            .map_err(|_| anyhow!("transport thread terminated"))?
    }
}

impl Drop for ThreadedTransport {
    fn drop(&mut self) {
        let _ = self.msg_tx.send(TransportMessage::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_closes_old_worker_before_opening_zero_write_replacement() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct DropObservedTransport(Arc<AtomicBool>);
        impl Drop for DropObservedTransport {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        impl Transport for DropObservedTransport {
            fn write(&self, _data: &[u8]) -> Result<()> {
                Ok(())
            }
            fn read(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
                Ok(None)
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let transport =
            ThreadedTransport::spawn_with_transport(DropObservedTransport(Arc::clone(&dropped)));
        drop(transport);
        assert!(dropped.load(Ordering::SeqCst));

        let replacement = MockTransport::default();
        let replacement_worker = ThreadedTransport::spawn_with_transport(replacement.clone());
        assert!(replacement.take_writes().is_empty());
        drop(replacement_worker);
    }

    #[test]
    fn runtime_device_selection_rejects_unknown_report_framing_before_hid_open() {
        let catalog = crate::device::ProfileCatalog::builtin();
        let entry = catalog
            .find(0x23e5, 0xa2bf)
            .expect("Discrete 4 Pro profile");
        let candidate = DeviceCandidate::new(
            "unknown-framing-no-open",
            0x23e5,
            0xa2bf,
            Some("DISCRETE-4-PRO-1".into()),
            Some("Discrete 4 Pro Synergy Core".into()),
            0,
            0,
            3,
        );

        let error = validate_runtime_transport(&candidate, entry)
            .expect_err("unknown framing must fail before HIDAPI construction");

        assert!(error
            .to_string()
            .contains("does not confirm HID report numbering"));
    }

    #[test]
    fn mock_transport_records_writes_and_reads_frames() {
        let transport = MockTransport::default();
        transport.push_read(vec![1, 2, 3]);
        transport.write(&[9, 8, 7]).expect("write");

        let read = transport.read(Duration::from_millis(10)).expect("read");
        assert_eq!(read, Some(vec![1, 2, 3]));
        assert_eq!(transport.take_writes(), vec![vec![9, 8, 7]]);
    }

    #[test]
    fn is_device_error_matches_context_wrapped_transport_error() {
        let error = anyhow!(TransportError::DeviceUnavailable).context("wrapped device error");

        assert!(is_device_error(&error));
    }

    #[test]
    fn non_numbered_320_byte_output_gets_zero_report_id_and_exact_hid_length() {
        let payload = vec![0xa5; 320];
        let report = prepare_output_report(&payload, 320, false).expect("frame");

        assert_eq!(report.len(), 321);
        assert_eq!(report[0], 0);
        assert_eq!(&report[1..], payload.as_slice());
        validate_hid_write_length(report.len(), 321).expect("exact HID length");
        assert!(validate_hid_write_length(320, 321).is_err());
    }

    #[test]
    fn numbered_output_requires_report_id_and_payload_size() {
        let error = prepare_output_report(&[1, 2, 3], 3, true).expect_err("missing report id");

        assert!(error.to_string().contains("including report ID"));
        assert_eq!(
            prepare_output_report(&[7, 1, 2, 3], 3, true).expect("explicit report id"),
            vec![7, 1, 2, 3]
        );
    }

    #[test]
    fn input_normalization_removes_numbered_report_id() {
        let report = normalize_input_report(&[9, 1, 2, 3], 3, true).expect("frame");

        assert_eq!(report, vec![1, 2, 3]);
    }

    #[test]
    fn non_retryable_identity_errors_are_not_device_errors() {
        assert!(!is_device_error(
            &anyhow!(TransportError::AmbiguousDevice).context("duplicate path")
        ));
        assert!(!is_device_error(
            &anyhow!(TransportError::UnsupportedDevice).context("disabled profile")
        ));
    }

    #[test]
    fn compatibility_selection_rejects_multiple_matching_paths() {
        let candidates = vec![
            DeviceCandidate::new(
                "/dev/hidraw-a",
                0x23e5,
                0xa015,
                Some("one".to_string()),
                None,
                0xffa0,
                3,
                3,
            ),
            DeviceCandidate::new(
                "/dev/hidraw-b",
                0x23e5,
                0xa015,
                Some("two".to_string()),
                None,
                0xffa0,
                3,
                3,
            ),
        ];

        let error = select_compatibility_candidate(&candidates, 0x23e5, 0xa015)
            .expect_err("compatibility wrapper must not choose first path");

        assert!(error.to_string().contains("select an exact path"));
        assert!(error
            .chain()
            .any(|cause| cause.downcast_ref::<TransportError>()
                == Some(&TransportError::AmbiguousDevice)));
    }

    #[test]
    fn exact_path_selection_rejects_mixed_serial_identity() {
        let candidate = DeviceCandidate::new(
            "/dev/hidraw-a",
            0x23e5,
            0xa015,
            Some("unit-a".to_string()),
            None,
            0xffa0,
            3,
            3,
        );
        let peer = DeviceCandidate::new("/dev/hidraw-b", 0x23e5, 0xa015, None, None, 0xffa0, 3, 3);

        let classification = classify_selected_candidate(
            &candidate,
            &[candidate.clone(), peer],
            crate::device::DEVICE_CATALOG,
        )
        .expect("selected path must be present");

        assert_eq!(classification.status, CandidateStatus::Ambiguous);
        assert!(classification
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("mixed serial metadata")));
    }

    #[derive(Clone)]
    struct WorkerTestTransport {
        inner: Arc<Mutex<WorkerTestTransportInner>>,
    }

    struct WorkerTestTransportInner {
        reads: VecDeque<Result<Option<Vec<u8>>>>,
        read_calls: usize,
        writes: usize,
        write_error: Option<anyhow::Error>,
    }

    impl WorkerTestTransport {
        fn with_reads(reads: Vec<Result<Option<Vec<u8>>>>) -> Self {
            Self {
                inner: Arc::new(Mutex::new(WorkerTestTransportInner {
                    reads: reads.into_iter().collect(),
                    read_calls: 0,
                    writes: 0,
                    write_error: None,
                })),
            }
        }

        fn with_write_error(error: anyhow::Error) -> Self {
            Self {
                inner: Arc::new(Mutex::new(WorkerTestTransportInner {
                    reads: VecDeque::new(),
                    read_calls: 0,
                    writes: 0,
                    write_error: Some(error),
                })),
            }
        }

        fn read_calls(&self) -> usize {
            self.inner.lock().expect("test transport lock").read_calls
        }

        fn writes(&self) -> usize {
            self.inner.lock().expect("test transport lock").writes
        }
    }

    impl Transport for WorkerTestTransport {
        fn write(&self, _data: &[u8]) -> Result<()> {
            let mut inner = self.inner.lock().expect("test transport lock");
            inner.writes += 1;
            inner.write_error.take().map_or(Ok(()), Err)
        }

        fn read(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
            let result = {
                let mut inner = self.inner.lock().expect("test transport lock");
                inner.read_calls += 1;
                inner.reads.pop_front().unwrap_or(Ok(None))
            };
            thread::sleep(Duration::from_millis(1));
            result
        }
    }

    #[test]
    fn threaded_transport_probes_after_disconnect_and_recovers_without_write() {
        let transport = WorkerTestTransport::with_reads(vec![
            Err(anyhow!(TransportError::DeviceDisconnected)),
            Err(anyhow!(TransportError::DeviceDisconnected)),
            Ok(Some(vec![1, 2, 3])),
        ]);
        let threaded = ThreadedTransport::spawn_with_transport(transport.clone());

        let disconnect = threaded
            .read(Duration::from_secs(1))
            .expect_err("disconnect must be surfaced");
        assert!(is_device_error(&disconnect));

        assert_eq!(
            threaded
                .read(Duration::from_secs(1))
                .expect("reconnect read")
                .expect("reconnected frame"),
            vec![1, 2, 3]
        );
        assert_eq!(
            transport.writes(),
            0,
            "read probing must recover without write"
        );
        assert!(
            transport.read_calls() >= 3,
            "worker must keep probing while unavailable"
        );
    }

    #[test]
    fn threaded_transport_surfaces_nonretryable_read_error() {
        let transport = WorkerTestTransport::with_reads(vec![Err(anyhow!(
            TransportError::InvalidReport
        )
        .context("malformed input report"))]);
        let threaded = ThreadedTransport::spawn_with_transport(transport);

        let error = threaded
            .read(Duration::from_secs(1))
            .expect_err("invalid report must reach caller");
        assert!(error
            .chain()
            .any(|cause| cause.downcast_ref::<TransportError>()
                == Some(&TransportError::InvalidReport)));
        assert!(error.to_string().contains("malformed input report"));
    }

    #[test]
    fn threaded_transport_propagates_nonretryable_write_error() {
        let transport = WorkerTestTransport::with_write_error(
            anyhow!(TransportError::InvalidReport).context("malformed output report"),
        );
        let threaded = ThreadedTransport::spawn_with_transport(transport);

        let error = threaded
            .write(&[1, 2, 3])
            .expect_err("invalid write must reach caller");
        assert!(error
            .chain()
            .any(|cause| cause.downcast_ref::<TransportError>()
                == Some(&TransportError::InvalidReport)));
        assert!(error.to_string().contains("malformed output report"));
    }

    #[test]
    fn threaded_transport_terminates_after_nonretryable_read_error() {
        let transport = WorkerTestTransport::with_reads(vec![
            Err(anyhow!(TransportError::InvalidReport).context("first malformed report")),
            Err(anyhow!(TransportError::InvalidReport).context("second malformed report")),
        ]);
        let threaded = ThreadedTransport::spawn_with_transport(transport);

        let first = threaded
            .read(Duration::from_secs(1))
            .expect_err("first invalid report must reach caller");
        assert!(first.to_string().contains("first malformed report"));

        let second = threaded
            .read(Duration::from_secs(1))
            .expect_err("worker must terminate after nonretryable read error");
        assert_eq!(second.to_string(), "transport thread terminated");
    }

    struct NonRetryableAvailabilityTransport;

    impl Transport for NonRetryableAvailabilityTransport {
        fn write(&self, _data: &[u8]) -> Result<()> {
            Ok(())
        }

        fn read(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn is_available(&self) -> Result<bool> {
            Err(anyhow!(TransportError::InvalidReport).context("invalid availability state"))
        }
    }

    #[test]
    fn threaded_transport_terminates_after_nonretryable_availability_error() {
        let threaded = ThreadedTransport::spawn_with_transport(NonRetryableAvailabilityTransport);

        let first = threaded
            .read(Duration::from_secs(1))
            .expect_err("first availability error must reach caller");
        assert!(first.to_string().contains("invalid availability state"));

        let second = threaded
            .read(Duration::from_secs(1))
            .expect_err("worker must terminate after nonretryable availability error");
        assert_eq!(second.to_string(), "transport thread terminated");
    }

    #[test]
    fn hid_path_context_preserves_non_utf8_bytes() {
        assert_eq!(path_context(b"/dev/hidraw-\xff"), "/dev/hidraw-\\xff");
    }

    #[test]
    fn short_hid_write_is_rejected() {
        let error = validate_hid_write_length(3, 4).expect_err("short HID write");

        assert!(error.chain().any(
            |cause| cause.downcast_ref::<TransportError>() == Some(&TransportError::ShortWrite)
        ));
        assert!(error.to_string().contains("wrote 3 of 4 bytes"));
    }

    #[test]
    fn missing_serial_metadata_does_not_match_present_serial() {
        assert!(!serial_metadata_matches(Some("unit-2"), None));
        assert!(serial_metadata_matches(None, None));
    }

    #[test]
    fn reconnect_serial_predicate_rejects_absent_and_empty_metadata() {
        assert!(!reconnect_serial_matches(None, Some("unit-1")));
        assert!(!reconnect_serial_matches(Some("unit-1"), None));
        assert!(!reconnect_serial_matches(Some(""), Some("")));
        assert!(!reconnect_serial_matches(Some(""), Some("unit-1")));
        assert!(!reconnect_serial_matches(Some("unit-1"), Some("")));
    }

    #[test]
    fn reconnect_serial_predicate_accepts_same_nonempty_serial() {
        assert!(reconnect_serial_matches(Some("unit-1"), Some("unit-1")));
        assert!(!reconnect_serial_matches(Some("unit-2"), Some("unit-1")));
    }

    #[test]
    fn reconnect_validation_rejects_empty_or_absent_saved_serial_before_hid() {
        for serial in [None, Some("")] {
            let candidate = DeviceCandidate::new(
                "never-open-this-path",
                0x23e5,
                0xa015,
                serial.map(str::to_owned),
                Some("Zen Go".into()),
                0,
                0,
                3,
            );
            let error = validate_saved_reconnect_serial(&candidate)
                .expect_err("missing serial must fail before HID open");
            assert!(error.chain().any(|cause| {
                cause.downcast_ref::<TransportError>() == Some(&TransportError::DeviceUnavailable)
            }));
        }
    }

    #[test]
    fn generic_not_connected_hid_message_is_not_retryable() {
        let error = map_hid_error(
            "read",
            "/dev/hidraw0",
            hidapi::HidError::HidApiError {
                message: "operation failed: not connected".to_string(),
            },
        );

        assert!(!is_device_error(&error));
    }

    #[test]
    fn hid_permission_error_is_not_retryable_but_preserves_hid_error() {
        let error = map_hid_error(
            "open",
            "/dev/hidraw0",
            hidapi::HidError::IoError {
                error: std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "permission denied",
                ),
            },
        );

        assert!(!is_device_error(&error));
        assert!(error
            .chain()
            .any(|cause| cause.downcast_ref::<hidapi::HidError>().is_some()));
        assert!(error
            .to_string()
            .contains("HID open failed for path /dev/hidraw0"));
    }

    #[test]
    fn hid_disconnect_error_is_retryable_and_preserves_hid_error() {
        let error = map_hid_error(
            "read",
            "/dev/hidraw0",
            hidapi::HidError::HidApiError {
                message: "unexpected poll error (device disconnected)".to_string(),
            },
        );

        assert!(is_device_error(&error));
        assert!(error
            .chain()
            .any(|cause| cause.downcast_ref::<hidapi::HidError>().is_some()));
    }

    #[test]
    fn hid_refresh_disconnect_error_is_retryable_and_preserves_hid_error() {
        let error = map_hid_refresh_error(hidapi::HidError::HidApiError {
            message: "device disconnected while refreshing".to_string(),
        });

        assert!(is_device_error(&error));
        assert!(error
            .chain()
            .any(|cause| cause.downcast_ref::<hidapi::HidError>().is_some()));
        assert!(error
            .to_string()
            .contains("refresh HID device list while reconnecting"));
    }
}
