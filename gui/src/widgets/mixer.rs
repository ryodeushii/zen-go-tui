use iced::widget::{column, row, text, Column};
use iced::{Alignment, Element, Length};

use zen_go_tui::app::{AppState, Intent};

use super::{meter, strip};
use crate::theme::ZenTheme;
use crate::Message;

pub fn view<'a>(state: &'a AppState, theme: &'a ZenTheme) -> Element<'a, Message> {
    let mixer = state.active_mixer_surface();
    let channels = &state.mixer.channels[mixer.index()];
    let scroll = state.mixer.strip_scroll;
    let visible_count = 8;

    let visible_channels: Vec<_> = channels
        .iter()
        .skip(scroll)
        .take(visible_count)
        .enumerate()
        .map(|(idx, ch)| strip::strip_view(ch, scroll + idx, state, theme))
        .collect();

    let surface_label = match mixer {
        antelope_protocol::MixerSurface::Mix1 => "MIX 1",
        antelope_protocol::MixerSurface::Mix2 => "MIX 2",
    };

    let header = row![
        text(surface_label).size(12).color(theme.text_dim()),
        iced::widget::horizontal_space(),
        if scroll > 0 {
            row![
                iced::widget::button(text("<").size(10))
                    .padding([2, 6])
                    .on_press(Message::UserIntent(Intent::PageMixerStripsLeft)),
                iced::widget::button(text(">").size(10))
                    .padding([2, 6])
                    .on_press(Message::UserIntent(Intent::PageMixerStripsRight)),
            ]
            .spacing(4)
        } else {
            row![iced::widget::button(text(">").size(10))
                .padding([2, 6])
                .on_press(Message::UserIntent(Intent::PageMixerStripsRight)),]
        }
    ]
    .padding([4, 8]);

    let strips_row = row(visible_channels).spacing(2).padding([0, 8]);

    Column::new().push(header).push(strips_row).into()
}
