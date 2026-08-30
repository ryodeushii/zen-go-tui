use antelope_protocol::{
    meter_ratio, ClockSource, MixerAssignment, OutputMode, OutputTarget, PanState, PreampMode,
    SampleRate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiPage {
    Mixer,
    Routing,
    Profiles,
    Raw,
    Settings,
}

impl GuiPage {
    pub const ALL: [Self; 5] = [
        Self::Mixer,
        Self::Routing,
        Self::Profiles,
        Self::Raw,
        Self::Settings,
    ];

    pub fn index(self) -> i32 {
        match self {
            Self::Mixer => 0,
            Self::Routing => 1,
            Self::Profiles => 2,
            Self::Raw => 3,
            Self::Settings => 4,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Mixer => "Mixer",
            Self::Routing => "Routing",
            Self::Profiles => "Profiles",
            Self::Raw => "Raw",
            Self::Settings => "Settings",
        }
    }

    pub fn from_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::Mixer),
            1 => Some(Self::Routing),
            2 => Some(Self::Profiles),
            3 => Some(Self::Raw),
            4 => Some(Self::Settings),
            _ => None,
        }
    }

    pub fn gui_index(self) -> Option<i32> {
        match self {
            Self::Mixer => Some(0),
            Self::Routing => None,
            Self::Profiles => Some(1),
            Self::Raw => Some(2),
            Self::Settings => Some(3),
        }
    }

    pub fn from_gui_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::Mixer),
            1 => Some(Self::Profiles),
            2 => Some(Self::Raw),
            3 => Some(Self::Settings),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

