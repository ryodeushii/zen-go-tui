use iced::widget::{row, text, Column};
use iced::{Alignment, Element, Length};

use zen_go_tui::app::{AppState, Intent};

use super::output;
use crate::theme::ZenTheme;
use crate::Message;

pub fn view<'a>(state: &'a AppState, theme: &'a ZenTheme) -> Element<'a, Message> {
    let outputs: Vec<Element<Message>> = state
        .output
        .states
        .iter()
        .enumerate()
        .map(|(idx, output)| output::output_view(output, idx, state, theme))
        .collect();

    let header = row![
        text("Outputs").size(12).color(theme.text_dim()),
        iced::widget::horizontal_space(),
    ]
    .padding([4, 8]);

    Column::new()
        .push(header)
        .push(row(outputs).spacing(8).padding([0, 8]))
        .into()
}
