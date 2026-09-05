//! Runtime-catalog device selection and session ownership.

use std::time::{Duration, Instant};

use antelope_protocol::{
    DeviceDriver, DriverDefinition, ProfileDriver, RuntimeDriverKind, RuntimeEntry,
    RuntimeReadiness, ZenGoDriver,
};
use anyhow::{anyhow, bail, Context, Result};
use hidapi::HidApi;

use crate::app::Controller;
use crate::transport::{HidTransport, MockTransport, ThreadedTransport};

use super::discovery::{path_context, CandidateStatus, DeviceCandidate, ANTELOPE_VID};
use super::ProfileCatalog;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMatch {
    ExactPath,
    Serial,
    Identity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSelection {
    Path(Vec<u8>),
    Serial(String),
    Identity { vid: u16, pid: u16 },
}

impl DeviceSelection {
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.is_empty() {
            bail!("device selection cannot be empty");
        }
        if let Some(serial) = value.strip_prefix("serial:") {
            if serial.is_empty() {
                bail!("serial: device selection requires a value");
            }
            return Ok(Self::Serial(serial.to_owned()));
        }
        if let Some(path) = value.strip_prefix("path:") {
            if path.is_empty() {
                bail!("path: device selection requires a value");
            }
            return Ok(Self::Path(path.as_bytes().to_vec()));
        }
        if let Some((vid, pid)) = value.split_once(':') {
            if !vid.is_empty()
                && !pid.is_empty()
                && vid.len() <= 4
                && pid.len() <= 4
                && vid.chars().all(|ch| ch.is_ascii_hexdigit())
                && pid.chars().all(|ch| ch.is_ascii_hexdigit())
            {
                return Ok(Self::Identity {
                    vid: u16::from_str_radix(vid, 16)
                        .with_context(|| format!("invalid device VID {vid}"))?,
                    pid: u16::from_str_radix(pid, 16)
                        .with_context(|| format!("invalid device PID {pid}"))?,
                });
            }
        }
        if value.contains('/') || value.starts_with("hid") || value.starts_with('\\') {
            Ok(Self::Path(value.as_bytes().to_vec()))
        } else {
            Ok(Self::Serial(value.to_owned()))
        }
    }

    pub fn path_bytes(path: Vec<u8>) -> Self {
        Self::Path(path)
    }

    pub fn match_candidate(&self, candidate: &DeviceCandidate) -> Option<SelectionMatch> {
        match self {
            Self::Path(path) if *path == candidate.path_bytes => Some(SelectionMatch::ExactPath),
            Self::Serial(serial) if candidate.serial() == Some(serial.as_str()) => {
                Some(SelectionMatch::Serial)
            }
            Self::Identity { vid, pid } if (*vid, *pid) == (candidate.vid, candidate.pid) => {
                Some(SelectionMatch::Identity)
            }
            _ => None,
        }
    }
}

