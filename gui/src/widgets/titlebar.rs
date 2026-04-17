use iced::widget::{button, container, horizontal_space, row, text};
use iced::{Alignment, Element, Length};

use zen_go_tui::app::AppState;
use zen_go_tui::app::Intent;

use crate::theme::ZenTheme;
use crate::Message;

pub fn view<'a>(state: &'a AppState, theme: &'a ZenTheme) -> Element<'a, Message> {
    let connected = state.device.connection.connected;
    let status_color = if connected {
        theme.connected_indicator()
    } else {
        theme.disconnected_indicator()
    };

    let device_name = state
        .device
        .status
        .metadata
        .as_ref()
        .map(|m| m.product_name.as_str())
        .unwrap_or("Zen Go Synergy Core");

    let sample_rate = state
        .device
        .status
        .sample_rate
        .map(|sr| sr.label())
        .unwrap_or("---".to_string());

    let clock_source = state
        .device
        .status
        .clock_source
        .map(|cs| cs.label())
        .unwrap_or("---");

    let status_indicator = container(text("●").color(status_color)).padding(4);

    let title = text(device_name).size(16).width(Length::Fill);

    let info = row![text(format!("{} / {}", sample_rate, clock_source))
        .size(12)
        .color(theme.text_dim()),]
    .spacing(16)
    .align_y(Alignment::Center);

    let buttons = row![
        button(text("Routing").size(11))
            .padding([4, 8])
            .on_press(Message::UserIntent(Intent::OpenRoutingPopup)),
        button(text("Profiles").size(11))
            .padding([4, 8])
            .on_press(Message::UserIntent(Intent::OpenProfilesPopup)),
        button(text("Options").size(11))
            .padding([4, 8])
            .on_press(Message::UserIntent(Intent::OpenOptionsPopup)),
        button(text("?").size(11))
            .padding([4, 8])
            .on_press(Message::UserIntent(Intent::ToggleHotkeysPopup)),
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    row![status_indicator, title, info, horizontal_space(), buttons,]
        .spacing(12)
        .align_y(Alignment::Center)
        .padding([6, 12])
        .into()
}
