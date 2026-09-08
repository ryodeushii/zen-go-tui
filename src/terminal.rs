use std::io::{self, stdout};
use std::sync::OnceLock;

use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::style::{Color, Modifier, Style};
use terminput::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    ScrollDirection,
};
use terminput_crossterm::to_terminput;
use termprofile::{DetectorSettings, TermProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AppModifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub super_key: bool,
    pub hyper: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKeyCode {
    Char(char),
    Tab,
    BackTab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Esc,
    F(u8),
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKeyEventKind {
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppKeyEvent {
    pub code: AppKeyCode,
    pub modifiers: AppModifiers,
    pub kind: AppKeyEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMouseButton {
    Left,
    Right,
    Middle,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMouseEventKind {
    Down(AppMouseButton),
    Up(AppMouseButton),
    Drag(AppMouseButton),
    Moved,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppMouseEvent {
    pub kind: AppMouseEventKind,
    pub column: u16,
    pub row: u16,
    pub modifiers: AppModifiers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppInputEvent {
    FocusGained,
    FocusLost,
    Key(AppKeyEvent),
    Mouse(AppMouseEvent),
    Paste(String),
    Resize { rows: u16, cols: u16 },
}

static TERM_PROFILE: OnceLock<TermProfile> = OnceLock::new();

/// Terminal operations used by [`TerminalSession`], separated for setup/cleanup tests.
pub trait TerminalControl {
    fn enable_raw_mode(&mut self) -> io::Result<()>;
    fn disable_raw_mode(&mut self) -> io::Result<()>;
    fn enter_alternate_screen(&mut self) -> io::Result<()>;
    fn leave_alternate_screen(&mut self) -> io::Result<()>;
    fn enable_mouse_capture(&mut self) -> io::Result<()>;
    fn disable_mouse_capture(&mut self) -> io::Result<()>;
    fn supports_keyboard_enhancement(&mut self) -> io::Result<bool>;
    fn push_keyboard_enhancement_flags(&mut self) -> io::Result<()>;
    fn pop_keyboard_enhancement_flags(&mut self) -> io::Result<()>;
}

#[derive(Debug, Default)]
pub struct CrosstermTerminalControl;

impl TerminalControl for CrosstermTerminalControl {
    fn enable_raw_mode(&mut self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn disable_raw_mode(&mut self) -> io::Result<()> {
        disable_raw_mode()
    }

    fn enter_alternate_screen(&mut self) -> io::Result<()> {
        stdout().execute(EnterAlternateScreen).map(|_| ())
    }

    fn leave_alternate_screen(&mut self) -> io::Result<()> {
        stdout().execute(LeaveAlternateScreen).map(|_| ())
    }

    fn enable_mouse_capture(&mut self) -> io::Result<()> {
        stdout().execute(EnableMouseCapture).map(|_| ())
    }

    fn disable_mouse_capture(&mut self) -> io::Result<()> {
        stdout().execute(DisableMouseCapture).map(|_| ())
    }

    fn supports_keyboard_enhancement(&mut self) -> io::Result<bool> {
        // Crossterm 0.29 performs the protocol query with a 2-second poll timeout.
        crossterm::terminal::supports_keyboard_enhancement()
    }

    fn push_keyboard_enhancement_flags(&mut self) -> io::Result<()> {
        stdout()
            .execute(PushKeyboardEnhancementFlags(keyboard_enhancement_flags()))
            .map(|_| ())
    }

    fn pop_keyboard_enhancement_flags(&mut self) -> io::Result<()> {
        stdout().execute(PopKeyboardEnhancementFlags).map(|_| ())
    }
}

fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
}

/// Owns terminal modes and restores every mode that setup may have changed.
pub struct TerminalSession<C: TerminalControl> {
    control: C,
    raw_mode: bool,
    alternate_screen: bool,
    mouse_capture: bool,
    keyboard_flags_pushed: bool,
    keyboard_release_events_enabled: bool,
}

impl<C: TerminalControl> TerminalSession<C> {
    pub fn setup(control: C) -> io::Result<Self> {
        let mut session = Self {
            control,
            raw_mode: false,
            alternate_screen: false,
            mouse_capture: false,
            keyboard_flags_pushed: false,
            keyboard_release_events_enabled: false,
        };

        if let Err(error) = session.setup_inner() {
            let _ = session.cleanup();
            return Err(error);
        }
        Ok(session)
    }

    fn setup_inner(&mut self) -> io::Result<()> {
        // Mark each mode before its enabling write so a partial write is still unwound.
        self.raw_mode = true;
        self.control.enable_raw_mode()?;

        self.alternate_screen = true;
        self.control.enter_alternate_screen()?;

        // Main and alternate screens have independent keyboard-protocol stacks, so negotiate
        // after entering the alternate screen and pop before leaving it during cleanup.
        // Unsupported terminals and bounded query errors keep the application usable, but
        // keyboard hold-to-talk remains disabled.
        if matches!(self.control.supports_keyboard_enhancement(), Ok(true)) {
            // A failed write can be partial, so always attempt the matching pop.
            self.keyboard_flags_pushed = true;
            if self.control.push_keyboard_enhancement_flags().is_ok() {
                self.keyboard_release_events_enabled = true;
            } else if self.control.pop_keyboard_enhancement_flags().is_ok() {
                self.keyboard_flags_pushed = false;
            }
        }

        self.mouse_capture = true;
        self.control.enable_mouse_capture()?;
        Ok(())
    }

    pub fn keyboard_release_events_enabled(&self) -> bool {
        self.keyboard_release_events_enabled
    }

    pub fn cleanup(&mut self) -> io::Result<()> {
        let mut first_error = None;
        if self.keyboard_flags_pushed {
            self.keyboard_flags_pushed = false;
            record_cleanup_result(
                &mut first_error,
                self.control.pop_keyboard_enhancement_flags(),
            );
        }
        self.keyboard_release_events_enabled = false;
        if self.mouse_capture {
            self.mouse_capture = false;
            record_cleanup_result(&mut first_error, self.control.disable_mouse_capture());
        }
        if self.alternate_screen {
            self.alternate_screen = false;
            record_cleanup_result(&mut first_error, self.control.leave_alternate_screen());
        }
        if self.raw_mode {
            self.raw_mode = false;
            record_cleanup_result(&mut first_error, self.control.disable_raw_mode());
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl<C: TerminalControl> Drop for TerminalSession<C> {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn record_cleanup_result(first_error: &mut Option<io::Error>, result: io::Result<()>) {
    if let Err(error) = result {
        if first_error.is_none() {
            *first_error = Some(error);
        }
    }
}

pub fn detect_profile() -> TermProfile {
    *TERM_PROFILE.get_or_init(|| TermProfile::detect(&stdout(), DetectorSettings::default()))
}

pub fn adapt_color_for_profile(profile: TermProfile, color: Color) -> Color {
    match profile {
        TermProfile::TrueColor => color,
        TermProfile::Ansi256 => adapt_color_ansi256(color),
        TermProfile::Ansi16 => adapt_color_ansi16(color),
        TermProfile::NoColor | TermProfile::NoTty => Color::Reset,
    }
}

pub fn adapt_color(color: Color) -> Color {
    adapt_color_for_profile(detect_profile(), color)
}

pub fn adapt_style(style: Style) -> Style {
    adapt_style_for_profile(detect_profile(), style)
}

pub fn adapt_style_for_profile(profile: TermProfile, mut style: Style) -> Style {
    style.fg = style
        .fg
        .map(|color| adapt_color_for_profile(profile, color));
    style.bg = style
        .bg
        .map(|color| adapt_color_for_profile(profile, color));
    style.underline_color = style
        .underline_color
        .map(|color| adapt_color_for_profile(profile, color));
    if profile == TermProfile::NoTty {
        style.add_modifier = Modifier::empty();
        style.sub_modifier = Modifier::empty();
    }
    style
}

pub fn read_input_event() -> Result<Option<AppInputEvent>> {
    normalize_crossterm_event(crossterm::event::read()?)
}

pub fn normalize_crossterm_event(event: crossterm::event::Event) -> Result<Option<AppInputEvent>> {
    let event = to_terminput(event).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(normalize_event(event))
}

fn normalize_event(event: Event) -> Option<AppInputEvent> {
    match event {
        Event::FocusGained => Some(AppInputEvent::FocusGained),
        Event::FocusLost => Some(AppInputEvent::FocusLost),
        Event::Key(key) => Some(AppInputEvent::Key(normalize_key(key))),
        Event::Mouse(mouse) => Some(AppInputEvent::Mouse(normalize_mouse(mouse))),
        Event::Paste(text) => Some(AppInputEvent::Paste(text)),
        Event::Resize { rows, cols } => Some(AppInputEvent::Resize {
            rows: rows.min(u16::MAX as u32) as u16,
            cols: cols.min(u16::MAX as u32) as u16,
        }),
    }
}

fn normalize_key(key: KeyEvent) -> AppKeyEvent {
    let modifiers = normalize_modifiers(key.modifiers);
    let code = match key.code {
        KeyCode::Char(ch) => AppKeyCode::Char(ch),
        KeyCode::Tab if modifiers.shift => AppKeyCode::BackTab,
        KeyCode::Tab => AppKeyCode::Tab,
        KeyCode::Backspace => AppKeyCode::Backspace,
        KeyCode::Up => AppKeyCode::Up,
        KeyCode::Down => AppKeyCode::Down,
        KeyCode::Left => AppKeyCode::Left,
        KeyCode::Right => AppKeyCode::Right,
        KeyCode::PageUp => AppKeyCode::PageUp,
        KeyCode::PageDown => AppKeyCode::PageDown,
        KeyCode::Home => AppKeyCode::Home,
        KeyCode::End => AppKeyCode::End,
        KeyCode::Enter => AppKeyCode::Enter,
        KeyCode::Esc => AppKeyCode::Esc,
        KeyCode::F(number) => AppKeyCode::F(number),
        _ => AppKeyCode::Unknown,
    };

    AppKeyEvent {
        code,
        modifiers,
        kind: match key.kind {
            KeyEventKind::Press => AppKeyEventKind::Press,
            KeyEventKind::Repeat => AppKeyEventKind::Repeat,
            KeyEventKind::Release => AppKeyEventKind::Release,
        },
    }
}

fn normalize_mouse(mouse: MouseEvent) -> AppMouseEvent {
    AppMouseEvent {
        kind: match mouse.kind {
            MouseEventKind::Down(button) => AppMouseEventKind::Down(normalize_mouse_button(button)),
            MouseEventKind::Up(button) => AppMouseEventKind::Up(normalize_mouse_button(button)),
            MouseEventKind::Drag(button) => AppMouseEventKind::Drag(normalize_mouse_button(button)),
            MouseEventKind::Moved => AppMouseEventKind::Moved,
            MouseEventKind::Scroll(direction) => match direction {
                ScrollDirection::Up => AppMouseEventKind::ScrollUp,
                ScrollDirection::Down => AppMouseEventKind::ScrollDown,
                ScrollDirection::Left => AppMouseEventKind::ScrollLeft,
                ScrollDirection::Right => AppMouseEventKind::ScrollRight,
            },
        },
        column: mouse.column,
        row: mouse.row,
        modifiers: normalize_modifiers(mouse.modifiers),
    }
}

fn normalize_mouse_button(button: MouseButton) -> AppMouseButton {
    match button {
        MouseButton::Left => AppMouseButton::Left,
        MouseButton::Right => AppMouseButton::Right,
        MouseButton::Middle => AppMouseButton::Middle,
        MouseButton::Unknown => AppMouseButton::Unknown,
    }
}

fn normalize_modifiers(modifiers: KeyModifiers) -> AppModifiers {
    AppModifiers {
        shift: modifiers.contains(KeyModifiers::SHIFT),
        alt: modifiers.contains(KeyModifiers::ALT),
        ctrl: modifiers.contains(KeyModifiers::CTRL),
        super_key: modifiers.contains(KeyModifiers::SUPER),
        hyper: modifiers.contains(KeyModifiers::HYPER),
        meta: modifiers.contains(KeyModifiers::META),
    }
}

fn adapt_color_ansi256(color: Color) -> Color {
    match color {
        Color::Reset
        | Color::Black
        | Color::Red
        | Color::Green
        | Color::Yellow
        | Color::Blue
        | Color::Magenta
        | Color::Cyan
        | Color::Gray
        | Color::DarkGray
        | Color::LightRed
        | Color::LightGreen
        | Color::LightYellow
        | Color::LightBlue
        | Color::LightMagenta
        | Color::LightCyan
        | Color::White
        | Color::Indexed(_) => color,
        Color::Rgb(r, g, b) => Color::Indexed(nearest_ansi256_index(r, g, b)),
    }
}

fn adapt_color_ansi16(color: Color) -> Color {
    let (r, g, b) = color_rgb(color);
    match nearest_ansi16_index(r, g, b) {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        _ => Color::White,
    }
}

fn nearest_ansi16_index(r: u8, g: u8, b: u8) -> u8 {
    ANSI16_PALETTE
        .iter()
        .enumerate()
        .min_by_key(|(_, candidate)| color_distance_sq((r, g, b), **candidate))
        .map(|(index, _)| index as u8)
        .unwrap_or(7)
}

fn nearest_ansi256_index(r: u8, g: u8, b: u8) -> u8 {
    xterm_256_palette()
        .iter()
        .enumerate()
        .min_by_key(|(_, candidate)| color_distance_sq((r, g, b), **candidate))
        .map(|(index, _)| index as u8)
        .unwrap_or(15)
}

fn xterm_256_palette() -> [(u8, u8, u8); 256] {
    let mut palette = [(0, 0, 0); 256];
    for (index, color) in ANSI16_PALETTE.iter().enumerate() {
        palette[index] = *color;
    }

    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let mut index = 16;
    for red in STEPS {
        for green in STEPS {
            for blue in STEPS {
                palette[index] = (red, green, blue);
                index += 1;
            }
        }
    }

    for gray_index in 0..24 {
        let value = 8 + gray_index * 10;
        palette[232 + gray_index as usize] = (value, value, value);
    }

    palette
}

fn color_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Reset => (255, 255, 255),
        Color::Black => ANSI16_PALETTE[0],
        Color::Red => ANSI16_PALETTE[1],
        Color::Green => ANSI16_PALETTE[2],
        Color::Yellow => ANSI16_PALETTE[3],
        Color::Blue => ANSI16_PALETTE[4],
        Color::Magenta => ANSI16_PALETTE[5],
        Color::Cyan => ANSI16_PALETTE[6],
        Color::Gray => ANSI16_PALETTE[7],
        Color::DarkGray => ANSI16_PALETTE[8],
        Color::LightRed => ANSI16_PALETTE[9],
        Color::LightGreen => ANSI16_PALETTE[10],
        Color::LightYellow => ANSI16_PALETTE[11],
        Color::LightBlue => ANSI16_PALETTE[12],
        Color::LightMagenta => ANSI16_PALETTE[13],
        Color::LightCyan => ANSI16_PALETTE[14],
        Color::White => ANSI16_PALETTE[15],
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(index) => xterm_256_palette()[index as usize],
    }
}

fn color_distance_sq(lhs: (u8, u8, u8), rhs: (u8, u8, u8)) -> u32 {
    let dr = lhs.0 as i32 - rhs.0 as i32;
    let dg = lhs.1 as i32 - rhs.1 as i32;
    let db = lhs.2 as i32 - rhs.2 as i32;
    (dr * dr + dg * dg + db * db) as u32
}

const ANSI16_PALETTE: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0xaa, 0x00, 0x00),
    (0x00, 0xaa, 0x00),
    (0xaa, 0x55, 0x00),
    (0x00, 0x00, 0xaa),
    (0xaa, 0x00, 0xaa),
    (0x00, 0xaa, 0xaa),
    (0xaa, 0xaa, 0xaa),
    (0x55, 0x55, 0x55),
    (0xff, 0x55, 0x55),
    (0x55, 0xff, 0x55),
    (0xff, 0xff, 0x55),
    (0x55, 0x55, 0xff),
    (0xff, 0x55, 0xff),
    (0x55, 0xff, 0xff),
    (0xff, 0xff, 0xff),
];

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::Color;
    use termprofile::TermProfile;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TerminalOperation {
        EnableRaw,
        DisableRaw,
        EnterAlternate,
        LeaveAlternate,
        EnableMouse,
        DisableMouse,
        QueryKeyboard,
        PushKeyboard,
        PopKeyboard,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Screen {
        Main,
        Alternate,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum KeyboardSetting {
        PreexistingMain,
        DefaultAlternate,
        Application,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct MockTerminalState {
        operations: Vec<TerminalOperation>,
        active_screen: Screen,
        main_keyboard_stack: Vec<KeyboardSetting>,
        alternate_keyboard_stack: Vec<KeyboardSetting>,
        raw_mode: bool,
        mouse_capture: bool,
    }

    impl Default for MockTerminalState {
        fn default() -> Self {
            Self {
                operations: Vec::new(),
                active_screen: Screen::Main,
                main_keyboard_stack: vec![KeyboardSetting::PreexistingMain],
                alternate_keyboard_stack: vec![KeyboardSetting::DefaultAlternate],
                raw_mode: false,
                mouse_capture: false,
            }
        }
    }

    impl MockTerminalState {
        fn active_keyboard_stack(&mut self) -> &mut Vec<KeyboardSetting> {
            match self.active_screen {
                Screen::Main => &mut self.main_keyboard_stack,
                Screen::Alternate => &mut self.alternate_keyboard_stack,
            }
        }
    }

    fn terminal_state(state: &Arc<Mutex<MockTerminalState>>) -> MockTerminalState {
        state.lock().unwrap().clone()
    }

    #[derive(Clone)]
    struct MockTerminalControl {
        state: Arc<Mutex<MockTerminalState>>,
        query_result: Result<bool, io::ErrorKind>,
        fail_operation: Option<TerminalOperation>,
    }

    impl MockTerminalControl {
        fn new(query_result: Result<bool, io::ErrorKind>) -> Self {
            Self {
                state: Arc::new(Mutex::new(MockTerminalState::default())),
                query_result,
                fail_operation: None,
            }
        }

        fn fail_on(mut self, operation: TerminalOperation) -> Self {
            self.fail_operation = Some(operation);
            self
        }

        fn run(&self, operation: TerminalOperation) -> io::Result<()> {
            let mut state = self.state.lock().unwrap();
            state.operations.push(operation);
            match operation {
                TerminalOperation::EnableRaw => state.raw_mode = true,
                TerminalOperation::DisableRaw => state.raw_mode = false,
                TerminalOperation::EnterAlternate => state.active_screen = Screen::Alternate,
                TerminalOperation::LeaveAlternate => state.active_screen = Screen::Main,
                TerminalOperation::EnableMouse => state.mouse_capture = true,
                TerminalOperation::DisableMouse => state.mouse_capture = false,
                TerminalOperation::PushKeyboard => state
                    .active_keyboard_stack()
                    .push(KeyboardSetting::Application),
                TerminalOperation::PopKeyboard => {
                    let stack = state.active_keyboard_stack();
                    if stack.len() > 1 {
                        stack.pop();
                    }
                }
                TerminalOperation::QueryKeyboard => {}
            }
            if self.fail_operation == Some(operation) {
                Err(io::Error::other(
                    "mock terminal failure after partial write",
                ))
            } else {
                Ok(())
            }
        }
    }

    impl TerminalControl for MockTerminalControl {
        fn enable_raw_mode(&mut self) -> io::Result<()> {
            self.run(TerminalOperation::EnableRaw)
        }

        fn disable_raw_mode(&mut self) -> io::Result<()> {
            self.run(TerminalOperation::DisableRaw)
        }

        fn enter_alternate_screen(&mut self) -> io::Result<()> {
            self.run(TerminalOperation::EnterAlternate)
        }

        fn leave_alternate_screen(&mut self) -> io::Result<()> {
            self.run(TerminalOperation::LeaveAlternate)
        }

        fn enable_mouse_capture(&mut self) -> io::Result<()> {
            self.run(TerminalOperation::EnableMouse)
        }

        fn disable_mouse_capture(&mut self) -> io::Result<()> {
            self.run(TerminalOperation::DisableMouse)
        }

        fn supports_keyboard_enhancement(&mut self) -> io::Result<bool> {
            self.run(TerminalOperation::QueryKeyboard)?;
            self.query_result
                .map_err(|kind| io::Error::new(kind, "mock query failure"))
        }

        fn push_keyboard_enhancement_flags(&mut self) -> io::Result<()> {
            self.run(TerminalOperation::PushKeyboard)
        }

        fn pop_keyboard_enhancement_flags(&mut self) -> io::Result<()> {
            self.run(TerminalOperation::PopKeyboard)
        }
    }

    #[test]
    fn keyboard_setup_requests_release_events_for_plain_text_and_control_keys() {
        assert_eq!(
            keyboard_enhancement_flags(),
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        );
    }

    #[test]
    fn keyboard_flags_use_alternate_stack_and_restore_preexisting_main_settings() {
        let control = MockTerminalControl::new(Ok(true));
        let state = control.state.clone();
        let mut session = TerminalSession::setup(control).expect("terminal setup");
        assert!(session.keyboard_release_events_enabled());
        let active_state = terminal_state(&state);
        assert_eq!(active_state.active_screen, Screen::Alternate);
        assert_eq!(
            active_state.main_keyboard_stack,
            vec![KeyboardSetting::PreexistingMain]
        );
        assert_eq!(
            active_state.alternate_keyboard_stack,
            vec![
                KeyboardSetting::DefaultAlternate,
                KeyboardSetting::Application
            ]
        );

        session.cleanup().expect("terminal cleanup");
        let state = terminal_state(&state);
        assert_eq!(
            state.operations,
            vec![
                TerminalOperation::EnableRaw,
                TerminalOperation::EnterAlternate,
                TerminalOperation::QueryKeyboard,
                TerminalOperation::PushKeyboard,
                TerminalOperation::EnableMouse,
                TerminalOperation::PopKeyboard,
                TerminalOperation::DisableMouse,
                TerminalOperation::LeaveAlternate,
                TerminalOperation::DisableRaw,
            ]
        );
        assert_eq!(state.active_screen, Screen::Main);
        assert_eq!(
            state.main_keyboard_stack,
            vec![KeyboardSetting::PreexistingMain]
        );
        assert_eq!(
            state.alternate_keyboard_stack,
            vec![KeyboardSetting::DefaultAlternate]
        );
        assert!(!state.raw_mode);
        assert!(!state.mouse_capture);
    }

    #[test]
    fn unsupported_or_failed_keyboard_query_keeps_terminal_available_without_push() {
        for query_result in [Ok(false), Err(io::ErrorKind::TimedOut)] {
            let control = MockTerminalControl::new(query_result);
            let state = control.state.clone();
            let mut session = TerminalSession::setup(control).expect("fallback setup");
            assert!(!session.keyboard_release_events_enabled());
            session.cleanup().expect("fallback cleanup");
            let state = state.lock().unwrap();
            assert!(!state.operations.contains(&TerminalOperation::PushKeyboard));
            assert!(!state.operations.contains(&TerminalOperation::PopKeyboard));
            assert!(state.operations.contains(&TerminalOperation::EnableMouse));
            assert_eq!(
                state.main_keyboard_stack,
                vec![KeyboardSetting::PreexistingMain]
            );
        }
    }

    #[test]
    fn partially_failed_keyboard_push_is_popped_on_alternate_stack() {
        let control = MockTerminalControl::new(Ok(true)).fail_on(TerminalOperation::PushKeyboard);
        let state = control.state.clone();
        let mut session = TerminalSession::setup(control).expect("fallback setup");
        assert!(!session.keyboard_release_events_enabled());
        let active_state = terminal_state(&state);
        assert_eq!(active_state.active_screen, Screen::Alternate);
        assert_eq!(
            active_state.alternate_keyboard_stack,
            vec![KeyboardSetting::DefaultAlternate]
        );
        assert_eq!(
            active_state.main_keyboard_stack,
            vec![KeyboardSetting::PreexistingMain]
        );
        session.cleanup().expect("fallback cleanup");
        let state = terminal_state(&state);
        assert_eq!(
            state.operations,
            vec![
                TerminalOperation::EnableRaw,
                TerminalOperation::EnterAlternate,
                TerminalOperation::QueryKeyboard,
                TerminalOperation::PushKeyboard,
                TerminalOperation::PopKeyboard,
                TerminalOperation::EnableMouse,
                TerminalOperation::DisableMouse,
                TerminalOperation::LeaveAlternate,
                TerminalOperation::DisableRaw,
            ]
        );
    }

    #[test]
    fn mouse_enable_failure_pops_flags_before_leaving_alternate_screen() {
        let control = MockTerminalControl::new(Ok(true)).fail_on(TerminalOperation::EnableMouse);
        let state = control.state.clone();
        assert!(TerminalSession::setup(control).is_err());
        let state = state.lock().unwrap();
        assert_eq!(
            state.operations,
            vec![
                TerminalOperation::EnableRaw,
                TerminalOperation::EnterAlternate,
                TerminalOperation::QueryKeyboard,
                TerminalOperation::PushKeyboard,
                TerminalOperation::EnableMouse,
                TerminalOperation::PopKeyboard,
                TerminalOperation::DisableMouse,
                TerminalOperation::LeaveAlternate,
                TerminalOperation::DisableRaw,
            ]
        );
        assert_eq!(state.active_screen, Screen::Main);
        assert_eq!(
            state.main_keyboard_stack,
            vec![KeyboardSetting::PreexistingMain]
        );
        assert_eq!(
            state.alternate_keyboard_stack,
            vec![KeyboardSetting::DefaultAlternate]
        );
        assert!(!state.raw_mode);
        assert!(!state.mouse_capture);
    }

    #[test]
    fn partial_alternate_screen_entry_failure_restores_raw_mode_without_keyboard_negotiation() {
        let control = MockTerminalControl::new(Ok(true)).fail_on(TerminalOperation::EnterAlternate);
        let state = control.state.clone();
        assert!(TerminalSession::setup(control).is_err());
        let state = state.lock().unwrap();
        assert_eq!(
            state.operations,
            vec![
                TerminalOperation::EnableRaw,
                TerminalOperation::EnterAlternate,
                TerminalOperation::LeaveAlternate,
                TerminalOperation::DisableRaw,
            ]
        );
        assert_eq!(state.active_screen, Screen::Main);
        assert_eq!(
            state.main_keyboard_stack,
            vec![KeyboardSetting::PreexistingMain]
        );
        assert_eq!(
            state.alternate_keyboard_stack,
            vec![KeyboardSetting::DefaultAlternate]
        );
        assert!(!state.raw_mode);
    }

    #[test]
    fn drop_pops_keyboard_flags_before_leaving_alternate_screen() {
        let control = MockTerminalControl::new(Ok(true));
        let state = control.state.clone();
        {
            let session = TerminalSession::setup(control).expect("terminal setup");
            assert!(session.keyboard_release_events_enabled());
        }

        let state = state.lock().unwrap();
        assert_eq!(
            state.operations,
            vec![
                TerminalOperation::EnableRaw,
                TerminalOperation::EnterAlternate,
                TerminalOperation::QueryKeyboard,
                TerminalOperation::PushKeyboard,
                TerminalOperation::EnableMouse,
                TerminalOperation::PopKeyboard,
                TerminalOperation::DisableMouse,
                TerminalOperation::LeaveAlternate,
                TerminalOperation::DisableRaw,
            ]
        );
        assert_eq!(state.active_screen, Screen::Main);
        assert_eq!(
            state.main_keyboard_stack,
            vec![KeyboardSetting::PreexistingMain]
        );
        assert_eq!(
            state.alternate_keyboard_stack,
            vec![KeyboardSetting::DefaultAlternate]
        );
        assert!(!state.raw_mode);
        assert!(!state.mouse_capture);
    }

    #[test]
    fn theme_adapts_ratatui_truecolor_to_detected_profile() {
        assert_eq!(
            adapt_color_for_profile(TermProfile::Ansi256, Color::Rgb(209, 234, 213)),
            Color::Indexed(253)
        );
    }

    #[test]
    fn normalize_crossterm_event_maps_tab_press() {
        let event = CrosstermEvent::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(
            normalize_crossterm_event(event).expect("normalize"),
            Some(AppInputEvent::Key(AppKeyEvent {
                code: AppKeyCode::Tab,
                modifiers: AppModifiers::default(),
                kind: AppKeyEventKind::Press,
            }))
        );
    }

    #[test]
    fn normalize_key_maps_page_navigation_keys() {
        assert_eq!(
            normalize_key(terminput::KeyEvent::new(terminput::KeyCode::PageUp)),
            AppKeyEvent {
                code: AppKeyCode::PageUp,
                modifiers: AppModifiers::default(),
                kind: AppKeyEventKind::Press,
            }
        );
        assert_eq!(
            normalize_key(terminput::KeyEvent::new(terminput::KeyCode::PageDown)),
            AppKeyEvent {
                code: AppKeyCode::PageDown,
                modifiers: AppModifiers::default(),
                kind: AppKeyEventKind::Press,
            }
        );
        assert_eq!(
            normalize_key(terminput::KeyEvent::new(terminput::KeyCode::Home)),
            AppKeyEvent {
                code: AppKeyCode::Home,
                modifiers: AppModifiers::default(),
                kind: AppKeyEventKind::Press,
            }
        );
        assert_eq!(
            normalize_key(terminput::KeyEvent::new(terminput::KeyCode::End)),
            AppKeyEvent {
                code: AppKeyCode::End,
                modifiers: AppModifiers::default(),
                kind: AppKeyEventKind::Press,
            }
        );
    }

    #[test]
    fn normalize_crossterm_event_maps_f2() {
        let f2 = CrosstermEvent::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));

        assert_eq!(
            normalize_crossterm_event(f2).expect("normalize F2"),
            Some(AppInputEvent::Key(AppKeyEvent {
                code: AppKeyCode::F(2),
                modifiers: AppModifiers::default(),
                kind: AppKeyEventKind::Press,
            }))
        );
    }

    #[test]
    fn normalize_crossterm_event_maps_popup_navigation_keys() {
        let up = CrosstermEvent::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let down = CrosstermEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let enter = CrosstermEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(
            normalize_crossterm_event(up).expect("normalize up"),
            Some(AppInputEvent::Key(AppKeyEvent {
                code: AppKeyCode::Up,
                modifiers: AppModifiers::default(),
                kind: AppKeyEventKind::Press,
            }))
        );
        assert_eq!(
            normalize_crossterm_event(down).expect("normalize down"),
            Some(AppInputEvent::Key(AppKeyEvent {
                code: AppKeyCode::Down,
                modifiers: AppModifiers::default(),
                kind: AppKeyEventKind::Press,
            }))
        );
        assert_eq!(
            normalize_crossterm_event(enter).expect("normalize enter"),
            Some(AppInputEvent::Key(AppKeyEvent {
                code: AppKeyCode::Enter,
                modifiers: AppModifiers::default(),
                kind: AppKeyEventKind::Press,
            }))
        );
    }
}
