use crate::models::GuiPage;
use antelope_protocol::{ClockSource, MixerAssignment, PanState, PreampMode, SampleRate};
use zen_go_tui::app::{Intent, PeakHoldDuration, RefreshRate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiCommand {
    SetPage(GuiPage),
    SetOutputLevel {
        index: usize,
        step: u8,
    },
    ToggleOutputMute(usize),
    ToggleOutputDim(usize),
    SelectMixerChannel(usize),
    SetMixerLevel {
        index: usize,
        level: u8,
    },
    SetMixerPan {
        index: usize,
        pan: PanState,
    },
    ToggleMixerMute(u8),
    ToggleMixerSolo(u8),
    ToggleMixerLink(u8),
    OpenAssignmentPicker(u8),
    PickAssignment {
        strip: u8,
        assignment: MixerAssignment,
    },
    SetPreampGain {
        input: u8,
        raw: u8,
    },
    PickPreampMode {
        input: u8,
        mode: PreampMode,
    },
    TogglePreampPhase(u8),
    TogglePreampPhantom(u8),
    PickSampleRate(SampleRate),
    PickClockSource(ClockSource),
    SelectProfile(usize),
    LoadSelectedProfile,
    SaveProfile(String),
    RenameProfile(String),
    DeleteSelectedProfile,
    CaptureRawBaseline,
    ClearRawBaseline,
    RefreshQueriedState,
    SetRefreshRate(RefreshRate),
    CyclePeakThreshold(bool),
    TogglePeakEnabled,
    SetPeakHoldDuration(PeakHoldDuration),
    ToggleAutoSave,
    Shutdown,
}

impl GuiCommand {
    pub fn set_page_from_index(index: i32) -> Option<Self> {
        GuiPage::from_gui_index(index).map(Self::SetPage)
    }

    pub fn set_output_level(index: usize, step: u8) -> Option<Self> {
        (index < 3).then_some(Self::SetOutputLevel { index, step })
    }

    pub fn adjust_output_level(index: usize, current: u8, delta: i16) -> Option<Self> {
        let step = adjust_device_step(current, delta);
        Self::set_output_level(index, step)
    }

    pub fn toggle_output_mute(index: usize) -> Option<Self> {
        (index < 3).then_some(Self::ToggleOutputMute(index))
    }

    pub fn toggle_output_dim(index: usize) -> Option<Self> {
        (index < 3).then_some(Self::ToggleOutputDim(index))
    }

    pub fn select_mixer_channel(index: usize) -> Option<Self> {
        (index < 16).then_some(Self::SelectMixerChannel(index))
    }

    pub fn select_mixer_channel_by_channel(channel: u8) -> Option<Self> {
        valid_channel(channel).then(|| Self::SelectMixerChannel(channel as usize - 1))
    }

    pub fn set_mixer_level(index: usize, level: u8) -> Option<Self> {
        (index < 16).then_some(Self::SetMixerLevel { index, level })
    }

    pub fn set_mixer_level_by_channel(channel: i32, level: i32) -> Option<Self> {
        let index = channel_index(channel)?;
        let level = u8::try_from(level).ok()?;
        (level <= MAX_DIRECT_MIXER_LEVEL).then_some(Self::SetMixerLevel { index, level })
    }

    pub fn adjust_mixer_level(index: usize, current: u8, delta: i16) -> Option<Self> {
        let level = adjust_device_step(current, delta);
        Self::set_mixer_level(index, level)
    }

    pub fn set_mixer_pan(index: usize, pan: PanState) -> Option<Self> {
        (index < 16).then_some(Self::SetMixerPan { index, pan })
    }

    pub fn set_mixer_pan_by_channel(channel: i32, raw: i32) -> Option<Self> {
        let index = channel_index(channel)?;
        let raw = u8::try_from(raw).ok()?;
        (PanState::MIN..=PanState::MAX)
            .contains(&raw)
            .then_some(Self::SetMixerPan {
                index,
                pan: PanState::from_raw(raw),
            })
    }

    pub fn adjust_mixer_pan(index: usize, current_raw: u8, delta: i16) -> Option<Self> {
        let raw =
            (current_raw as i16 + delta).clamp(PanState::MIN as i16, PanState::MAX as i16) as u8;
        Self::set_mixer_pan(index, PanState::from_raw(raw))
    }

    pub fn toggle_mixer_mute(channel: u8) -> Option<Self> {
        valid_channel(channel).then_some(Self::ToggleMixerMute(channel))
    }

    pub fn toggle_mixer_solo(channel: u8) -> Option<Self> {
        valid_channel(channel).then_some(Self::ToggleMixerSolo(channel))
    }

    pub fn toggle_mixer_link(channel: u8) -> Option<Self> {
        (valid_channel(channel) && channel % 2 == 1).then_some(Self::ToggleMixerLink(channel))
    }

    pub fn toggle_mixer_link_by_channel(channel: i32) -> Option<Self> {
        let channel = u8::try_from(channel).ok()?;
        Self::toggle_mixer_link(channel)
    }

    pub fn open_assignment_picker(strip: u8) -> Option<Self> {
        valid_channel(strip).then_some(Self::OpenAssignmentPicker(strip))
    }

    pub fn pick_assignment(strip: u8, assignment: MixerAssignment) -> Option<Self> {
        valid_channel(strip).then_some(Self::PickAssignment { strip, assignment })
    }

    pub fn pick_assignment_from_index(strip: u8, choice_index: usize) -> Option<Self> {
        let assignment = *MixerAssignment::grounded_choices().get(choice_index)?;
        Self::pick_assignment(strip, assignment)
    }

    pub fn pick_assignment_from_indices(channel: i32, choice_index: i32) -> Option<Self> {
        let channel = u8::try_from(channel).ok()?;
        let choice_index = usize::try_from(choice_index).ok()?;
        Self::pick_assignment_from_index(channel, choice_index)
    }

    pub fn set_preamp_gain(input: u8, raw: u8) -> Option<Self> {
        valid_preamp(input).then_some(Self::SetPreampGain { input, raw })
    }

    pub fn adjust_preamp_gain(input: u8, current: u8, delta: i16) -> Option<Self> {
        let raw = adjust_device_step(current, delta);
        Self::set_preamp_gain(input, raw)
    }

    pub fn pick_preamp_mode(input: u8, mode: PreampMode) -> Option<Self> {
        valid_preamp(input).then_some(Self::PickPreampMode { input, mode })
    }

    pub fn pick_preamp_mode_from_index(input: u8, mode_index: i32) -> Option<Self> {
        let mode = match mode_index {
            0 => PreampMode::Mic,
            1 => PreampMode::Line,
            2 => PreampMode::HiZ,
            _ => return None,
        };
        Self::pick_preamp_mode(input, mode)
    }

    pub fn pick_sample_rate_from_index(index: usize) -> Option<Self> {
        SampleRate::all_confirmed()
            .get(index)
            .copied()
            .map(Self::PickSampleRate)
    }

    pub fn pick_clock_source_from_index(index: usize) -> Option<Self> {
        ClockSource::all_confirmed()
            .get(index)
            .copied()
            .map(Self::PickClockSource)
    }

    pub fn set_refresh_rate_from_index(index: usize) -> Option<Self> {
        RefreshRate::all()
            .get(index)
            .copied()
            .map(Self::SetRefreshRate)
    }

    pub fn set_peak_hold_from_index(index: usize) -> Option<Self> {
        PeakHoldDuration::all()
            .get(index)
            .copied()
            .map(Self::SetPeakHoldDuration)
    }

    pub fn toggle_preamp_phase(input: u8) -> Option<Self> {
        valid_preamp(input).then_some(Self::TogglePreampPhase(input))
    }

    pub fn toggle_preamp_phantom(input: u8) -> Option<Self> {
        valid_preamp(input).then_some(Self::TogglePreampPhantom(input))
    }

    pub fn to_intent(&self) -> Option<Intent> {
        match self {
            Self::SetPage(_) => None,
            Self::SetOutputLevel { index, step } => Some(Intent::SetOutputLevel {
                index: *index,
                step: *step,
            }),
            Self::ToggleOutputMute(index) => Some(Intent::ToggleOutputMute(*index)),
            Self::ToggleOutputDim(index) => Some(Intent::ToggleOutputDim(*index)),
            Self::SelectMixerChannel(index) => Some(Intent::SelectMixerChannel(*index)),
            Self::SetMixerLevel { index, level } => Some(Intent::SetMixerLevel {
                index: *index,
                level: *level,
            }),
            Self::SetMixerPan { index, pan } => Some(Intent::SetMixerPan {
                index: *index,
                pan: *pan,
            }),
            Self::ToggleMixerMute(channel) => Some(Intent::ToggleMixerMute(*channel)),
            Self::ToggleMixerSolo(channel) => Some(Intent::ToggleMixerSolo(*channel)),
            Self::ToggleMixerLink(channel) => Some(Intent::ToggleMixerLink(*channel)),
            Self::OpenAssignmentPicker(strip) => Some(Intent::OpenAssignmentPicker(*strip)),
            Self::PickAssignment { strip, assignment } => Some(Intent::PickAssignment {
                strip: *strip,
                assignment: *assignment,
            }),
            Self::SetPreampGain { input, raw } => Some(Intent::SetPreampGain {
                input: preamp_input_to_controller(*input),
                raw: *raw,
            }),
            Self::PickPreampMode { input, mode } => Some(Intent::PickPreampMode {
                input: preamp_input_to_controller(*input),
                mode: *mode,
            }),
            Self::TogglePreampPhase(input) => Some(Intent::TogglePreampPhase(
                preamp_input_to_controller(*input),
            )),
            Self::TogglePreampPhantom(input) => Some(Intent::TogglePreampPhantom(
                preamp_input_to_controller(*input),
            )),
            Self::PickSampleRate(rate) => Some(Intent::PickSampleRate(*rate)),
            Self::PickClockSource(source) => Some(Intent::PickClockSource(*source)),
            Self::SelectProfile(index) => Some(Intent::SelectProfile(*index)),
            Self::LoadSelectedProfile => Some(Intent::LoadSelectedProfile),
            Self::SaveProfile(_) => Some(Intent::StartSaveProfile),
            Self::RenameProfile(_) => Some(Intent::StartRenameProfile),
            Self::DeleteSelectedProfile => Some(Intent::DeleteSelectedProfile),
            Self::CaptureRawBaseline => Some(Intent::CaptureRawBaseline),
            Self::ClearRawBaseline => Some(Intent::ClearRawBaseline),
            Self::RefreshQueriedState => Some(Intent::RefreshQueriedState),
            Self::SetRefreshRate(rate) => Some(Intent::SetRefreshRate(*rate)),
            Self::CyclePeakThreshold(increase) => Some(Intent::CyclePeakThreshold(*increase)),
            Self::TogglePeakEnabled => Some(Intent::TogglePeakEnabled),
            Self::SetPeakHoldDuration(duration) => Some(Intent::CyclePeakHoldDuration(*duration)),
            Self::ToggleAutoSave => Some(Intent::ToggleAutoSave),
            Self::Shutdown => Some(Intent::Quit),
        }
    }
}

const MAX_DIRECT_MIXER_LEVEL: u8 = 0x5a;

fn channel_index(channel: i32) -> Option<usize> {
    let channel = u8::try_from(channel).ok()?;
    valid_channel(channel).then(|| usize::from(channel - 1))
}

fn valid_channel(channel: u8) -> bool {
    (1..=16).contains(&channel)
}

fn valid_preamp(input: u8) -> bool {
    (1..=2).contains(&input)
}

fn preamp_input_to_controller(input: u8) -> u8 {
    input.saturating_sub(1).min(1)
}

fn adjust_device_step(current: u8, delta: i16) -> u8 {
    (current as i16 + delta).clamp(0, 96) as u8
}

#[cfg(test)]
mod tests {
    use super::GuiCommand;
    use crate::models::GuiPage;
    use antelope_protocol::{MixerAssignment, PanState};
    use zen_go_tui::app::{Intent, PeakHoldDuration, RefreshRate};

    #[test]
    fn command_rejects_invalid_page_index() {
        assert_eq!(
            GuiCommand::set_page_from_index(0),
            Some(GuiCommand::SetPage(GuiPage::Mixer))
        );
        assert_eq!(
            GuiCommand::set_page_from_index(1),
            Some(GuiCommand::SetPage(GuiPage::Profiles))
        );
        assert_eq!(
            GuiCommand::set_page_from_index(2),
            Some(GuiCommand::SetPage(GuiPage::Raw))
        );
        assert_eq!(
            GuiCommand::set_page_from_index(3),
            Some(GuiCommand::SetPage(GuiPage::Settings))
        );
        assert_eq!(GuiCommand::set_page_from_index(4), None);
    }

    #[test]
    fn command_rejects_invalid_control_indexes() {
        assert_eq!(
            GuiCommand::set_output_level(2, 42),
            Some(GuiCommand::SetOutputLevel { index: 2, step: 42 })
        );
        assert_eq!(GuiCommand::set_output_level(3, 42), None);
        assert_eq!(GuiCommand::set_mixer_level(16, 42), None);
        assert_eq!(GuiCommand::set_preamp_gain(3, 12), None);
    }

    #[test]
    fn command_accepts_core_mixer_controls() {
        assert_eq!(
            GuiCommand::select_mixer_channel_by_channel(4),
            Some(GuiCommand::SelectMixerChannel(3))
        );
        assert_eq!(GuiCommand::select_mixer_channel_by_channel(0), None);
        assert_eq!(GuiCommand::select_mixer_channel_by_channel(17), None);
        assert_eq!(
            GuiCommand::set_mixer_pan(0, PanState::center()),
            Some(GuiCommand::SetMixerPan {
                index: 0,
                pan: PanState::center()
            })
        );
        assert_eq!(
            GuiCommand::pick_assignment(16, MixerAssignment::Mute),
            Some(GuiCommand::PickAssignment {
                strip: 16,
                assignment: MixerAssignment::Mute
            })
        );
        assert_eq!(GuiCommand::pick_assignment(17, MixerAssignment::Mute), None);
    }

    #[test]
    fn command_adjusts_dashboard_numeric_values_safely() {
        assert_eq!(
            GuiCommand::adjust_output_level(0, 95, 8),
            Some(GuiCommand::SetOutputLevel { index: 0, step: 96 })
        );
        assert_eq!(
            GuiCommand::adjust_mixer_level(0, 2, -8),
            Some(GuiCommand::SetMixerLevel { index: 0, level: 0 })
        );
        assert_eq!(
            GuiCommand::adjust_mixer_pan(0, PanState::CENTER, 99),
            Some(GuiCommand::SetMixerPan {
                index: 0,
                pan: PanState::right()
            })
        );
        assert_eq!(
            GuiCommand::adjust_preamp_gain(1, 95, 8),
            Some(GuiCommand::SetPreampGain { input: 1, raw: 96 })
        );
    }

    #[test]
    fn command_maps_preamp_mode_indexes() {
        assert_eq!(
            GuiCommand::pick_preamp_mode_from_index(2, 2)
                .unwrap()
                .to_intent(),
            Some(Intent::PickPreampMode {
                input: 1,
                mode: antelope_protocol::PreampMode::HiZ
            })
        );
        assert_eq!(GuiCommand::pick_preamp_mode_from_index(1, 3), None);
        assert_eq!(GuiCommand::pick_preamp_mode_from_index(3, 0), None);
    }

    #[test]
    fn command_maps_selector_indexes() {
        assert_eq!(
            GuiCommand::pick_assignment_from_index(4, 0),
            Some(GuiCommand::PickAssignment {
                strip: 4,
                assignment: MixerAssignment::Mute
            })
        );
        assert_eq!(GuiCommand::pick_assignment_from_index(17, 0), None);
        assert_eq!(GuiCommand::pick_assignment_from_index(4, 999), None);
        assert!(matches!(
            GuiCommand::pick_sample_rate_from_index(2),
            Some(GuiCommand::PickSampleRate(_))
        ));
        assert_eq!(GuiCommand::pick_sample_rate_from_index(99), None);
        assert!(matches!(
            GuiCommand::pick_clock_source_from_index(1),
            Some(GuiCommand::PickClockSource(_))
        ));
        assert_eq!(GuiCommand::pick_clock_source_from_index(99), None);
    }

    #[test]
    fn direct_mixer_events_validate_channels_and_ranges() {
        assert_eq!(
            GuiCommand::set_mixer_level_by_channel(1, 0),
            Some(GuiCommand::SetMixerLevel { index: 0, level: 0 })
        );
        assert_eq!(
            GuiCommand::set_mixer_level_by_channel(16, 90),
            Some(GuiCommand::SetMixerLevel {
                index: 15,
                level: 90
            })
        );
        assert_eq!(GuiCommand::set_mixer_level_by_channel(0, 20), None);
        assert_eq!(GuiCommand::set_mixer_level_by_channel(17, 20), None);
        assert_eq!(GuiCommand::set_mixer_level_by_channel(1, -1), None);
        assert_eq!(GuiCommand::set_mixer_level_by_channel(1, 91), None);

        assert_eq!(
            GuiCommand::set_mixer_pan_by_channel(1, i32::from(PanState::MIN)),
            Some(GuiCommand::SetMixerPan {
                index: 0,
                pan: PanState::from_raw(PanState::MIN)
            })
        );
        assert_eq!(
            GuiCommand::set_mixer_pan_by_channel(16, i32::from(PanState::MAX)),
            Some(GuiCommand::SetMixerPan {
                index: 15,
                pan: PanState::from_raw(PanState::MAX)
            })
        );
        assert_eq!(GuiCommand::set_mixer_pan_by_channel(1, 1), None);
        assert_eq!(GuiCommand::set_mixer_pan_by_channel(1, 0x3f), None);
        assert_eq!(GuiCommand::set_mixer_pan_by_channel(-1, 0x20), None);
    }

    #[test]
    fn link_and_assignment_events_use_device_channels() {
        assert!(GuiCommand::toggle_mixer_link(1).is_some());
        assert_eq!(GuiCommand::toggle_mixer_link(2), None);
        assert_eq!(GuiCommand::toggle_mixer_link(17), None);
        assert!(GuiCommand::toggle_mixer_link_by_channel(1).is_some());
        assert_eq!(GuiCommand::toggle_mixer_link_by_channel(2), None);
        assert_eq!(GuiCommand::toggle_mixer_link_by_channel(-1), None);

        assert!(GuiCommand::pick_assignment_from_indices(16, 0).is_some());
        assert_eq!(GuiCommand::pick_assignment_from_indices(0, 0), None);
        assert_eq!(GuiCommand::pick_assignment_from_indices(1, -1), None);
    }

    #[test]
    fn settings_commands_validate_indices_and_map_to_existing_intents() {
        assert_eq!(
            GuiCommand::set_refresh_rate_from_index(0).and_then(|command| command.to_intent()),
            Some(Intent::SetRefreshRate(RefreshRate::Fps15))
        );
        assert_eq!(GuiCommand::set_refresh_rate_from_index(3), None);
        assert_eq!(
            GuiCommand::CyclePeakThreshold(true).to_intent(),
            Some(Intent::CyclePeakThreshold(true))
        );
        assert_eq!(
            GuiCommand::TogglePeakEnabled.to_intent(),
            Some(Intent::TogglePeakEnabled)
        );
        assert_eq!(
            GuiCommand::set_peak_hold_from_index(3).and_then(|command| command.to_intent()),
            Some(Intent::CyclePeakHoldDuration(PeakHoldDuration::Sec10))
        );
        assert_eq!(GuiCommand::set_peak_hold_from_index(4), None);
    }

    #[test]
    fn command_maps_profile_raw_and_settings_intents() {
        assert_eq!(
            GuiCommand::SaveProfile(String::new()).to_intent(),
            Some(Intent::StartSaveProfile)
        );
        assert_eq!(
            GuiCommand::RenameProfile(String::new()).to_intent(),
            Some(Intent::StartRenameProfile)
        );
        assert_eq!(
            GuiCommand::CaptureRawBaseline.to_intent(),
            Some(Intent::CaptureRawBaseline)
        );
        assert_eq!(
            GuiCommand::RefreshQueriedState.to_intent(),
            Some(Intent::RefreshQueriedState)
        );
        assert_eq!(
            GuiCommand::ToggleAutoSave.to_intent(),
            Some(Intent::ToggleAutoSave)
        );
    }

    #[test]
    fn command_converts_to_existing_controller_intents() {
        assert_eq!(GuiCommand::SetPage(GuiPage::Routing).to_intent(), None);
        assert_eq!(GuiCommand::Shutdown.to_intent(), Some(Intent::Quit));
        assert_eq!(
            GuiCommand::ToggleOutputMute(1).to_intent(),
            Some(Intent::ToggleOutputMute(1))
        );
        assert_eq!(
            GuiCommand::SetPreampGain { input: 2, raw: 33 }.to_intent(),
            Some(Intent::SetPreampGain { input: 1, raw: 33 })
        );
        assert_eq!(
            GuiCommand::TogglePreampPhantom(1).to_intent(),
            Some(Intent::TogglePreampPhantom(0))
        );
    }
}
