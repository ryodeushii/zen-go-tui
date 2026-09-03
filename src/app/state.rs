use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use antelope_protocol::{
    ClockSource, DeviceMetadata, DynamicInputState, DynamicMixerSurface, DynamicOutputState,
    GlobalControl, InputAddress, InputControl, MixerAddress, MixerChannelState, MixerControl,
    OutputAddress, OutputControl, OutputMode, OutputState, OutputTarget, PreampState,
    RuntimeDriverKind, RuntimeEntry, RuntimeInputControlKind, RuntimeProfile, RuntimeReadiness,
    SampleRate, Surface,
};

use super::types::{FocusArea, PeakHoldDuration, RawMapScope, RawPacketTab, RefreshRate};
use super::{AssignmentPickerState, ProfileEditorState, QueryReplyLogEntry, SelectorPopupState};

/// Device connection and status tracking.
#[derive(Debug, Clone)]
pub struct DeviceStatus {
    pub sample_rate: Option<SampleRate>,
    pub sample_rate_hz: Option<u32>,
    pub clock_source: Option<ClockSource>,
    pub lock_known: bool,
    pub locked: Option<bool>,
    pub metadata: Option<DeviceMetadata>,
    pub startup_query_summaries: [Option<String>; 3],
    pub last_refresh_summary: String,
}

