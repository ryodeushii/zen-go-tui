use iced::widget::{button, column, container, row, slider, text, Column};
use iced::{Alignment, Element, Length};

use antelope_protocol::{
    MixerAssignment, MixerChannelState, MixerSurface, OutputMode, OutputTarget, PanState,
};
use zen_go_tui::app::AppState;
use zen_go_tui::app::Intent;

use crate::theme::ZenTheme;
use crate::Message;

pub fn view<'a>(state: &'a AppState, theme: &'a ZenTheme) -> Element<'a, Message> {
    let mixer = state.active_mixer_surface();
    let channels = &state.mixer.channels[mixer.index()];

    let mut strips: Vec<Element<Message>> = Vec::new();

    for (idx, ch) in channels.iter().enumerate() {
        strips.push(strip_view(ch, idx, state, theme));
    }

    let surface_label = match mixer {
        MixerSurface::Mix1 => "MIX 1",
        MixerSurface::Mix2 => "MIX 2",
    };

    let header = row![text(surface_label).size(12).color(theme.text_dim()),].padding([4, 8]);

    let strips_row = row(strips).spacing(2).padding([0, 8]);

    Column::new().push(header).push(strips_row).into()
}

pub fn strip_view<'a>(
    channel: &'a MixerChannelState,
    index: usize,
    state: &'a AppState,
    theme: &'a ZenTheme,
) -> Element<'a, Message> {
    let ch_num = channel.channel as usize;
    let assignment_label = channel
        .assignment
        .map(|a| assignment_short_label(a))
        .unwrap_or("--");

    let level = channel.level.unwrap_or(0x20);
    let level_db = format_level_db(level);

    let pan_label = format_pan(channel.pan);

    let meter_value = channel.meter.unwrap_or(0);

    let is_selected = state.mixer.selected_channel == index;

    let mute_button = button(text("M").size(10))
        .padding([2, 4])
        .style(move |_theme, status| {
            if channel.muted.unwrap_or(false) {
                iced::widget::button::Style {
                    background: Some(iced::Background::Color(theme.button_muted())),
                    text_color: iced::Color::WHITE,
                    ..Default::default()
                }
            } else {
                iced::widget::button::Style::default()
            }
        })
        .on_press(Message::UserIntent(Intent::ToggleMixerMute(
            channel.channel,
        )));

    let solo_button = button(text("S").size(10))
        .padding([2, 4])
        .style(move |_theme, status| {
            if channel.soloed.unwrap_or(false) {
                iced::widget::button::Style {
                    background: Some(iced::Background::Color(theme.button_solo())),
                    text_color: iced::Color::BLACK,
                    ..Default::default()
                }
            } else {
                iced::widget::button::Style::default()
            }
        })
        .on_press(Message::UserIntent(Intent::ToggleMixerSolo(
            channel.channel,
        )));

    let link_button = button(text("L").size(10))
        .padding([2, 4])
        .style(move |_theme, status| {
            if channel.linked.unwrap_or(false) {
                iced::widget::button::Style {
                    background: Some(iced::Background::Color(theme.button_active())),
                    text_color: iced::Color::WHITE,
                    ..Default::default()
                }
            } else {
                iced::widget::button::Style::default()
            }
        })
        .on_press(Message::UserIntent(Intent::ToggleMixerLink(
            channel.channel,
        )));

    let assignment_button = button(text(assignment_label).size(9))
        .padding([2, 4])
        .on_press(Message::UserIntent(Intent::OpenAssignmentPicker(
            channel.channel,
        )));

    let fader = iced::widget::vertical_slider(0.0..=1.0, level as f32 / 0x60 as f32, move |v| {
        Message::UserIntent(Intent::SetMixerLevel {
            index,
            level: (v * 0x60 as f32) as u8,
        })
    })
    .height(120);

    let channel_label = text(format!("CH{:02}", ch_num))
        .size(10)
        .color(if is_selected {
            theme.selection_highlight()
        } else {
            theme.text_dim()
        });

    let level_display = text(level_db).size(10).color(theme.text_bright());

    let pan_display = text(pan_label).size(9).color(theme.text_dim());

    let buttons = row![mute_button, solo_button, link_button]
        .spacing(2)
        .align_y(Alignment::Center);

    let content = Column::new()
        .spacing(2)
        .align_x(Alignment::Center)
        .push(channel_label)
        .push(assignment_button)
        .push(fader)
        .push(level_display)
        .push(pan_display)
        .push(buttons);

    let strip_style = if is_selected {
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
            background: Some(iced::Background::Color(theme.strip_background())),
            border: iced::Border {
                radius: 4.0.into(),
                width: 1.0,
                color: theme.panel_border(),
            },
            ..Default::default()
        }
    };

    container(content)
        .padding(6)
        .style(move |_theme| strip_style)
        .width(60)
        .into()
}

fn assignment_short_label(assignment: MixerAssignment) -> &'static str {
    match assignment {
        MixerAssignment::Preamp(1) => "P1",
        MixerAssignment::Preamp(2) => "P2",
        MixerAssignment::ComputerPlay(n) => match n {
            1 => "C1",
            2 => "C2",
            3 => "C3",
            4 => "C4",
            5 => "C5",
            6 => "C6",
            7 => "C7",
            8 => "C8",
            _ => "C?",
        },
        MixerAssignment::Mute => "--",
        _ => "??",
    }
}

fn format_level_db(level: u8) -> String {
    let db = (level as f32 * 0.5) - 48.0;
    format!("{:+.0}", db)
}

fn format_pan(pan: PanState) -> String {
    let raw = pan.raw();
    if raw == 0x1e {
        "C".to_string()
    } else if raw < 0x1e {
        format!("L{}", 0x1e - raw)
    } else {
        format!("R{}", raw - 0x1e)
    }
}
