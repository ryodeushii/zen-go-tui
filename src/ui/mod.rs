mod layouts;
mod mouse;
mod render;
mod styles;
#[cfg(test)]
mod tests;

// Re-export MouseAction enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    ToggleRawView,
    ToggleHotkeysPopup,
    OpenProfilesPopup,
    CloseProfilesPopup,
    OpenRoutingPopup,
    CloseRoutingPopup,
    SelectProfile(usize),
    LoadSelectedProfile,
    StartSaveProfile,
    StartRenameProfile,
    DeleteSelectedProfile,
    PageMixerStripsLeft,
    PageMixerStripsRight,
    OpenSampleRateSelector,
    OpenClockSourceSelector,
    SelectPage(crate::app::MainPage),
    SelectOutput(usize),
    AdjustOutputLevel {
        index: usize,
        increase: bool,
    },
    SetOutputLevel {
        index: usize,
        step: u8,
    },
    ToggleOutputDim(usize),
    ToggleOutputMute(usize),
    SelectRawPacketTab(crate::app::RawPacketTab),
    SelectQueryReplyEntry(usize),
    SelectSurface(antelope_protocol::Surface),
    SelectMixerChannel(usize),
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
    CloseAssignmentPicker,
    SelectPreampInput(usize),
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
    PickSampleRate(antelope_protocol::SampleRate),
    PickClockSource(antelope_protocol::ClockSource),
    PickPreampMode {
        input: u8,
        mode: antelope_protocol::PreampMode,
    },
    CloseSelectorPopup,
    TogglePreampPhase(u8),
    TogglePreampPhantom(u8),
}

// Re-export public API
pub use mouse::mixer_strip_panel_contains;
pub use mouse::mixer_strip_viewport_capacity;
pub use mouse::mouse_action;
pub use mouse::slider_mouse_action;
pub use mouse::slider_wheel_action;
pub use render::draw;
pub use render::profile_editor_cursor;
