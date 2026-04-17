use iced::widget::{button, container, row, text, Column};
use iced::{Alignment, Element, Length};

use antelope_protocol::{OutputMode, OutputState, OutputTarget};
use zen_go_tui::app::{AppState, Intent};

use crate::theme::ZenTheme;
use crate::Message;

pub fn view<'a>(state: &'a AppState, theme: &'a ZenTheme) -> Element<'a, Message> {
    let outputs: Vec<Element<Message>> = state
        .output
        .states
        .iter()
        .enumerate()
        .map(|(idx, output)| output_view(output, idx, state, theme))
        .collect();

    row(outputs).spacing(8).padding([8, 12]).into()
}

pub fn output_view<'a>(
    output: &'a OutputState,
    index: usize,
    state: &'a AppState,
    theme: &'a ZenTheme,
) -> Element<'a, Message> {
    let label = match output.target {
        OutputTarget::Monitor => "MONITOR",
        OutputTarget::Hp1 => "HP 1",
        OutputTarget::Hp2 => "HP 2",
        _ => "UNKNOWN",
    };

    let volume = output.volume;
    let volume_db = format_output_db(volume);

    let mode_label = match output.mode {
        OutputMode::Normal => "Normal",
        OutputMode::Mute => "Mute",
        OutputMode::Dim => "Dim",
        OutputMode::Unknown(_) => "Unknown",
    };

    let is_selected = state.output.selected == index;

    let mute_button = button(text("MUTE").size(10))
        .padding([3, 8])
        .style(move |_theme, status| {
            if output.mode == OutputMode::Mute {
                iced::widget::button::Style {
                    background: Some(iced::Background::Color(theme.button_muted())),
                    text_color: iced::Color::WHITE,
                    ..Default::default()
                }
            } else {
                iced::widget::button::Style::default()
            }
        })
        .on_press(Message::UserIntent(Intent::ToggleOutputMute(index)));

    let dim_button = button(text("DIM").size(10))
        .padding([3, 8])
        .style(move |_theme, status| {
            if output.mode == OutputMode::Dim {
                iced::widget::button::Style {
                    background: Some(iced::Background::Color(theme.button_solo())),
                    text_color: iced::Color::BLACK,
                    ..Default::default()
                }
            } else {
                iced::widget::button::Style::default()
            }
        })
        .on_press(Message::UserIntent(Intent::ToggleOutputDim(index)));

    let fader = iced::widget::vertical_slider(0.0..=1.0, volume as f32 / 0x60 as f32, move |v| {
        Message::UserIntent(Intent::SetOutputLevel {
            index,
            step: (v * 0x60 as f32) as u8,
        })
    })
    .height(80);

    let volume_display = text(volume_db).size(12).color(theme.text_bright());

    let mode_display = text(mode_label).size(10).color(theme.text_dim());

    let label_text = text(label).size(11).color(if is_selected {
        theme.selection_highlight()
    } else {
        theme.text_dim()
    });

    let buttons = row![mute_button, dim_button]
        .spacing(4)
        .align_y(Alignment::Center);

    let content = Column::new()
        .spacing(4)
        .align_x(Alignment::Center)
        .push(label_text)
        .push(fader)
        .push(volume_display)
        .push(mode_display)
        .push(buttons);

    let panel_style = if is_selected {
        iced::widget::container::Style {
            background: Some(iced::Background::Color(theme.strip_header())),
            border: iced::Border {
                radius: 4.0.into(),
                width: 1.0,
                color: theme.selection_highlight(),
            },
            ..Default::default()
        }
    } else {
        iced::widget::container::Style {
            background: Some(iced::Background::Color(theme.panel_background())),
            border: iced::Border {
                radius: 4.0.into(),
                width: 1.0,
                color: theme.panel_border(),
            },
            ..Default::default()
        }
    };

    container(content)
        .padding(8)
        .style(move |_theme| panel_style)
        .width(Length::FillPortion(1))
        .into()
}

fn format_output_db(volume: u8) -> String {
    let db = (volume as f32 * 0.5) - 48.0;
    format!("{:+.0}", db)
}
