use iced::widget::container;
use iced::{Element, Length};

use zen_go_tui::app::MeterPeak;

use crate::theme::ZenTheme;
use crate::Message;

pub fn meter_bar<'a>(
    value: u8,
    peak: Option<MeterPeak>,
    theme: &'a ZenTheme,
) -> Element<'a, Message> {
    let normalized = value as f32 / 0x60 as f32;

    let color = if normalized < 0.6 {
        theme.meter_gradient_low()
    } else if normalized < 0.85 {
        theme.meter_gradient_mid()
    } else {
        theme.meter_gradient_high()
    };

    container(iced::widget::horizontal_rule(2))
        .style(move |_theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme.fader_track())),
            ..Default::default()
        })
        .width(Length::Fill)
        .into()
}
