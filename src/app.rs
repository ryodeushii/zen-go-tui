use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use ratatui::layout::Rect;

use crate::command_queue::CommandQueue;
use crate::profile::{preamp_mode_raw, DeviceProfile};
use crate::transport::Transport;
use antelope_protocol::{
    control_panel_startup_queries, encode_command, encode_link_companion,
    encode_mixer_assignment_frames_with_table, encode_query, ClockSource, Command, DeviceMetadata,
    DeviceSnapshot, DeviceStateSnapshot, Frame, MixerAssignment, MixerChannelState,
    MixerLinkTarget, MixerPassiveStripState, MixerSurface, OutputMode, OutputState, OutputTarget,
    PanState, PreampMode, PreampState, QueryResponse, SampleRate, Surface,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    // Application lifecycle
    Quit,

    // View navigation
    ToggleRawView,
    ToggleHotkeysPopup,
    SelectPage(MainPage),
    SelectRawPacketTab(RawPacketTab),

    // Popup management
    OpenProfilesPopup,
    CloseProfilesPopup,
    OpenRoutingPopup,
    CloseRoutingPopup,
    OpenOptionsPopup,
    CloseOptionsPopup,
    CloseSelectorPopup,
    CloseAssignmentPicker,

    // Settings
    SetRefreshRate(RefreshRate),
    CyclePeakThreshold(bool),
    TogglePeakEnabled,
    CyclePeakHoldDuration(PeakHoldDuration),
    ToggleAutoSave,

    // Profile management
    SelectProfile(usize),
    LoadSelectedProfile,
    StartSaveProfile,
    StartRenameProfile,
    DeleteSelectedProfile,

    // Navigation/selection
    PageMixerStripsLeft,
    PageMixerStripsRight,
    SelectSurface(antelope_protocol::Surface),
    SelectMixerChannel(usize),
    SelectOutput(usize),
    SelectPreampInput(usize),
    SelectQueryReplyEntry(usize),
    ScrollQueryReplyList {
        increase: bool,
    },

    // Output controls
    AdjustOutputLevel {
        index: usize,
        increase: bool,
    },
    SetOutputLevel {
        index: usize,
        step: u8,
    },
    ToggleOutputMute(usize),
    ToggleOutputDim(usize),

    // Mixer controls
    AdjustMixerLevel {
        index: usize,
        increase: bool,
    },
    SetMixerLevel {
        index: usize,
        level: u8,
    },
    AdjustMixerPan {
        index: usize,
        right: bool,
    },
    SetMixerPan {
        index: usize,
        pan: antelope_protocol::PanState,
    },
    ToggleMixerMute(u8),
    ToggleMixerSolo(u8),
    ToggleMixerLink(u8),
    OpenAssignmentPicker(u8),
    PickAssignment {
        strip: u8,
        assignment: antelope_protocol::MixerAssignment,
    },

    // Preamp controls
    AdjustPreampGain {
        input: u8,
        increase: bool,
    },
    SetPreampGain {
        input: u8,
        raw: u8,
    },
    OpenPreampModeSelector(u8),
    CyclePreampMode(u8),
    PickPreampMode {
        input: u8,
        mode: antelope_protocol::PreampMode,
    },
    TogglePreampPhase(u8),
    TogglePreampPhantom(u8),

    // Selector popups
    OpenSampleRateSelector,
    OpenClockSourceSelector,
    PickSampleRate(antelope_protocol::SampleRate),
    PickClockSource(antelope_protocol::ClockSource),

    // Keyboard-only (context-resolved in handle_key_press)
    AdjustFocused(bool),
    ToggleFocusedMute,
    ToggleFocusedDim,
    ToggleRoutingPopup,
    RefreshQueriedState,
    CycleFocus,
    MovePopupSelection(bool),
    ProfileEditorChar(String),
    ProfileEditorBackspace,
    ProfileEditorCommit,
    ProfileEditorCancel,
    CaptureRawBaseline,
    ClearRawBaseline,
    ToggleOptionsPopup,
}

impl Intent {
    fn pending_mutation(&self, state: &AppState) -> Option<PendingMutation> {
        match self {
            Intent::SetOutputLevel { index, step } => {
                let target = match index {
                    0 => OutputTarget::Monitor,
                    1 => OutputTarget::Hp1,
                    2 => OutputTarget::Hp2,
                    _ => return None,
                };
                Some(PendingMutation::OutputVolume {
                    target,
                    step: *step,
                })
            }
            Intent::ToggleOutputMute(index) => {
                let target = match index {
                    0 => OutputTarget::Monitor,
                    1 => OutputTarget::Hp1,
                    2 => OutputTarget::Hp2,
                    _ => return None,
                };
                let current = state.output.states[target.index() as usize].mode;
                let mode = if current == OutputMode::Mute {
                    OutputMode::Normal
                } else {
                    OutputMode::Mute
                };
                Some(PendingMutation::OutputMode { target, mode })
            }
            Intent::ToggleOutputDim(index) => {
                let target = match index {
                    0 => OutputTarget::Monitor,
                    1 => OutputTarget::Hp1,
                    2 => OutputTarget::Hp2,
                    _ => return None,
                };
                let current = state.output.states[target.index() as usize].mode;
                let mode = if current == OutputMode::Dim {
                    OutputMode::Normal
                } else {
                    OutputMode::Dim
                };
                Some(PendingMutation::OutputMode { target, mode })
            }
            Intent::SetMixerLevel { index, level } => {
                let channel = (*index + 1) as u8;
                let mixer = state.active_mixer_surface();
                let idx = *index;
                let Some(active) = state.mixer.channels[mixer.index()].get(idx).copied() else {
                    return None;
                };
                if active.linked == Some(true) {
                    let (left_ch, right_ch, left, right) =
                        resolve_linked_pair_from_state(state, mixer, channel)?;
                    Some(PendingMutation::MixerLinkedLevel {
                        mixer,
                        left_channel: left_ch,
                        right_channel: right_ch,
                        level: *level,
                        left_pan: left.pan,
                        right_pan: right.pan,
                        left_muted: left.muted.unwrap_or(false),
                        right_muted: right.muted.unwrap_or(false),
                    })
                } else {
                    Some(PendingMutation::MixerLevel {
                        mixer,
                        channel,
                        level: *level,
                        pan: active.pan,
                        muted: active.muted.unwrap_or(false),
                    })
                }
            }
            Intent::SetMixerPan { index, pan } => {
                let channel = (*index + 1) as u8;
                let mixer = state.active_mixer_surface();
                Some(PendingMutation::MixerPan {
                    mixer,
                    channel,
                    pan: *pan,
                })
            }
            Intent::ToggleMixerMute(channel) => {
                let mixer = state.active_mixer_surface();
                let idx = channel.saturating_sub(1) as usize;
                let Some(active) = state.mixer.channels[mixer.index()].get(idx).copied() else {
                    return None;
                };
                let muted = !active.muted.unwrap_or(false);
                if active.linked == Some(true) {
                    let (left_ch, right_ch, _, _) =
                        resolve_linked_pair_from_state(state, mixer, *channel)?;
                    Some(PendingMutation::MixerLinkedMute {
                        mixer,
                        left_channel: left_ch,
                        right_channel: right_ch,
                        muted,
                    })
                } else {
                    Some(PendingMutation::MixerMute {
                        mixer,
                        channel: *channel,
                        muted,
                    })
                }
            }
            Intent::ToggleMixerSolo(channel) => {
                let mixer = state.active_mixer_surface();
                let idx = channel.saturating_sub(1) as usize;
                let Some(active) = state.mixer.channels[mixer.index()].get(idx).copied() else {
                    return None;
                };
                let soloed = !active.soloed.unwrap_or(false);
                if active.linked == Some(true) {
                    let (left_ch, right_ch, _, _) =
                        resolve_linked_pair_from_state(state, mixer, *channel)?;
                    Some(PendingMutation::MixerLinkedSolo {
                        mixer,
                        left_channel: left_ch,
                        right_channel: right_ch,
                        soloed,
                    })
                } else {
                    Some(PendingMutation::MixerSolo {
                        mixer,
                        channel: *channel,
                        soloed,
                    })
                }
            }
            Intent::ToggleMixerLink(channel) => {
                let mixer = state.active_mixer_surface();
                let Some(target) = MixerLinkTarget::from_channel(mixer, *channel) else {
                    return None;
                };
                let enabled = !state.mixer.channels[mixer.index()]
                    .get(channel.saturating_sub(1) as usize)
                    .and_then(|c| c.linked)
                    .unwrap_or(false);
                Some(PendingMutation::MixerLinkExplicit {
                    mixer,
                    left_channel: target.left_channel,
                    right_channel: target.right_channel,
                    enabled,
                })
            }
            Intent::PickAssignment { strip, assignment } => {
                Some(PendingMutation::MixerAssignment {
                    strip: *strip,
                    assignment: *assignment,
                })
            }
            Intent::SetPreampGain { input, raw } => Some(PendingMutation::PreampGain {
                input: *input,
                raw: *raw,
            }),
            Intent::PickPreampMode { input, mode } => Some(PendingMutation::PreampMode {
                input: *input,
                mode: *mode,
            }),
            Intent::TogglePreampPhantom(input) => {
                let input_state = if *input == 0 {
                    &state.preamp.state.input1
                } else {
                    &state.preamp.state.input2
                };
                Some(PendingMutation::PreampPhantom {
                    input: *input,
                    enabled: !input_state.phantom_on,
                })
            }
            Intent::TogglePreampPhase(input) => {
                let input_state = if *input == 0 {
                    &state.preamp.state.input1
                } else {
                    &state.preamp.state.input2
                };
                let phase_inverted = input_state.mode_raw & 0x40 != 0;
                Some(PendingMutation::PreampPhase {
                    input: *input,
                    enabled: !phase_inverted,
                })
            }
            Intent::PickSampleRate(_) | Intent::PickClockSource(_) => {
                // Handled by apply_command_state_update, no pending mutation needed
                None
            }
            _ => None,
        }
    }
}

