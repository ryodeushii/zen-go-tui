use iced::color;
use iced::theme::{Custom, Palette};
use iced::Theme;

#[derive(Clone, Copy, Debug, Default)]
pub struct ZenTheme;

pub const ZEN_THEME: ZenTheme = ZenTheme;

impl ZenTheme {
    pub fn dark_palette() -> Palette {
        Palette {
            background: color!(0x1a1a1e),
            text: color!(0xe0e0e0),
            primary: color!(0x4a9eff),
            success: color!(0x4caf50),
            danger: color!(0xf44336),
        }
    }

    pub fn meter_gradient_low(&self) -> iced::Color {
        color!(0x4caf50)
    }

    pub fn meter_gradient_mid(&self) -> iced::Color {
        color!(0xffc107)
    }

    pub fn meter_gradient_high(&self) -> iced::Color {
        color!(0xf44336)
    }

    pub fn panel_background(&self) -> iced::Color {
        color!(0x242428)
    }

    pub fn panel_border(&self) -> iced::Color {
        color!(0x3a3a3e)
    }

    pub fn strip_background(&self) -> iced::Color {
        color!(0x2a2a2e)
    }

    pub fn strip_header(&self) -> iced::Color {
        color!(0x333338)
    }

    pub fn button_active(&self) -> iced::Color {
        color!(0x4a9eff)
    }

    pub fn button_muted(&self) -> iced::Color {
        color!(0xf44336)
    }

    pub fn button_solo(&self) -> iced::Color {
        color!(0xffc107)
    }

    pub fn fader_track(&self) -> iced::Color {
        color!(0x404044)
    }

    pub fn fader_handle(&self) -> iced::Color {
        color!(0x888888)
    }

    pub fn text_dim(&self) -> iced::Color {
        color!(0x888888)
    }

    pub fn text_bright(&self) -> iced::Color {
        color!(0xffffff)
    }

    pub fn connected_indicator(&self) -> iced::Color {
        color!(0x4caf50)
    }

    pub fn disconnected_indicator(&self) -> iced::Color {
        color!(0xf44336)
    }

    pub fn popup_overlay(&self) -> iced::Color {
        iced::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.6,
        }
    }

    pub fn popup_background(&self) -> iced::Color {
        color!(0x2a2a2e)
    }

    pub fn popup_border(&self) -> iced::Color {
        color!(0x4a4a4e)
    }

    pub fn selection_highlight(&self) -> iced::Color {
        color!(0x4a9eff)
    }
}
