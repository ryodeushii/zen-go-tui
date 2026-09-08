use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use antelope_protocol::{
    AuraVerbState, DeviceMetadata, DynamicInputState, DynamicMixerSurface, DynamicOutputState,
    GlobalControl, InputAddress, InputControl, MixerAddress, MixerChannelState, MixerControl,
    OutputAddress, OutputControl, OutputMode, OutputState, OutputTarget, OutputTrimAddress,
    PreampState, ProfileDriver, RoutingSource, RuntimeDriverKind, RuntimeEntry,
    RuntimeInputControlKind, RuntimeLinkDomainKind, RuntimeProfile, RuntimeReadiness, SampleRate,
    Surface, SurroundGlobalState,
};

use super::types::{
    AuraVerbControlFocus, FocusArea, PeakHoldDuration, RawMapScope, RawPacketTab, RawViewMode,
    RefreshRate, SurroundControlFocus, UiPage,
};
use super::{
    AssignmentPickerState, ProfileEditorState, QueryReplyLogEntry, RoutingEditorState,
    RoutingSourcePickerState, SelectorPopupState,
};
use crate::traffic::{
    FilteredTrafficView, TrafficCounters, TrafficDirection, TrafficFilter, TrafficJournal,
    TrafficQueryHead, TrafficSelectionMove, TrafficSequence,
};

