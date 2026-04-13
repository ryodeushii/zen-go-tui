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