impl GuiConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Error => "Error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceSnapshot {
    pub index: usize,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeaderSnapshot {
    pub active_page: GuiPage,
    pub connection: GuiConnectionState,
    pub status_label: String,
    pub sample_rate_label: String,
    pub clock_source_label: String,
    pub profile_label: String,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputSnapshot {
    pub index: usize,
    pub name: String,
    pub level_step: u8,
    pub level_db: i16,
    pub level_ratio: f32,
    pub meter_ratio: f32,
    pub mode_label: String,
    pub muted: bool,
    pub dimmed: bool,
}

impl OutputSnapshot {
    pub fn silent(target: OutputTarget) -> Self {
        let index = target.index() as usize;
        let level_step = 96;
        Self {
            index,
            name: target.label().to_string(),
            level_step,
            level_db: -(level_step as i16),
            level_ratio: 0.0,
            meter_ratio: 0.0,
            mode_label: OutputMode::Mute.label().to_string(),
            muted: true,
            dimmed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MixerStripSnapshot {
    pub channel: u8,
    pub name: String,
    pub assignment_label: String,
    pub assignment_short_label: String,
    pub level: u8,
    pub level_ratio: f32,
    pub meter_ratio: f32,
    pub pan_raw: u8,
    pub pan_ratio: f32,
    pub pan_display: i16,
    pub muted: bool,
    pub soloed: bool,
    pub linked: bool,
    pub linkable: bool,
    pub selected: bool,
}

impl MixerStripSnapshot {
    pub fn empty(channel: u8) -> Self {
        assert!((1..=16).contains(&channel));

        Self {
            channel,
            name: format!("CH {channel:02}"),
            assignment_label: "Unassigned".to_string(),
            assignment_short_label: "--".to_string(),
            level: 96,
            level_ratio: 0.0,
            meter_ratio: 0.0,
            pan_raw: PanState::CENTER,
            pan_ratio: PanState::center().ratio() as f32,
            pan_display: 0,
            muted: false,
            soloed: false,
            linked: false,
            linkable: channel % 2 == 1,
            selected: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MixerSnapshot {
    pub active_surface_index: usize,
    pub active_surface_label: String,
    pub strips: Vec<MixerStripSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreampSnapshot {
    pub input: u8,
    pub name: String,
    pub mode_label: String,
    pub gain_raw: u8,
    pub gain_ratio: f32,
    pub meter_ratio: f32,
    pub phantom: bool,
    pub phase_inverted: bool,
    pub selected: bool,
}

impl PreampSnapshot {
    pub fn empty(input: u8) -> Self {
        assert!((1..=2).contains(&input));

        Self {
            input,
            name: format!("A{input}"),
            mode_label: PreampMode::Mic.label().to_string(),
            gain_raw: 0,
            gain_ratio: 0.0,
            meter_ratio: 0.0,
            phantom: false,
            phase_inverted: false,
            selected: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileSnapshot {
    pub index: usize,
    pub name: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawPacketSnapshot {
    pub tab_label: String,
    pub summary: String,
    pub rows: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsSnapshot {
    pub refresh_rate_label: String,
    pub peak_enabled: bool,
    pub peak_threshold_label: String,
    pub peak_hold_label: String,
    pub auto_save: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuiSnapshot {
    pub header: HeaderSnapshot,
    pub outputs: Vec<OutputSnapshot>,
    pub mixer: MixerSnapshot,
    pub preamps: Vec<PreampSnapshot>,
    pub profiles: Vec<ProfileSnapshot>,
    pub raw: RawPacketSnapshot,
    pub settings: SettingsSnapshot,
    pub sample_rate_choices: Vec<ChoiceSnapshot>,
    pub clock_source_choices: Vec<ChoiceSnapshot>,
    pub assignment_choices: Vec<ChoiceSnapshot>,
    pub notice: String,
}

impl GuiSnapshot {
    pub fn disconnected(active_page: GuiPage) -> Self {
        let outputs = [OutputTarget::Monitor, OutputTarget::Hp1, OutputTarget::Hp2]
            .into_iter()
            .map(OutputSnapshot::silent)
            .collect();
        let strips = (1..=16).map(MixerStripSnapshot::empty).collect();
        let preamps = (1..=2).map(PreampSnapshot::empty).collect();
        let sample_rate_choices = SampleRate::all_confirmed()
            .iter()
            .enumerate()
            .map(|(index, rate)| ChoiceSnapshot {
                index,
                label: rate.label(),
                selected: false,
            })
            .collect();
        let clock_source_choices = ClockSource::all_confirmed()
            .iter()
            .enumerate()
            .map(|(index, source)| ChoiceSnapshot {
                index,
                label: source.label().to_string(),
                selected: false,
            })
            .collect();
        let assignment_choices = MixerAssignment::grounded_choices()
            .iter()
            .enumerate()
            .map(|(index, assignment)| ChoiceSnapshot {
                index,
                label: assignment.label().to_string(),
                selected: false,
            })
            .collect();

        Self {
            header: HeaderSnapshot {
                active_page,
                connection: GuiConnectionState::Disconnected,
                status_label: GuiConnectionState::Disconnected.label().to_string(),
                sample_rate_label: "-- Hz".to_string(),
                clock_source_label: "--".to_string(),
                profile_label: "No profile".to_string(),
                stale: false,
            },
            outputs,
            mixer: MixerSnapshot {
                active_surface_index: 0,
                active_surface_label: "Mix 1".to_string(),
                strips,
            },
            preamps,
            profiles: Vec::new(),
            raw: RawPacketSnapshot {
                tab_label: "State 73".to_string(),
                summary: "No packets captured".to_string(),
                rows: Vec::new(),
            },
            settings: SettingsSnapshot {
                refresh_rate_label: "100 ms".to_string(),
                peak_enabled: true,
                peak_threshold_label: "-6 dB".to_string(),
                peak_hold_label: "2 s".to_string(),
                auto_save: true,
            },
            sample_rate_choices,
            clock_source_choices,
            assignment_choices,
            notice: "Waiting for Zen Go device".to_string(),
        }
    }
}

pub fn meter_ratio_f32(raw: Option<u8>) -> f32 {
    raw.map(meter_ratio).unwrap_or(0.0) as f32
}

#[cfg(test)]
mod tests {
    use super::{GuiPage, GuiSnapshot, OutputSnapshot};
    use antelope_protocol::OutputTarget;

    #[test]
    fn page_index_round_trips_known_pages() {
        assert_eq!(GuiPage::from_index(0), Some(GuiPage::Mixer));
        assert_eq!(GuiPage::from_index(4), Some(GuiPage::Settings));
        assert_eq!(GuiPage::from_index(5), None);
    }

    #[test]
    fn gui_page_index_round_trips_visible_pages() {
        assert_eq!(GuiPage::Mixer.gui_index(), Some(0));
        assert_eq!(GuiPage::Profiles.gui_index(), Some(1));
        assert_eq!(GuiPage::Raw.gui_index(), Some(2));
        assert_eq!(GuiPage::Settings.gui_index(), Some(3));
        assert_eq!(GuiPage::Routing.gui_index(), None);

        assert_eq!(GuiPage::from_gui_index(0), Some(GuiPage::Mixer));
        assert_eq!(GuiPage::from_gui_index(1), Some(GuiPage::Profiles));
        assert_eq!(GuiPage::from_gui_index(2), Some(GuiPage::Raw));
        assert_eq!(GuiPage::from_gui_index(3), Some(GuiPage::Settings));
        assert_eq!(GuiPage::from_gui_index(4), None);
    }

    #[test]
    fn snapshot_starts_with_complete_static_controls() {
        let snapshot = GuiSnapshot::disconnected(GuiPage::Mixer);

        assert_eq!(snapshot.outputs.len(), 3);
        assert_eq!(snapshot.mixer.strips.len(), 16);
        assert_eq!(snapshot.preamps.len(), 2);
        assert_eq!(snapshot.sample_rate_choices.len(), 7);
        assert_eq!(snapshot.clock_source_choices.len(), 3);
        assert_eq!(snapshot.assignment_choices.len(), 17);
    }

    #[test]
    fn empty_mixer_strips_expose_linkability() {
        assert!(super::MixerStripSnapshot::empty(1).linkable);
        assert!(!super::MixerStripSnapshot::empty(2).linkable);
    }

    #[test]
    fn output_snapshot_preserves_target_identity() {
        let output = OutputSnapshot::silent(OutputTarget::Hp1);

        assert_eq!(output.index, 1);
        assert_eq!(output.name, "HP1");
        assert_eq!(output.level_db, -96);
        assert!(output.muted);
    }
}