/// Device connection and status tracking.
#[derive(Debug, Clone)]
pub struct DeviceStatus {
    pub sample_rate: Option<SampleRate>,
    pub sample_rate_hz: Option<u32>,
    /// Current profile-defined raw clock-source enum value.
    pub clock_source: Option<i32>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuraVerbFreshness {
    AwaitingReadback,
    Authoritative,
    PendingReadback,
    Stale,
}

#[derive(Debug, Clone)]
pub struct AuraVerbCache {
    /// Latest confirmed Mix-1 device state; never replaced by delayed unmatched replies.
    pub state: Option<AuraVerbState>,
    /// Complete state expected from the latest successful whole-frame write.
    pub pending_expected: Option<AuraVerbState>,
    pub freshness: AuraVerbFreshness,
}

impl Default for AuraVerbCache {
    fn default() -> Self {
        Self {
            state: None,
            pending_expected: None,
            freshness: AuraVerbFreshness::AwaitingReadback,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurroundFreshness {
    AwaitingReadback,
    Authoritative,
    PendingReadback,
    Stale,
}

#[derive(Debug, Clone)]
pub struct SurroundGlobalCache {
    /// Latest confirmed device state; never replaced by an unmatched delayed reply.
    pub state: Option<SurroundGlobalState>,
    /// Complete meaningful state expected from the latest successful write.
    pub pending_expected: Option<SurroundGlobalState>,
    pub freshness: SurroundFreshness,
}

impl Default for SurroundGlobalCache {
    fn default() -> Self {
        Self {
            state: None,
            pending_expected: None,
            freshness: SurroundFreshness::AwaitingReadback,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SurroundSpeakerEqRecordCache {
    pub state: Option<antelope_protocol::SurroundSpeakerEqState>,
    pub freshness: SurroundFreshness,
}

impl Default for SurroundSpeakerEqRecordCache {
    fn default() -> Self {
        Self {
            state: None,
            freshness: SurroundFreshness::AwaitingReadback,
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
    pub kind: String,
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
pub struct RoutingSourceChoice {
    pub source: RoutingSource,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockSourceChoice {
    pub value: i32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiInputCapability {
    pub kind: RuntimeInputControlKind,
    pub parameter: String,
    pub parameter_id: Option<u16>,
    pub label: String,
    pub control: Option<InputControl>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiInputLinkTarget {
    pub protocol_space: u8,
    pub pair: u16,
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
    input_link_domains: HashMap<u16, (u8, u16)>,
    parameter_values: HashMap<String, HashMap<i32, String>>,
    output_trim_targets: HashMap<u8, String>,
    clock_source_choices: Vec<ClockSourceChoice>,
    internal_clock_value: Option<i32>,
    output_controls: HashSet<(OutputAddress, OutputControl)>,
    mixer_controls: HashSet<(u8, MixerControl)>,
    link_surfaces: HashSet<u8>,
    global_controls: HashSet<GlobalControl>,
    routing_destinations: HashSet<u16>,
    routing_channel_counts: HashMap<u16, u16>,
    mixer_assignment_destinations: HashMap<u8, u16>,
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
        let settings_valid = ProfileDriver::supports_complete_settings_contract(profile);
        let talkback_valid = ProfileDriver::supports_complete_talkback_contract(profile);
        let parameter_values = profile
            .params
            .iter()
            .map(|parameter| {
                (
                    parameter.name.clone(),
                    parameter.values.iter().cloned().collect(),
                )
            })
            .collect();
        let clock_parameter = profile.params.iter().find(|parameter| {
            parameter.name == "clock_source"
                && parameter.id.is_some()
                && parameter.applies_to == "globals"
                && parameter.status.eq_ignore_ascii_case("confirmed")
        });
        let clock_source_choices = clock_parameter
            .map(|parameter| {
                parameter
                    .values
                    .iter()
                    .map(|(value, label)| ClockSourceChoice {
                        value: *value,
                        label: label.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let internal_clock_value = clock_parameter.and_then(|parameter| parameter.internal_value);

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

        let input_link_domains = profile
            .link_domains
            .iter()
            .filter_map(|domain| match domain.kind {
                RuntimeLinkDomainKind::Mixer => None,
                RuntimeLinkDomainKind::Spdif => profile
                    .address_spaces
                    .iter()
                    .find(|space| space.kind == "spdif_inputs")
                    .map(|space| (space.space_id, (domain.protocol_space, domain.pair_count))),
            })
            .collect();

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
        let mut output_controls: HashSet<_> = profile
            .outputs
            .iter()
            .flat_map(|output| {
                output_kinds
                    .iter()
                    .copied()
                    .map(move |control| (OutputAddress { id: output.id }, control))
            })
            .collect();
        if confirmed("bus_mono") {
            if let Some(targets) = profile.constraints.iter().find(|constraint| {
                constraint.name == "output_mono_targets"
                    && constraint.status.eq_ignore_ascii_case("confirmed")
            }) {
                for target in &targets.values {
                    if let Ok(id) = u16::try_from(*target) {
                        if profile.outputs.iter().any(|output| output.id == id) {
                            output_controls.insert((OutputAddress { id }, OutputControl::Mono));
                        }
                    }
                }
            }
        }

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
        let mut global_controls: HashSet<_> = [
            ("sample_rate", GlobalControl::SampleRate),
            ("clock_source", GlobalControl::ClockSource),
        ]
        .into_iter()
        .filter(|(name, _)| confirmed(name))
        .map(|(_, control)| control)
        .collect();
        if settings_valid {
            global_controls.insert(GlobalControl::Brightness);
        }
        let output_trim_targets: HashMap<u8, String> = profile
            .constraints
            .iter()
            .filter_map(|constraint| {
                let target = constraint
                    .name
                    .strip_prefix("output_trim_target.")?
                    .parse::<u8>()
                    .ok()?;
                (constraint.status.eq_ignore_ascii_case("confirmed")
                    && constraint.scalar == Some(i32::from(target))
                    && !constraint.text.trim().is_empty())
                .then(|| (target, constraint.text.clone()))
            })
            .collect();
        if settings_valid {
            for target in 0..=2 {
                global_controls.insert(GlobalControl::OutputTrim(OutputTrimAddress { target }));
            }
        }
        if talkback_valid {
            global_controls.extend([
                GlobalControl::TalkbackButton,
                GlobalControl::TalkbackSource,
                GlobalControl::TalkbackGain,
            ]);
        }
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
        let mixer_assignment_destinations = profile
            .routing_groups
            .iter()
            .filter_map(|group| {
                group
                    .mixer_surface
                    .map(|surface| (surface, group.destination))
            })
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
            input_link_domains,
            parameter_values,
            output_trim_targets,
            clock_source_choices,
            internal_clock_value,
            output_controls,
            mixer_controls,
            link_surfaces,
            global_controls,
            routing_destinations,
            routing_channel_counts,
            mixer_assignment_destinations,
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
            input_link_domains: HashMap::new(),
            parameter_values: HashMap::new(),
            output_trim_targets: HashMap::new(),
            clock_source_choices: Vec::new(),
            internal_clock_value: None,
            output_controls: HashSet::new(),
            mixer_controls: HashSet::new(),
            link_surfaces: HashSet::new(),
            global_controls: HashSet::new(),
            routing_destinations: HashSet::new(),
            routing_channel_counts: HashMap::new(),
            mixer_assignment_destinations: HashMap::new(),
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

    pub fn input_value_label(
        &self,
        address: InputAddress,
        control: InputControl,
        value: i32,
    ) -> Option<&str> {
        self.input_capabilities(address)
            .iter()
            .find(|capability| capability.control == Some(control))
            .and_then(|capability| self.parameter_values.get(&capability.parameter))
            .and_then(|values| values.get(&value))
            .map(String::as_str)
    }

    pub fn declares_input(&self, address: InputAddress, control: InputControl) -> bool {
        self.input_controls.contains(&(address, control))
    }

    pub fn supports_input(&self, address: InputAddress, control: InputControl) -> bool {
        self.actionable && self.declares_input(address, control)
    }

    pub fn input_link_target(&self, address: InputAddress) -> Option<UiInputLinkTarget> {
        let (protocol_space, pair_count) = *self.input_link_domains.get(&address.space)?;
        if !self
            .input_capabilities(address)
            .iter()
            .any(|capability| capability.kind == RuntimeInputControlKind::Link)
        {
            return None;
        }
        let pair = address.index / 2;
        (pair < pair_count).then_some(UiInputLinkTarget {
            protocol_space,
            pair,
        })
    }

    pub fn supports_input_link(&self, address: InputAddress) -> bool {
        self.actionable && self.input_link_target(address).is_some()
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

    pub fn supports_settings(&self) -> bool {
        self.supports_global(GlobalControl::Brightness)
            || (0..=2).any(|target| {
                self.supports_global(GlobalControl::OutputTrim(OutputTrimAddress { target }))
            })
            || self.supports_global(GlobalControl::TalkbackButton)
            || self.supports_global(GlobalControl::TalkbackSource)
            || self.supports_global(GlobalControl::TalkbackGain)
    }

    pub fn setting_rows(&self) -> Vec<GlobalControl> {
        let mut rows = Vec::new();
        if self.supports_global(GlobalControl::Brightness) {
            rows.push(GlobalControl::Brightness);
        }
        let mut targets = self.output_trim_targets.keys().copied().collect::<Vec<_>>();
        targets.sort_unstable();
        rows.extend(targets.into_iter().filter_map(|target| {
            let control = GlobalControl::OutputTrim(OutputTrimAddress { target });
            self.supports_global(control).then_some(control)
        }));
        rows.extend(
            [
                GlobalControl::TalkbackButton,
                GlobalControl::TalkbackSource,
                GlobalControl::TalkbackGain,
            ]
            .into_iter()
            .filter(|control| self.supports_global(*control)),
        );
        rows
    }

    pub fn output_trim_target_label(&self, target: u8) -> Option<&str> {
        self.output_trim_targets.get(&target).map(String::as_str)
    }

    pub fn talkback_source_choices(&self) -> Vec<(i32, String)> {
        self.parameter_values
            .get("talkback_source")
            .map(|values| {
                let mut values = values
                    .iter()
                    .map(|(value, label)| (*value, label.clone()))
                    .collect::<Vec<_>>();
                values.sort_by_key(|(value, _)| *value);
                values
            })
            .unwrap_or_default()
    }

    pub fn output_trim_value_labels(&self) -> Vec<(i32, String)> {
        self.parameter_values
            .get("output_trim")
            .map(|values| {
                let mut values = values
                    .iter()
                    .map(|(value, label)| (*value, label.clone()))
                    .collect::<Vec<_>>();
                values.sort_by_key(|(value, _)| *value);
                values
            })
            .unwrap_or_default()
    }

    pub fn clock_source_choices(&self) -> &[ClockSourceChoice] {
        &self.clock_source_choices
    }

    pub fn clock_source_label(&self, value: i32) -> String {
        self.clock_source_choices
            .iter()
            .find(|choice| choice.value == value)
            .map(|choice| choice.label.clone())
            .unwrap_or_else(|| format!("Clock raw {value} (unavailable)"))
    }

    pub fn clock_source_is_internal(&self, value: Option<i32>) -> bool {
        value.is_some() && value == self.internal_clock_value
    }

    pub fn supports_routing(&self, destination: u16) -> bool {
        self.actionable && self.routing_destinations.contains(&destination)
    }

    pub fn supports_any_routing(&self) -> bool {
        self.actionable && !self.routing_destinations.is_empty()
    }

    pub fn supports_assignment(&self, surface: u8, strip: u16) -> bool {
        if !self.actionable || strip == 0 {
            return false;
        }
        if self.driver_kind == RuntimeDriverKind::ZenGo && surface < 2 && strip <= 16 {
            return true;
        }
        self.mixer_assignment_destinations
            .get(&surface)
            .and_then(|destination| self.routing_channel_counts.get(destination))
            .is_some_and(|count| strip <= *count)
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
            input_link_domains: HashMap::new(),
            parameter_values: HashMap::new(),
            output_trim_targets: HashMap::new(),
            clock_source_choices: (0..=2)
                .map(|value| ClockSourceChoice {
                    value,
                    label: format!("Raw {value} (label unconfirmed)"),
                })
                .collect(),
            internal_clock_value: Some(0),
            output_controls,
            mixer_controls,
            link_surfaces: [0, 1].into_iter().collect(),
            global_controls: [GlobalControl::SampleRate, GlobalControl::ClockSource]
                .into_iter()
                .collect(),
            routing_destinations: [0].into_iter().collect(),
            routing_channel_counts: [(0, 16)].into_iter().collect(),
            mixer_assignment_destinations: HashMap::new(),
        }
    }
}

/// UI navigation, messaging, and settings.
#[derive(Debug, Clone)]
pub struct UiState {
    pub focus: FocusArea,
    pub page: UiPage,
    pub auraverb_focus: AuraVerbControlFocus,
    /// Logical row at the top of the responsive AuraVerb control viewport.
    pub auraverb_scroll: usize,
    /// Set only for a pointer press that began on an enabled visible AuraVerb track.
    pub auraverb_drag: Option<AuraVerbControlFocus>,
    pub surround_focus: SurroundControlFocus,
    pub surround_speaker_index: u8,
    pub surround_eq_bank: u8,
    /// Set only for a pointer press that began on an enabled Surround global track.
    pub surround_drag: Option<SurroundControlFocus>,
    pub last_message: String,
    pub settings: AppSettings,
    /// True only when terminal negotiation successfully enabled key release events.
    pub keyboard_release_events_enabled: bool,
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
    pub routing_editor: Option<RoutingEditorState>,
    pub routing_source_picker: Option<RoutingSourcePickerState>,
    pub selector_popup: Option<SelectorPopupState>,
    pub selector_parent_index: Option<usize>,
    /// True only after this UI successfully sent a talkback press and before release.
    pub talkback_button_held: bool,
    pub profile_names: Vec<String>,
    pub profile_editor: Option<ProfileEditorState>,
    pub selected_index: usize,
}

/// Raw packet debug view state — buffers, baselines, query logs.
#[derive(Debug, Clone)]
pub struct RawViewState {
    pub mode: RawViewMode,
    pub selected_tab: RawPacketTab,
    pub raw_map_scope: RawMapScope,
    pub raw_dump_scroll: usize,
    pub raw_map_scroll: usize,
    pub traffic_filter: TrafficFilter,
    pub traffic_selected_sequence: Option<TrafficSequence>,
    pub traffic_frozen: bool,
    pub traffic_frozen_head: Option<TrafficSequence>,
    pub traffic_observed_at_freeze: u64,
    pub traffic_evictions_at_freeze: u64,
    pub traffic_notice: Option<String>,
    traffic_journal: TrafficJournal,
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
                        pan: Some(0),
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
                    mono: None,
                    parameters: Vec::new(),
                },
                DynamicOutputState {
                    address: antelope_protocol::OutputAddress { id: 1 },
                    name: "HP 1".into(),
                    level: Some(0),
                    muted: Some(false),
                    dimmed: Some(false),
                    mono: None,
                    parameters: Vec::new(),
                },
                DynamicOutputState {
                    address: antelope_protocol::OutputAddress { id: 2 },
                    name: "HP 2".into(),
                    level: Some(0),
                    muted: Some(false),
                    dimmed: Some(false),
                    mono: None,
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
            page: UiPage::Mixer,
            auraverb_focus: AuraVerbControlFocus::ALL[0],
            auraverb_scroll: 0,
            auraverb_drag: None,
            surround_focus: SurroundControlFocus::Level,
            surround_speaker_index: 0,
            surround_eq_bank: 0,
            surround_drag: None,
            last_message:
                "Press ? for help. Device state is authoritative where decoding is confirmed."
                    .to_string(),
            settings: AppSettings::default(),
            keyboard_release_events_enabled: false,
            quit_requested: false,
        }
    }
}

impl Default for RawViewState {
    fn default() -> Self {
        Self {
            mode: RawViewMode::Legacy,
            selected_tab: RawPacketTab::State73,
            raw_map_scope: RawMapScope::All,
            raw_dump_scroll: 0,
            raw_map_scroll: 0,
            traffic_filter: TrafficFilter::default(),
            traffic_selected_sequence: None,
            traffic_frozen: false,
            traffic_frozen_head: None,
            traffic_observed_at_freeze: 0,
            traffic_evictions_at_freeze: 0,
            traffic_notice: None,
            traffic_journal: TrafficJournal::default(),
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

fn lifetime_observed(counters: TrafficCounters) -> u64 {
    counters
        .rx_returned
        .saturating_add(counters.rx_errors)
        .saturating_add(counters.tx_succeeded)
        .saturating_add(counters.tx_failed_delivery_uncertain)
}

fn lifetime_evictions(counters: TrafficCounters) -> u64 {
    counters
        .evicted_by_count
        .saturating_add(counters.evicted_by_bytes)
}

fn cycle_optional_numeric(current: Option<u8>, available: &[u8], forward: bool) -> Option<u8> {
    let mut values = available.to_vec();
    if let Some(current) = current {
        match values.binary_search(&current) {
            Ok(_) => {}
            Err(index) => values.insert(index, current),
        }
    }
    let position = current
        .and_then(|current| {
            values
                .iter()
                .position(|value| *value == current)
                .map(|index| index + 1)
        })
        .unwrap_or(0);
    let option_count = values.len() + 1;
    let next = if forward {
        (position + 1) % option_count
    } else {
        position.checked_sub(1).unwrap_or(option_count - 1)
    };
    next.checked_sub(1).map(|index| values[index])
}

impl RawViewState {
    pub(crate) fn bind_traffic_journal(&mut self, journal: TrafficJournal) {
        self.traffic_journal = journal;
        self.mode = RawViewMode::Legacy;
        self.traffic_filter = TrafficFilter::default();
        self.traffic_selected_sequence = None;
        self.traffic_frozen = false;
        self.traffic_frozen_head = None;
        self.traffic_observed_at_freeze = 0;
        self.traffic_evictions_at_freeze = 0;
        self.traffic_notice = None;
        self.reset_raw_view_scroll();
    }

    pub fn traffic_journal(&self) -> &TrafficJournal {
        &self.traffic_journal
    }

    pub fn traffic_head(&self) -> TrafficQueryHead {
        if !self.traffic_frozen {
            TrafficQueryHead::Live
        } else if let Some(sequence) = self.traffic_frozen_head {
            TrafficQueryHead::Through(sequence)
        } else {
            TrafficQueryHead::Empty
        }
    }

    pub fn traffic_view(&self, max_rows: usize) -> FilteredTrafficView {
        self.traffic_journal.filtered_view(
            self.traffic_filter,
            self.traffic_head(),
            self.traffic_selected_sequence,
            max_rows,
        )
    }

    pub fn traffic_display_sequence(&self, view: &FilteredTrafficView) -> Option<TrafficSequence> {
        if self.traffic_frozen {
            self.traffic_selected_sequence
        } else {
            view.rows.last().map(|row| row.sequence)
        }
    }

    pub fn select_legacy_tab(&mut self, tab: RawPacketTab) {
        self.mode = RawViewMode::Legacy;
        self.select_tab(tab);
    }

    pub fn toggle_traffic_mode(&mut self) {
        self.mode = match self.mode {
            RawViewMode::Legacy => RawViewMode::AllTraffic,
            RawViewMode::AllTraffic => RawViewMode::Legacy,
        };
        self.reset_raw_view_scroll();
    }

    pub fn select_all_traffic(&mut self) {
        self.mode = RawViewMode::AllTraffic;
        self.reset_raw_view_scroll();
    }

    pub fn select_traffic_direction(&mut self, direction: Option<TrafficDirection>) {
        self.change_traffic_filter(|filter| filter.direction = direction);
    }

    pub fn toggle_traffic_errors(&mut self) {
        self.change_traffic_filter(|filter| filter.errors_only = !filter.errors_only);
    }

    pub fn cycle_traffic_family(&mut self, forward: bool) {
        let values = self.traffic_view(0).family_values;
        let next = cycle_optional_numeric(self.traffic_filter.family, &values, forward);
        self.change_traffic_filter(|filter| filter.family = next);
    }

    pub fn cycle_traffic_discriminator(&mut self, forward: bool) {
        let values = self.traffic_view(0).discriminator_values;
        let next = cycle_optional_numeric(self.traffic_filter.discriminator, &values, forward);
        self.change_traffic_filter(|filter| filter.discriminator = next);
    }

    pub fn cycle_traffic_category(&mut self, forward: bool) {
        let values = self.traffic_view(0).query_category_values;
        let next = cycle_optional_numeric(self.traffic_filter.query_category, &values, forward);
        self.change_traffic_filter(|filter| filter.query_category = next);
    }

    pub fn toggle_traffic_freeze(&mut self) {
        if self.traffic_frozen {
            self.traffic_frozen = false;
            self.traffic_frozen_head = None;
            self.traffic_selected_sequence = self.traffic_journal.matching_sequence(
                self.traffic_filter,
                TrafficQueryHead::Live,
                None,
                TrafficSelectionMove::Newest,
            );
            self.traffic_notice = Some("Live resumed at newest matching retained event".into());
        } else {
            let stats = self.traffic_journal.stats();
            self.traffic_frozen = true;
            self.traffic_frozen_head = stats.newest_sequence;
            self.traffic_observed_at_freeze = lifetime_observed(stats.counters);
            self.traffic_evictions_at_freeze = lifetime_evictions(stats.counters);
            self.traffic_selected_sequence = self.traffic_journal.matching_sequence(
                self.traffic_filter,
                self.traffic_head(),
                self.traffic_selected_sequence,
                TrafficSelectionMove::Newest,
            );
            self.traffic_notice =
                Some("Display head and selection frozen; journal continues".into());
        }
        self.reset_raw_view_scroll();
    }

    pub fn move_traffic_selection(&mut self, movement: TrafficSelectionMove) {
        if !self.traffic_frozen {
            self.toggle_traffic_freeze();
        }
        let selected = self.traffic_selected_sequence;
        if let Some(sequence) = self.traffic_journal.matching_sequence(
            self.traffic_filter,
            self.traffic_head(),
            selected,
            movement,
        ) {
            self.traffic_selected_sequence = Some(sequence);
            self.traffic_notice = None;
            self.reset_raw_view_scroll();
        }
    }

    pub fn select_traffic_sequence(&mut self, sequence: TrafficSequence) {
        if !self.traffic_frozen {
            self.toggle_traffic_freeze();
        }
        if self
            .traffic_journal
            .selected_event(sequence, self.traffic_filter, self.traffic_head())
            .is_some()
        {
            self.traffic_selected_sequence = Some(sequence);
            self.traffic_notice = None;
            self.reset_raw_view_scroll();
        }
    }

    pub fn frozen_traffic_changes(&self) -> (u64, u64) {
        if !self.traffic_frozen {
            return (0, 0);
        }
        let counters = self.traffic_journal.stats().counters;
        (
            lifetime_observed(counters).saturating_sub(self.traffic_observed_at_freeze),
            lifetime_evictions(counters).saturating_sub(self.traffic_evictions_at_freeze),
        )
    }

    fn change_traffic_filter(&mut self, change: impl FnOnce(&mut TrafficFilter)) {
        let previous = if self.traffic_frozen {
            self.traffic_selected_sequence
        } else {
            self.traffic_journal.matching_sequence(
                self.traffic_filter,
                TrafficQueryHead::Live,
                None,
                TrafficSelectionMove::Newest,
            )
        };
        change(&mut self.traffic_filter);
        let head = self.traffic_head();
        let keep = previous.filter(|sequence| {
            self.traffic_journal
                .selected_event(*sequence, self.traffic_filter, head)
                .is_some()
        });
        self.traffic_selected_sequence = keep.or_else(|| {
            self.traffic_journal.matching_sequence(
                self.traffic_filter,
                head,
                None,
                TrafficSelectionMove::Newest,
            )
        });
        self.traffic_notice = if keep.is_some() {
            None
        } else if previous.is_some() && self.traffic_selected_sequence.is_some() {
            Some("Filter moved selection to newest matching retained event".into())
        } else if self.traffic_selected_sequence.is_none() {
            Some("No retained events match; this is not evidence of wire absence".into())
        } else {
            None
        };
        self.reset_raw_view_scroll();
    }

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
