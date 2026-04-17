use iced::widget::{button, column, container, row, text, Column};
use iced::{Alignment, Element, Length};

use zen_go_tui::app::{AppState, Intent};

use crate::theme::ZenTheme;
use crate::Message;

pub fn view<'a>(state: &'a AppState, theme: &'a ZenTheme) -> Element<'a, Message> {
    if state.popup.profiles_open {
        profiles_popup(state, theme)
    } else if state.popup.routing_open {
        routing_popup(state, theme)
    } else if state.popup.options_open {
        options_popup(state, theme)
    } else {
        container(text("")).into()
    }
}

fn profiles_popup<'a>(state: &'a AppState, theme: &'a ZenTheme) -> Element<'a, Message> {
    let mut items: Vec<Element<Message>> = Vec::new();

    items.push(text("Profiles").size(16).color(theme.text_bright()).into());

    if state.popup.profile_names.is_empty() {
        items.push(
            text("No saved profiles")
                .size(12)
                .color(theme.text_dim())
                .into(),
        );
    } else {
        for (idx, name) in state.popup.profile_names.iter().enumerate() {
            let is_selected = idx == state.popup.selected_index;
            let item = button(text(name).size(12))
                .padding([6, 12])
                .width(Length::Fill)
                .style(move |_theme, status| {
                    if is_selected {
                        iced::widget::button::Style {
                            background: Some(iced::Background::Color(theme.selection_highlight())),
                            text_color: iced::Color::WHITE,
                            ..Default::default()
                        }
                    } else {
                        iced::widget::button::Style::default()
                    }
                })
                .on_press(Message::UserIntent(Intent::SelectProfile(idx)));
            items.push(item.into());
        }
    }

    let actions = row![
        button(text("Load").size(11))
            .padding([4, 12])
            .on_press(Message::UserIntent(Intent::LoadSelectedProfile)),
        button(text("Close").size(11))
            .padding([4, 12])
            .on_press(Message::UserIntent(Intent::CloseProfilesPopup)),
    ]
    .spacing(8);

    let content = Column::new()
        .spacing(8)
        .push(column(items).spacing(4))
        .push(actions);

    popup_container(content, theme)
}

fn routing_popup<'a>(state: &'a AppState, theme: &'a ZenTheme) -> Element<'a, Message> {
    let mut items: Vec<Element<Message>> = Vec::new();

    items.push(
        text("USB Recording Channels")
            .size(14)
            .color(theme.text_bright())
            .into(),
    );

    let mixer = state.active_mixer_surface();
    for ch_idx in 0..8 {
        if let Some(channel) = state.mixer.channels[mixer.index()].get(ch_idx) {
            let assignment = channel
                .assignment
                .map(|a| format_assignment(a))
                .unwrap_or("--".to_string());

            let row = row![
                text(format!("USB {}", ch_idx + 1)).size(11).width(60),
                text(assignment).size(11).color(theme.text_bright()),
            ]
            .spacing(8)
            .align_y(Alignment::Center);

            items.push(row.into());
        }
    }

    let content = Column::new()
        .spacing(8)
        .push(column(items).spacing(4))
        .push(
            button(text("Close").size(11))
                .padding([4, 12])
                .on_press(Message::UserIntent(Intent::CloseRoutingPopup)),
        );

    popup_container(content, theme)
}

fn options_popup<'a>(state: &'a AppState, theme: &'a ZenTheme) -> Element<'a, Message> {
    let content = Column::new()
        .spacing(8)
        .push(text("Options").size(14).color(theme.text_bright()))
        .push(text("Peak detection").size(12).color(theme.text_dim()))
        .push(
            button(
                text(if state.ui.settings.peak_enabled {
                    "ON"
                } else {
                    "OFF"
                })
                .size(11),
            )
            .padding([4, 12])
            .on_press(Message::UserIntent(Intent::TogglePeakEnabled)),
        )
        .push(
            button(text("Close").size(11))
                .padding([4, 12])
                .on_press(Message::UserIntent(Intent::CloseOptionsPopup)),
        );

    popup_container(content, theme)
}

fn popup_container<'a>(content: Column<'a, Message>, theme: &'a ZenTheme) -> Element<'a, Message> {
    container(content)
        .padding(16)
        .style(move |_theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme.popup_background())),
            border: iced::Border {
                radius: 8.0.into(),
                width: 1.0,
                color: theme.popup_border(),
            },
            ..Default::default()
        })
        .width(300)
        .max_height(400)
        .into()
}

fn format_assignment(assignment: antelope_protocol::MixerAssignment) -> String {
    match assignment {
        antelope_protocol::MixerAssignment::Preamp(n) => format!("Preamp {}", n),
        antelope_protocol::MixerAssignment::ComputerPlay(n) => format!("Computer Play {}", n),
        antelope_protocol::MixerAssignment::Mute => "Mute".to_string(),
        _ => "Unknown".to_string(),
    }
}
