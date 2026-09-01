use std::time::{Duration, Instant};

use antelope_protocol::{
    ClockSource, DeviceMetadata, MixerChannelState, OutputMode, OutputState, OutputTarget,
    PreampState, SampleRate, Surface,
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
#[derive(Debug, Clone, Default)]
pub struct PreampData {
    pub state: PreampState,
    pub selected_input: usize,
    pub peaks: [Option<MeterPeak>; 2],
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
