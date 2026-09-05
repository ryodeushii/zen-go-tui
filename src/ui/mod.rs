#[cfg(test)]
mod dynamic_profile_tests;
#[cfg(test)]
mod dynamic_tests;
mod layouts;
mod mouse;
mod raw_map;
mod render;
mod styles;
#[cfg(test)]
mod tests;
mod widgets;

// Re-export Intent enum from app module
pub use crate::app::Intent;

// Re-export public API
pub use layouts::device_header_area;
pub use mouse::device_header_name_hit;
pub use mouse::device_picker_activation_row;
pub use mouse::mixer_strip_panel_contains;
pub use mouse::mixer_strip_viewport_capacity;
pub use mouse::mouse_action;
pub use mouse::slider_mouse_action;
pub use mouse::slider_wheel_action;
pub use render::draw;
pub use render::draw_device_picker;
pub use render::profile_editor_cursor;