pub fn select_candidate<'a>(
    candidates: &'a [DeviceCandidate],
    selection: &DeviceSelection,
) -> Result<&'a DeviceCandidate> {
    let matches: Vec<_> = candidates
        .iter()
        .filter(|candidate| selection.match_candidate(candidate).is_some())
        .collect();
    match matches.as_slice() {
        [candidate] => Ok(*candidate),
        [] => bail!("device selection did not match any discovered Antelope device"),
        matches => {
            let paths = matches
                .iter()
                .map(|candidate| path_context(&candidate.path_bytes))
                .collect::<Vec<_>>()
                .join(", ");
            bail!("ambiguous device selection; matching paths: {paths}")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerEntry {
    pub candidate: DeviceCandidate,
    pub status: CandidateStatus,
    pub profile_id: Option<String>,
    pub profile_name: String,
    pub diagnostic: String,
}

impl PickerEntry {
    pub const fn is_selectable(&self) -> bool {
        self.status.is_selectable()
    }
}

#[derive(Debug, Clone)]
pub struct DevicePickerState {
    entries: Vec<PickerEntry>,
    selected: Option<usize>,
    active: Option<DeviceCandidate>,
    notice: Option<String>,
    pub last_discovery_at: Instant,
    pub retry_after: Duration,
}

impl DevicePickerState {
    pub fn new(candidates: Vec<DeviceCandidate>, catalog: &ProfileCatalog) -> Self {
        let mut entries = classify_runtime_candidates(&candidates, catalog);
        entries.sort_by(|left, right| {
            left.status
                .sort_rank()
                .cmp(&right.status.sort_rank())
                .then_with(|| left.profile_name.cmp(&right.profile_name))
                .then_with(|| left.candidate.path_bytes.cmp(&right.candidate.path_bytes))
        });
        Self {
            selected: (!entries.is_empty()).then_some(0),
            entries,
            active: None,
            notice: None,
            last_discovery_at: Instant::now(),
            retry_after: Duration::from_millis(500),
        }
    }

    /// Start a new retry window after attempting discovery.
    pub fn mark_discovery_attempt(&mut self) {
        self.last_discovery_at = Instant::now();
    }

    pub fn entries(&self) -> &[PickerEntry] {
        &self.entries
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn set_notice(&mut self, notice: impl Into<String>) {
        self.notice = Some(notice.into());
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_entry(&self) -> Option<&PickerEntry> {
        self.selected.and_then(|index| self.entries.get(index))
    }

    /// Mark the currently connected candidate without changing the discovered entries.
    /// Serial identity is preferred; HID path is used only when serial metadata is absent.
    pub fn set_active_candidate(&mut self, candidate: Option<DeviceCandidate>) {
        self.active = candidate;
        if let Some(active) = self.active.as_ref() {
            if let Some(index) = self
                .entries
                .iter()
                .position(|entry| same_selector_identity(active, &entry.candidate))
            {
                self.selected = Some(index);
            }
        }
    }

    pub fn is_active(&self, candidate: &DeviceCandidate) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| same_selector_identity(active, candidate))
    }

    pub fn selected_candidate(&self) -> Option<&DeviceCandidate> {
        self.selected_entry().map(|entry| &entry.candidate)
    }

    pub fn select_candidate(&mut self, candidate: &DeviceCandidate) -> bool {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| same_selector_identity(candidate, &entry.candidate))
        else {
            return false;
        };
        self.selected = Some(index);
        true
    }

    pub fn select_next(&mut self) {
        if self.entries.is_empty() {
            self.selected = None;
        } else {
            self.selected = Some((self.selected.unwrap_or(0) + 1) % self.entries.len());
        }
    }

    pub fn select_previous(&mut self) {
        if self.entries.is_empty() {
            self.selected = None;
        } else {
            let current = self.selected.unwrap_or(0);
            self.selected = Some(current.checked_sub(1).unwrap_or(self.entries.len() - 1));
        }
    }

    pub fn select_row(&mut self, row: usize) {
        if row < self.entries.len() {
            self.selected = Some(row);
        }
    }

    pub fn activate_selected(&self) -> Option<&DeviceCandidate> {
        self.selected_entry()
            .filter(|entry| entry.is_selectable())
            .map(|entry| &entry.candidate)
    }

    pub fn activate_row(&self, row: usize) -> Option<&DeviceCandidate> {
        self.entries
            .get(row)
            .filter(|entry| entry.is_selectable())
            .map(|entry| &entry.candidate)
    }

    pub fn selectable_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.is_selectable())
            .count()
    }
}

