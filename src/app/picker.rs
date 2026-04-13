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
