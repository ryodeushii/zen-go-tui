use std::io::stdout;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;
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
    Left,
    Right,
    Esc,
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

pub fn poll_input(timeout: Duration) -> Result<bool> {
    Ok(crossterm::event::poll(timeout)?)
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
        KeyCode::Left => AppKeyCode::Left,
        KeyCode::Right => AppKeyCode::Right,
        KeyCode::Esc => AppKeyCode::Esc,
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
    use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::Color;
    use termprofile::TermProfile;

    use super::*;

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
}
