use std::time::Duration;

use antelope_protocol::{
    ClockSource, MixerAssignment, MixerChannelState, MixerLinkTarget, MixerSurface, OutputMode,
    OutputTarget, PanState, PreampMode, SampleRate, Surface,
};

use crate::app::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    // Application lifecycle
    Quit,

    // View navigation
    ToggleRawView,
    ToggleHotkeysPopup,
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
    OpenAssignmentPicker(u8),
    PickAssignment {
        strip: u8,
        assignment: MixerAssignment,
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
    pub fn pending_mutation(&self, state: &AppState) -> Option<PendingMutation> {
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
                let active = state.mixer.channels[mixer.index()].get(idx).copied()?;
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
                let active = state.mixer.channels[mixer.index()].get(idx).copied()?;
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
                let active = state.mixer.channels[mixer.index()].get(idx).copied()?;
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
                let target = MixerLinkTarget::from_channel(mixer, *channel)?;
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
