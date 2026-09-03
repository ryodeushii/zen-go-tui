use std::time::Duration;

use antelope_protocol::{
    ClockSource, DynamicInputState, DynamicMixerStrip, DynamicOutputState, DynamicRoutingGroup,
    InputAddress, MixerAddress, MixerAssignment, PanState, PreampMode, SampleRate, Surface,
};

use crate::app::AppState;

#[cfg(test)]
use antelope_protocol::{MixerSurface, OutputMode, OutputTarget};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    // Application lifecycle
    Quit,

    // View navigation
    ToggleRawView,
    ToggleHotkeysPopup,
    SelectRawPacketTab(RawPacketTab),
    SelectRawMapScope(RawMapScope),
    CycleRawMapScope {
        forward: bool,
    },
    ScrollRawDump {
        increase: bool,
        page: bool,
    },

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
    SelectMixerSurface {
        surface: u8,
    },
    SelectSurface(Surface),
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
        pan: PanState,
    },
    ToggleMixerMute(u8),
    ToggleMixerSolo(u8),
    ToggleMixerLink(u8),
    /// Compatibility path for callers that already provide one-based strip numbers.
    OpenAssignmentPicker(u8),
    OpenAssignmentPickerAt {
        address: MixerAddress,
    },
    PickAssignment {
        strip: u8,
        assignment: MixerAssignment,
    },
    PickAssignmentAt {
        address: MixerAddress,
        assignment: MixerAssignment,
    },
    AdjustMixerLevelAt {
        address: MixerAddress,
        increase: bool,
    },
    SetMixerLevelAt {
        address: MixerAddress,
        level: u8,
    },
    AdjustMixerPanAt {
        address: MixerAddress,
        right: bool,
    },
    SetMixerPanAt {
        address: MixerAddress,
        pan: PanState,
    },
    SetMixerSendAt {
        address: MixerAddress,
        send: i32,
    },
    ToggleMixerMuteAt {
        address: MixerAddress,
    },
    ToggleMixerSoloAt {
        address: MixerAddress,
    },
    ToggleMixerLinkAt {
        address: MixerAddress,
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
        mode: PreampMode,
    },
    TogglePreampPhase(u8),
    TogglePreampPhantom(u8),
    AdjustInputGainAt {
        address: InputAddress,
        increase: bool,
    },
    SetInputGainAt {
        address: InputAddress,
        raw: i32,
    },
    AdjustInputParameterAt {
        address: InputAddress,
        parameter_id: u16,
        increase: bool,
    },
    SetInputParameterAt {
        address: InputAddress,
        parameter_id: u16,
        value: i32,
    },
    CycleInputModeAt {
        address: InputAddress,
    },
    SetInputModeAt {
        address: InputAddress,
        mode: PreampMode,
    },
    ToggleInputPhaseAt {
        address: InputAddress,
    },
    ToggleInputPhantomAt {
        address: InputAddress,
    },

    // Selector popups
    OpenSampleRateSelector,
    OpenClockSourceSelector,
    PickSampleRate(SampleRate),
    PickClockSource(ClockSource),

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
    pub fn writes_hardware(&self) -> bool {
        matches!(
            self,
            Self::SelectSurface(_)
                | Self::AdjustOutputLevel { .. }
                | Self::SetOutputLevel { .. }
                | Self::ToggleOutputMute(_)
                | Self::ToggleOutputDim(_)
                | Self::AdjustMixerLevel { .. }
                | Self::SetMixerLevel { .. }
                | Self::AdjustMixerPan { .. }
                | Self::SetMixerPan { .. }
                | Self::ToggleMixerMute(_)
                | Self::ToggleMixerSolo(_)
                | Self::ToggleMixerLink(_)
                | Self::PickAssignment { .. }
                | Self::AdjustMixerLevelAt { .. }
                | Self::SetMixerLevelAt { .. }
                | Self::AdjustMixerPanAt { .. }
                | Self::SetMixerPanAt { .. }
                | Self::SetMixerSendAt { .. }
                | Self::ToggleMixerMuteAt { .. }
                | Self::ToggleMixerSoloAt { .. }
                | Self::ToggleMixerLinkAt { .. }
                | Self::AdjustPreampGain { .. }
                | Self::SetPreampGain { .. }
                | Self::CyclePreampMode(_)
                | Self::PickPreampMode { .. }
                | Self::TogglePreampPhase(_)
                | Self::TogglePreampPhantom(_)
                | Self::AdjustInputGainAt { .. }
                | Self::SetInputGainAt { .. }
                | Self::AdjustInputParameterAt { .. }
                | Self::SetInputParameterAt { .. }
                | Self::CycleInputModeAt { .. }
                | Self::SetInputModeAt { .. }
                | Self::ToggleInputPhaseAt { .. }
                | Self::ToggleInputPhantomAt { .. }
                | Self::PickSampleRate(_)
                | Self::PickClockSource(_)
                | Self::AdjustFocused(_)
                | Self::ToggleFocusedMute
                | Self::ToggleFocusedDim
        )
    }

    /// Resolve legacy UI indexes through profile-owned collections before creating
    /// a hardware mutation. Unavailable indexes produce no mutation.
    pub fn pending_mutation(&self, state: &AppState) -> Option<PendingMutation> {
        match self {
            Intent::SetOutputLevel { index, step } => {
                let mut output = state.outputs().get(*index)?.clone();
                output.level = Some(i32::from(*step));
                Some(PendingMutation::Output(output))
            }
            Intent::ToggleOutputMute(index) => {
                let mut output = state.outputs().get(*index)?.clone();
                output.muted = Some(!output.muted.unwrap_or(false));
                if output.muted == Some(true) {
                    output.dimmed = Some(false);
                }
                Some(PendingMutation::Output(output))
            }
            Intent::ToggleOutputDim(index) => {
                let mut output = state.outputs().get(*index)?.clone();
                output.dimmed = Some(!output.dimmed.unwrap_or(false));
                if output.dimmed == Some(true) {
                    output.muted = Some(false);
                }
                Some(PendingMutation::Output(output))
            }
            Intent::SetMixerLevel { index, level } => {
                pending_mixer_change(state, *index, |strip| {
                    strip.fader = Some(i32::from(*level));
                })
            }
            Intent::SetMixerLevelAt { address, level } => {
                pending_mixer_change_at(state, *address, |strip| {
                    strip.fader = Some(i32::from(*level));
                })
            }
            Intent::SetMixerPanAt { address, pan } => {
                pending_mixer_change_at(state, *address, |strip| {
                    strip.pan = Some(i32::from(pan.raw()));
                })
            }
            Intent::SetMixerSendAt { address, send } => {
                pending_mixer_change_at(state, *address, |strip| strip.send = Some(*send))
            }
            Intent::ToggleMixerMuteAt { address } => {
                pending_mixer_change_at(state, *address, |strip| {
                    strip.muted = Some(!strip.muted.unwrap_or(false));
                })
            }
            Intent::ToggleMixerSoloAt { address } => {
                pending_mixer_change_at(state, *address, |strip| {
                    strip.soloed = Some(!strip.soloed.unwrap_or(false));
                })
            }
            Intent::SetMixerPan { index, pan } => pending_mixer_change(state, *index, |strip| {
                strip.pan = Some(i32::from(pan.raw()));
            }),
            Intent::ToggleMixerMute(channel) => {
                let index = usize::from(channel.checked_sub(1)?);
                pending_mixer_change(state, index, |strip| {
                    strip.muted = Some(!strip.muted.unwrap_or(false));
                })
            }
            Intent::ToggleMixerSolo(channel) => {
                let index = usize::from(channel.checked_sub(1)?);
                pending_mixer_change(state, index, |strip| {
                    strip.soloed = Some(!strip.soloed.unwrap_or(false));
                })
            }
            Intent::ToggleMixerLink(channel) => {
                let index = usize::from(channel.checked_sub(1)?);
                pending_mixer_change(state, index, |strip| {
                    strip.linked = Some(!strip.linked.unwrap_or(false));
                })
            }
            Intent::SetPreampGain { input, raw } => pending_input_change(state, *input, |slot| {
                slot.gain = Some(i32::from(*raw));
            }),
            Intent::SetInputGainAt { address, raw } => {
                pending_input_change_at(state, *address, |slot| slot.gain = Some(*raw))
            }
            Intent::SetInputParameterAt { address, value, .. } => {
                pending_input_change_at(state, *address, |slot| slot.gain = Some(*value))
            }
            Intent::SetInputModeAt { address, mode } => {
                pending_input_change_at(state, *address, |slot| {
                    slot.mode = Some(i32::from(mode.code()))
                })
            }
            Intent::ToggleInputPhantomAt { address } => {
                pending_input_change_at(state, *address, |slot| {
                    slot.phantom = Some(!slot.phantom.unwrap_or(false));
                })
            }
            Intent::ToggleInputPhaseAt { address } => {
                pending_input_change_at(state, *address, |slot| {
                    slot.phase = Some(!slot.phase.unwrap_or(false));
                })
            }
            Intent::PickPreampMode { input, mode } => pending_input_change(state, *input, |slot| {
                slot.mode = Some(i32::from(mode.code()));
            }),
            Intent::TogglePreampPhantom(input) => pending_input_change(state, *input, |slot| {
                slot.phantom = Some(!slot.phantom.unwrap_or(false));
            }),
            Intent::TogglePreampPhase(input) => pending_input_change(state, *input, |slot| {
                slot.phase = Some(!slot.phase.unwrap_or(false));
            }),
            _ => None,
        }
    }
}

