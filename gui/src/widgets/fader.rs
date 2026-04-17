use iced::widget::vertical_slider;
use iced::Element;

use crate::theme::ZenTheme;
use crate::Message;

pub fn fader<'a>(
    value: f32,
    height: f32,
    range: std::ops::RangeInclusive<f32>,
    on_change: impl Fn(f32) -> Message + 'a,
) -> Element<'a, Message> {
    vertical_slider(range, value, on_change)
        .height(height)
        .into()
}