pub fn select_reconnect_candidate<'a>(
    previous: &DeviceCandidate,
    entries: &'a [PickerEntry],
) -> Option<&'a DeviceCandidate> {
    nonempty_candidate_serial(previous)?;
    let exact = entries
        .iter()
        .filter(|entry| entry.candidate.path_bytes == previous.path_bytes)
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return match exact.as_slice() {
            [entry]
                if entry.is_selectable() && same_reconnect_identity(previous, &entry.candidate) =>
            {
                Some(&entry.candidate)
            }
            _ => None,
        };
    }

    let mut matches = entries.iter().filter(|entry| {
        entry.is_selectable() && same_reconnect_identity(previous, &entry.candidate)
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(&first.candidate)
}

fn nonempty_candidate_serial(candidate: &DeviceCandidate) -> Option<&str> {
    candidate.serial().filter(|serial| !serial.is_empty())
}

fn same_selector_identity(active: &DeviceCandidate, candidate: &DeviceCandidate) -> bool {
    if (active.vid, active.pid) != (candidate.vid, candidate.pid) {
        return false;
    }

    match (
        nonempty_candidate_serial(active),
        nonempty_candidate_serial(candidate),
    ) {
        (Some(left), Some(right)) => {
            left == right
                && active.interface_number == candidate.interface_number
                && active.usage_page == candidate.usage_page
                && active.usage == candidate.usage
        }
        _ => active.path_bytes == candidate.path_bytes,
    }
}

fn same_reconnect_identity(previous: &DeviceCandidate, candidate: &DeviceCandidate) -> bool {
    nonempty_candidate_serial(previous) == nonempty_candidate_serial(candidate)
        && nonempty_candidate_serial(previous).is_some()
        && previous.vid == candidate.vid
        && previous.pid == candidate.pid
        && previous.interface_number == candidate.interface_number
        && previous.usage_page == candidate.usage_page
        && previous.usage == candidate.usage
}

pub fn classify_runtime_candidates(
    candidates: &[DeviceCandidate],
    catalog: &ProfileCatalog,
) -> Vec<PickerEntry> {
    let mut entries: Vec<_> = candidates
        .iter()
        .map(|candidate| classify_runtime_candidate(candidate, catalog))
        .collect();

    for index in 0..candidates.len() {
        if entries[index].status == CandidateStatus::Unsupported {
            continue;
        }
        let candidate = &candidates[index];
        let Some(profile) = catalog.find(candidate.vid, candidate.pid) else {
            continue;
        };
        let peers: Vec<_> = candidates
            .iter()
            .enumerate()
            .filter(|(peer_index, peer)| {
                *peer_index != index
                    && matches_runtime_transport(peer, profile)
                    && match (&candidate.serial_number, &peer.serial_number) {
                        (Some(left), Some(right)) => left == right,
                        (None, None) => true,
                        _ => true,
                    }
            })
            .map(|(_, peer)| peer)
            .collect();
        if !peers.is_empty() {
            entries[index].status = CandidateStatus::Ambiguous;
            let mut paths = peers
                .iter()
                .map(|peer| path_context(&peer.path_bytes))
                .collect::<Vec<_>>();
            paths.push(path_context(&candidate.path_bytes));
            entries[index].diagnostic = format!(
                "ambiguous HID identity for {:04x}:{:04x}; matching paths: {}",
                candidate.vid,
                candidate.pid,
                paths.join(", ")
            );
        }
    }
    entries
}

fn matches_runtime_transport(candidate: &DeviceCandidate, entry: &RuntimeEntry) -> bool {
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

fn classify_runtime_candidate(
    candidate: &DeviceCandidate,
    catalog: &ProfileCatalog,
) -> PickerEntry {
    let Some(entry) = catalog.find(candidate.vid, candidate.pid) else {
        return PickerEntry {
            candidate: candidate.clone(),
            status: CandidateStatus::Unsupported,
            profile_id: None,
            profile_name: candidate
                .product()
                .unwrap_or("Unknown Antelope device")
                .to_owned(),
            diagnostic: format!(
                "unsupported Antelope HID product {:04x}:{:04x}",
                candidate.vid, candidate.pid
            ),
        };
    };
    let profile = &entry.profile;
    let transport = &profile.transport;
    let mut status = match entry.readiness {
        RuntimeReadiness::Supported => CandidateStatus::Supported,
        RuntimeReadiness::Partial => CandidateStatus::Partial,
        RuntimeReadiness::Unverified => CandidateStatus::Unverified,
        RuntimeReadiness::Disabled => CandidateStatus::Disabled,
    };
    let mut diagnostic = if status.is_selectable() {
        String::new()
    } else if entry.support_reason.is_empty() {
        format!("{} is {status}", profile.identity.name)
    } else {
        format!("{}: {}", status, entry.support_reason)
    };
    if entry.driver_kind == RuntimeDriverKind::None && status.is_selectable() {
        status = CandidateStatus::Disabled;
        diagnostic = format!("{} has no selectable runtime driver", profile.identity.name);
    }
    let mismatch = if candidate.interface_number < 0 {
        Some("interface number unavailable".to_owned())
    } else if transport
        .expected_interface_number
        .is_some_and(|expected| candidate.interface_number != expected)
    {
        Some(format!(
            "expected control interface {}, found {}",
            transport.expected_interface_number.unwrap(),
            candidate.interface_number
        ))
    } else if transport
        .expected_usage_page
        .is_some_and(|expected| candidate.usage_page != expected)
    {
        Some(format!(
            "expected control usage page 0x{:04x}, found 0x{:04x}",
            transport.expected_usage_page.unwrap(),
            candidate.usage_page
        ))
    } else if transport
        .expected_usage
        .is_some_and(|expected| candidate.usage != expected)
    {
        Some(format!(
            "expected control usage 0x{:04x}, found 0x{:04x}",
            transport.expected_usage.unwrap(),
            candidate.usage
        ))
    } else {
        None
    };
    if let Some(mismatch) = mismatch {
        status = CandidateStatus::Ambiguous;
        diagnostic = mismatch;
    }
    PickerEntry {
        candidate: candidate.clone(),
        status,
        profile_id: Some(entry.id.clone()),
        profile_name: profile.identity.name.clone(),
        diagnostic,
    }
}

pub struct DeviceSession {
    controller: Controller,
    candidate: Option<DeviceCandidate>,
    entry: RuntimeEntry,
}

impl DeviceSession {
    pub fn discover(catalog: &ProfileCatalog) -> Result<Vec<DeviceCandidate>> {
        let api = HidApi::new()?;
        let candidates = api
            .device_list()
            .filter(|device| device.vendor_id() == ANTELOPE_VID)
            .map(DeviceCandidate::from_device_info)
            .collect::<Vec<_>>();
        let picker = DevicePickerState::new(candidates, catalog);
        Ok(picker
            .entries
            .into_iter()
            .map(|entry| entry.candidate)
            .collect())
    }

    pub fn open_candidate(candidate: &DeviceCandidate, catalog: &ProfileCatalog) -> Result<Self> {
        let picker = DevicePickerState::new(vec![candidate.clone()], catalog);
        let picker_entry = picker.entries().first().expect("one candidate");
        if !picker_entry.is_selectable() {
            bail!(
                "refusing {} candidate {}: {}",
                picker_entry.status,
                path_context(&candidate.path_bytes),
                picker_entry.diagnostic
            );
        }
        let entry = catalog
            .find(candidate.vid, candidate.pid)
            .expect("selectable candidate has catalog entry")
            .clone();
        let driver = driver_for_entry(&entry)?;
        // Driver construction intentionally precedes HID creation/opening.
        let hid = HidTransport::open_path_for_entry(candidate, &entry)?;
        let transport = Box::new(ThreadedTransport::spawn(hid));
        let controller = Controller::new_for_entry(transport, driver, &entry)?;
        Ok(Self {
            controller,
            candidate: Some(candidate.clone()),
            entry,
        })
    }

    pub fn open_mock() -> Result<Self> {
        let catalog = ProfileCatalog::builtin();
        let entry = catalog
            .find(0x23e5, 0xa015)
            .expect("built-in Zen Go profile")
            .clone();
        let controller = Controller::new_for_entry(
            Box::new(MockTransport::default()),
            Box::new(ZenGoDriver::new(entry.profile().clone()).map_err(|error| anyhow!(error))?),
            &entry,
        )?;
        Ok(Self {
            controller,
            candidate: None,
            entry,
        })
    }

    pub fn controller(&self) -> &Controller {
        &self.controller
    }

    pub fn controller_mut(&mut self) -> &mut Controller {
        &mut self.controller
    }

    pub fn candidate(&self) -> Option<&DeviceCandidate> {
        self.candidate.as_ref()
    }

    pub fn entry(&self) -> &RuntimeEntry {
        &self.entry
    }

    pub fn device_name(&self) -> &str {
        &self.entry.profile.identity.name
    }

    pub fn driver_definition(&self) -> &DriverDefinition {
        self.controller.driver_definition()
    }
}

pub struct RuntimeDeviceState {
    catalog: ProfileCatalog,
    selection: Option<DeviceSelection>,
    picker: DevicePickerState,
    session: Option<DeviceSession>,
    selector: Option<DevicePickerState>,
    selector_active: Option<DeviceCandidate>,
}

impl RuntimeDeviceState {
    pub fn new(catalog: ProfileCatalog, selection: Option<DeviceSelection>) -> Result<Self> {
        let candidates = DeviceSession::discover(&catalog)?;
        let picker = DevicePickerState::new(candidates, &catalog);
        let mut state = Self {
            catalog,
            selection,
            picker,
            session: None,
            selector: None,
            selector_active: None,
        };
        state.open_resolved_selection()?;
        Ok(state)
    }

    pub fn mock(catalog: ProfileCatalog) -> Result<Self> {
        let picker = DevicePickerState::new(Vec::new(), &ProfileCatalog::builtin());
        Ok(Self {
            catalog,
            selection: None,
            picker,
            session: Some(DeviceSession::open_mock()?),
            selector: None,
            selector_active: None,
        })
    }

    pub fn catalog(&self) -> &ProfileCatalog {
        &self.catalog
    }

    pub fn picker(&self) -> &DevicePickerState {
        &self.picker
    }

    pub fn picker_mut(&mut self) -> &mut DevicePickerState {
        &mut self.picker
    }

    pub fn set_picker_notice(&mut self, notice: impl Into<String>) {
        self.picker.set_notice(notice);
    }

    pub fn selector(&self) -> Option<&DevicePickerState> {
        self.selector.as_ref()
    }

    pub fn selector_mut(&mut self) -> Option<&mut DevicePickerState> {
        self.selector.as_mut()
    }

    pub fn open_selector(&mut self) -> Result<()> {
        let active = self
            .session
            .as_ref()
            .and_then(|session| session.candidate().cloned());
        self.open_selector_for(active)
    }

    pub fn open_selector_for(&mut self, active: Option<DeviceCandidate>) -> Result<()> {
        let candidates = DeviceSession::discover(&self.catalog)?;
        let mut picker = DevicePickerState::new(candidates, &self.catalog);
        picker.set_active_candidate(active.clone());
        self.selector_active = active;
        self.selector = Some(picker);
        Ok(())
    }

    /// Refresh selector rows without resolving startup selections or changing the session.
    pub fn refresh_selector(&mut self) -> Result<()> {
        self.refresh_selector_with(DeviceSession::discover)
    }

    fn refresh_selector_with<F>(&mut self, discover: F) -> Result<()>
    where
        F: FnOnce(&ProfileCatalog) -> Result<Vec<DeviceCandidate>>,
    {
        if self.selector.is_none() {
            return Ok(());
        }
        let selected = self
            .selector
            .as_ref()
            .and_then(DevicePickerState::selected_candidate)
            .cloned();
        // Record the attempt before discovery so failures cannot trigger a tight
        // retry loop in the selector's 500 ms refresh window.
        self.selector
            .as_mut()
            .expect("selector exists")
            .mark_discovery_attempt();
        let candidates = discover(&self.catalog)?;
        let mut picker = DevicePickerState::new(candidates, &self.catalog);
        picker.set_active_candidate(self.selector_active.clone());
        if let Some(selected) = selected {
            picker.select_candidate(&selected);
        }
        self.selector = Some(picker);
        Ok(())
    }

    pub fn selector_selected(&self) -> Option<DeviceCandidate> {
        self.selector
            .as_ref()
            .and_then(DevicePickerState::activate_selected)
            .cloned()
    }

    pub fn selector_selected_is_active(&self) -> bool {
        self.selector.as_ref().is_some_and(|picker| {
            picker
                .selected_candidate()
                .is_some_and(|candidate| picker.is_active(candidate))
        })
    }

    pub fn close_selector(&mut self) {
        self.selector = None;
        self.selector_active = None;
    }

    pub fn switch_to(&mut self, candidate: DeviceCandidate) -> Result<()> {
        let old = self.session.take();
        drop(old);
        self.close_selector();
        // A manual choice supersedes only the startup selector; future reconnects use
        // the explicitly selected session rather than reopening the CLI target.
        self.selection = None;
        self.session = Some(DeviceSession::open_candidate(&candidate, &self.catalog)?);
        Ok(())
    }

    pub fn session(&self) -> Option<&DeviceSession> {
        self.session.as_ref()
    }

    pub fn session_mut(&mut self) -> Option<&mut DeviceSession> {
        self.session.as_mut()
    }

    pub fn take_session(&mut self) -> Option<DeviceSession> {
        self.session.take()
    }

    pub fn open_selected(&mut self) -> Result<bool> {
        let Some(candidate) = self.picker.activate_selected().cloned() else {
            return Ok(false);
        };
        self.session = Some(DeviceSession::open_candidate(&candidate, &self.catalog)?);
        Ok(true)
    }

    pub fn rediscover(&mut self) -> Result<()> {
        if self.selector.is_some() {
            return self.refresh_selector();
        }
        let candidates = DeviceSession::discover(&self.catalog)?;
        self.picker = DevicePickerState::new(candidates, &self.catalog);
        if let Some(selection) = &self.selection {
            let matching = self
                .picker
                .entries()
                .iter()
                .filter(|entry| selection.match_candidate(&entry.candidate).is_some())
                .count();
            if matching == 1 {
                self.open_resolved_selection()?;
            }
        }
        Ok(())
    }

    /// Drop the active controller/worker before any replacement discovery or open.
    pub fn disconnect_and_rediscover(&mut self) -> Result<()> {
        let previous = self
            .session
            .take()
            .and_then(|session| session.candidate().cloned());
        self.disconnect_and_rediscover_from(previous)
    }

    pub fn disconnect_and_rediscover_from(
        &mut self,
        previous: Option<DeviceCandidate>,
    ) -> Result<()> {
        self.close_selector();
        // `session` is dropped by the caller before discovery creates a new HIDAPI handle.
        let candidates = DeviceSession::discover(&self.catalog)?;
        self.picker = DevicePickerState::new(candidates, &self.catalog);
        let Some(previous) = previous else {
            return Ok(());
        };
        let replacement = select_reconnect_candidate(&previous, self.picker.entries()).cloned();
        if let Some(candidate) = replacement {
            self.session = Some(DeviceSession::open_candidate(&candidate, &self.catalog)?);
        }
        Ok(())
    }

    fn open_resolved_selection(&mut self) -> Result<()> {
        if let Some(selection) = &self.selection {
            let candidates = self
                .picker
                .entries()
                .iter()
                .map(|entry| entry.candidate.clone())
                .collect::<Vec<_>>();
            let candidate = select_candidate(&candidates, selection)?.clone();
            let entry = self
                .picker
                .entries()
                .iter()
                .find(|entry| entry.candidate.path_bytes == candidate.path_bytes)
                .expect("selected candidate has picker entry");
            if !entry.is_selectable() {
                bail!(
                    "selected device is {} and cannot be opened: {}",
                    entry.status,
                    entry.diagnostic
                );
            }
            self.session = Some(DeviceSession::open_candidate(&candidate, &self.catalog)?);
        } else if self.picker.selectable_count() == 1 {
            let candidate = self
                .picker
                .entries()
                .iter()
                .find(|entry| entry.is_selectable())
                .expect("one selectable entry")
                .candidate
                .clone();
            self.session = Some(DeviceSession::open_candidate(&candidate, &self.catalog)?);
        }
        Ok(())
    }
}

pub fn replace_session<F>(old: DeviceSession, open: F) -> Result<DeviceSession>
where
    F: FnOnce() -> Result<DeviceSession>,
{
    drop(old);
    open()
}

pub fn builtin_zen_go_driver() -> Result<ZenGoDriver> {
    let catalog = ProfileCatalog::builtin();
    let entry = catalog
        .find(0x23e5, 0xa015)
        .expect("built-in Zen Go profile");
    ZenGoDriver::new(entry.profile().clone()).map_err(|error| anyhow!(error))
}

fn driver_for_entry(entry: &RuntimeEntry) -> Result<Box<dyn DeviceDriver>> {
    if entry.readiness != RuntimeReadiness::Supported {
        bail!(
            "profile {} is {:?} and cannot be opened",
            entry.profile.identity.name,
            entry.readiness
        );
    }
    match entry.driver_kind {
        RuntimeDriverKind::ZenGo => Ok(Box::new(
            ZenGoDriver::new(entry.profile().clone()).map_err(|error| anyhow!(error))?,
        )),
        RuntimeDriverKind::Profile => Ok(Box::new(
            ProfileDriver::new(entry.clone()).map_err(|error| anyhow!(error))?,
        )),
        RuntimeDriverKind::None => bail!(
            "profile {} has no selectable runtime driver",
            entry.profile.identity.name
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{driver_for_entry, DevicePickerState, RuntimeDeviceState};
    use crate::device::ProfileCatalog;
    use antelope_protocol::{RuntimeDriverKind, RuntimeReadiness};
    use std::time::{Duration, Instant};

    #[test]
    fn profile_driver_mapping_constructs_profile_driver_for_supported_orion() {
        let catalog = ProfileCatalog::builtin();
        let entry = catalog.find(0x23e5, 0xa221).expect("Orion profile");
        assert_eq!(entry.readiness, RuntimeReadiness::Supported);
        assert_eq!(entry.driver_kind, RuntimeDriverKind::Profile);

        let driver = driver_for_entry(entry).expect("Profile driver mapping");
        assert_eq!(driver.definition().id, "orion_studio_3");
    }

    #[test]
    fn selector_refresh_failure_starts_new_retry_window() {
        let catalog = ProfileCatalog::builtin();
        let picker = DevicePickerState::new(Vec::new(), &catalog);
        let mut state = RuntimeDeviceState {
            catalog: catalog.clone(),
            selection: None,
            picker: DevicePickerState::new(Vec::new(), &catalog),
            session: None,
            selector: Some(picker),
            selector_active: None,
        };
        state.selector.as_mut().expect("selector").last_discovery_at =
            Instant::now() - Duration::from_secs(1);
        let before_attempt = Instant::now();

        let error = state
            .refresh_selector_with(|_| Err(anyhow::anyhow!("discovery failed")))
            .expect_err("discovery failure");

        assert_eq!(error.to_string(), "discovery failed");
        let picker = state.selector().expect("selector remains open");
        assert!(picker.last_discovery_at >= before_attempt);
        assert!(picker.last_discovery_at.elapsed() < picker.retry_after);
    }
}