fn pending_mixer_change<F>(state: &AppState, index: usize, mutate: F) -> Option<PendingMutation>
where
    F: Fn(&mut DynamicMixerStrip),
{
    let surface_index = state.active_mixer_surface()?;
    let surface = state.mixers().get(surface_index)?;
    let active = surface.strips.get(index)?;
    let mut strips = Vec::new();
    if active.linked == Some(true) {
        let left_index = index - (index % 2);
        for pair_index in [left_index, left_index.checked_add(1)?] {
            let mut strip = surface.strips.get(pair_index)?.clone();
            mutate(&mut strip);
            strips.push(PendingMixerStrip {
                address: MixerAddress {
                    surface: surface.surface,
                    strip: strip.strip,
                },
                strip,
            });
        }
    } else {
        let mut strip = active.clone();
        mutate(&mut strip);
        strips.push(PendingMixerStrip {
            address: MixerAddress {
                surface: surface.surface,
                strip: strip.strip,
            },
            strip,
        });
    }
    Some(PendingMutation::Mixer(strips))
}

fn pending_mixer_change_at<F>(
    state: &AppState,
    address: MixerAddress,
    mutate: F,
) -> Option<PendingMutation>
where
    F: Fn(&mut DynamicMixerStrip),
{
    let surface = state
        .mixers()
        .iter()
        .find(|surface| surface.surface == address.surface)?;
    let mut strip = if address.strip == 0 {
        surface.master.as_ref()?.clone()
    } else {
        surface
            .strips
            .iter()
            .find(|strip| strip.strip == address.strip)?
            .clone()
    };
    mutate(&mut strip);
    Some(PendingMutation::Mixer(vec![PendingMixerStrip {
        address,
        strip,
    }]))
}

