use iced::widget::{button, container, row, text, Column};
use iced::{Alignment, Element, Length};

use antelope_protocol::PreampMode;
use zen_go_tui::app::{AppState, Intent, MeterPeak};

use super::meter;
use crate::theme::ZenTheme;
use crate::Message;

pub fn view<'a>(state: &'a AppState, theme: &'a ZenTheme) -> Element<'a, Message> {
    let input1 = &state.preamp.state.input1;
    let input2 = &state.preamp.state.input2;

    let input1_panel = preamp_input_view("IN 1", input1, state.preamp.peaks[0], 0, theme);

    let input2_panel = preamp_input_view("IN 2", input2, state.preamp.peaks[1], 1, theme);

    row![input1_panel, input2_panel,]
        .spacing(8)
        .padding([8, 12])
        .into()
}

fn preamp_input_view<'a>(
    label: &'a str,
    input: &'a antelope_protocol::PreampInputState,
    peak: Option<MeterPeak>,
    input_index: u8,
    theme: &'a ZenTheme,
) -> Element<'a, Message> {
    let mode_label = match input.mode {
        PreampMode::Mic => "MIC",
        PreampMode::Line => "LINE",
        PreampMode::HiZ => "Hi-Z",
        PreampMode::Unknown(_) => "UNK",
    };

    let gain_db = format_gain_db(input.gain_raw);

    let meter_value = input.observed_meter.unwrap_or(0);
    let meter_bar = meter::meter_bar(meter_value, peak, theme);

    let mode_button =
        button(text(mode_label).size(10))
            .padding([2, 6])
            .on_press(Message::UserIntent(Intent::OpenPreampModeSelector(
                input_index,
            )));

    let phantom_button = button(text("48V").size(10))
        .padding([2, 6])
        .style(move |_theme, status| {
            if input.phantom_on {
                iced::widget::button::Style {
                    background: Some(iced::Background::Color(theme.button_muted())),
                    text_color: iced::Color::WHITE,
                    ..Default::default()
                }
            } else {
                iced::widget::button::Style::default()
            }
        })
        .on_press(Message::UserIntent(Intent::TogglePreampPhantom(
            input_index,
        )));

    let phase_button = button(text("Ø").size(10))
        .padding([2, 6])
        .style(move |_theme, status| {
            if input.mode_raw & 0x40 != 0 {
                iced::widget::button::Style {
                    background: Some(iced::Background::Color(theme.button_active())),
                    text_color: iced::Color::WHITE,
                    ..Default::default()
                }
            } else {
                iced::widget::button::Style::default()
            }
        })
        .on_press(Message::UserIntent(Intent::TogglePreampPhase(input_index)));

    let gain_display = text(gain_db)
        .size(14)
        .color(theme.text_bright())
        .width(Length::Fill)
        .align_x(iced::alignment::Horizontal::Center);

    let controls = row![mode_button, phantom_button, phase_button,]
        .spacing(4)
        .align_y(Alignment::Center);

    let content = Column::new()
        .spacing(4)
        .align_x(Alignment::Center)
        .push(text(label).size(11).color(theme.text_dim()))
        .push(gain_display)
        .push(controls)
        .push(meter_bar);

    container(content)
        .padding(8)
        .style(move |_theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme.panel_background())),
            border: iced::Border {
                radius: 4.0.into(),
                width: 1.0,
                color: theme.panel_border(),
            },
            ..Default::default()
        })
        .width(Length::FillPortion(1))
        .into()
}

fn format_gain_db(raw: u8) -> String {
    let db = (raw as f32 * 0.5) - 12.0;
    format!("{:+.1} dB", db)
}