impl Default for DeviceStatus {
    fn default() -> Self {
        Self {
            sample_rate: None,
            sample_rate_hz: None,
            clock_source: None,
            lock_known: false,
            locked: None,
            metadata: None,
            startup_query_summaries: [None, None, None],
            last_refresh_summary: "waiting for device snapshot".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConnectionState {
    pub connected: bool,
    pub last_snapshot_at: Option<Instant>,
    pub last_frame_type: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct AppSettings {
    pub refresh_rate: RefreshRate,
    pub peak_threshold_raw: u8,
    pub peak_enabled: bool,
    pub peak_hold_duration: PeakHoldDuration,
    pub auto_save: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            refresh_rate: RefreshRate::default(),
            peak_threshold_raw: PEAK_THRESHOLD_RAW,
            peak_enabled: true,
            peak_hold_duration: PeakHoldDuration::default(),
            auto_save: false,
        }
    }
}

impl AppSettings {
    pub fn peak_threshold_db(&self) -> i16 {
        match self.peak_threshold_raw {
            0x00 => 0,
            0x01 => -1,
            0x02 => -2,
            0x03 => -3,
            0x04 => -4,
            0x05 => -5,
            0x06 => -6,
            0x0a => -10,
            0x0f => -15,
            0x14 => -20,
            _ => -3,
        }
    }
}

/// Device connection and status tracking.
#[derive(Debug, Clone, Default)]
pub struct DeviceState {
    pub status: DeviceStatus,
    pub connection: ConnectionState,
    pub dsp_cluster: [u8; 4],
}

/// One profile-owned input address space. String IDs remain display/catalog identifiers;
/// mutations use `space_id` plus each input's numeric index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSpaceState {
    pub id: String,
    pub space_id: u16,
    pub name: String,
    pub inputs: Vec<DynamicInputState>,
}

/// Mixer surface state. Dynamic surfaces are authoritative; `channels` mirrors strips for
/// existing Zen Go rendering until Task 4 migrates widgets.
#[derive(Debug, Clone)]
pub struct MixerState {
    pub surface: Surface,
    pub surface_index: usize,
    pub surfaces: Vec<DynamicMixerSurface>,
    pub channels: Vec<Vec<MixerChannelState>>,
    pub selected_channel: usize,
    pub strip_scroll: usize,
    pub visible_strip_count: usize,
    pub peaks: Vec<Vec<Option<MeterPeak>>>,
}

/// Output state. Dynamic records are authoritative; `states` preserves Zen Go rendering.
#[derive(Debug, Clone)]
pub struct OutputData {
    pub dynamic: Vec<DynamicOutputState>,
    pub states: Vec<OutputState>,
    pub selected: usize,
}

/// Input selection and peak state remain separate from profile-owned storage.
#[derive(Debug, Clone)]
pub struct PreampData {
    pub state: PreampState,
    pub selected_input: usize,
    pub peaks: Vec<Option<MeterPeak>>,
}

impl Default for PreampData {
    fn default() -> Self {
        Self {
            state: PreampState::default(),
            selected_input: 0,
            peaks: vec![None; 2],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingGroupCapability {
    pub destination: u16,
    pub name: String,
    pub channel_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiInputCapability {
    pub kind: RuntimeInputControlKind,
    pub parameter: String,
    pub parameter_id: Option<u16>,
    pub label: String,
    pub control: Option<InputControl>,
}

/// Profile facts retained by UI. Capability sets are compiled once from canonical typed records;
/// observed `None` values never imply that a control is unsupported.
#[derive(Debug, Clone)]
pub struct UiProfileState {
    pub id: String,
    pub device_name: String,
    pub readiness: Option<RuntimeReadiness>,
    pub driver_kind: RuntimeDriverKind,
    pub support_reason: String,
    pub actionable: bool,
    input_controls: HashSet<(InputAddress, InputControl)>,
    input_capabilities: HashMap<InputAddress, Vec<UiInputCapability>>,
    output_controls: HashSet<(OutputAddress, OutputControl)>,
    mixer_controls: HashSet<(u8, MixerControl)>,
    link_surfaces: HashSet<u8>,
    global_controls: HashSet<GlobalControl>,
    routing_destinations: HashSet<u16>,
    routing_channel_counts: HashMap<u16, u16>,
}

impl UiProfileState {
    pub fn from_entry(entry: &RuntimeEntry) -> Self {
        let profile = &entry.profile;
        let actionable = entry.readiness.is_selectable()
            && !matches!(entry.driver_kind, RuntimeDriverKind::None);
        let confirmed = |name: &str| {
            profile
                .params
                .iter()
                .any(|param| param.name == name && param.status.eq_ignore_ascii_case("confirmed"))
        };

        let mut input_capabilities = HashMap::new();
        let mut input_controls = HashSet::new();
        for input in &profile.inputs {
            let address = InputAddress {
                space: input.space_id,
                index: input.index,
            };
            let Some(space) = profile
                .address_spaces
                .iter()
                .find(|space| space.space_id == input.space_id)
            else {
                continue;
            };
            let capabilities = space
                .input_capabilities
                .iter()
                .map(|capability| {
                    let control = match (capability.kind, capability.parameter.as_str()) {
                        (RuntimeInputControlKind::Gain, "gain") => Some(InputControl::Gain),
                        (RuntimeInputControlKind::Mode, "input_mode") => Some(InputControl::Mode),
                        (RuntimeInputControlKind::Phantom, "phantom") => {
                            Some(InputControl::Phantom)
                        }
                        (RuntimeInputControlKind::Phase, "phase_invert") => {
                            Some(InputControl::Phase)
                        }
                        _ => capability.parameter_id.map(InputControl::Parameter),
                    };
                    if let Some(control) = control {
                        input_controls.insert((address, control));
                    }
                    UiInputCapability {
                        kind: capability.kind,
                        parameter: capability.parameter.clone(),
                        parameter_id: capability.parameter_id,
                        label: capability.label.clone(),
                        control,
                    }
                })
                .collect();
            input_capabilities.insert(address, capabilities);
        }

        let mut output_kinds = Vec::new();
        for (name, control) in [
            ("bus_level", OutputControl::Level),
            ("bus_mute", OutputControl::Mute),
            ("bus_dim", OutputControl::Dim),
        ] {
            if confirmed(name) {
                output_kinds.push(control);
            }
        }
        let output_controls = profile
            .outputs
            .iter()
            .flat_map(|output| {
                output_kinds
                    .iter()
                    .copied()
                    .map(move |control| (OutputAddress { id: output.id }, control))
            })
            .collect();

        let mut mixer_kinds = Vec::new();
        for (name, control) in [
            ("mix_fader", MixerControl::Fader),
            ("mix_pan", MixerControl::Pan),
            ("mix_send", MixerControl::Send),
            ("mix_mute", MixerControl::Mute),
            ("mix_solo", MixerControl::Solo),
        ] {
            if confirmed(name) {
                mixer_kinds.push(control);
            }
        }
        let mixer_controls = profile
            .mixers
            .iter()
            .flat_map(|mixer| {
                mixer_kinds
                    .iter()
                    .copied()
                    .map(move |control| (mixer.mix_index, control))
            })
            .collect();
        let link_confirmed = confirmed("mix_link") || confirmed("mix_channel_link");
        let link_surfaces = profile
            .mixers
            .iter()
            .filter(|_| link_confirmed)
            .map(|mixer| mixer.mix_index)
            .collect();
        let global_controls = [
            ("sample_rate", GlobalControl::SampleRate),
            ("clock_source", GlobalControl::ClockSource),
        ]
        .into_iter()
        .filter(|(name, _)| confirmed(name))
        .map(|(_, control)| control)
        .collect();
        let routing_destinations = profile
            .routing_groups
            .iter()
            .filter(|group| {
                confirmed("routing_batch_marker")
                    || confirmed("mix_channel_link")
                    || !group.source_domains.is_empty()
            })
            .map(|group| group.destination)
            .collect();
        let routing_channel_counts = profile
            .routing_groups
            .iter()
            .map(|group| (group.destination, group.channel_count))
            .collect();

        Self {
            id: entry.id.clone(),
            device_name: profile.identity.name.clone(),
            readiness: Some(entry.readiness),
            driver_kind: entry.driver_kind,
            support_reason: entry.support_reason.clone(),
            actionable,
            input_controls,
            input_capabilities,
            output_controls,
            mixer_controls,
            link_surfaces,
            global_controls,
            routing_destinations,
            routing_channel_counts,
        }
    }

    pub fn compatibility(profile: &RuntimeProfile) -> Self {
        Self {
            id: String::new(),
            device_name: profile.identity.name.clone(),
            readiness: None,
            driver_kind: RuntimeDriverKind::None,
            support_reason: "readiness unavailable".into(),
            actionable: false,
            input_controls: HashSet::new(),
            input_capabilities: HashMap::new(),
            output_controls: HashSet::new(),
            mixer_controls: HashSet::new(),
            link_surfaces: HashSet::new(),
            global_controls: HashSet::new(),
            routing_destinations: HashSet::new(),
            routing_channel_counts: HashMap::new(),
        }
    }

    pub fn readiness_label(&self) -> &'static str {
        match self.readiness {
            Some(RuntimeReadiness::Supported) => "supported",
            Some(RuntimeReadiness::Partial) => "partial",
            Some(RuntimeReadiness::Unverified) => "unverified",
            Some(RuntimeReadiness::Disabled) => "disabled",
            None => "readiness unavailable",
        }
    }

    pub fn input_capabilities(&self, address: InputAddress) -> &[UiInputCapability] {
        self.input_capabilities
            .get(&address)
            .map_or(&[], Vec::as_slice)
    }

    pub fn declares_input(&self, address: InputAddress, control: InputControl) -> bool {
        self.input_controls.contains(&(address, control))
    }

    pub fn supports_input(&self, address: InputAddress, control: InputControl) -> bool {
        self.actionable && self.declares_input(address, control)
    }

    pub fn declares_output(&self, address: OutputAddress, control: OutputControl) -> bool {
        self.output_controls.contains(&(address, control))
    }

    pub fn supports_output(&self, address: OutputAddress, control: OutputControl) -> bool {
        self.actionable && self.declares_output(address, control)
    }

    pub fn declares_mixer(&self, surface: u8, control: MixerControl) -> bool {
        self.mixer_controls.contains(&(surface, control))
    }

    pub fn supports_mixer(&self, surface: u8, control: MixerControl) -> bool {
        self.actionable && self.declares_mixer(surface, control)
    }

    pub fn declares_link(&self, surface: u8) -> bool {
        self.link_surfaces.contains(&surface)
    }

    pub fn supports_link(&self, surface: u8) -> bool {
        self.actionable && self.declares_link(surface)
    }

    pub fn supports_global(&self, control: GlobalControl) -> bool {
        self.actionable && self.global_controls.contains(&control)
    }

    pub fn supports_routing(&self, destination: u16) -> bool {
        self.actionable && self.routing_destinations.contains(&destination)
    }

    pub fn supports_any_routing(&self) -> bool {
        self.actionable && !self.routing_destinations.is_empty()
    }

    pub fn supports_assignment(&self, surface: u8, strip: u16) -> bool {
        self.actionable
            && self.routing_destinations.contains(&u16::from(surface))
            && self
                .routing_channel_counts
                .get(&u16::from(surface))
                .is_some_and(|count| strip > 0 && strip <= *count)
    }
}

impl Default for UiProfileState {
    fn default() -> Self {
        let mut input_controls = HashSet::new();
        for index in 0..2 {
            for control in [
                InputControl::Mode,
                InputControl::Gain,
                InputControl::Phantom,
                InputControl::Phase,
            ] {
                input_controls.insert((InputAddress { space: 0, index }, control));
            }
        }
        let mut output_controls = HashSet::new();
        for id in 0..3 {
            for control in [
                OutputControl::Level,
                OutputControl::Mute,
                OutputControl::Dim,
            ] {
                output_controls.insert((OutputAddress { id }, control));
            }
        }
        let mut mixer_controls = HashSet::new();
        for surface in 0..2 {
            for control in [
                MixerControl::Fader,
                MixerControl::Pan,
                MixerControl::Mute,
                MixerControl::Solo,
            ] {
                mixer_controls.insert((surface, control));
            }
        }
        Self {
            id: "legacy_zen_go".into(),
            device_name: "ZEN GO SYNERGY CORE".into(),
            readiness: Some(RuntimeReadiness::Supported),
            driver_kind: RuntimeDriverKind::ZenGo,
            support_reason: "validated built-in driver".into(),
            actionable: true,
            input_controls,
            input_capabilities: HashMap::new(),
            output_controls,
            mixer_controls,
            link_surfaces: [0, 1].into_iter().collect(),
            global_controls: [GlobalControl::SampleRate, GlobalControl::ClockSource]
                .into_iter()
                .collect(),
            routing_destinations: [0].into_iter().collect(),
            routing_channel_counts: [(0, 16)].into_iter().collect(),
        }
    }
}

/// UI navigation, messaging, and settings.
#[derive(Debug, Clone)]
pub struct UiState {
    pub focus: FocusArea,
    pub last_message: String,
    pub settings: AppSettings,
    pub quit_requested: bool,
}

/// Popup and overlay state — mutually exclusive overlays.
#[derive(Debug, Clone, Default)]
pub struct PopupState {
    pub hotkeys_open: bool,
    pub options_open: bool,
    pub routing_open: bool,
    pub profiles_open: bool,
    pub raw_view_open: bool,
    pub assignment_picker: Option<AssignmentPickerState>,
    pub assignment_picker_address: Option<MixerAddress>,
    pub selector_popup: Option<SelectorPopupState>,
    pub profile_names: Vec<String>,
    pub profile_editor: Option<ProfileEditorState>,
    pub selected_index: usize,
}

/// Raw packet debug view state — buffers, baselines, query logs.
#[derive(Debug, Clone)]
pub struct RawViewState {
    pub selected_tab: RawPacketTab,
    pub raw_map_scope: RawMapScope,
    pub raw_dump_scroll: usize,
    pub raw_map_scroll: usize,
    pub latest_raw_73: Option<Vec<u8>>,
    pub latest_raw_83: Option<Vec<u8>>,
    pub latest_raw_74: Option<Vec<u8>>,
    pub latest_raw_75: Option<Vec<u8>>,
    pub latest_raw_81: Option<Vec<u8>>,
    pub baseline_raw_73: Option<Vec<u8>>,
    pub baseline_raw_83: Option<Vec<u8>>,
    pub baseline_raw_74: Option<Vec<u8>>,
    pub baseline_raw_75: Option<Vec<u8>>,
    pub baseline_raw_81: Option<Vec<u8>>,
    pub recent_query_request_log: Vec<String>,
    pub recent_query_reply_log: Vec<String>,
    pub recent_query_reply_entries: Vec<QueryReplyLogEntry>,
    pub selected_query_reply_entry: Option<usize>,
    pub query_reply_scroll: usize,
    pub last_auxiliary_len: Option<usize>,
}

pub const MIXER_STRIP_PAGE_SIZE: usize = 8;
pub const QUERY_REPLY_VISIBLE_COUNT: usize = 8;
pub const PEAK_HOLD_DURATION: Duration = Duration::from_secs(3);
pub const PEAK_THRESHOLD_RAW: u8 = 0x03;

/// Tracks a detected peak level for meter displays.
#[derive(Debug, Clone, Copy)]
pub struct MeterPeak {
    /// Raw meter byte value at peak (0x00–0x60).
    pub raw: u8,
    /// When the peak was detected.
    pub detected_at: Instant,
}

impl MeterPeak {
    pub fn is_active(&self) -> bool {
        self.detected_at.elapsed() < PEAK_HOLD_DURATION
    }
}

impl Default for MixerState {
    fn default() -> Self {
        let channels: Vec<Vec<_>> = (0..2)
            .map(|_| (1..=16).map(MixerChannelState::unknown).collect())
            .collect();
        let surfaces = (0..2)
            .map(|surface| DynamicMixerSurface {
                surface,
                name: format!("Mix {}", surface + 1),
                master: None,
                strips: (1..=16)
                    .map(|strip| antelope_protocol::DynamicMixerStrip {
                        strip,
                        name: format!("CH {strip:02}"),
                        fader: None,
                        pan: Some(0x20),
                        send: None,
                        muted: None,
                        soloed: None,
                        linked: None,
                        meter: None,
                        parameters: Vec::new(),
                    })
                    .collect(),
            })
            .collect();
        Self {
            surface: Surface::MonitorHp1,
            surface_index: 0,
            surfaces,
            peaks: channels
                .iter()
                .map(|surface| vec![None; surface.len()])
                .collect(),
            channels,
            selected_channel: 0,
            strip_scroll: 0,
            visible_strip_count: MIXER_STRIP_PAGE_SIZE,
        }
    }
}

impl Default for OutputData {
    fn default() -> Self {
        Self {
            dynamic: vec![
                DynamicOutputState {
                    address: antelope_protocol::OutputAddress { id: 0 },
                    name: "Monitor".into(),
                    level: Some(0),
                    muted: Some(false),
                    dimmed: Some(false),
                    parameters: Vec::new(),
                },
                DynamicOutputState {
                    address: antelope_protocol::OutputAddress { id: 1 },
                    name: "HP 1".into(),
                    level: Some(0),
                    muted: Some(false),
                    dimmed: Some(false),
                    parameters: Vec::new(),
                },
                DynamicOutputState {
                    address: antelope_protocol::OutputAddress { id: 2 },
                    name: "HP 2".into(),
                    level: Some(0),
                    muted: Some(false),
                    dimmed: Some(false),
                    parameters: Vec::new(),
                },
            ],
            states: vec![
                OutputState::new(OutputTarget::Monitor, 0, OutputMode::Normal),
                OutputState::new(OutputTarget::Hp1, 0, OutputMode::Normal),
                OutputState::new(OutputTarget::Hp2, 0, OutputMode::Normal),
            ],
            selected: 0,
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            focus: FocusArea::Outputs,
            last_message:
                "Press ? for help. Device state is authoritative where decoding is confirmed."
                    .to_string(),
            settings: AppSettings::default(),
            quit_requested: false,
        }
    }
}

impl Default for RawViewState {
    fn default() -> Self {
        Self {
            selected_tab: RawPacketTab::State73,
            raw_map_scope: RawMapScope::All,
            raw_dump_scroll: 0,
            raw_map_scroll: 0,
            latest_raw_73: None,
            latest_raw_83: None,
            latest_raw_74: None,
            latest_raw_75: None,
            latest_raw_81: None,
            baseline_raw_73: None,
            baseline_raw_83: None,
            baseline_raw_74: None,
            baseline_raw_75: None,
            baseline_raw_81: None,
            recent_query_request_log: Vec::new(),
            recent_query_reply_log: Vec::new(),
            recent_query_reply_entries: Vec::new(),
            selected_query_reply_entry: None,
            query_reply_scroll: 0,
            last_auxiliary_len: None,
        }
    }
}

impl RawViewState {
    pub fn reset_raw_view_scroll(&mut self) {
        self.raw_dump_scroll = 0;
        self.raw_map_scroll = 0;
    }

    pub fn select_tab(&mut self, tab: RawPacketTab) {
        self.selected_tab = tab;
        if !RawMapScope::options_for(tab).contains(&self.raw_map_scope) {
            self.raw_map_scope = RawMapScope::All;
        }
        self.reset_raw_view_scroll();
    }

    pub fn select_scope(&mut self, scope: RawMapScope) {
        if RawMapScope::options_for(self.selected_tab).contains(&scope) {
            self.raw_map_scope = scope;
            self.reset_raw_view_scroll();
        }
    }

    pub fn cycle_scope(&mut self, forward: bool) {
        self.raw_map_scope = self.raw_map_scope.next_for(self.selected_tab, forward);
        self.reset_raw_view_scroll();
    }

    pub fn scroll_raw_view(&mut self, increase: bool, page: bool) {
        let amount = if page { 10 } else { 1 };
        if increase {
            self.raw_dump_scroll = self.raw_dump_scroll.saturating_add(amount);
            self.raw_map_scroll = self.raw_map_scroll.saturating_add(amount);
        } else {
            self.raw_dump_scroll = self.raw_dump_scroll.saturating_sub(amount);
            self.raw_map_scroll = self.raw_map_scroll.saturating_sub(amount);
        }
    }
}