fn pending_input_change_at<F>(
    state: &AppState,
    address: InputAddress,
    mutate: F,
) -> Option<PendingMutation>
where
    F: FnOnce(&mut DynamicInputState),
{
    let mut slot = state
        .input_spaces
        .iter()
        .find(|space| space.space_id == address.space)?
        .inputs
        .iter()
        .find(|input| input.address == address)?
        .clone();
    mutate(&mut slot);
    Some(PendingMutation::Input(slot))
}

fn pending_input_change<F>(state: &AppState, input: u8, mutate: F) -> Option<PendingMutation>
where
    F: FnOnce(&mut DynamicInputState),
{
    let mut slot = state
        .input_spaces
        .first()?
        .inputs
        .get(usize::from(input))?
        .clone();
    mutate(&mut slot);
    Some(PendingMutation::Input(slot))
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
pub enum RawMapScope {
    All,
    Base,
    Outputs,
    Preamps,
    Mixer,
    Query,
    Metadata,
    Status,
    Parser,
    Unmapped,
}

impl RawMapScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Base => "BASE",
            Self::Outputs => "OUTPUTS",
            Self::Preamps => "PREAMPS",
            Self::Mixer => "MIXER",
            Self::Query => "QUERY",
            Self::Metadata => "METADATA",
            Self::Status => "STATUS",
            Self::Parser => "PARSER",
            Self::Unmapped => "UNMAPPED",
        }
    }

    pub fn options_for(tab: RawPacketTab) -> &'static [Self] {
        const STATE73: &[RawMapScope] = &[
            RawMapScope::All,
            RawMapScope::Base,
            RawMapScope::Outputs,
            RawMapScope::Preamps,
            RawMapScope::Mixer,
            RawMapScope::Unmapped,
        ];
        const QUERY74: &[RawMapScope] = &[
            RawMapScope::All,
            RawMapScope::Query,
            RawMapScope::Mixer,
            RawMapScope::Unmapped,
        ];
        const QUERY75: &[RawMapScope] = &[
            RawMapScope::All,
            RawMapScope::Metadata,
            RawMapScope::Mixer,
            RawMapScope::Status,
            RawMapScope::Unmapped,
        ];
        const PARSER_ONLY: &[RawMapScope] =
            &[RawMapScope::All, RawMapScope::Parser, RawMapScope::Unmapped];

        match tab {
            RawPacketTab::State73 => STATE73,
            RawPacketTab::Query74 => QUERY74,
            RawPacketTab::Query75 => QUERY75,
            RawPacketTab::Auxiliary | RawPacketTab::DeviceNotification => PARSER_ONLY,
        }
    }

    pub fn next_for(self, tab: RawPacketTab, forward: bool) -> Self {
        let scopes = Self::options_for(tab);
        let index = scopes.iter().position(|scope| *scope == self).unwrap_or(0);
        let next = if forward {
            (index + 1) % scopes.len()
        } else {
            index.checked_sub(1).unwrap_or(scopes.len() - 1)
        };
        scopes[next]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_scope_options_match_packet_kind() {
        assert_eq!(
            RawMapScope::options_for(RawPacketTab::State73),
            &[
                RawMapScope::All,
                RawMapScope::Base,
                RawMapScope::Outputs,
                RawMapScope::Preamps,
                RawMapScope::Mixer,
                RawMapScope::Unmapped,
            ]
        );
        assert_eq!(
            RawMapScope::options_for(RawPacketTab::Query75),
            &[
                RawMapScope::All,
                RawMapScope::Metadata,
                RawMapScope::Mixer,
                RawMapScope::Status,
                RawMapScope::Unmapped,
            ]
        );
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

#[derive(Debug, Clone)]
pub struct PendingMixerStrip {
    pub address: MixerAddress,
    pub strip: DynamicMixerStrip,
}

#[derive(Debug, Clone)]
pub enum PendingMutation {
    Mixer(Vec<PendingMixerStrip>),
    Output(DynamicOutputState),
    Input(DynamicInputState),
    Routing(DynamicRoutingGroup),
    #[cfg(test)]
    MixerLevel {
        mixer: MixerSurface,
        channel: u8,
        level: u8,
        pan: PanState,
        muted: bool,
    },
    #[cfg(test)]
    MixerMute {
        mixer: MixerSurface,
        channel: u8,
        muted: bool,
    },
    #[cfg(test)]
    MixerPan {
        mixer: MixerSurface,
        channel: u8,
        pan: PanState,
    },
    #[cfg(test)]
    MixerAssignment {
        strip: u8,
        assignment: MixerAssignment,
    },
    #[cfg(test)]
    MixerLink {
        mixer: MixerSurface,
        selector: u8,
        enabled: bool,
    },
    #[cfg(test)]
    OutputVolume {
        target: OutputTarget,
        step: u8,
    },
    #[cfg(test)]
    OutputMode {
        target: OutputTarget,
        mode: OutputMode,
    },
    #[cfg(test)]
    PreampGain {
        input: u8,
        raw: u8,
    },
    #[cfg(test)]
    PreampMode {
        input: u8,
        mode: PreampMode,
    },
    #[cfg(test)]
    PreampPhantom {
        input: u8,
        enabled: bool,
    },
    #[cfg(test)]
    PreampPhase {
        input: u8,
        enabled: bool,
    },
}