fn resolve_linked_pair_from_state(
    state: &AppState,
    mixer: MixerSurface,
    channel: u8,
) -> Option<(u8, u8, MixerChannelState, MixerChannelState)> {
    let (left_channel, right_channel) = if channel % 2 == 1 {
        (channel, channel.saturating_add(1))
    } else {
        (channel.saturating_sub(1), channel)
    };
    let left_index = left_channel.saturating_sub(1) as usize;
    let right_index = right_channel.saturating_sub(1) as usize;
    let left = state.mixer.channels[mixer.index()]
        .get(left_index)
        .copied()?;
    let right = state.mixer.channels[mixer.index()]
        .get(right_index)
        .copied()?;
    Some((left_channel, right_channel, left, right))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusArea {
    Status,
    Outputs,
    Mixer,
    Preamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPacketTab {
    Query74,
    State73,
    Auxiliary,
    Query75,
    DeviceNotification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainPage {
    Mixer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignmentPickerState {
    pub strip: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorPopupKind {
    SampleRate,
    ClockSource,
    PreampMode { input: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectorPopupState {
    pub kind: SelectorPopupKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryReplyLogEntry {
    pub summary: String,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileEditorMode {
    Save,
    Rename,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileEditorState {
    pub mode: ProfileEditorMode,
    pub original_name: Option<String>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructuralSnapshot {
    sample_rate: SampleRate,
    sample_rate_hz: u32,
    clock_source: ClockSource,
    status_flags: [u8; 2],
    front_panel_bytes: [u8; 3],
    outputs: [OutputState; 3],
    dsp_cluster: [u8; 4],
    surface: Surface,
    mixer_surfaces: [[MixerPassiveStripState; 16]; 2],
}

impl StructuralSnapshot {
    fn from_snapshot(snapshot: &DeviceStateSnapshot) -> Self {
        Self {
            sample_rate: snapshot.sample_rate,
            sample_rate_hz: snapshot.sample_rate_hz,
            clock_source: snapshot.clock_source,
            status_flags: snapshot.status_flags,
            front_panel_bytes: snapshot.front_panel_bytes,
            outputs: snapshot.outputs,
            dsp_cluster: snapshot.dsp_cluster,
            surface: snapshot.surface,
            mixer_surfaces: snapshot.mixer_decode.surfaces,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RefreshRate {
    Fps15,
    #[default]
    Fps30,
    Fps60,
}

impl RefreshRate {
    pub fn all() -> &'static [RefreshRate] {
        &[RefreshRate::Fps15, RefreshRate::Fps30, RefreshRate::Fps60]
    }

    pub fn label(&self) -> &'static str {
        match self {
            RefreshRate::Fps15 => "15 FPS",
            RefreshRate::Fps30 => "30 FPS",
            RefreshRate::Fps60 => "60 FPS",
        }
    }

    pub fn fps(&self) -> u8 {
        match self {
            RefreshRate::Fps15 => 15,
            RefreshRate::Fps30 => 30,
            RefreshRate::Fps60 => 60,
        }
    }

    pub fn loop_sleep_ms(&self) -> u64 {
        match self {
            RefreshRate::Fps15 => 30,
            RefreshRate::Fps30 => 16,
            RefreshRate::Fps60 => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PeakHoldDuration {
    Sec1,
    #[default]
    Sec3,
    Sec5,
    Sec10,
}

impl PeakHoldDuration {
    pub fn all() -> &'static [PeakHoldDuration] {
        &[
            PeakHoldDuration::Sec1,
            PeakHoldDuration::Sec3,
            PeakHoldDuration::Sec5,
            PeakHoldDuration::Sec10,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            PeakHoldDuration::Sec1 => "1s",
            PeakHoldDuration::Sec3 => "3s",
            PeakHoldDuration::Sec5 => "5s",
            PeakHoldDuration::Sec10 => "10s",
        }
    }

    pub fn duration(&self) -> Duration {
        match self {
            PeakHoldDuration::Sec1 => Duration::from_secs(1),
            PeakHoldDuration::Sec3 => Duration::from_secs(3),
            PeakHoldDuration::Sec5 => Duration::from_secs(5),
            PeakHoldDuration::Sec10 => Duration::from_secs(10),
        }
    }
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

/// Mixer surface state — channels, selection, scroll, and peak meters.
#[derive(Debug, Clone)]
pub struct MixerState {
    pub surface: Surface,
    pub channels: [Vec<MixerChannelState>; 2],
    pub selected_channel: usize,
    pub strip_scroll: usize,
    pub peaks: [[Option<MeterPeak>; 16]; 2],
}

/// Output state — physical outputs and selection.
#[derive(Debug, Clone)]
pub struct OutputData {
    pub states: [OutputState; 3],
    pub selected: usize,
}

/// Preamp state — inputs, selection, and peak meters.
#[derive(Debug, Clone)]
pub struct PreampData {
    pub state: PreampState,
    pub selected_input: usize,
    pub peaks: [Option<MeterPeak>; 2],
}

/// UI navigation, messaging, and settings.
#[derive(Debug, Clone)]
pub struct UiState {
    pub focus: FocusArea,
    pub page: MainPage,
    pub last_message: String,
    pub settings: AppSettings,
    pub quit_requested: bool,
}

/// Popup and overlay state — mutually exclusive overlays.
#[derive(Debug, Clone)]
pub struct PopupState {
    pub hotkeys_open: bool,
    pub options_open: bool,
    pub routing_open: bool,
    pub profiles_open: bool,
    pub raw_view_open: bool,
    pub assignment_picker: Option<AssignmentPickerState>,
    pub selector_popup: Option<SelectorPopupState>,
    pub profile_names: Vec<String>,
    pub profile_editor: Option<ProfileEditorState>,
    pub selected_index: usize,
}

/// Raw packet debug view state — buffers, baselines, query logs.
#[derive(Debug, Clone)]
pub struct RawViewState {
    pub selected_tab: RawPacketTab,
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

#[derive(Debug, Clone)]
pub struct AppState {
    pub device: DeviceState,
    pub mixer: MixerState,
    pub output: OutputData,
    pub preamp: PreampData,
    pub ui: UiState,
    pub popup: PopupState,
    pub raw_view: RawViewState,
    latest_structural_snapshot: Option<StructuralSnapshot>,
}

impl Default for MixerState {
    fn default() -> Self {
        Self {
            surface: Surface::MonitorHp1,
            channels: [
                (1..=16).map(MixerChannelState::unknown).collect(),
                (1..=16).map(MixerChannelState::unknown).collect(),
            ],
            selected_channel: 0,
            strip_scroll: 0,
            peaks: [[None; 16]; 2],
        }
    }
}

impl Default for OutputData {
    fn default() -> Self {
        Self {
            states: [
                OutputState::new(OutputTarget::Monitor, 0, OutputMode::Normal),
                OutputState::new(OutputTarget::Hp1, 0, OutputMode::Normal),
                OutputState::new(OutputTarget::Hp2, 0, OutputMode::Normal),
            ],
            selected: 0,
        }
    }
}

impl Default for PreampData {
    fn default() -> Self {
        Self {
            state: PreampState::default(),
            selected_input: 0,
            peaks: [None, None],
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            focus: FocusArea::Outputs,
            page: MainPage::Mixer,
            last_message:
                "Press ? for help. Device state is authoritative where decoding is confirmed."
                    .to_string(),
            settings: AppSettings::default(),
            quit_requested: false,
        }
    }
}

impl Default for PopupState {
    fn default() -> Self {
        Self {
            hotkeys_open: false,
            options_open: false,
            routing_open: false,
            profiles_open: false,
            raw_view_open: false,
            assignment_picker: None,
            selector_popup: None,
            profile_names: Vec::new(),
            profile_editor: None,
            selected_index: 0,
        }
    }
}

impl Default for RawViewState {
    fn default() -> Self {
        Self {
            selected_tab: RawPacketTab::State73,
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

impl Default for AppState {
    fn default() -> Self {
        Self {
            device: DeviceState::default(),
            mixer: MixerState::default(),
            output: OutputData::default(),
            preamp: PreampData::default(),
            ui: UiState::default(),
            popup: PopupState::default(),
            raw_view: RawViewState::default(),
            latest_structural_snapshot: None,
        }
    }
}

impl AppState {
    pub fn prune_expired_peaks(&mut self) {
        let hold = self.ui.settings.peak_hold_duration.duration();
        for mix_idx in 0..2 {
            for ch_idx in 0..16 {
                if let Some(peak) = self.mixer.peaks[mix_idx][ch_idx] {
                    if peak.detected_at.elapsed() >= hold {
                        self.mixer.peaks[mix_idx][ch_idx] = None;
                    }
                }
            }
        }
        for input_idx in 0..2 {
            if let Some(peak) = self.preamp.peaks[input_idx] {
                if peak.detected_at.elapsed() >= hold {
                    self.preamp.peaks[input_idx] = None;
                }
            }
        }
    }

    pub fn startup_query_summary(&self, query_id: u8) -> Option<&str> {
        startup_query_slot(query_id)
            .and_then(|index| self.device.status.startup_query_summaries[index].as_deref())
    }

    pub fn selected_query_reply_entry(&self) -> Option<&QueryReplyLogEntry> {
        self.raw_view
            .selected_query_reply_entry
            .and_then(|index| self.raw_view.recent_query_reply_entries.get(index))
    }

    pub fn active_mixer_surface(&self) -> MixerSurface {
        MixerSurface::from_surface(self.mixer.surface)
    }

    pub fn active_mixer_channels(&self) -> &[MixerChannelState] {
        &self.mixer.channels[self.active_mixer_surface().index()]
    }

    pub fn clamp_mixer_strip_scroll(&mut self, visible_count: usize) {
        let visible_count = visible_count.max(1);
        let total = self.active_mixer_channels().len();
        let max_scroll = total.saturating_sub(visible_count);
        self.mixer.strip_scroll = self.mixer.strip_scroll.min(max_scroll);
    }

    pub fn ensure_selected_mixer_channel_visible(&mut self, visible_count: usize) {
        let visible_count = visible_count.max(1);
        self.clamp_mixer_strip_scroll(visible_count);

        if self.mixer.selected_channel < self.mixer.strip_scroll {
            self.mixer.strip_scroll = self.mixer.selected_channel;
        } else if self.mixer.selected_channel >= self.mixer.strip_scroll + visible_count {
            self.mixer.strip_scroll = self.mixer.selected_channel + 1 - visible_count;
        }

        self.clamp_mixer_strip_scroll(visible_count);
    }

    pub fn scroll_mixer_strip_viewport(&mut self, delta: isize, visible_count: usize) {
        let visible_count = visible_count.max(1);
        let total = self.active_mixer_channels().len();
        let max_scroll = total.saturating_sub(visible_count);

        self.mixer.strip_scroll = if delta >= 0 {
            self.mixer
                .strip_scroll
                .saturating_add(delta as usize)
                .min(max_scroll)
        } else {
            self.mixer
                .strip_scroll
                .saturating_sub(delta.saturating_abs() as usize)
        };
    }

    pub fn page_mixer_strip_viewport(&mut self, right: bool, page_size: usize) {
        let total = self.active_mixer_channels().len();
        let page_size = page_size.max(1);
        let max_page_start =
            total.saturating_sub(1).checked_div(page_size).unwrap_or(0) * page_size;

        self.mixer.strip_scroll = if right {
            self.mixer
                .strip_scroll
                .saturating_add(page_size)
                .min(max_page_start)
        } else {
            self.mixer.strip_scroll.saturating_sub(page_size)
        };

        if self.mixer.selected_channel < self.mixer.strip_scroll
            || self.mixer.selected_channel >= self.mixer.strip_scroll + page_size
        {
            self.mixer.selected_channel = self.mixer.strip_scroll.min(total.saturating_sub(1));
        }
    }

    fn snapshot_structurally_differs(&self, snapshot: &DeviceStateSnapshot) -> bool {
        let Some(prev) = &self.latest_structural_snapshot else {
            return true;
        };
        prev.sample_rate != snapshot.sample_rate
            || prev.sample_rate_hz != snapshot.sample_rate_hz
            || prev.clock_source != snapshot.clock_source
            || prev.status_flags != snapshot.status_flags
            || prev.front_panel_bytes != snapshot.front_panel_bytes
            || prev.outputs != snapshot.outputs
            || prev.dsp_cluster != snapshot.dsp_cluster
            || prev.surface != snapshot.surface
            || prev.mixer_surfaces != snapshot.mixer_decode.surfaces
    }

    fn apply_meters_only(&mut self, snapshot: &DeviceStateSnapshot) {
        let mixer = self.active_mixer_surface();
        let mut meter_updates: Vec<(usize, usize, u8)> = Vec::new();
        for channel in 1..=16 {
            let Some(decoded) = snapshot.mixer_decode.strip(mixer, channel) else {
                continue;
            };
            let Some(slot) = self.state_slot_mut(mixer, channel) else {
                continue;
            };
            if let Some(meter) = decoded.meter {
                slot.meter = Some(meter);
                meter_updates.push((mixer.index(), channel as usize - 1, meter));
            }
        }
        for (mix_idx, ch_idx, meter) in meter_updates {
            self.track_mixer_peak(mix_idx, ch_idx, meter);
        }
        if let Some(meter) = snapshot.mixer_decode.observed_preamp1_meter {
            self.track_preamp_peak(0, meter);
        }
        self.preamp.state.input1.observed_meter = snapshot.mixer_decode.observed_preamp1_meter;
        if let Some(meter) = snapshot.mixer_decode.observed_preamp2_meter {
            self.track_preamp_peak(1, meter);
        }
        self.preamp.state.input2.observed_meter = snapshot.mixer_decode.observed_preamp2_meter;
    }

    fn track_mixer_peak(&mut self, mix_idx: usize, channel_idx: usize, meter: u8) {
        if !self.ui.settings.peak_enabled || meter > self.ui.settings.peak_threshold_raw {
            return;
        }
        let existing = self.mixer.peaks[mix_idx][channel_idx];
        match existing {
            Some(peak) if meter < peak.raw => {
                self.mixer.peaks[mix_idx][channel_idx] = Some(MeterPeak {
                    raw: meter,
                    detected_at: Instant::now(),
                });
            }
            None => {
                self.mixer.peaks[mix_idx][channel_idx] = Some(MeterPeak {
                    raw: meter,
                    detected_at: Instant::now(),
                });
            }
            _ => {}
        }
    }

    fn track_preamp_peak(&mut self, input_idx: usize, meter: u8) {
        if !self.ui.settings.peak_enabled || meter > self.ui.settings.peak_threshold_raw {
            return;
        }
        let existing = self.preamp.peaks[input_idx];
        match existing {
            Some(peak) if meter < peak.raw => {
                self.preamp.peaks[input_idx] = Some(MeterPeak {
                    raw: meter,
                    detected_at: Instant::now(),
                });
            }
            None => {
                self.preamp.peaks[input_idx] = Some(MeterPeak {
                    raw: meter,
                    detected_at: Instant::now(),
                });
            }
            _ => {}
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: &DeviceStateSnapshot) {
        self.device.status.sample_rate = Some(snapshot.sample_rate);
        self.device.status.sample_rate_hz = Some(snapshot.sample_rate_hz);
        self.device.status.clock_source = Some(snapshot.clock_source);
        self.device.status.last_refresh_summary = format!(
            "snapshot {} / {} / surface {}",
            snapshot.sample_rate.label(),
            snapshot.clock_source.label(),
            snapshot.surface.label()
        );
        self.output.states = snapshot.outputs;
        self.device.dsp_cluster = snapshot.dsp_cluster;
        self.preamp.state = PreampState::from_cluster(snapshot.dsp_cluster);
        if let Some(meter) = snapshot.mixer_decode.observed_preamp1_meter {
            self.track_preamp_peak(0, meter);
        }
        self.preamp.state.input1.observed_meter = snapshot.mixer_decode.observed_preamp1_meter;
        if let Some(meter) = snapshot.mixer_decode.observed_preamp2_meter {
            self.track_preamp_peak(1, meter);
        }
        self.preamp.state.input2.observed_meter = snapshot.mixer_decode.observed_preamp2_meter;
        self.mixer.surface = snapshot.surface;
        self.apply_passive_mixer_decode(snapshot);
    }

    fn apply_passive_mixer_decode(&mut self, snapshot: &DeviceStateSnapshot) {
        let mut meter_updates: Vec<(usize, usize, u8)> = Vec::new();
        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            for channel in 1..=16 {
                let Some(decoded) = snapshot.mixer_decode.strip(mixer, channel) else {
                    continue;
                };
                let Some(slot) = self.state_slot_mut(mixer, channel) else {
                    continue;
                };

                if let Some(meter) = decoded.meter {
                    slot.meter = Some(meter);
                    meter_updates.push((mixer.index(), channel as usize - 1, meter));
                }
                if let Some(muted) = decoded.muted {
                    slot.muted = Some(muted);
                }
                if let Some(linked) = decoded.linked {
                    slot.linked = Some(linked);
                }
            }
        }
        for (mix_idx, ch_idx, meter) in meter_updates {
            self.track_mixer_peak(mix_idx, ch_idx, meter);
        }
    }

    fn state_slot_mut(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
    ) -> Option<&mut MixerChannelState> {
        self.mixer.channels[mixer.index()].get_mut(channel.checked_sub(1)? as usize)
    }

    fn refresh_preamp_from_cluster_preserving_observed_meter(&mut self) {
        let observed_meter_input1 = self.preamp.state.input1.observed_meter;
        let observed_meter_input2 = self.preamp.state.input2.observed_meter;
        self.preamp.state = PreampState::from_cluster(self.device.dsp_cluster);
        self.preamp.state.input1.observed_meter = observed_meter_input1;
        self.preamp.state.input2.observed_meter = observed_meter_input2;
    }

    pub fn observe_frame(&mut self, frame: DeviceSnapshot, raw: Vec<u8>) -> bool {
        let was_connected = self.device.connection.connected;
        self.device.connection.connected = true;
        self.device.connection.last_snapshot_at = Some(Instant::now());
        match frame {
            DeviceSnapshot::Snapshot(snapshot) => {
                let structural_changed =
                    !was_connected || self.snapshot_structurally_differs(&snapshot);
                let raw_changed =
                    self.popup.raw_view_open && self.raw_view.latest_raw_73.as_ref() != Some(&raw);
                let changed = structural_changed || raw_changed;
                self.device.connection.last_frame_type = Some("0x73 snapshot");
                if structural_changed {
                    self.apply_snapshot(&snapshot);
                } else {
                    self.apply_meters_only(&snapshot);
                }
                self.latest_structural_snapshot =
                    Some(StructuralSnapshot::from_snapshot(&snapshot));
                self.raw_view.latest_raw_73 = Some(raw);
                changed
            }
            DeviceSnapshot::Auxiliary(bytes) => {
                let changed = !was_connected
                    || (self.popup.raw_view_open
                        && self.raw_view.latest_raw_83.as_ref() != Some(&raw));
                self.device.connection.last_frame_type = Some("0x83 auxiliary");
                self.raw_view.last_auxiliary_len = Some(bytes.len());
                self.raw_view.latest_raw_83 = Some(raw);
                changed
            }
            DeviceSnapshot::QueryReply(reply) => {
                self.device.connection.last_frame_type = Some("0x75 query reply");
                self.raw_view.latest_raw_75 = Some(raw.clone());
                self.store_startup_query_summary(&reply);
                self.apply_query_reply_readback(&reply);
                self.push_query_reply_log(&reply, raw);
                if let Some(metadata) = reply.metadata() {
                    self.ui.last_message = format!(
                        "Connected to {} (hw {}, serial {})",
                        metadata.product_name, metadata.hardware_version, metadata.serial
                    );
                    self.device.status.metadata = Some(metadata);
                }
                true
            }
            DeviceSnapshot::Notification(_) => {
                let changed = !was_connected || self.popup.raw_view_open;
                self.device.connection.last_frame_type = Some("0x81 notification");
                self.raw_view.latest_raw_81 = Some(raw);
                changed
            }
        }
    }

    fn store_startup_query_summary(&mut self, reply: &QueryResponse) {
        if let Some(index) = startup_query_slot(reply.query_id) {
            self.device.status.startup_query_summaries[index] = Some(reply.summary_label());
        }
    }

    fn push_query_reply_log(&mut self, reply: &QueryResponse, raw: Vec<u8>) {
        let detail = if reply.selector_bitmap().is_some() || reply.selector_pair_bank().is_some() {
            reply.summary_label()
        } else {
            let preview = reply
                .body
                .iter()
                .take(8)
                .map(|byte| format!("{:02x}", byte))
                .collect::<Vec<_>>()
                .join(" ");
            format!("[{} bytes] {}", reply.body.len(), preview)
        };
        let summary = format!(
            "0x75 {:02x}/{:02x} {}",
            reply.query_id, reply.sub_id, detail
        );
        self.raw_view.recent_query_reply_log.push(summary.clone());
        self.raw_view
            .recent_query_reply_entries
            .push(QueryReplyLogEntry { summary, raw });
        if self.raw_view.recent_query_reply_log.len() > 16 {
            let drop_count = self.raw_view.recent_query_reply_log.len() - 16;
            self.raw_view.recent_query_reply_log.drain(0..drop_count);
            self.raw_view
                .recent_query_reply_entries
                .drain(0..drop_count);
        }
        self.raw_view.selected_query_reply_entry =
            Some(self.raw_view.recent_query_reply_entries.len() - 1);
    }

    fn apply_query_reply_readback(&mut self, reply: &QueryResponse) {
        if let Some(assignments) = reply.assignment_readback() {
            for (index, assignment) in assignments.into_iter().enumerate() {
                let Some(assignment) = assignment else {
                    continue;
                };
                for channels in &mut self.mixer.channels {
                    channels[index].assignment = Some(assignment);
                }
            }
        }

        if let Some(startup_links) = reply.startup_link_readback_from_bitmap() {
            for (mixer, links) in startup_links {
                for (index, linked) in links.into_iter().enumerate() {
                    let Some(linked) = linked else {
                        continue;
                    };
                    let Some(slot) = self.mixer.channels[mixer.index()].get_mut(index) else {
                        continue;
                    };
                    slot.linked = Some(linked);
                }
            }
        }

        if let Some((mixer, states)) = reply.startup_pan_state_readback() {
            for (index, state) in states.into_iter().enumerate() {
                let Some(state) = state else {
                    continue;
                };
                let Some(slot) = self.mixer.channels[mixer.index()].get_mut(index) else {
                    continue;
                };
                slot.level = Some(state.level);
                slot.pan = state.pan;
                slot.muted = Some(state.muted);
                slot.soloed = Some(state.soloed);
            }
        }

        if let Some(readback) = reply.mixer_strip_readback() {
            for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
                for (index, state) in readback.surfaces[mixer.index()].into_iter().enumerate() {
                    let Some(slot) = self.mixer.channels[mixer.index()].get_mut(index) else {
                        continue;
                    };
                    slot.soloed = Some(state.soloed);
                }
            }
        }
    }

    pub fn mark_disconnected(&mut self) {
        self.device.connection.connected = false;
        self.device.connection.last_frame_type = Some("disconnected");
    }

    pub fn cycle_focus(&mut self) {
        self.ui.focus = match self.ui.focus {
            FocusArea::Status => FocusArea::Outputs,
            FocusArea::Outputs => FocusArea::Mixer,
            FocusArea::Mixer => FocusArea::Preamp,
            FocusArea::Preamp => FocusArea::Outputs,
        };
    }

    pub fn toggle_raw_view(&mut self) {
        self.popup.raw_view_open = !self.popup.raw_view_open;
    }

    pub fn toggle_hotkeys_popup(&mut self) {
        self.popup.hotkeys_open = !self.popup.hotkeys_open;
    }

    pub fn toggle_options_popup(&mut self) {
        self.popup.options_open = !self.popup.options_open;
    }

    pub fn selected_profile_name(&self) -> Option<&str> {
        self.popup
            .profile_names
            .get(self.popup.selected_index)
            .map(String::as_str)
    }

    pub fn clamp_profile_selection(&mut self) {
        if self.popup.profile_names.is_empty() {
            self.popup.selected_index = 0;
        } else {
            self.popup.selected_index = self
                .popup
                .selected_index
                .min(self.popup.profile_names.len().saturating_sub(1));
        }
    }

    pub fn cycle_raw_packet(&mut self, forward: bool) {
        let tabs = [
            RawPacketTab::Query74,
            RawPacketTab::State73,
            RawPacketTab::Auxiliary,
            RawPacketTab::Query75,
            RawPacketTab::DeviceNotification,
        ];
        let index = tabs
            .iter()
            .position(|tab| *tab == self.raw_view.selected_tab)
            .unwrap_or(0);
        self.raw_view.selected_tab = if forward {
            tabs[(index + 1) % tabs.len()]
        } else {
            tabs[index.checked_sub(1).unwrap_or(tabs.len() - 1)]
        };
    }

    pub fn cycle_query_reply_entry(&mut self, forward: bool) {
        if self.raw_view.recent_query_reply_entries.is_empty() {
            self.raw_view.selected_query_reply_entry = None;
            return;
        }
        let current = self
            .raw_view
            .selected_query_reply_entry
            .unwrap_or(self.raw_view.recent_query_reply_entries.len() - 1);
        self.raw_view.selected_query_reply_entry = Some(if forward {
            (current + 1) % self.raw_view.recent_query_reply_entries.len()
        } else {
            current
                .checked_sub(1)
                .unwrap_or(self.raw_view.recent_query_reply_entries.len() - 1)
        });
        self.ensure_query_reply_visible();
    }

    fn ensure_query_reply_visible(&mut self) {
        let Some(selected) = self.raw_view.selected_query_reply_entry else {
            return;
        };
        let total = self.raw_view.recent_query_reply_entries.len();
        let visible = QUERY_REPLY_VISIBLE_COUNT.min(total);
        let reversed_index = total - 1 - selected;
        if reversed_index < self.raw_view.query_reply_scroll {
            self.raw_view.query_reply_scroll = reversed_index;
        } else if reversed_index >= self.raw_view.query_reply_scroll + visible {
            self.raw_view.query_reply_scroll = reversed_index - visible + 1;
        }
    }

    pub fn capture_raw_baseline(&mut self) {
        self.raw_view.baseline_raw_73 = self.raw_view.latest_raw_73.clone();
        self.raw_view.baseline_raw_83 = self.raw_view.latest_raw_83.clone();
        self.raw_view.baseline_raw_74 = self.raw_view.latest_raw_74.clone();
        self.raw_view.baseline_raw_75 = self.raw_view.latest_raw_75.clone();
        self.raw_view.baseline_raw_81 = self.raw_view.latest_raw_81.clone();
    }

    pub fn clear_raw_baseline(&mut self) {
        self.raw_view.baseline_raw_73 = None;
        self.raw_view.baseline_raw_83 = None;
        self.raw_view.baseline_raw_74 = None;
        self.raw_view.baseline_raw_75 = None;
        self.raw_view.baseline_raw_81 = None;
    }

    pub fn observe_query_request(&mut self, raw: Vec<u8>) {
        self.raw_view.latest_raw_74 = Some(raw.clone());
        let query_id = raw.get(0x08).copied().unwrap_or(0);
        let sub_id = raw.get(0x0c).copied().unwrap_or(0);
        self.raw_view
            .recent_query_request_log
            .push(format!("0x74 {:02x}/{:02x}", query_id, sub_id));
        if self.raw_view.recent_query_request_log.len() > 16 {
            let drop_count = self.raw_view.recent_query_request_log.len() - 16;
            self.raw_view.recent_query_request_log.drain(0..drop_count);
        }
    }
}

fn startup_query_slot(query_id: u8) -> Option<usize> {
    match query_id {
        0x01 => Some(0),
        0x00 => Some(1),
        0x11 => Some(2),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PendingMutation {
    MixerLevel {
        mixer: MixerSurface,
        channel: u8,
        level: u8,
        pan: PanState,
        muted: bool,
    },
    MixerLinkedLevel {
        mixer: MixerSurface,
        left_channel: u8,
        right_channel: u8,
        level: u8,
        left_pan: PanState,
        right_pan: PanState,
        left_muted: bool,
        right_muted: bool,
    },
    MixerMute {
        mixer: MixerSurface,
        channel: u8,
        muted: bool,
    },
    MixerLinkedMute {
        mixer: MixerSurface,
        left_channel: u8,
        right_channel: u8,
        muted: bool,
    },
    MixerSolo {
        mixer: MixerSurface,
        channel: u8,
        soloed: bool,
    },
    MixerLinkedSolo {
        mixer: MixerSurface,
        left_channel: u8,
        right_channel: u8,
        soloed: bool,
    },
    MixerPan {
        mixer: MixerSurface,
        channel: u8,
        pan: PanState,
    },
    MixerAssignment {
        strip: u8,
        assignment: MixerAssignment,
    },
    MixerLink {
        mixer: MixerSurface,
        selector: u8,
        enabled: bool,
    },
    MixerLinkExplicit {
        mixer: MixerSurface,
        left_channel: u8,
        right_channel: u8,
        enabled: bool,
    },
    OutputVolume {
        target: OutputTarget,
        step: u8,
    },
    OutputMode {
        target: OutputTarget,
        mode: OutputMode,
    },
    PreampGain {
        input: u8,
        raw: u8,
    },
    PreampMode {
        input: u8,
        mode: PreampMode,
    },
    PreampPhantom {
        input: u8,
        enabled: bool,
    },
    PreampPhase {
        input: u8,
        enabled: bool,
    },
}

/// Computes the next preamp gain raw value for increment/decrement.
fn next_preamp_gain_raw(current: u8, up: bool) -> u8 {
    if up {
        current.saturating_add(1).min(0x41)
    } else {
        current.saturating_sub(1)
    }
}

pub struct Controller {
    transport: Box<dyn Transport>,
    pub state: AppState,
    pending_mutation: Option<PendingMutation>,
    command_queue: CommandQueue,
}

const MAX_FRAMES_PER_POLL: usize = 32;

impl Controller {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            state: AppState::default(),
            pending_mutation: None,
            command_queue: CommandQueue::new(),
        }
    }

    pub fn bootstrap(&mut self) -> Result<()> {
        self.refresh_queried_state()
    }

    pub fn transport_available(&self) -> Result<bool> {
        self.transport.is_available()
    }

    pub fn refresh_queried_state(&mut self) -> Result<()> {
        for query in control_panel_startup_queries() {
            let frame = encode_query(*query);
            self.state.observe_query_request(frame.clone());
            self.transport.write(&frame)?;
        }
        Ok(())
    }

    pub fn apply_profile(&mut self, profile: &DeviceProfile) -> Result<()> {
        profile.validate()?;

        // Flush any pending commands before applying profile
        self.flush_commands()?;

        for (target, output) in [
            (OutputTarget::Monitor, &profile.outputs.monitor),
            (OutputTarget::Hp1, &profile.outputs.hp1),
            (OutputTarget::Hp2, &profile.outputs.hp2),
        ] {
            self.send(
                Command::SetOutputVolume {
                    target,
                    step: output.volume_step,
                },
                None,
            )?;
            self.send(
                Command::SetOutputDim {
                    target,
                    enabled: false,
                },
                None,
            )?;
            self.send(
                Command::SetOutputMute {
                    target,
                    enabled: false,
                },
                None,
            )?;
            match output.mode.into_device() {
                OutputMode::Normal => {}
                OutputMode::Mute => self.send(
                    Command::SetOutputMute {
                        target,
                        enabled: true,
                    },
                    None,
                )?,
                OutputMode::Dim => self.send(
                    Command::SetOutputDim {
                        target,
                        enabled: true,
                    },
                    None,
                )?,
                OutputMode::Unknown(_) => unreachable!(),
            }
        }
        self.flush_commands()?;

        for (input, preamp) in [
            (0_u8, &profile.preamps.input1),
            (1_u8, &profile.preamps.input2),
        ] {
            self.send(
                Command::SetPreampMode {
                    input,
                    mode: preamp.mode.into_device(),
                },
                None,
            )?;
            self.send(
                Command::SetPreampGain {
                    input,
                    raw: preamp.gain_raw,
                },
                None,
            )?;
            self.send(
                Command::SetPreampPhantom {
                    input,
                    enabled: preamp.phantom_on,
                },
                None,
            )?;
            self.send(
                Command::SetPreampPhase {
                    input,
                    enabled: preamp.phase_inverted,
                },
                None,
            )?;
        }
        self.flush_commands()?;

        let assignments = profile.assignment_table()?;
        for entry in &profile.assignments {
            for frame in encode_mixer_assignment_frames_with_table(
                entry.channel,
                entry.source.into_device(),
                &assignments,
            ) {
                self.transport.write(&frame)?;
            }
        }

        for (mixer, strips) in [
            (MixerSurface::Mix1, &profile.mixers.mix1),
            (MixerSurface::Mix2, &profile.mixers.mix2),
        ] {
            for strip in strips.iter().step_by(2) {
                self.send_mixer_link_change(mixer, strip.channel, strip.linked)?;
            }
            for strip in strips {
                self.send(
                    Command::SetMixerLevel {
                        mixer,
                        channel: strip.channel,
                        level: strip.level_raw,
                        pan_state: PanState::from_raw(strip.pan_raw),
                        muted: strip.muted,
                        soloed: strip.soloed,
                    },
                    None,
                )?;
            }
        }
        self.flush_commands()?;

        profile.apply_to_state(&mut self.state);
        self.pending_mutation = None;
        self.state.preamp.state.cluster = [
            self.state.preamp.state.input1.gain_raw,
            self.state.preamp.state.input2.gain_raw,
            preamp_mode_raw(
                profile.preamps.input1.mode,
                profile.preamps.input1.phantom_on,
                profile.preamps.input1.phase_inverted,
            ),
            preamp_mode_raw(
                profile.preamps.input2.mode,
                profile.preamps.input2.phantom_on,
                profile.preamps.input2.phase_inverted,
            ),
        ];
        self.state.ui.last_message = "Applied profile".to_string();
        Ok(())
    }

    fn shared_assignment_table(&self) -> Result<[MixerAssignment; 16]> {
        let mut assignments = [MixerAssignment::Mute; 16];
        for (index, slot) in assignments.iter_mut().enumerate() {
            *slot = self.state.mixer.channels[0][index]
                .assignment
                .or(self.state.mixer.channels[1][index].assignment)
                .ok_or_else(|| {
                    anyhow::anyhow!("assignment table is incomplete for CH {:02}", index + 1)
                })?;
        }
        Ok(assignments)
    }

    pub fn send(&mut self, command: Command, pending: Option<PendingMutation>) -> Result<()> {
        // Multi-frame assignment: always write directly
        if let Command::SetMixerAssignment { strip, assignment } = command {
            let assignments = self.shared_assignment_table()?;
            for frame in encode_mixer_assignment_frames_with_table(strip, assignment, &assignments)
            {
                self.transport.write(&frame)?;
            }
            self.pending_mutation = pending;
            self.state.ui.last_message = format!("Sent {:?}", command);
            return Ok(());
        }

        // Link state with companion bank: flush queue first, then write directly
        if let Command::SetLinkState {
            enabled,
            companion_bank: Some(bank),
            ..
        } = command
        {
            self.flush_commands()?;
            self.transport
                .write(&encode_link_companion(bank, enabled))?;
            self.transport.write(&encode_command(command))?;
            self.apply_command_state_update(&command);
            self.pending_mutation = pending;
            self.state.ui.last_message = format!("Sent {:?}", command);
            return Ok(());
        }

        // SelectSurface: flush, write directly, then refresh queried state
        if let Command::SelectSurface(_) = &command {
            self.flush_commands()?;
            self.transport.write(&encode_command(command))?;
            self.apply_command_state_update(&command);
            self.pending_mutation = pending;
            self.state.ui.last_message = format!("Sent {:?}", command);
            self.refresh_queried_state()?;
            return Ok(());
        }

        // All other commands: enqueue for coalescing
        self.command_queue.enqueue(command.clone());
        self.apply_command_state_update(&command);
        self.pending_mutation = pending;
        self.state.ui.last_message = format!("Sent {:?}", command);
        Ok(())
    }

    /// Applies immediate state updates for commands that affect visible state.
    fn apply_command_state_update(&mut self, command: &Command) {
        match command {
            Command::SetClockSource(source) => {
                self.state.device.status.clock_source = Some(*source);
            }
            Command::SetSampleRate(rate) => {
                self.state.device.status.sample_rate = Some(*rate);
                self.state.device.status.sample_rate_hz = rate.hz();
            }
            _ => {}
        }
    }

    /// Flushes all pending commands from the queue to the transport.
    pub fn flush_commands(&mut self) -> Result<()> {
        self.command_queue.flush(self.transport.as_ref())?;
        Ok(())
    }

    fn resolve_linked_pair(
        &self,
        mixer: MixerSurface,
        channel: u8,
    ) -> Result<(u8, u8, MixerChannelState, MixerChannelState)> {
        let (left_channel, right_channel) = if channel % 2 == 1 {
            (channel, channel.saturating_add(1))
        } else {
            (channel.saturating_sub(1), channel)
        };
        let left_index = left_channel.saturating_sub(1) as usize;
        let right_index = right_channel.saturating_sub(1) as usize;
        let Some(left) = self.state.mixer.channels[mixer.index()]
            .get(left_index)
            .copied()
        else {
            bail!("invalid linked left channel {left_channel}");
        };
        let Some(right) = self.state.mixer.channels[mixer.index()]
            .get(right_index)
            .copied()
        else {
            bail!("invalid linked right channel {right_channel}");
        };
        Ok((left_channel, right_channel, left, right))
    }

    pub fn send_mixer_level_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        level: u8,
    ) -> Result<()> {
        let index = channel.saturating_sub(1) as usize;
        let Some(active) = self.state.mixer.channels[mixer.index()].get(index).copied() else {
            bail!("invalid mixer channel {channel}");
        };

        if active.linked == Some(true) {
            let (left_ch, right_ch, left, right) = self.resolve_linked_pair(mixer, channel)?;

            if let Some(slot) =
                self.state.mixer.channels[mixer.index()].get_mut(left_ch.saturating_sub(1) as usize)
            {
                slot.level = Some(level);
                slot.pan = left.pan;
                slot.muted = left.muted;
            }
            if let Some(slot) = self.state.mixer.channels[mixer.index()]
                .get_mut(right_ch.saturating_sub(1) as usize)
            {
                slot.level = Some(level);
                slot.pan = right.pan;
                slot.muted = right.muted;
            }

            let pending_mutation = Some(PendingMutation::MixerLinkedLevel {
                mixer,
                left_channel: left_ch,
                right_channel: right_ch,
                level,
                left_pan: left.pan,
                right_pan: right.pan,
                left_muted: left.muted.unwrap_or(false),
                right_muted: right.muted.unwrap_or(false),
            });
            self.flush_commands()?;
            self.transport
                .write(&encode_command(Command::SetMixerLevel {
                    mixer,
                    channel: left_ch,
                    level,
                    pan_state: left.pan,
                    muted: left.muted.unwrap_or(false),
                    soloed: left.soloed.unwrap_or(false),
                }))?;
            self.transport
                .write(&encode_command(Command::SetMixerLevel {
                    mixer,
                    channel: right_ch,
                    level,
                    pan_state: right.pan,
                    muted: right.muted.unwrap_or(false),
                    soloed: right.soloed.unwrap_or(false),
                }))?;
            self.pending_mutation = pending_mutation;
            self.state.ui.last_message = format!(
                "Sent linked mixer level {:?} ch {}-{}",
                mixer, left_ch, right_ch
            );
            return Ok(());
        }

        if let Some(slot) = self.state.mixer.channels[mixer.index()].get_mut(index) {
            slot.level = Some(level);
            slot.pan = active.pan;
            slot.muted = active.muted;
        }

        self.send(
            Command::SetMixerLevel {
                mixer,
                channel,
                level,
                pan_state: active.pan,
                muted: active.muted.unwrap_or(false),
                soloed: active.soloed.unwrap_or(false),
            },
            Some(PendingMutation::MixerLevel {
                mixer,
                channel,
                level,
                pan: active.pan,
                muted: active.muted.unwrap_or(false),
            }),
        )
    }

    pub fn send_mixer_mute_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        muted: bool,
    ) -> Result<()> {
        let index = channel.saturating_sub(1) as usize;
        let Some(active) = self.state.mixer.channels[mixer.index()].get(index).copied() else {
            bail!("invalid mixer channel {channel}");
        };

        if active.linked == Some(true) {
            let (left_ch, right_ch, left, right) = self.resolve_linked_pair(mixer, channel)?;

            if let Some(slot) =
                self.state.mixer.channels[mixer.index()].get_mut(left_ch.saturating_sub(1) as usize)
            {
                slot.muted = Some(muted);
            }
            if let Some(slot) = self.state.mixer.channels[mixer.index()]
                .get_mut(right_ch.saturating_sub(1) as usize)
            {
                slot.muted = Some(muted);
            }

            let pending_mutation = Some(PendingMutation::MixerLinkedMute {
                mixer,
                left_channel: left_ch,
                right_channel: right_ch,
                muted,
            });
            self.flush_commands()?;
            self.transport
                .write(&encode_command(Command::SetMixerMute {
                    mixer,
                    channel: left_ch,
                    muted,
                    pan_state: left.pan,
                    soloed: left.soloed.unwrap_or(false),
                }))?;
            self.transport
                .write(&encode_command(Command::SetMixerMute {
                    mixer,
                    channel: right_ch,
                    muted,
                    pan_state: right.pan,
                    soloed: right.soloed.unwrap_or(false),
                }))?;
            self.pending_mutation = pending_mutation;
            self.state.ui.last_message = format!(
                "Sent linked mixer mute {:?} ch {}-{}",
                mixer, left_ch, right_ch
            );
            return Ok(());
        }

        if let Some(slot) = self.state.mixer.channels[mixer.index()].get_mut(index) {
            slot.muted = Some(muted);
        }

        self.send(
            Command::SetMixerMute {
                mixer,
                channel,
                muted,
                pan_state: active.pan,
                soloed: active.soloed.unwrap_or(false),
            },
            Some(PendingMutation::MixerMute {
                mixer,
                channel,
                muted,
            }),
        )
    }

    pub fn send_mixer_solo_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        soloed: bool,
    ) -> Result<()> {
        let index = channel.saturating_sub(1) as usize;
        let Some(active) = self.state.mixer.channels[mixer.index()].get(index).copied() else {
            bail!("invalid mixer channel {channel}");
        };

        if active.linked == Some(true) {
            let (left_ch, right_ch, left, right) = self.resolve_linked_pair(mixer, channel)?;

            if let Some(slot) =
                self.state.mixer.channels[mixer.index()].get_mut(left_ch.saturating_sub(1) as usize)
            {
                slot.soloed = Some(soloed);
            }
            if let Some(slot) = self.state.mixer.channels[mixer.index()]
                .get_mut(right_ch.saturating_sub(1) as usize)
            {
                slot.soloed = Some(soloed);
            }

            let pending_mutation = Some(PendingMutation::MixerLinkedSolo {
                mixer,
                left_channel: left_ch,
                right_channel: right_ch,
                soloed,
            });
            self.flush_commands()?;
            self.transport
                .write(&encode_command(Command::SetMixerSolo {
                    mixer,
                    channel: left_ch,
                    soloed,
                    muted: left.muted.unwrap_or(false),
                    pan_state: left.pan,
                }))?;
            self.transport
                .write(&encode_command(Command::SetMixerSolo {
                    mixer,
                    channel: right_ch,
                    soloed,
                    muted: right.muted.unwrap_or(false),
                    pan_state: right.pan,
                }))?;
            self.pending_mutation = pending_mutation;
            self.state.ui.last_message = format!(
                "Sent linked mixer solo {:?} ch {}-{}",
                mixer, left_ch, right_ch
            );
            return Ok(());
        }

        if let Some(slot) = self.state.mixer.channels[mixer.index()].get_mut(index) {
            slot.soloed = Some(soloed);
        }

        self.send(
            Command::SetMixerSolo {
                mixer,
                channel,
                soloed,
                muted: active.muted.unwrap_or(false),
                pan_state: active.pan,
            },
            Some(PendingMutation::MixerSolo {
                mixer,
                channel,
                soloed,
            }),
        )
    }

    pub fn send_mixer_link_change(
        &mut self,
        mixer: MixerSurface,
        channel: u8,
        enabled: bool,
    ) -> Result<()> {
        let Some(target) = MixerLinkTarget::from_channel(mixer, channel) else {
            bail!("invalid mixer link channel {channel}");
        };
        let pending_mutation = Some(PendingMutation::MixerLinkExplicit {
            mixer,
            left_channel: target.left_channel,
            right_channel: target.right_channel,
            enabled,
        });
        self.flush_commands()?;
        if let Some(bank) = target.companion_bank() {
            self.transport
                .write(&encode_link_companion(bank, enabled))?;
        }
        self.transport
            .write(&encode_command(Command::SetLinkState {
                selector: target.selector,
                enabled,
                companion_bank: None,
            }))?;
        self.pending_mutation = pending_mutation;
        self.state.ui.last_message = format!(
            "Sent mixer link {:?} ch {}-{}",
            mixer, target.left_channel, target.right_channel
        );
        Ok(())
    }

    pub fn apply_intent(&mut self, intent: Intent, area: Rect) -> Result<()> {
        let pending = intent.pending_mutation(&self.state);
        match intent {
            Intent::Quit => {
                self.state.ui.quit_requested = true;
            }
            Intent::ToggleRawView => self.state.toggle_raw_view(),
            Intent::ToggleHotkeysPopup => self.state.toggle_hotkeys_popup(),
            Intent::OpenProfilesPopup => {
                self.state.popup.assignment_picker = None;
                self.state.popup.selector_popup = None;
                self.state.popup.routing_open = false;
                self.state.popup.profile_editor = None;
                self.state.popup.profile_names =
                    crate::profile::list_profile_names().unwrap_or_default();
                self.state.clamp_profile_selection();
                self.state.popup.profiles_open = true;
                self.state.ui.last_message = if self.state.popup.profile_names.is_empty() {
                    "No saved profiles yet. Use SAVE to create one.".to_string()
                } else {
                    "Select a profile to load, or use SAVE/RENAME/DELETE.".to_string()
                };
            }
            Intent::CloseProfilesPopup => {
                self.state.popup.profiles_open = false;
                self.state.popup.profile_editor = None;
                self.state.ui.last_message = "Closed profiles popup".to_string();
            }
            Intent::OpenRoutingPopup => {
                self.state.popup.profiles_open = false;
                self.state.popup.profile_editor = None;
                self.state.popup.routing_open = true;
                self.state.ui.focus = FocusArea::Mixer;
                self.state.mixer.selected_channel = self.state.mixer.selected_channel.min(7);
                self.state.ui.last_message =
                    "Routing popup mirrors mixer assignments for USB recording channels 1-8"
                        .to_string();
            }
            Intent::CloseRoutingPopup => {
                self.state.popup.routing_open = false;
                self.state.ui.last_message = "Closed routing popup".to_string();
            }
            Intent::OpenOptionsPopup => {
                self.state.popup.profiles_open = false;
                self.state.popup.profile_editor = None;
                self.state.popup.routing_open = false;
                self.state.popup.options_open = true;
                self.state.ui.last_message = "Options popup opened".to_string();
            }
            Intent::CloseOptionsPopup => {
                self.state.popup.options_open = false;
                self.state.ui.last_message = "Closed options popup".to_string();
            }
            Intent::SetRefreshRate(rate) => {
                self.state.ui.settings.refresh_rate = rate;
                self.state.ui.last_message = format!("Refresh rate set to {}", rate.label());
                if self.state.ui.settings.auto_save {
                    let _ = crate::settings::save_settings(&self.state.ui.settings);
                }
            }
            Intent::CyclePeakThreshold(increase) => {
                const PEAK_THRESHOLD_CHOICES: [u8; 10] =
                    [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x0a, 0x0f, 0x14];
                let current = self.state.ui.settings.peak_threshold_raw;
                let pos = PEAK_THRESHOLD_CHOICES
                    .iter()
                    .position(|&v| v == current)
                    .unwrap_or(3);
                let next_pos = if increase {
                    (pos + 1).min(PEAK_THRESHOLD_CHOICES.len() - 1)
                } else {
                    pos.saturating_sub(1)
                };
                self.state.ui.settings.peak_threshold_raw = PEAK_THRESHOLD_CHOICES[next_pos];
                let db = self.state.ui.settings.peak_threshold_db();
                self.state.ui.last_message = format!("Peak threshold set to {} dB", db);
                if self.state.ui.settings.auto_save {
                    let _ = crate::settings::save_settings(&self.state.ui.settings);
                }
            }
            Intent::TogglePeakEnabled => {
                self.state.ui.settings.peak_enabled = !self.state.ui.settings.peak_enabled;
                if self.state.ui.settings.peak_enabled {
                    self.state.ui.last_message = "Peak detection enabled".to_string();
                } else {
                    self.state.preamp.peaks = [None, None];
                    self.state.mixer.peaks = [[None; 16]; 2];
                    self.state.ui.last_message = "Peak detection disabled".to_string();
                }
                if self.state.ui.settings.auto_save {
                    let _ = crate::settings::save_settings(&self.state.ui.settings);
                }
            }
            Intent::CyclePeakHoldDuration(duration) => {
                self.state.ui.settings.peak_hold_duration = duration;
                self.state.ui.last_message =
                    format!("Peak hold duration set to {}", duration.label());
                if self.state.ui.settings.auto_save {
                    let _ = crate::settings::save_settings(&self.state.ui.settings);
                }
            }
            Intent::ToggleAutoSave => {
                self.state.ui.settings.auto_save = !self.state.ui.settings.auto_save;
                if self.state.ui.settings.auto_save {
                    self.state.ui.last_message = "Auto-save enabled".to_string();
                    let _ = crate::settings::save_settings(&self.state.ui.settings);
                } else {
                    self.state.ui.last_message = "Auto-save disabled".to_string();
                }
            }
            Intent::SelectProfile(index) => {
                self.state.popup.selected_index =
                    index.min(self.state.popup.profile_names.len().saturating_sub(1));
            }
            Intent::LoadSelectedProfile => {
                if let Some(name) = self.state.selected_profile_name().map(str::to_string) {
                    let profile_result = crate::profile::DeviceProfile::read_named(&name);
                    match profile_result {
                        Ok(profile) => {
                            let apply_result = self.apply_profile(&profile);
                            if let Err(e) = apply_result {
                                self.state.ui.last_message = format!("Profile error: {e}");
                            } else {
                                self.state.popup.profiles_open = false;
                                self.state.popup.profile_editor = None;
                                self.state.ui.last_message = format!("Loaded profile {name}");
                            }
                        }
                        Err(e) => {
                            self.state.ui.last_message = format!("Profile error: {e}");
                        }
                    }
                } else {
                    self.state.ui.last_message = "No profile selected to load.".to_string();
                }
            }
            Intent::StartSaveProfile => {
                if self.state.popup.profiles_open {
                    let current_name = self.state.selected_profile_name().map(str::to_string);
                    let value = current_name.clone().unwrap_or_default();
                    self.state.popup.profile_editor = Some(ProfileEditorState {
                        mode: ProfileEditorMode::Save,
                        original_name: current_name,
                        value,
                    });
                    self.state.ui.last_message =
                        "Enter a profile name, then press Enter to save.".to_string();
                }
            }
            Intent::StartRenameProfile => {
                if self.state.selected_profile_name().is_some() {
                    let current_name = self.state.selected_profile_name().map(str::to_string);
                    let value = current_name.clone().unwrap_or_default();
                    self.state.popup.profile_editor = Some(ProfileEditorState {
                        mode: ProfileEditorMode::Rename,
                        original_name: current_name,
                        value,
                    });
                    self.state.ui.last_message =
                        "Edit the profile name, then press Enter to rename.".to_string();
                } else {
                    self.state.ui.last_message = "No profile selected to rename.".to_string();
                }
            }
            Intent::DeleteSelectedProfile => {
                if let Some(name) = self.state.selected_profile_name().map(str::to_string) {
                    match crate::profile::delete_profile(&name) {
                        Ok(()) => {
                            self.state.popup.profile_names =
                                crate::profile::list_profile_names().unwrap_or_default();
                            self.state.clamp_profile_selection();
                            self.state.ui.last_message = format!("Deleted profile {name}");
                        }
                        Err(e) => {
                            self.state.ui.last_message = format!("Profile error: {e}");
                        }
                    }
                } else {
                    self.state.ui.last_message = "No profile selected to delete.".to_string();
                }
            }
            Intent::PageMixerStripsLeft => {
                self.state.ui.focus = FocusArea::Mixer;
                let visible = crate::ui::mixer_strip_viewport_capacity(area, &self.state);
                self.state.page_mixer_strip_viewport(false, visible);
            }
            Intent::PageMixerStripsRight => {
                self.state.ui.focus = FocusArea::Mixer;
                let visible = crate::ui::mixer_strip_viewport_capacity(area, &self.state);
                self.state.page_mixer_strip_viewport(true, visible);
            }
            Intent::OpenSampleRateSelector => {
                if self.state.device.status.clock_source == Some(ClockSource::Internal) {
                    self.state.popup.selected_index = self
                        .state
                        .device
                        .status
                        .sample_rate
                        .and_then(|current| {
                            SampleRate::all_confirmed()
                                .iter()
                                .position(|rate| *rate == current)
                        })
                        .unwrap_or(0);
                    self.state.popup.selector_popup = Some(SelectorPopupState {
                        kind: SelectorPopupKind::SampleRate,
                    });
                }
            }
            Intent::OpenClockSourceSelector => {
                self.state.popup.selected_index = self
                    .state
                    .device
                    .status
                    .clock_source
                    .and_then(|current| {
                        ClockSource::all_confirmed()
                            .iter()
                            .position(|source| *source == current)
                    })
                    .unwrap_or(0);
                self.state.popup.selector_popup = Some(SelectorPopupState {
                    kind: SelectorPopupKind::ClockSource,
                });
            }
            Intent::SelectPage(page) => self.state.ui.page = page,
            Intent::SelectRawPacketTab(tab) => self.state.raw_view.selected_tab = tab,
            Intent::SelectOutput(index) => {
                self.state.ui.focus = FocusArea::Outputs;
                self.state.output.selected = index.min(self.state.output.states.len() - 1);
            }
            Intent::AdjustOutputLevel { index, increase } => {
                self.state.ui.focus = FocusArea::Outputs;
                self.state.output.selected = index.min(self.state.output.states.len() - 1);
                let output = self.state.output.states[self.state.output.selected];
                let next = if increase {
                    output.volume.saturating_sub(1)
                } else {
                    output.volume.saturating_add(1).min(0x60)
                };
                self.state.output.states[self.state.output.selected].volume = next;
                self.send(
                    Command::SetOutputVolume {
                        target: output.target,
                        step: next,
                    },
                    pending.clone(),
                )?;
            }
            Intent::SetOutputLevel { index, step } => {
                self.state.ui.focus = FocusArea::Outputs;
                self.state.output.selected = index.min(self.state.output.states.len() - 1);
                let output = self.state.output.states[self.state.output.selected];
                self.state.output.states[self.state.output.selected].volume = step.min(0x60);
                self.send(
                    Command::SetOutputVolume {
                        target: output.target,
                        step: step.min(0x60),
                    },
                    pending.clone(),
                )?;
            }
            Intent::ToggleOutputDim(index) => {
                self.state.ui.focus = FocusArea::Outputs;
                self.state.output.selected = index.min(self.state.output.states.len() - 1);
                let output = self.state.output.states[self.state.output.selected];
                let new_mode = if output.mode != OutputMode::Dim {
                    OutputMode::Dim
                } else {
                    OutputMode::Normal
                };
                self.state.output.states[self.state.output.selected].mode = new_mode;
                self.send(
                    Command::SetOutputDim {
                        target: output.target,
                        enabled: output.mode != OutputMode::Dim,
                    },
                    pending.clone(),
                )?;
            }
            Intent::ToggleOutputMute(index) => {
                self.state.ui.focus = FocusArea::Outputs;
                self.state.output.selected = index.min(self.state.output.states.len() - 1);
                let output = self.state.output.states[self.state.output.selected];
                let new_mode = if output.mode != OutputMode::Mute {
                    OutputMode::Mute
                } else {
                    OutputMode::Normal
                };
                self.state.output.states[self.state.output.selected].mode = new_mode;
                self.send(
                    Command::SetOutputMute {
                        target: output.target,
                        enabled: output.mode != OutputMode::Mute,
                    },
                    pending.clone(),
                )?;
            }
            Intent::SelectQueryReplyEntry(index) => {
                self.state.raw_view.selected_query_reply_entry = Some(
                    index.min(
                        self.state
                            .raw_view
                            .recent_query_reply_entries
                            .len()
                            .saturating_sub(1),
                    ),
                );
            }
            Intent::ScrollQueryReplyList { increase } => {
                self.state.cycle_query_reply_entry(increase);
            }
            Intent::SelectSurface(surface) => {
                self.state.ui.focus = FocusArea::Mixer;
                self.send(Command::SelectSurface(surface), pending.clone())?;
                self.flush_commands()?;
                self.refresh_queried_state()?;
            }
            Intent::SelectMixerChannel(index) => {
                self.state.ui.focus = FocusArea::Mixer;
                self.state.mixer.selected_channel = index;
            }
            Intent::AdjustMixerLevel { index, increase } => {
                self.state.ui.focus = FocusArea::Mixer;
                self.state.mixer.selected_channel =
                    index.min(self.state.active_mixer_channels().len() - 1);
                let active_channel =
                    self.state.active_mixer_channels()[self.state.mixer.selected_channel];
                let current = active_channel.level.unwrap_or(0x20);
                let next = if increase {
                    current.saturating_sub(1)
                } else {
                    current.saturating_add(1).min(0x60)
                };
                self.send_mixer_level_change(
                    MixerSurface::from_surface(self.state.mixer.surface),
                    active_channel.channel,
                    next,
                )?;
            }
            Intent::SetMixerLevel { index, level } => {
                self.state.ui.focus = FocusArea::Mixer;
                self.state.mixer.selected_channel =
                    index.min(self.state.active_mixer_channels().len() - 1);
                let active_channel =
                    self.state.active_mixer_channels()[self.state.mixer.selected_channel];
                self.send_mixer_level_change(
                    MixerSurface::from_surface(self.state.mixer.surface),
                    active_channel.channel,
                    level.min(0x5a),
                )?;
            }
            Intent::AdjustMixerPan { index, right } => {
                self.state.ui.focus = FocusArea::Mixer;
                self.state.mixer.selected_channel =
                    index.min(self.state.active_mixer_channels().len() - 1);
                let active_channel =
                    self.state.active_mixer_channels()[self.state.mixer.selected_channel];
                let next = if right {
                    active_channel
                        .pan
                        .raw()
                        .saturating_add(1)
                        .min(PanState::MAX)
                } else {
                    active_channel
                        .pan
                        .raw()
                        .saturating_sub(1)
                        .max(PanState::MIN)
                };
                let surface = MixerSurface::from_surface(self.state.mixer.surface);
                if let Some(slot) = self.state.mixer.channels[surface.index()]
                    .get_mut(active_channel.channel.saturating_sub(1) as usize)
                {
                    slot.pan = PanState::from_raw(next);
                }
                self.send(
                    Command::SetMixerPan {
                        mixer: surface,
                        channel: active_channel.channel,
                        pan: PanState::from_raw(next),
                        muted: active_channel.muted.unwrap_or(false),
                        soloed: active_channel.soloed.unwrap_or(false),
                    },
                    pending.clone(),
                )?;
            }
            Intent::SetMixerPan { index, pan } => {
                self.state.ui.focus = FocusArea::Mixer;
                self.state.mixer.selected_channel =
                    index.min(self.state.active_mixer_channels().len() - 1);
                let active_channel =
                    self.state.active_mixer_channels()[self.state.mixer.selected_channel];
                let surface = MixerSurface::from_surface(self.state.mixer.surface);
                if let Some(slot) = self.state.mixer.channels[surface.index()]
                    .get_mut(active_channel.channel.saturating_sub(1) as usize)
                {
                    slot.pan = pan;
                }
                self.send(
                    Command::SetMixerPan {
                        mixer: surface,
                        channel: active_channel.channel,
                        pan,
                        muted: active_channel.muted.unwrap_or(false),
                        soloed: active_channel.soloed.unwrap_or(false),
                    },
                    pending.clone(),
                )?;
            }
            Intent::ToggleMixerMute(channel) => {
                self.state.ui.focus = FocusArea::Mixer;
                self.state.mixer.selected_channel = channel.saturating_sub(1) as usize;
                let mixer = MixerSurface::from_surface(self.state.mixer.surface);
                let active_channel = self.state.mixer.channels[mixer.index()][channel as usize - 1];
                self.send_mixer_mute_change(
                    mixer,
                    channel,
                    !active_channel.muted.unwrap_or(false),
                )?;
            }
            Intent::ToggleMixerSolo(channel) => {
                self.state.ui.focus = FocusArea::Mixer;
                self.state.mixer.selected_channel = channel.saturating_sub(1) as usize;
                let mixer = MixerSurface::from_surface(self.state.mixer.surface);
                let active_channel = self.state.mixer.channels[mixer.index()][channel as usize - 1];
                self.send_mixer_solo_change(
                    mixer,
                    channel,
                    !active_channel.soloed.unwrap_or(false),
                )?;
            }
            Intent::ToggleMixerLink(channel) => {
                self.state.ui.focus = FocusArea::Mixer;
                self.state.mixer.selected_channel = channel.saturating_sub(1) as usize;
                let mixer = MixerSurface::from_surface(self.state.mixer.surface);
                let active_channel = self.state.mixer.channels[mixer.index()][channel as usize - 1];
                self.send_mixer_link_change(
                    mixer,
                    channel,
                    !active_channel.linked.unwrap_or(false),
                )?;
            }
            Intent::OpenAssignmentPicker(strip) => {
                self.state.ui.focus = FocusArea::Mixer;
                self.state.mixer.selected_channel = strip.saturating_sub(1) as usize;
                if !antelope_protocol::MixerStrip::assignment_write_is_grounded(strip) {
                    self.state.ui.last_message =
                        "Assignment picking is not grounded for the selected strip.".to_string();
                } else {
                    self.state.popup.selected_index = self.state.mixer.channels
                        [MixerSurface::from_surface(self.state.mixer.surface).index()]
                        [self.state.mixer.selected_channel]
                        .assignment
                        .and_then(|current| {
                            MixerAssignment::grounded_choices()
                                .iter()
                                .position(|assignment| *assignment == current)
                        })
                        .unwrap_or(0);
                    self.state.popup.assignment_picker = Some(AssignmentPickerState { strip });
                    self.state.ui.last_message =
                        format!("Pick source assignment for CH {strip:02}");
                }
            }
            Intent::PickAssignment { strip, assignment } => {
                self.state.popup.assignment_picker = None;
                self.state.popup.selected_index = 0;
                self.send(
                    Command::SetMixerAssignment { strip, assignment },
                    pending.clone(),
                )?;
            }
            Intent::CloseAssignmentPicker => {
                self.state.popup.assignment_picker = None;
                self.state.popup.selected_index = 0;
                self.state.ui.last_message = "Closed assignment picker".to_string();
            }
            Intent::CloseSelectorPopup => {
                self.state.popup.selector_popup = None;
                self.state.popup.selected_index = 0;
                self.state.ui.last_message = "Closed selector".to_string();
            }
            Intent::SelectPreampInput(input) => {
                self.state.ui.focus = FocusArea::Preamp;
                self.state.preamp.selected_input = input.min(1);
            }
            Intent::AdjustPreampGain { input, increase } => {
                self.state.ui.focus = FocusArea::Preamp;
                self.state.preamp.selected_input = input.min(1) as usize;
                let current = if input == 0 {
                    self.state.preamp.state.input1.gain_raw
                } else {
                    self.state.preamp.state.input2.gain_raw
                };
                let next = next_preamp_gain_raw(current, increase);
                self.state.device.dsp_cluster[input.min(1) as usize] = next;
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                self.send(Command::SetPreampGain { input, raw: next }, pending.clone())?;
            }
            Intent::SetPreampGain { input, raw } => {
                self.state.ui.focus = FocusArea::Preamp;
                self.state.preamp.selected_input = input.min(1) as usize;
                self.state.device.dsp_cluster[input.min(1) as usize] = raw;
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                self.send(
                    Command::SetPreampGain {
                        input: input.min(1),
                        raw,
                    },
                    pending.clone(),
                )?;
            }
            Intent::OpenPreampModeSelector(input) => {
                self.state.ui.focus = FocusArea::Preamp;
                self.state.preamp.selected_input = input.min(1) as usize;
                let current = if input == 0 {
                    self.state.preamp.state.input1.mode
                } else {
                    self.state.preamp.state.input2.mode
                };
                self.state.popup.selected_index =
                    [PreampMode::Mic, PreampMode::Line, PreampMode::HiZ]
                        .iter()
                        .position(|mode| *mode == current)
                        .unwrap_or(0);
                self.state.popup.selector_popup = Some(SelectorPopupState {
                    kind: SelectorPopupKind::PreampMode { input },
                });
            }
            Intent::CyclePreampMode(input) => {
                self.state.ui.focus = FocusArea::Preamp;
                self.state.preamp.selected_input = input.min(1) as usize;
                let current = if input == 0 {
                    self.state.preamp.state.input1.mode
                } else {
                    self.state.preamp.state.input2.mode
                };
                let next = match current {
                    PreampMode::Mic => PreampMode::Line,
                    PreampMode::Line => PreampMode::HiZ,
                    PreampMode::HiZ | PreampMode::Unknown(_) => PreampMode::Mic,
                };
                self.send(
                    Command::SetPreampMode { input, mode: next },
                    pending.clone(),
                )?;
            }
            Intent::PickSampleRate(rate) => {
                self.state.popup.selector_popup = None;
                self.state.popup.selected_index = 0;
                self.send(Command::SetSampleRate(rate), pending.clone())?;
            }
            Intent::PickClockSource(source) => {
                self.state.popup.selector_popup = None;
                self.state.popup.selected_index = 0;
                self.send(Command::SetClockSource(source), pending.clone())?;
            }
            Intent::PickPreampMode { input, mode } => {
                self.state.popup.selector_popup = None;
                self.state.popup.selected_index = 0;
                self.state.ui.focus = FocusArea::Preamp;
                self.state.preamp.selected_input = input.min(1) as usize;
                self.send(Command::SetPreampMode { input, mode }, pending.clone())?;
            }
            Intent::TogglePreampPhase(input) => {
                self.state.ui.focus = FocusArea::Preamp;
                self.state.preamp.selected_input = input.min(1) as usize;
                let mode_raw = if input == 0 {
                    self.state.preamp.state.input1.mode_raw
                } else {
                    self.state.preamp.state.input2.mode_raw
                };
                self.send(
                    Command::SetPreampPhase {
                        input,
                        enabled: mode_raw & 0x40 == 0,
                    },
                    pending.clone(),
                )?;
            }
            Intent::TogglePreampPhantom(input) => {
                self.state.ui.focus = FocusArea::Preamp;
                self.state.preamp.selected_input = input.min(1) as usize;
                let current = if input == 0 {
                    self.state.preamp.state.input1
                } else {
                    self.state.preamp.state.input2
                };
                self.send(
                    Command::SetPreampPhantom {
                        input,
                        enabled: !current.phantom_on,
                    },
                    pending.clone(),
                )?;
            }
            Intent::AdjustFocused(increase) => match self.state.ui.focus {
                FocusArea::Outputs => {
                    let index = self.state.output.selected;
                    let output = self.state.output.states[index];
                    let next = if increase {
                        output.volume.saturating_sub(1)
                    } else {
                        output.volume.saturating_add(1).min(0x60)
                    };
                    self.send(
                        Command::SetOutputVolume {
                            target: output.target,
                            step: next,
                        },
                        pending.clone(),
                    )?;
                }
                FocusArea::Mixer => {
                    let active_channel =
                        self.state.active_mixer_channels()[self.state.mixer.selected_channel];
                    let channel = active_channel.channel;
                    let current = active_channel.level.unwrap_or(0x20);
                    let next = if increase {
                        current.saturating_sub(1)
                    } else {
                        current.saturating_add(1).min(0x60)
                    };
                    self.send_mixer_level_change(
                        MixerSurface::from_surface(self.state.mixer.surface),
                        channel,
                        next,
                    )?;
                }
                FocusArea::Preamp => {
                    let input = self.state.preamp.selected_input as u8;
                    let preamp_input = if input == 0 {
                        &self.state.preamp.state.input1
                    } else {
                        &self.state.preamp.state.input2
                    };
                    let next = match preamp_input.mode {
                        PreampMode::Mic => {
                            if increase {
                                preamp_input.gain_raw.saturating_add(1).min(0x41)
                            } else {
                                preamp_input.gain_raw.saturating_sub(1)
                            }
                        }
                        PreampMode::Line => {
                            let current = i8::from_ne_bytes([preamp_input.gain_raw]);
                            let next = if increase {
                                (current + 1).min(20)
                            } else {
                                (current - 1).max(-6)
                            };
                            next as u8
                        }
                        PreampMode::HiZ => {
                            if increase {
                                preamp_input.gain_raw.saturating_add(1).min(0x2d)
                            } else {
                                preamp_input.gain_raw.saturating_sub(1)
                            }
                        }
                        PreampMode::Unknown(_) => preamp_input.gain_raw,
                    };
                    self.send(Command::SetPreampGain { input, raw: next }, pending.clone())?;
                }
                _ => {}
            },
            Intent::ToggleFocusedMute => match self.state.ui.focus {
                FocusArea::Outputs => {
                    let index = self.state.output.selected;
                    let output = self.state.output.states[index];
                    self.send(
                        Command::SetOutputMute {
                            target: output.target,
                            enabled: output.mode != OutputMode::Mute,
                        },
                        pending.clone(),
                    )?;
                }
                FocusArea::Mixer => {
                    let active_channel =
                        self.state.active_mixer_channels()[self.state.mixer.selected_channel];
                    let channel = active_channel.channel;
                    let muted = !active_channel.muted.unwrap_or(false);
                    self.send_mixer_mute_change(
                        MixerSurface::from_surface(self.state.mixer.surface),
                        channel,
                        muted,
                    )?;
                }
                FocusArea::Preamp => {
                    let input = self.state.preamp.selected_input as u8;
                    let current = if input == 0 {
                        self.state.preamp.state.input1
                    } else {
                        self.state.preamp.state.input2
                    };
                    self.send(
                        Command::SetPreampPhantom {
                            input,
                            enabled: !current.phantom_on,
                        },
                        pending.clone(),
                    )?;
                }
                _ => {}
            },
            Intent::ToggleFocusedDim => {
                if self.state.ui.focus == FocusArea::Outputs {
                    let index = self.state.output.selected;
                    let output = self.state.output.states[index];
                    self.send(
                        Command::SetOutputDim {
                            target: output.target,
                            enabled: output.mode != OutputMode::Dim,
                        },
                        pending.clone(),
                    )?;
                }
            }
            Intent::ToggleRoutingPopup => {
                self.state.popup.routing_open = !self.state.popup.routing_open;
                self.state.ui.last_message = if self.state.popup.routing_open {
                    "Routing popup mirrors mixer assignments for USB recording channels 1-8"
                        .to_string()
                } else {
                    "Closed routing popup".to_string()
                };
            }
            Intent::RefreshQueriedState => {
                self.refresh_queried_state()?;
                self.state.ui.last_message =
                    "Sent captured 0x74 startup/state refresh sweep".to_string();
            }
            Intent::CycleFocus => {
                if self.state.ui.page == MainPage::Mixer {
                    self.state.cycle_focus();
                }
            }
            Intent::MovePopupSelection(down) => {
                let item_count = if self.state.popup.assignment_picker.is_some() {
                    antelope_protocol::MixerAssignment::grounded_choices().len()
                } else if self.state.popup.profiles_open {
                    self.state.popup.profile_names.len()
                } else if let Some(popup) = self.state.popup.selector_popup {
                    match popup.kind {
                        SelectorPopupKind::SampleRate => SampleRate::all_confirmed().len(),
                        SelectorPopupKind::ClockSource => ClockSource::all_confirmed().len(),
                        SelectorPopupKind::PreampMode { .. } => 3,
                    }
                } else {
                    0
                };
                if item_count == 0 {
                    return Ok(());
                }
                self.state.popup.selected_index = if down {
                    (self.state.popup.selected_index + 1) % item_count
                } else {
                    self.state
                        .popup
                        .selected_index
                        .checked_sub(1)
                        .unwrap_or(item_count - 1)
                };
            }
            Intent::ProfileEditorChar(ch) => {
                if let Some(editor) = self.state.popup.profile_editor.as_mut() {
                    editor.value.push_str(&ch);
                }
            }
            Intent::ProfileEditorBackspace => {
                if let Some(editor) = self.state.popup.profile_editor.as_mut() {
                    editor.value.pop();
                }
            }
            Intent::ProfileEditorCommit => {
                if let Some(editor) = self.state.popup.profile_editor.take() {
                    let name = editor.value.trim().to_string();
                    if name.is_empty() {
                        self.state.ui.last_message = "Profile name cannot be empty".to_string();
                        self.state.popup.profile_editor = Some(editor);
                    } else {
                        match editor.mode {
                            ProfileEditorMode::Save => {
                                let profile = DeviceProfile::capture(&self.state);
                                match profile {
                                    Ok(profile) => match profile.write_named(&name) {
                                        Ok(path) => {
                                            self.state.popup.profiles_open = false;
                                            self.state.ui.last_message =
                                                format!("Saved profile to {}", path.display());
                                        }
                                        Err(e) => {
                                            self.state.ui.last_message =
                                                format!("Profile error: {e}");
                                        }
                                    },
                                    Err(e) => {
                                        self.state.ui.last_message = format!("Profile error: {e}");
                                    }
                                }
                            }
                            ProfileEditorMode::Rename => {
                                if let Some(original) = &editor.original_name {
                                    if original != &name {
                                        match crate::profile::rename_profile(original, &name) {
                                            Ok(_path) => {
                                                self.state.popup.profile_names =
                                                    crate::profile::list_profile_names()
                                                        .unwrap_or_default();
                                                self.state.clamp_profile_selection();
                                                self.state.ui.last_message =
                                                    format!("Renamed {original} to {name}");
                                            }
                                            Err(e) => {
                                                self.state.ui.last_message =
                                                    format!("Profile error: {e}");
                                            }
                                        }
                                    } else {
                                        self.state.ui.last_message =
                                            "Profile name unchanged".to_string();
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Intent::ProfileEditorCancel => {
                self.state.popup.profile_editor = None;
                self.state.ui.last_message = "Cancelled profile edit".to_string();
            }
            Intent::CaptureRawBaseline => {
                self.state.capture_raw_baseline();
                self.state.ui.last_message =
                    "Captured raw baseline for 0x73/0x83/0x75/0x81".to_string();
            }
            Intent::ClearRawBaseline => {
                self.state.clear_raw_baseline();
                self.state.ui.last_message = "Cleared raw baseline".to_string();
            }
            Intent::ToggleOptionsPopup => {
                self.state.toggle_options_popup();
                self.state.ui.last_message = if self.state.popup.options_open {
                    "Options popup opened".to_string()
                } else {
                    "Closed options popup".to_string()
                };
            }
        }
        Ok(())
    }

    pub fn poll_device(&mut self, timeout: Duration) -> Result<bool> {
        // Flush pending commands before reading so device sees latest state
        self.flush_commands()?;

        let mut next_timeout = timeout;
        let mut state_dirty = false;

        for _ in 0..MAX_FRAMES_PER_POLL {
            let Some(bytes) = self.transport.read(next_timeout)? else {
                break;
            };

            next_timeout = Duration::ZERO;

            if let Ok(frame) = Frame::parse_owned(bytes) {
                let (snapshot, raw) = frame.into_snapshot_and_raw();
                if matches!(&snapshot, DeviceSnapshot::Snapshot(_)) {
                    state_dirty |= self.confirm_pending_write();
                }
                state_dirty |= self.state.observe_frame(snapshot, raw);
            }
        }

        Ok(state_dirty)
    }

    pub fn confirm_pending_write(&mut self) -> bool {
        let Some(pending) = self.pending_mutation.take() else {
            return false;
        };
        match pending {
            PendingMutation::MixerLevel {
                mixer,
                channel,
                level,
                pan,
                muted,
            } => {
                if let Some(slot) = self.state.mixer.channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.level = Some(level);
                    slot.muted = Some(muted);
                    slot.pan = pan;
                }
                true
            }
            PendingMutation::MixerLinkedLevel {
                mixer,
                left_channel,
                right_channel,
                level,
                left_pan,
                right_pan,
                left_muted,
                right_muted,
            } => {
                for (channel, pan, muted) in [
                    (left_channel, left_pan, left_muted),
                    (right_channel, right_pan, right_muted),
                ] {
                    if let Some(slot) = self.state.mixer.channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.level = Some(level);
                        slot.muted = Some(muted);
                        slot.pan = pan;
                    }
                }
                true
            }
            PendingMutation::MixerMute {
                mixer,
                channel,
                muted,
            } => {
                if let Some(slot) = self.state.mixer.channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.muted = Some(muted);
                }
                true
            }
            PendingMutation::MixerSolo {
                mixer,
                channel,
                soloed,
            } => {
                if let Some(slot) = self.state.mixer.channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.soloed = Some(soloed);
                }
                true
            }
            PendingMutation::MixerLinkedMute {
                mixer,
                left_channel,
                right_channel,
                muted,
            } => {
                for channel in [left_channel, right_channel] {
                    if let Some(slot) = self.state.mixer.channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.muted = Some(muted);
                    }
                }
                true
            }
            PendingMutation::MixerLinkedSolo {
                mixer,
                left_channel,
                right_channel,
                soloed,
            } => {
                for channel in [left_channel, right_channel] {
                    if let Some(slot) = self.state.mixer.channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.soloed = Some(soloed);
                    }
                }
                true
            }
            PendingMutation::MixerPan {
                mixer,
                channel,
                pan,
            } => {
                if let Some(slot) = self.state.mixer.channels[mixer.index()]
                    .get_mut(channel.saturating_sub(1) as usize)
                {
                    slot.pan = pan;
                }
                true
            }
            PendingMutation::MixerAssignment { strip, assignment } => {
                let index = strip.saturating_sub(1) as usize;
                for channels in &mut self.state.mixer.channels {
                    if let Some(slot) = channels.get_mut(index) {
                        slot.assignment = Some(assignment);
                    }
                }
                true
            }
            PendingMutation::MixerLink {
                mixer,
                selector,
                enabled,
            } => {
                if let Some((left, right)) = link_pair_from_selector(mixer, selector) {
                    for channel in [left, right] {
                        if let Some(slot) = self.state.mixer.channels[mixer.index()]
                            .get_mut(channel.saturating_sub(1) as usize)
                        {
                            slot.linked = Some(enabled);
                        }
                    }
                }
                true
            }
            PendingMutation::MixerLinkExplicit {
                mixer,
                left_channel,
                right_channel,
                enabled,
            } => {
                for channel in [left_channel, right_channel] {
                    if let Some(slot) = self.state.mixer.channels[mixer.index()]
                        .get_mut(channel.saturating_sub(1) as usize)
                    {
                        slot.linked = Some(enabled);
                    }
                }
                true
            }
            PendingMutation::OutputVolume { target, step } => {
                self.state.output.states[target.index() as usize].volume = step;
                true
            }
            PendingMutation::OutputMode { target, mode } => {
                self.state.output.states[target.index() as usize].mode = mode;
                true
            }
            PendingMutation::PreampGain { input, raw } => {
                self.state.device.dsp_cluster[input.min(1) as usize] = raw;
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                true
            }
            PendingMutation::PreampMode { input, mode } => {
                let offset = 2 + input.min(1) as usize;
                let preserved_bits = self.state.device.dsp_cluster[offset] & 0xf0;
                self.state.device.dsp_cluster[offset] = preserved_bits | mode.code();
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                true
            }
            PendingMutation::PreampPhantom { input, enabled } => {
                let offset = 2 + input.min(1) as usize;
                let low = self.state.device.dsp_cluster[offset] & 0x0f;
                self.state.device.dsp_cluster[offset] = low | if enabled { 0x10 } else { 0x00 };
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                true
            }
            PendingMutation::PreampPhase { input, enabled } => {
                let offset = 2 + input.min(1) as usize;
                let low = self.state.device.dsp_cluster[offset] & 0x1f;
                self.state.device.dsp_cluster[offset] = low | if enabled { 0x40 } else { 0x00 };
                self.state
                    .refresh_preamp_from_cluster_preserving_observed_meter();
                true
            }
        }
    }
}

fn link_pair_from_selector(mixer: MixerSurface, selector: u8) -> Option<(u8, u8)> {
    MixerLinkTarget::from_selector(mixer, selector)
        .map(|target| (target.left_channel, target.right_channel))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crate::profile::{
        DeviceProfile, MixerAssignmentEntry, MixerAssignmentProfile, MixerProfiles,
        MixerStripProfile, OutputModeProfile, OutputProfile, OutputProfiles, PreampInputProfile,
        PreampModeProfile, PreampProfiles,
    };
    use crate::transport::MockTransport;
    use antelope_protocol::{
        ClockSource, Command, DeviceSnapshot, DeviceStateSnapshot, Frame, MixerAssignment,
        MixerChannelState, MixerStrip, MixerSurface, OutputMode, OutputState, OutputTarget,
        PanState, PreampMode, PreampState, SampleRate, Surface,
    };

    use super::*;

    fn snapshot() -> DeviceStateSnapshot {
        DeviceStateSnapshot {
            sample_rate: SampleRate::Hz48000,
            clock_source: ClockSource::Internal,
            sample_rate_hz: 48_000,
            status_flags: [0x08, 0x00],
            front_panel_bytes: [0, 0, 0],
            outputs: [
                OutputState::new(OutputTarget::Monitor, 0x50, OutputMode::Normal),
                OutputState::new(OutputTarget::Hp1, 0x40, OutputMode::Mute),
                OutputState::new(OutputTarget::Hp2, 0x30, OutputMode::Dim),
            ],
            dsp_cluster: [0x2f, 0x34, 0x50, 0x10],
            preamp: PreampState::from_cluster([0x2f, 0x34, 0x50, 0x10]),
            surface: Surface::MonitorHp1,
            mixer_decode: Default::default(),
            late_shadow: [0; 12],
        }
    }

    fn seed_shared_assignments(state: &mut AppState) {
        let assignments = [
            MixerAssignment::Preamp(1),
            MixerAssignment::Preamp(2),
            MixerAssignment::ComputerPlay(1),
            MixerAssignment::ComputerPlay(2),
            MixerAssignment::ComputerPlay(3),
            MixerAssignment::ComputerPlay(4),
            MixerAssignment::ComputerPlay(5),
            MixerAssignment::ComputerPlay(6),
            MixerAssignment::ComputerPlay(7),
            MixerAssignment::ComputerPlay(8),
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
            MixerAssignment::Mute,
        ];

        for surface in &mut state.mixer.channels {
            for (channel, assignment) in surface.iter_mut().zip(assignments) {
                channel.assignment = Some(assignment);
            }
        }
    }

    fn assignment_pairs(frame: &[u8], count: usize) -> Vec<[u8; 2]> {
        let payload = &frame[0x10 + 0x03..];
        payload
            .chunks_exact(2)
            .take(count)
            .map(|chunk| [chunk[0], chunk[1]])
            .collect()
    }

    fn snapshot_frame_bytes(meter: u8) -> Vec<u8> {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x00] = 0x08;
        payload[0x02] = 0x02;
        payload[0x03] = 0x00;
        payload[0x04..0x08].copy_from_slice(&48_000_u32.to_be_bytes());
        payload[0x0c] = 0x50;
        payload[0x0d] = 0x00;
        payload[0x0e] = 0x40;
        payload[0x0f] = 0x01;
        payload[0x10] = 0x30;
        payload[0x11] = 0x02;
        payload[0x18..0x1c].copy_from_slice(&[0x2f, 0x34, 0x50, 0x10]);
        payload[0x6a] = 0x0f;
        payload[0xcf] = meter;
        frame
    }

    #[test]
    fn intent_enum_exists_and_can_be_created() {
        // Test that Intent enum exists and can be constructed
        let intent = Intent::Quit;
        assert!(matches!(intent, Intent::Quit));
    }

    #[test]
    fn intent_enum_covers_output_actions() {
        // Test output-related intents
        let adjust = Intent::AdjustOutputLevel {
            index: 0,
            increase: true,
        };
        assert!(matches!(adjust, Intent::AdjustOutputLevel { .. }));

        let set = Intent::SetOutputLevel {
            index: 0,
            step: 0x30,
        };
        assert!(matches!(set, Intent::SetOutputLevel { .. }));

        let mute = Intent::ToggleOutputMute(0);
        assert!(matches!(mute, Intent::ToggleOutputMute(0)));

        let dim = Intent::ToggleOutputDim(0);
        assert!(matches!(dim, Intent::ToggleOutputDim(0)));
    }

    #[test]
    fn intent_enum_covers_mixer_actions() {
        // Test mixer-related intents
        let adjust = Intent::AdjustMixerLevel {
            index: 0,
            increase: true,
        };
        assert!(matches!(adjust, Intent::AdjustMixerLevel { .. }));

        let set = Intent::SetMixerLevel {
            index: 0,
            level: 0x50,
        };
        assert!(matches!(set, Intent::SetMixerLevel { .. }));

        let pan = Intent::AdjustMixerPan {
            index: 0,
            right: true,
        };
        assert!(matches!(pan, Intent::AdjustMixerPan { .. }));

        let set_pan = Intent::SetMixerPan {
            index: 0,
            pan: PanState::center(),
        };
        assert!(matches!(set_pan, Intent::SetMixerPan { .. }));

        let mute = Intent::ToggleMixerMute(1);
        assert!(matches!(mute, Intent::ToggleMixerMute(1)));

        let solo = Intent::ToggleMixerSolo(1);
        assert!(matches!(solo, Intent::ToggleMixerSolo(1)));
    }

    #[test]
    fn intent_enum_covers_preamp_actions() {
        // Test preamp-related intents
        let adjust = Intent::AdjustPreampGain {
            input: 0,
            increase: true,
        };
        assert!(matches!(adjust, Intent::AdjustPreampGain { .. }));

        let set = Intent::SetPreampGain {
            input: 0,
            raw: 0x30,
        };
        assert!(matches!(set, Intent::SetPreampGain { .. }));

        let mode = Intent::PickPreampMode {
            input: 0,
            mode: PreampMode::Mic,
        };
        assert!(matches!(mode, Intent::PickPreampMode { .. }));

        let phase = Intent::TogglePreampPhase(0);
        assert!(matches!(phase, Intent::TogglePreampPhase(0)));

        let phantom = Intent::TogglePreampPhantom(0);
        assert!(matches!(phantom, Intent::TogglePreampPhantom(0)));
    }

    #[test]
    fn intent_enum_covers_navigation_actions() {
        // Test navigation intents
        let quit = Intent::Quit;
        assert!(matches!(quit, Intent::Quit));

        let raw = Intent::ToggleRawView;
        assert!(matches!(raw, Intent::ToggleRawView));

        let page = Intent::SelectPage(MainPage::Mixer);
        assert!(matches!(page, Intent::SelectPage(_)));

        let surface = Intent::SelectSurface(Surface::MonitorHp1);
        assert!(matches!(surface, Intent::SelectSurface(_)));
    }

    #[test]
    fn intent_enum_covers_selector_actions() {
        // Test selector popup intents
        let sample = Intent::PickSampleRate(SampleRate::Hz48000);
        assert!(matches!(sample, Intent::PickSampleRate(_)));

        let clock = Intent::PickClockSource(ClockSource::Internal);
        assert!(matches!(clock, Intent::PickClockSource(_)));
    }

    #[test]
    fn reducer_prefers_device_snapshot_state() {
        let mut state = AppState::default();
        state.output.states[0].volume = 0x10;

        state.apply_snapshot(&snapshot());

        assert_eq!(state.device.status.sample_rate, Some(SampleRate::Hz48000));
        assert_eq!(state.output.states[0].volume, 0x50);
        assert_eq!(state.output.states[1].mode, OutputMode::Mute);
        assert_eq!(state.mixer.surface, Surface::MonitorHp1);
    }

    #[test]
    fn reducer_updates_preamp_state_from_snapshot() {
        let mut state = AppState::default();
        let mut device_snapshot = snapshot();
        device_snapshot.dsp_cluster = [0x14, 0x2a, 0x11, 0x00];

        state.apply_snapshot(&device_snapshot);

        assert_eq!(state.preamp.state.input1.mode, PreampMode::Line);
        assert_eq!(state.preamp.state.input1.gain_raw, 0x14);
        assert_eq!(state.preamp.state.input2.mode, PreampMode::Mic);
        assert_eq!(state.preamp.state.input2.gain_raw, 0x2a);
        assert!(!state.preamp.state.input2.phantom_on);
    }

    #[test]
    fn reducer_applies_grounded_passive_mixer_decode_from_snapshot() {
        let mut state = AppState::default();
        let mut device_snapshot = snapshot();
        device_snapshot.mixer_decode.observed_preamp1_meter = Some(0x28);
        device_snapshot.mixer_decode.observed_preamp2_meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix2.index()][0].meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].muted = Some(false);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].linked = Some(true);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][1].linked = Some(true);

        state.apply_snapshot(&device_snapshot);

        assert_eq!(state.preamp.state.input1.observed_meter, Some(0x28));
        assert_eq!(state.preamp.state.input2.observed_meter, Some(0x30));
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].meter,
            Some(0x30)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix2.index()][0].meter,
            Some(0x30)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].level,
            None
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].muted,
            Some(false)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].pan,
            PanState::center()
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][1].linked,
            Some(true)
        );
    }

    #[test]
    fn passive_snapshot_pan_decode_does_not_override_channel_pan() {
        let mut state = AppState::default();
        state.mixer.channels[MixerSurface::Mix1.index()][0].pan = PanState::center();

        let mut device_snapshot = snapshot();
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].pan =
            Some(PanState::from_raw(0x1e));

        state.apply_snapshot(&device_snapshot);

        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].pan,
            PanState::center()
        );
    }

    #[test]
    fn query_reply_assignment_readback_updates_shared_strip_assignments() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x03,
                sub_id: 0x05,
                body: vec![0x05, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x01, 0x01],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x03,
                sub_id: 0x06,
                body: vec![
                    0x06, 0x03, 0x00, 0x03, 0x01, 0x03, 0x02, 0x03, 0x03, 0x01, 0x02, 0x01, 0x03,
                    0x01, 0x04, 0x01, 0x05, 0x01, 0x06, 0x01, 0x07, 0x08, 0x00, 0x08, 0x00, 0x08,
                    0x00, 0x08, 0x00, 0x08, 0x00, 0x08, 0x00,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            let channels = &state.mixer.channels[mixer.index()];
            assert_eq!(channels[0].assignment, Some(MixerAssignment::Preamp(1)));
            assert_eq!(channels[1].assignment, Some(MixerAssignment::Preamp(2)));
            assert_eq!(
                channels[2].assignment,
                Some(MixerAssignment::ComputerPlay(1))
            );
            assert_eq!(
                channels[3].assignment,
                Some(MixerAssignment::ComputerPlay(2))
            );
            assert_eq!(
                channels[4].assignment,
                Some(MixerAssignment::ComputerPlay(3))
            );
            assert_eq!(
                channels[9].assignment,
                Some(MixerAssignment::ComputerPlay(8))
            );
            assert!(channels[10..]
                .iter()
                .all(|slot| slot.assignment == Some(MixerAssignment::Mute)));
        }
    }

    #[test]
    fn query_reply_startup_link_readback_updates_visible_pairs_from_bitmap() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x0b,
                sub_id: 0x03,
                body: vec![
                    0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            let channels = &state.mixer.channels[mixer.index()];
            let expected_primary = if mixer == MixerSurface::Mix1 {
                Some(true)
            } else {
                Some(false)
            };
            assert_eq!(channels[0].linked, expected_primary);
            assert_eq!(channels[1].linked, expected_primary);
            for index in (2..10).step_by(2) {
                assert_eq!(channels[index].linked, Some(true));
                assert_eq!(channels[index + 1].linked, Some(true));
            }
            assert!(channels[10..].iter().all(|slot| slot.linked == Some(false)));
        }

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x0b,
                sub_id: 0x03,
                body: vec![
                    0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        let mix1 = &state.mixer.channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer.channels[MixerSurface::Mix2.index()];
        assert!(mix1[10..].iter().all(|slot| slot.linked == Some(true)));
        assert!(mix2[10..].iter().all(|slot| slot.linked == Some(true)));
    }

    #[test]
    fn query_reply_startup_pan_state_readback_updates_mix_pan_and_mute() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x04,
                sub_id: 0x00,
                body: vec![
                    0x00, 0x20, 0x00, 0x5e, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x04,
                sub_id: 0x01,
                body: vec![
                    0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x02, 0x00, 0x3e,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        let mix1 = &state.mixer.channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer.channels[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, Some(0x00));
        assert_eq!(mix1[0].pan, PanState::from_raw(0x1e));
        assert_eq!(mix1[0].muted, Some(true));
        assert_eq!(mix1[1].level, Some(0x00));
        assert_eq!(mix1[1].pan, PanState::center());
        assert_eq!(mix1[1].muted, Some(true));
        assert_eq!(mix1[2].level, Some(0x00));
        assert_eq!(mix1[2].pan, PanState::left());
        assert_eq!(mix1[2].muted, Some(false));
        assert_eq!(mix1[3].level, Some(0x00));
        assert_eq!(mix1[3].pan, PanState::right());
        assert_eq!(mix1[3].muted, Some(false));
        assert_eq!(mix2[10].level, Some(0x00));
        assert_eq!(mix2[10].pan, PanState::left());
        assert_eq!(mix2[10].muted, Some(false));
        assert_eq!(mix2[11].level, Some(0x00));
        assert_eq!(mix2[11].pan, PanState::right());
        assert_eq!(mix2[11].muted, Some(false));
    }

    #[test]
    fn query_reply_startup_level_readback_updates_mix_levels() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x04,
                sub_id: 0x00,
                body: vec![
                    0x00, 0x20, 0x12, 0x5e, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x04,
                sub_id: 0x01,
                body: vec![
                    0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00,
                    0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x1e, 0x02, 0x1e, 0x3e,
                    0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        let mix1 = &state.mixer.channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer.channels[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, Some(0x12));
        assert_eq!(mix2[10].level, Some(0x1e));
        assert_eq!(mix2[11].level, Some(0x1e));
    }

    #[test]
    fn query_reply_strip_readback_does_not_seed_unstable_startup_state() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x18,
                sub_id: 0x00,
                body: vec![
                    0x00, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x02, 0x60, 0x3e, 0x2e, 0x02, 0x60,
                    0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e,
                    0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60,
                    0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                    0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        let mix1 = &state.mixer.channels[MixerSurface::Mix1.index()];
        let mix2 = &state.mixer.channels[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, None);
        assert_eq!(mix1[0].pan, PanState::center());
        assert_eq!(mix1[0].muted, None);
        assert!(mix1.iter().all(|slot| slot.level.is_none()));
        assert!(mix1.iter().all(|slot| slot.muted.is_none()));
        assert!(mix1.iter().all(|slot| slot.pan == PanState::center()));
        assert!(mix2.iter().all(|slot| slot.level.is_none()));
        assert!(mix2.iter().all(|slot| slot.muted.is_none()));
        assert!(mix2.iter().all(|slot| slot.pan == PanState::center()));
    }

    #[test]
    fn query_reply_strip_readback_does_not_apply_pan_or_mute_overlay() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x18,
                sub_id: 0x00,
                body: vec![
                    0x12, 0x3e, 0x60, 0x60, 0x60, 0x60, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x20, 0x60,
                    0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                    0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60,
                    0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                    0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        let mix1 = &state.mixer.channels[MixerSurface::Mix1.index()];
        assert_eq!(mix1[0].muted, None);
        assert_eq!(mix1[0].pan, PanState::center());
        assert!(mix1.iter().all(|slot| slot.muted.is_none()));
        assert!(mix1.iter().all(|slot| slot.pan == PanState::center()));
    }

    #[test]
    fn passive_meter_does_not_override_known_level_value() {
        let mut state = AppState::default();
        state.mixer.channels[MixerSurface::Mix1.index()][0].level = Some(0x00);

        let mut device_snapshot = snapshot();
        device_snapshot.mixer_decode.observed_preamp2_meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix1.index()][0].meter = Some(0x30);
        device_snapshot.mixer_decode.surfaces[MixerSurface::Mix2.index()][0].meter = Some(0x30);

        state.apply_snapshot(&device_snapshot);

        assert_eq!(state.preamp.state.input2.observed_meter, Some(0x30));
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].level,
            Some(0x00)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][0].meter,
            Some(0x30)
        );
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix2.index()][0].meter,
            Some(0x30)
        );
    }

    #[test]
    fn preamp_pending_gain_updates_authoritative_cluster() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.device.dsp_cluster = [0x0a, 0x0a, 0x00, 0x00];
        controller.state.preamp.state =
            PreampState::from_cluster(controller.state.device.dsp_cluster);

        controller
            .send(
                Command::SetPreampGain {
                    input: 1,
                    raw: 0x2d,
                },
                Some(PendingMutation::PreampGain {
                    input: 1,
                    raw: 0x2d,
                }),
            )
            .expect("send preamp gain");
        controller.confirm_pending_write();

        assert_eq!(controller.state.preamp.state.input2.gain_raw, 0x2d);
        assert_eq!(controller.state.device.dsp_cluster[1], 0x2d);
    }

    #[test]
    fn preamp_pending_updates_preserve_observed_input_meters() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.device.dsp_cluster = [0x0a, 0x0a, 0x00, 0x00];
        controller.state.preamp.state =
            PreampState::from_cluster(controller.state.device.dsp_cluster);
        controller.state.preamp.state.input2.observed_meter = Some(0x30);

        controller
            .send(
                Command::SetPreampGain {
                    input: 1,
                    raw: 0x2d,
                },
                Some(PendingMutation::PreampGain {
                    input: 1,
                    raw: 0x2d,
                }),
            )
            .expect("send preamp gain");
        controller.confirm_pending_write();

        assert_eq!(controller.state.preamp.state.input2.gain_raw, 0x2d);
        assert_eq!(controller.state.preamp.state.input1.observed_meter, None);
        assert_eq!(
            controller.state.preamp.state.input2.observed_meter,
            Some(0x30)
        );
    }

    #[test]
    fn preamp_pending_mode_phantom_and_phase_update_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.device.dsp_cluster = [0x0a, 0x0a, 0x00, 0x00];
        controller.state.preamp.state =
            PreampState::from_cluster(controller.state.device.dsp_cluster);

        controller
            .send(
                Command::SetPreampMode {
                    input: 0,
                    mode: PreampMode::Line,
                },
                Some(PendingMutation::PreampMode {
                    input: 0,
                    mode: PreampMode::Line,
                }),
            )
            .expect("send preamp mode");
        controller.confirm_pending_write();
        assert_eq!(controller.state.preamp.state.input1.mode, PreampMode::Line);

        controller.state.device.dsp_cluster[3] = 0x00;
        controller.state.preamp.state =
            PreampState::from_cluster(controller.state.device.dsp_cluster);
        controller
            .send(
                Command::SetPreampPhantom {
                    input: 1,
                    enabled: true,
                },
                Some(PendingMutation::PreampPhantom {
                    input: 1,
                    enabled: true,
                }),
            )
            .expect("send preamp phantom");
        controller.confirm_pending_write();
        assert!(controller.state.preamp.state.input2.phantom_on);

        controller.state.device.dsp_cluster[3] = 0x00;
        controller.state.preamp.state =
            PreampState::from_cluster(controller.state.device.dsp_cluster);
        controller
            .send(
                Command::SetPreampPhase {
                    input: 1,
                    enabled: true,
                },
                Some(PendingMutation::PreampPhase {
                    input: 1,
                    enabled: true,
                }),
            )
            .expect("send preamp phase");
        controller.confirm_pending_write();
        assert_eq!(controller.state.device.dsp_cluster[3], 0x40);
    }

    #[test]
    fn apply_profile_updates_known_controls_and_writes_commands() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        let profile = DeviceProfile {
            outputs: OutputProfiles {
                monitor: OutputProfile {
                    volume_step: 0x12,
                    mode: OutputModeProfile::Dim,
                },
                hp1: OutputProfile {
                    volume_step: 0x24,
                    mode: OutputModeProfile::Mute,
                },
                hp2: OutputProfile {
                    volume_step: 0x08,
                    mode: OutputModeProfile::Normal,
                },
            },
            preamps: PreampProfiles {
                input1: PreampInputProfile {
                    gain_raw: 0x20,
                    mode: PreampModeProfile::Mic,
                    phantom_on: true,
                    phase_inverted: true,
                },
                input2: PreampInputProfile {
                    gain_raw: 0x10,
                    mode: PreampModeProfile::Line,
                    phantom_on: false,
                    phase_inverted: false,
                },
            },
            assignments: (1..=16)
                .map(|channel| MixerAssignmentEntry {
                    channel,
                    source: if channel == 1 {
                        MixerAssignmentProfile::Preamp(1)
                    } else {
                        MixerAssignmentProfile::Mute
                    },
                })
                .collect(),
            mixers: MixerProfiles {
                mix1: (1..=16)
                    .map(|channel| MixerStripProfile {
                        channel,
                        level_raw: channel - 1,
                        pan_raw: if channel == 1 {
                            PanState::right().raw()
                        } else {
                            PanState::center().raw()
                        },
                        muted: channel % 2 == 0,
                        soloed: channel == 2,
                        linked: channel <= 2,
                    })
                    .collect(),
                mix2: (1..=16)
                    .map(|channel| MixerStripProfile {
                        channel,
                        level_raw: 0x30,
                        pan_raw: PanState::left().raw(),
                        muted: false,
                        soloed: false,
                        linked: false,
                    })
                    .collect(),
            },
        };

        controller.apply_profile(&profile).expect("apply profile");

        assert_eq!(controller.state.output.states[0].volume, 0x12);
        assert_eq!(controller.state.output.states[0].mode, OutputMode::Dim);
        assert_eq!(controller.state.output.states[1].mode, OutputMode::Mute);
        assert_eq!(controller.state.preamp.state.input1.mode, PreampMode::Mic);
        assert!(controller.state.preamp.state.input1.phantom_on);
        assert_eq!(controller.state.preamp.state.input1.mode_raw & 0x40, 0x40);
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].assignment,
            Some(MixerAssignment::Preamp(1))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].pan,
            PanState::right()
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][1].soloed,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][0].pan,
            PanState::left()
        );
        assert!(!transport.take_writes().is_empty());
    }

    #[test]
    fn bootstrap_sends_queries_and_mutations_use_transport() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller.bootstrap().expect("bootstrap");
        controller
            .send(Command::SetClockSource(ClockSource::Usb), None)
            .expect("write command");
        controller.flush_commands().expect("flush");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 48);
        assert_eq!(&writes[0][0x08..0x10], &[0x01, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[1][0x08..0x10], &[0x11, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[2][0x08..0x10], &[0x0a, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[46][0x08..0x10], &[0x12, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[47][0x10..0x12], &[0x04, 0x02]);
    }

    #[test]
    fn clock_source_command_updates_visible_state_immediately() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        controller.state.device.status.clock_source = Some(ClockSource::Usb);

        controller
            .send(Command::SetClockSource(ClockSource::Internal), None)
            .expect("set clock source");

        assert_eq!(
            controller.state.device.status.clock_source,
            Some(ClockSource::Internal)
        );
    }

    #[test]
    fn bootstrap_queries_include_metadata_request() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller.bootstrap().expect("bootstrap");

        let writes = transport.take_writes();
        assert!(writes
            .iter()
            .any(|frame| &frame[0x08..0x10] == [0x01, 0, 0, 0, 0, 0, 0, 0]));
    }

    #[test]
    fn surface_select_refreshes_query_readback() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .send(Command::SelectSurface(Surface::Hp2), None)
            .expect("select surface");
        controller.flush_commands().expect("flush");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 48);
        assert_eq!(&writes[0][0x10..0x13], &[0x49, 0x00, Surface::Hp2.code()]);
        assert_eq!(&writes[1][0x08..0x10], &[0x01, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&writes[47][0x08..0x10], &[0x12, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn clock_source_change_does_not_force_refresh_query_readback() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .send(Command::SetClockSource(ClockSource::Usb), None)
            .expect("set clock source");
        controller.flush_commands().expect("flush");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x12], &[0x04, 0x02]);
    }

    #[test]
    fn sample_rate_change_does_not_force_refresh_query_readback() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .send(Command::SetSampleRate(SampleRate::Hz96000), None)
            .expect("set sample rate");
        controller.flush_commands().expect("flush");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x12], &[0x03, 0x04]);
    }

    #[test]
    fn mixer_overlay_is_tracked_only_after_command_round_trip() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .send(
                Command::SetMixerLevel {
                    mixer: antelope_protocol::MixerSurface::Mix1,
                    channel: 3,
                    level: 0x2c,
                    pan_state: antelope_protocol::PanState::left(),
                    muted: false,
                    soloed: false,
                },
                Some(PendingMutation::MixerLevel {
                    mixer: MixerSurface::Mix1,
                    channel: 3,
                    level: 0x2c,
                    pan: antelope_protocol::PanState::left(),
                    muted: false,
                }),
            )
            .expect("send mixer");

        assert!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2]
                .level
                .is_none()
        );

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2],
            MixerChannelState::known(3, Some(0x2c), Some(false), PanState::left(), None, None)
        );
    }

    #[test]
    fn linked_mixer_level_change_writes_and_updates_both_channels() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].pan = PanState::left();
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].pan = PanState::right();

        controller
            .send_mixer_level_change(MixerSurface::Mix1, 4, 0x2c)
            .expect("send linked mixer level");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x03, 0x2c, 0x02]
        );
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x04, 0x2c, 0x3e]
        );

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2].level,
            Some(0x2c)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][3].level,
            Some(0x2c)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2].pan,
            PanState::left()
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][3].pan,
            PanState::right()
        );
    }

    #[test]
    fn linked_mixer_mute_change_writes_and_updates_both_channels() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].pan = PanState::left();
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].pan = PanState::right();

        controller
            .send_mixer_mute_change(MixerSurface::Mix1, 3, true)
            .expect("send linked mixer mute");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x03, 0x00, 0x42]
        );
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x04, 0x00, 0x7e]
        );

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2].muted,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][3].muted,
            Some(true)
        );
    }

    #[test]
    fn linked_mixer_solo_change_writes_and_updates_both_channels() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].linked = Some(true);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].pan = PanState::left();
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].pan = PanState::right();
        controller.state.mixer.channels[MixerSurface::Mix1.index()][2].muted = Some(false);
        controller.state.mixer.channels[MixerSurface::Mix1.index()][3].muted = Some(false);

        controller
            .send_mixer_solo_change(MixerSurface::Mix1, 4, true)
            .expect("send linked mixer solo");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x03, 0x00, 0x82]
        );
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x04, 0x00, 0xbe]
        );

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2].soloed,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][3].soloed,
            Some(true)
        );
    }

    #[test]
    fn queried_mixer_strip_readback_updates_solo_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));
        let mut body = [0x5a, 0x20].repeat(32);
        body[0] = 0x10;
        body[1] = 0xa0;
        body[32] = 0x10;
        body[33] = 0x20;

        controller.state.observe_frame(
            DeviceSnapshot::QueryReply(QueryResponse {
                query_id: 0x18,
                sub_id: 0x00,
                body,
            }),
            vec![0x75, 0x18, 0x00],
        );

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].soloed,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][0].soloed,
            Some(false)
        );
    }

    #[test]
    fn grounded_link_target_maps_extended_pair_selectors() {
        let mix1 = MixerLinkTarget::from_channel(MixerSurface::Mix1, 11).expect("mix1 target");
        assert_eq!(
            (mix1.left_channel, mix1.right_channel, mix1.selector),
            (11, 12, 0x05)
        );
        assert_eq!(mix1.companion_bank(), None);

        let mix2 = MixerLinkTarget::from_channel(MixerSurface::Mix2, 15).expect("mix2 target");
        assert_eq!(
            (mix2.left_channel, mix2.right_channel, mix2.selector),
            (15, 16, 0x17)
        );
        assert_eq!(mix2.companion_bank(), None);
    }

    #[test]
    fn mixer_link_change_writes_selector_and_updates_pair() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        controller
            .send_mixer_link_change(MixerSurface::Mix1, 11, true)
            .expect("send mix1 link");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x14], &[0xa2, 0x03, 0x05, 0x01]);
        controller.confirm_pending_write();
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][10].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][11].linked,
            Some(true)
        );

        controller
            .send_mixer_link_change(MixerSurface::Mix2, 15, true)
            .expect("send mix2 link");
        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x14], &[0xa2, 0x03, 0x17, 0x01]);
        controller.confirm_pending_write();
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][14].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][15].linked,
            Some(true)
        );
    }

    #[test]
    fn app_state_starts_with_16_strips_per_surface() {
        let state = AppState::default();

        assert_eq!(state.mixer.channels[MixerSurface::Mix1.index()].len(), 16);
        assert_eq!(state.mixer.channels[MixerSurface::Mix2.index()].len(), 16);
        assert_eq!(
            state.mixer.channels[MixerSurface::Mix1.index()][15].channel,
            16
        );
    }

    #[test]
    fn mixer_assignment_is_shared_across_surfaces_but_link_is_not() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller.pending_mutation = Some(PendingMutation::MixerAssignment {
            strip: 11,
            assignment: MixerAssignment::Oscillator(2),
        });
        controller.confirm_pending_write();

        let target = MixerLinkTarget::from_channel(MixerSurface::Mix2, 1).expect("mix2 1-2");
        controller
            .send(
                Command::SetLinkState {
                    selector: target.selector,
                    enabled: true,
                    companion_bank: target.companion_bank(),
                },
                Some(PendingMutation::MixerLink {
                    mixer: MixerSurface::Mix2,
                    selector: target.selector,
                    enabled: true,
                }),
            )
            .expect("send link");
        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][10].assignment,
            Some(MixerAssignment::Oscillator(2))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][10].assignment,
            Some(MixerAssignment::Oscillator(2))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][1].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].linked,
            None
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][1].linked,
            None
        );
    }

    #[test]
    fn mixer_assignment_overlay_updates_both_surfaces_for_strip_11() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller.pending_mutation = Some(PendingMutation::MixerAssignment {
            strip: 11,
            assignment: MixerAssignment::Mute,
        });
        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][10].assignment,
            Some(MixerAssignment::Mute)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][10].assignment,
            Some(MixerAssignment::Mute)
        );
    }

    #[test]
    fn mixer_assignment_write_sends_ordinary_strip_frames_and_updates_shared_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller.state);

        controller
            .send(
                Command::SetMixerAssignment {
                    strip: 5,
                    assignment: MixerAssignment::Oscillator(1),
                },
                Some(PendingMutation::MixerAssignment {
                    strip: 5,
                    assignment: MixerAssignment::Oscillator(1),
                }),
            )
            .expect("assignment write should succeed");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 5);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x03]);
        assert_eq!(&writes[0][0x10 + 0x0b..0x10 + 0x0d], &[0x09, 0x00]);

        controller.confirm_pending_write();
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][4].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][4].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
    }

    #[test]
    fn mixer_assignment_write_sends_early_strip_frames_and_updates_shared_state() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller.state);

        controller
            .send(
                Command::SetMixerAssignment {
                    strip: 1,
                    assignment: MixerAssignment::Oscillator(1),
                },
                Some(PendingMutation::MixerAssignment {
                    strip: 1,
                    assignment: MixerAssignment::Oscillator(1),
                }),
            )
            .expect("assignment write should succeed");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][0x10..0x13], &[0xd3, 0x41, 0x05]);
        assert_eq!(&writes[0][0x10 + 0x03..0x10 + 0x05], &[0x09, 0x00]);

        controller.confirm_pending_write();
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][0].assignment,
            Some(MixerAssignment::Oscillator(1))
        );
    }

    #[test]
    fn late_strip_assignment_write_preserves_existing_assignment_table_entries() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        seed_shared_assignments(&mut controller.state);

        controller
            .send(
                Command::SetMixerAssignment {
                    strip: 11,
                    assignment: MixerAssignment::ComputerPlay(1),
                },
                None,
            )
            .expect("assignment write should succeed");

        let writes = transport.take_writes();
        let bank06 = writes
            .iter()
            .find(|frame| frame[0x10..0x13] == [0xd3, 0x41, 0x06])
            .expect("bank 06 frame");

        assert_eq!(
            assignment_pairs(bank06, 16),
            vec![
                [0x03, 0x00],
                [0x03, 0x01],
                [0x03, 0x02],
                [0x03, 0x03],
                [0x01, 0x02],
                [0x01, 0x03],
                [0x01, 0x04],
                [0x01, 0x05],
                [0x01, 0x06],
                [0x01, 0x07],
                [0x01, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
                [0x08, 0x00],
            ]
        );
    }

    #[test]
    fn link_overlay_respects_full_visible_pair_mapping() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        for target in [
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 1).expect("mix1 1-2"),
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 5).expect("mix1 5-6"),
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 7).expect("mix1 7-8"),
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 1).expect("mix2 1-2"),
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 7).expect("mix2 7-8"),
        ] {
            controller
                .send(
                    Command::SetLinkState {
                        selector: target.selector,
                        enabled: true,
                        companion_bank: target.companion_bank(),
                    },
                    Some(PendingMutation::MixerLink {
                        mixer: target.mixer,
                        selector: target.selector,
                        enabled: true,
                    }),
                )
                .expect("send grounded link");
            controller.confirm_pending_write();

            assert_eq!(
                controller.state.mixer.channels[target.mixer.index()]
                    [target.left_channel as usize - 1]
                    .linked,
                Some(true)
            );
            assert_eq!(
                controller.state.mixer.channels[target.mixer.index()]
                    [target.right_channel as usize - 1]
                    .linked,
                Some(true)
            );
        }
        assert!(MixerStrip::ordinary(4).is_none());
    }

    #[test]
    fn grounded_link_with_companion_writes_helper_before_selector_write() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        let target = MixerLinkTarget::from_channel(MixerSurface::Mix1, 1).expect("mix1 1-2");

        controller
            .send(
                Command::SetLinkState {
                    selector: target.selector,
                    enabled: true,
                    companion_bank: target.companion_bank(),
                },
                Some(PendingMutation::MixerLink {
                    mixer: MixerSurface::Mix1,
                    selector: target.selector,
                    enabled: true,
                }),
            )
            .expect("send link with companion");

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(&writes[0][0x10..0x14], &[0xa2, 0x04, 0x00, 0x01]);
        assert_eq!(&writes[1][0x10..0x14], &[0xa2, 0x03, 0x00, 0x01]);

        controller.confirm_pending_write();
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][0].linked,
            Some(true)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][1].linked,
            Some(true)
        );
    }

    #[test]
    fn mixer_pan_updates_are_tracked_per_surface() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller
            .send(
                Command::SetMixerPan {
                    mixer: MixerSurface::Mix1,
                    channel: 4,
                    pan: PanState::from_raw(0x08),
                    muted: false,
                    soloed: false,
                },
                Some(PendingMutation::MixerPan {
                    mixer: MixerSurface::Mix1,
                    channel: 4,
                    pan: PanState::from_raw(0x08),
                }),
            )
            .expect("mix1 pan");
        controller.confirm_pending_write();

        controller
            .send(
                Command::SetMixerPan {
                    mixer: MixerSurface::Mix2,
                    channel: 4,
                    pan: PanState::from_raw(0x36),
                    muted: false,
                    soloed: false,
                },
                Some(PendingMutation::MixerPan {
                    mixer: MixerSurface::Mix2,
                    channel: 4,
                    pan: PanState::from_raw(0x36),
                }),
            )
            .expect("mix2 pan");
        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][3]
                .pan
                .raw(),
            0x08
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][3]
                .pan
                .raw(),
            0x36
        );
    }

    #[test]
    fn mixer_mute_does_not_invent_zero_level_for_undecoded_channel() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller
            .send(
                Command::SetMixerMute {
                    mixer: antelope_protocol::MixerSurface::Mix1,
                    channel: 7,
                    muted: true,
                    pan_state: antelope_protocol::PanState::center(),
                    soloed: false,
                },
                Some(PendingMutation::MixerMute {
                    mixer: MixerSurface::Mix1,
                    channel: 7,
                    muted: true,
                }),
            )
            .expect("send mute");

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][6].level,
            None
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][6].muted,
            Some(true)
        );

        controller
            .send(
                Command::SetMixerMute {
                    mixer: antelope_protocol::MixerSurface::Mix1,
                    channel: 7,
                    muted: false,
                    pan_state: antelope_protocol::PanState::center(),
                    soloed: false,
                },
                Some(PendingMutation::MixerMute {
                    mixer: MixerSurface::Mix1,
                    channel: 7,
                    muted: false,
                }),
            )
            .expect("send unmute");

        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][6].level,
            None
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][6].muted,
            Some(false)
        );
    }

    #[test]
    fn mixer_state_is_tracked_per_surface() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        controller
            .send(
                Command::SetMixerLevel {
                    mixer: MixerSurface::Mix1,
                    channel: 3,
                    level: 0x2c,
                    pan_state: antelope_protocol::PanState::center(),
                    muted: false,
                    soloed: false,
                },
                Some(PendingMutation::MixerLevel {
                    mixer: MixerSurface::Mix1,
                    channel: 3,
                    level: 0x2c,
                    pan: antelope_protocol::PanState::center(),
                    muted: false,
                }),
            )
            .expect("mix1 send");
        controller.confirm_pending_write();

        controller
            .send(
                Command::SetMixerLevel {
                    mixer: MixerSurface::Mix2,
                    channel: 3,
                    level: 0x10,
                    pan_state: antelope_protocol::PanState::center(),
                    muted: false,
                    soloed: false,
                },
                Some(PendingMutation::MixerLevel {
                    mixer: MixerSurface::Mix2,
                    channel: 3,
                    level: 0x10,
                    pan: antelope_protocol::PanState::center(),
                    muted: false,
                }),
            )
            .expect("mix2 send");
        controller.confirm_pending_write();

        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix1.index()][2].level,
            Some(0x2c)
        );
        assert_eq!(
            controller.state.mixer.channels[MixerSurface::Mix2.index()][2].level,
            Some(0x10)
        );
    }

    #[test]
    fn mixer_first_adjustment_starts_from_safe_midpoint_not_minimum() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        controller.state.ui.focus = FocusArea::Mixer;
        controller.state.mixer.selected_channel = 0;

        let channel = controller.state.active_mixer_channels()[0].channel;
        controller
            .send(
                Command::SetMixerLevel {
                    mixer: MixerSurface::from_surface(controller.state.mixer.surface),
                    channel,
                    level: 0x1f,
                    pan_state: antelope_protocol::PanState::center(),
                    muted: false,
                    soloed: false,
                },
                None,
            )
            .expect("send first adjustment");
        controller.flush_commands().expect("flush");

        let writes = transport.take_writes();
        let mixer_write = writes.last().expect("mixer write");
        assert_eq!(
            &mixer_write[0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x1f, 0x20]
        );
    }

    #[test]
    fn connection_status_changes_when_frames_arrive() {
        let mut state = AppState::default();
        state.mark_disconnected();
        assert!(!state.device.connection.connected);

        state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), vec![0x73, 0, 0, 0]);

        assert!(state.device.connection.connected);
        assert!(state.device.connection.last_snapshot_at.is_some());
    }

    #[test]
    fn identical_snapshot_does_not_report_visible_change_twice() {
        let mut state = AppState::default();
        let raw = vec![0x73, 0, 0, 0];

        assert!(state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), raw.clone()));
        assert!(!state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), raw));
    }

    #[test]
    fn raw_only_snapshot_difference_is_not_visible_when_raw_view_is_closed() {
        let mut state = AppState::default();
        state.device.connection.connected = true;
        state.latest_structural_snapshot = Some(StructuralSnapshot::from_snapshot(&snapshot()));
        state.raw_view.latest_raw_73 = Some(vec![0x73, 0, 0, 0]);

        assert!(!state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), vec![0x73, 0, 0, 1],));
    }

    #[test]
    fn raw_only_snapshot_difference_is_visible_when_raw_view_is_open() {
        let mut state = AppState::default();
        state.device.connection.connected = true;
        state.popup.raw_view_open = true;
        state.latest_structural_snapshot = Some(StructuralSnapshot::from_snapshot(&snapshot()));
        state.raw_view.latest_raw_73 = Some(vec![0x73, 0, 0, 0]);

        assert!(state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), vec![0x73, 0, 0, 1],));
    }

    #[test]
    fn auxiliary_frame_is_not_visible_when_raw_view_is_closed() {
        let mut state = AppState::default();
        state.device.connection.connected = true;

        assert!(!state.observe_frame(
            DeviceSnapshot::Auxiliary(vec![0x60, 0xc0, 0x60, 0x00]),
            vec![0x83, 0, 0, 0],
        ));
    }

    #[test]
    fn auxiliary_frame_is_visible_when_raw_view_is_open() {
        let mut state = AppState::default();
        state.device.connection.connected = true;
        state.popup.raw_view_open = true;

        assert!(state.observe_frame(
            DeviceSnapshot::Auxiliary(vec![0x60, 0xc0, 0x60, 0x00]),
            vec![0x83, 0, 0, 0],
        ));
    }

    #[test]
    fn poll_device_does_not_mark_identical_snapshot_dirty_when_view_is_unchanged() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));
        let raw = snapshot_frame_bytes(0x12);
        let snapshot = Frame::parse(&raw)
            .expect("snapshot frame")
            .as_snapshot()
            .expect("snapshot")
            .clone();
        controller.state.device.connection.connected = true;
        controller.state.latest_structural_snapshot =
            Some(StructuralSnapshot::from_snapshot(&snapshot));
        controller.state.raw_view.latest_raw_73 = Some(raw.clone());
        controller.state.apply_snapshot(&snapshot);

        transport.push_read(raw);

        assert!(!controller.poll_device(Duration::ZERO).expect("poll"));
    }

    #[test]
    #[ignore = "benchmark"]
    fn perf_poll_device_snapshot_backlog() {
        const FRAMES: usize = 20_000;
        let polls = FRAMES.div_ceil(MAX_FRAMES_PER_POLL) + 1;

        let transport = MockTransport::default();
        for meter in 0..FRAMES {
            transport.push_read(snapshot_frame_bytes((meter % 0x3d) as u8));
        }

        let mut controller = Controller::new(Box::new(transport));
        let started = Instant::now();
        let mut dirty_polls = 0_usize;
        for _ in 0..polls {
            dirty_polls += usize::from(controller.poll_device(Duration::ZERO).expect("poll"));
        }
        let elapsed = started.elapsed();

        println!(
            "poll_device backlog: frames={FRAMES} polls={polls} dirty_polls={dirty_polls} elapsed_ms={} ns_per_frame={}",
            elapsed.as_millis(),
            elapsed.as_nanos() / FRAMES as u128
        );
    }

    #[test]
    fn poll_device_drains_backlog_to_latest_snapshot() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport.clone()));

        let mut first = vec![0_u8; 320];
        first[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        first[4..8].copy_from_slice(&0x20_u32.to_le_bytes());
        let first_payload = &mut first[0x10..];
        first_payload[0x02] = SampleRate::Hz44100.code();
        first_payload[0x03] = ClockSource::Internal.code();
        first_payload[0x04..0x08].copy_from_slice(&44_100_u32.to_be_bytes());

        let mut second = vec![0_u8; 320];
        second[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        second[4..8].copy_from_slice(&0x20_u32.to_le_bytes());
        let second_payload = &mut second[0x10..];
        second_payload[0x02] = SampleRate::Hz48000.code();
        second_payload[0x03] = ClockSource::Usb.code();
        second_payload[0x04..0x08].copy_from_slice(&48_000_u32.to_be_bytes());

        transport.push_read(first);
        transport.push_read(second);

        let observed_frame = controller.poll_device(Duration::ZERO).expect("poll");

        assert!(observed_frame);
        assert_eq!(
            controller.state.device.status.sample_rate,
            Some(SampleRate::Hz48000)
        );
        assert_eq!(
            controller.state.device.status.clock_source,
            Some(ClockSource::Usb)
        );
    }

    #[test]
    fn poll_device_reports_idle_reads_without_marking_state_dirty() {
        let transport = MockTransport::default();
        let mut controller = Controller::new(Box::new(transport));

        let observed_frame = controller.poll_device(Duration::ZERO).expect("idle poll");

        assert!(!observed_frame);
    }

    #[test]
    fn raw_state_tracks_latest_snapshot_and_auxiliary_frames() {
        let mut state = AppState::default();
        let mut raw73 = vec![0_u8; 320];
        raw73[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        raw73[0x10 + 0xcf] = 0x4c;
        state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), raw73.clone());

        let mut raw83 = vec![0_u8; 0x14];
        raw83[0..4].copy_from_slice(&0x83_u32.to_le_bytes());
        raw83[0x10..0x14].copy_from_slice(&[0x60, 0xc0, 0x60, 0x00]);
        state.observe_frame(
            DeviceSnapshot::Auxiliary(vec![0x60, 0xc0, 0x60, 0x00]),
            raw83.clone(),
        );

        assert!(state.raw_view.latest_raw_73.is_some());
        assert!(state.raw_view.latest_raw_83.is_some());
        assert_eq!(
            state.raw_view.latest_raw_73.as_ref().expect("0x73")[0x10 + 0xcf],
            0x4c
        );
        assert_eq!(
            &state.raw_view.latest_raw_83.as_ref().expect("0x83")[0..4],
            &raw83[0..4]
        );

        let raw75 = vec![
            0x75, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0x01, 0, 0, 0, 0, 0, 0, 0, b'Z',
        ];
        let raw74 = vec![0x74, 0x00, 0x00, 0x00, 0, 0, 0, 0, 0x11, 0, 0, 0, 0x03];
        state.observe_query_request(raw74.clone());
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x01,
                sub_id: 0x00,
                body: vec![b'Z'],
            }),
            raw75.clone(),
        );

        let raw81 = vec![0x81, 0x10, 0x20, 0x30, 0x40, 0x50];
        state.observe_frame(
            DeviceSnapshot::Notification(antelope_protocol::DeviceNotification {
                bytes: [0x81, 0x10, 0x20, 0x30, 0x40, 0x50],
            }),
            raw81.clone(),
        );

        assert_eq!(state.raw_view.latest_raw_75, Some(raw75));
        assert_eq!(state.raw_view.latest_raw_81, Some(raw81));
        assert_eq!(state.raw_view.latest_raw_74, Some(raw74));
        assert_eq!(state.raw_view.recent_query_request_log.len(), 1);
        assert!(state.raw_view.recent_query_request_log[0].contains("0x74 11/03"));
        assert_eq!(state.raw_view.recent_query_reply_log.len(), 1);
        assert!(state.raw_view.recent_query_reply_log[0].contains("0x75 01/00"));
        assert_eq!(
            state.startup_query_summary(0x01),
            Some("Metadata: undecoded")
        );
    }

    #[test]
    fn raw_baseline_captures_latest_packets() {
        let mut state = AppState::default();
        state.observe_frame(DeviceSnapshot::Snapshot(snapshot()), vec![0x73, 0, 0, 0]);
        state.observe_frame(
            DeviceSnapshot::Auxiliary(vec![0x60, 0xc0, 0x60, 0x00]),
            vec![0x83, 0, 0, 0],
        );
        state.observe_query_request(vec![0x74, 0, 0, 0]);
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x11,
                sub_id: 0x00,
                body: vec![0xaa, 0xbb],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::Notification(antelope_protocol::DeviceNotification {
                bytes: [1, 2, 3, 4, 5, 6],
            }),
            vec![1, 2, 3, 4, 5, 6],
        );

        state.capture_raw_baseline();
        assert_eq!(state.raw_view.baseline_raw_73, state.raw_view.latest_raw_73);
        assert_eq!(state.raw_view.baseline_raw_83, state.raw_view.latest_raw_83);
        assert_eq!(state.raw_view.baseline_raw_74, state.raw_view.latest_raw_74);
        assert_eq!(state.raw_view.baseline_raw_75, state.raw_view.latest_raw_75);
        assert_eq!(state.raw_view.baseline_raw_81, state.raw_view.latest_raw_81);

        state.clear_raw_baseline();
        assert!(state.raw_view.baseline_raw_73.is_none());
        assert!(state.raw_view.baseline_raw_83.is_none());
        assert!(state.raw_view.baseline_raw_74.is_none());
        assert!(state.raw_view.baseline_raw_75.is_none());
        assert!(state.raw_view.baseline_raw_81.is_none());
    }

    #[test]
    fn stores_grounded_startup_query_summaries_for_all_bootstrap_replies() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x00,
                sub_id: 0x00,
                body: vec![0xaa, 0xbb, 0xcc],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x11,
                sub_id: 0x00,
                body: vec![0x12],
            }),
            vec![0x75, 0, 0, 0],
        );

        assert_eq!(
            state.startup_query_summary(0x00),
            Some("Capability/default block: 3 bytes [aa bb cc]")
        );
        assert_eq!(
            state.startup_query_summary(0x11),
            Some("Status/capability value: 1 bytes [12]")
        );
    }

    #[test]
    fn metadata_reply_updates_serial_and_hardware_version() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x01,
                sub_id: 0x00,
                body: [
                    b"Zen Go Synergy Core".as_slice(),
                    b"\0".as_slice(),
                    b"4502721001300".as_slice(),
                    b"\0".as_slice(),
                    b"6.6".as_slice(),
                    b"\0".as_slice(),
                ]
                .concat(),
            }),
            vec![0x75, 0, 0, 0],
        );

        let metadata = state.device.status.metadata.expect("metadata");
        assert_eq!(metadata.product_name, "Zen Go Synergy Core");
        assert_eq!(metadata.serial, "4502721001300");
        assert_eq!(metadata.hardware_version, "6.6");
        assert_eq!(
            state.ui.last_message,
            "Connected to Zen Go Synergy Core (hw 6.6, serial 4502721001300)"
        );
    }

    #[test]
    fn query_reply_log_keeps_recent_entries() {
        let mut state = AppState::default();

        for sub_id in 0..20_u8 {
            state.observe_frame(
                DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                    query_id: 0x03,
                    sub_id,
                    body: vec![sub_id, 0xaa],
                }),
                vec![0x75, 0, 0, 0],
            );
        }

        assert_eq!(state.raw_view.recent_query_reply_log.len(), 16);
        assert!(state
            .raw_view
            .recent_query_reply_log
            .first()
            .unwrap()
            .contains("0x75 03/04"));
        assert!(state
            .raw_view
            .recent_query_reply_log
            .last()
            .unwrap()
            .contains("0x75 03/13"));
        assert_eq!(state.raw_view.selected_query_reply_entry, Some(15));
    }

    #[test]
    fn query_reply_log_surfaces_selector_family_summaries() {
        let mut state = AppState::default();

        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x0b,
                sub_id: 0x03,
                body: vec![
                    0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );
        state.observe_frame(
            DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                query_id: 0x04,
                sub_id: 0x01,
                body: vec![
                    0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00,
                    0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                    0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00,
                    0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                    0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                ],
            }),
            vec![0x75, 0, 0, 0],
        );

        assert!(state.raw_view.recent_query_reply_log[0].contains("Selector bitmap"));
        assert!(state.raw_view.recent_query_reply_log[1].contains("Startup Mix2 pan categories"));
    }

    #[test]
    fn selected_query_reply_entry_tracks_latest_reply_and_cycles() {
        let mut state = AppState::default();
        for sub_id in 0..3_u8 {
            state.observe_frame(
                DeviceSnapshot::QueryReply(antelope_protocol::QueryResponse {
                    query_id: 0x03,
                    sub_id,
                    body: vec![sub_id],
                }),
                vec![0x75, sub_id],
            );
        }

        assert_eq!(state.raw_view.selected_query_reply_entry, Some(2));
        assert_eq!(
            state
                .selected_query_reply_entry()
                .map(|entry| entry.raw.clone()),
            Some(vec![0x75, 0x02])
        );

        state.cycle_query_reply_entry(false);
        assert_eq!(state.raw_view.selected_query_reply_entry, Some(1));
        state.cycle_query_reply_entry(true);
        assert_eq!(state.raw_view.selected_query_reply_entry, Some(2));
    }

    #[test]
    fn query_request_log_keeps_recent_entries() {
        let mut state = AppState::default();

        for sub_id in 0..20_u8 {
            state.observe_query_request(vec![0x74, 0, 0, 0, 0, 0, 0, 0, 0x03, 0, 0, 0, sub_id]);
        }

        assert_eq!(state.raw_view.recent_query_request_log.len(), 16);
        assert!(state
            .raw_view
            .recent_query_request_log
            .first()
            .unwrap()
            .contains("0x74 03/04"));
        assert!(state
            .raw_view
            .recent_query_request_log
            .last()
            .unwrap()
            .contains("0x74 03/13"));
    }

    #[test]
    fn focus_cycle_skips_raw_view_state() {
        let mut state = AppState::default();
        state.ui.focus = FocusArea::Status;

        state.cycle_focus();
        assert_eq!(state.ui.focus, FocusArea::Outputs);
        state.cycle_focus();
        assert_eq!(state.ui.focus, FocusArea::Mixer);
        state.cycle_focus();
        assert_eq!(state.ui.focus, FocusArea::Preamp);
        state.cycle_focus();
        assert_eq!(state.ui.focus, FocusArea::Outputs);
    }

    #[test]
    fn raw_view_toggle_and_packet_tab_cycle_are_independent_of_focus() {
        let mut state = AppState::default();

        state.toggle_raw_view();
        assert!(state.popup.raw_view_open);
        assert_eq!(state.raw_view.selected_tab, RawPacketTab::State73);

        state.cycle_raw_packet(true);
        assert_eq!(state.raw_view.selected_tab, RawPacketTab::Auxiliary);
        state.cycle_raw_packet(false);
        assert_eq!(state.raw_view.selected_tab, RawPacketTab::State73);

        state.toggle_raw_view();
        assert!(!state.popup.raw_view_open);
    }

    #[test]
    fn ensure_selected_mixer_channel_visible_advances_scroll_window() {
        let mut state = AppState::default();
        state.mixer.selected_channel = 6;

        state.ensure_selected_mixer_channel_visible(4);

        assert_eq!(state.mixer.strip_scroll, 3);
    }

    #[test]
    fn mixer_strip_viewport_scroll_clamps_to_available_channels() {
        let mut state = AppState::default();

        state.scroll_mixer_strip_viewport(99, 5);
        assert_eq!(state.mixer.strip_scroll, 11);

        state.scroll_mixer_strip_viewport(-99, 5);
        assert_eq!(state.mixer.strip_scroll, 0);
    }

    #[test]
    fn mixer_strip_viewport_paging_moves_between_banks() {
        let mut state = AppState::default();
        let page = 8;

        state.page_mixer_strip_viewport(true, page);
        assert_eq!(state.mixer.strip_scroll, 8);

        state.page_mixer_strip_viewport(true, page);
        assert_eq!(state.mixer.strip_scroll, 8);

        state.page_mixer_strip_viewport(false, page);
        assert_eq!(state.mixer.strip_scroll, 0);
    }
}
