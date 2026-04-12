use std::time::Instant;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;
use ratatui::widgets::Widget;
use ratatui::Terminal;

use crate::app::{
    AppState, AssignmentPickerState, FocusArea, Intent, RawPacketTab, SelectorPopupKind,
    SelectorPopupState, QUERY_REPLY_VISIBLE_COUNT,
};
use antelope_protocol::{
    ClockSource, MixerAssignment, MixerChannelState, MixerLinkTarget, MixerSurface, OutputMode,
    OutputState, OutputTarget, PanState, PreampInputState, PreampMode, SampleRate, Surface,
};

use super::*;

fn render_buffer(area: Rect, render: impl FnOnce(Rect, &mut Buffer)) -> String {
    let mut buffer = Buffer::empty(area);
    render(area, &mut buffer);
    let mut out = String::new();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn output_card_rendering_surfaces_level_mode_and_focus() {
    let output = OutputState::new(OutputTarget::Hp1, 0x30, OutputMode::Mute);

    let rendered = render_buffer(
        Rect::new(0, 0, 40, layouts::output_card_height()),
        |area, buffer| {
            render::render_output_card_widget(area, buffer, &output, true);
        },
    );

    assert!(rendered.contains("HP1"));
    assert!(rendered.contains("48 dB"));
    assert!(rendered.contains("ACTIVE"));
    assert!(rendered.contains("LVL -48 dB"));
    assert!(rendered.contains("─"));
    assert!(rendered.contains("●"));
    assert!(rendered.contains(layouts::ADJUST_DOWN_BUTTON_LABEL));
    assert!(rendered.contains(layouts::ADJUST_UP_BUTTON_LABEL));
    assert!(rendered.contains(" DIM "));
    assert!(rendered.contains(" MUTE "));
    assert!(!rendered.contains("raw 30"));
}

#[test]
fn output_card_areas_split_horizontally_across_bottom_panel() {
    let areas = layouts::output_card_areas(Rect::new(10, 5, 90, layouts::output_card_height()));

    assert_eq!(areas.len(), 3);
    assert_eq!(areas[0].y, areas[1].y);
    assert_eq!(areas[1].y, areas[2].y);
    assert!(areas[0].x < areas[1].x);
    assert!(areas[1].x < areas[2].x);
}

#[test]
fn hotkeys_popup_text_lists_core_shortcuts() {
    let rendered = render::render_hotkeys_popup_text().to_string();

    assert!(rendered.contains("Global"));
    assert!(rendered.contains("? hotkeys"));
    assert!(rendered.contains("Ctrl+d raw inspector"));
    assert!(rendered.contains("r routing"));
    assert!(rendered.contains("p profiles"));
}

#[test]
fn mouse_action_returns_intent_not_mouse_action() {
    let area = Rect::new(0, 0, 120, 50);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let button = layouts::output_hotkeys_button_rect(page[1]);

    let result = mouse_action(area, &AppState::default(), button.x + 1, button.y);
    assert_eq!(result, Some(Intent::ToggleHotkeysPopup));
}

#[test]
fn mouse_action_hits_output_hotkeys_button() {
    let area = Rect::new(0, 0, 120, 50);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let button = layouts::output_hotkeys_button_rect(page[1]);

    assert_eq!(
        mouse_action(area, &AppState::default(), button.x + 1, button.y),
        Some(Intent::ToggleHotkeysPopup)
    );
}

#[test]
fn meter_value_labels_reserve_width_and_use_negative_infinity() {
    assert_eq!(layouts::format_meter_value_label(Some(0)), "  0 dB");
    assert_eq!(layouts::format_meter_value_label(Some(-48)), "-48 dB");
    assert_eq!(layouts::format_meter_value_label(None), " -∞ dB");
    assert_eq!(
        layouts::format_meter_value_label(Some(0)).chars().count(),
        6
    );
    assert_eq!(
        layouts::format_meter_value_label(Some(-48)).chars().count(),
        6
    );
    assert_eq!(layouts::format_meter_value_label(None).chars().count(), 6);
}

#[test]
fn mixer_strip_widget_renders_compact_vertical_strip_layout() {
    let mut state = AppState::default();
    state.focus = FocusArea::Mixer;
    state.selected_channel = 10;
    state.mixer_channels[0][10] = MixerChannelState {
        channel: 11,
        level: Some(0x10),
        meter: Some(0x30),
        muted: Some(false),
        soloed: Some(true),
        pan: PanState::from_raw(0x3e),
        assignment: Some(MixerAssignment::ComputerPlay(8)),
        linked: Some(true),
    };

    let rendered = render_buffer(
        Rect::new(0, 0, 18, layouts::mixer_strip_height()),
        |area, buffer| {
            render::render_mixer_strip_widget(
                area,
                buffer,
                &state,
                10,
                &state.mixer_channels[0][10],
            );
        },
    );

    assert!(rendered.contains("CH 11"));
    assert!(rendered.contains(" C8 "));
    assert!(!rendered.contains("Computer Play 8"));
    assert!(rendered.contains("PAN 30"));
    assert!(rendered.contains("-30"));
    assert!(rendered.contains(" 30"));
    assert!(rendered.contains("-48 dB"));
    assert!(rendered.contains("-16 dB"));
    assert!(rendered.contains(" 60"));
    assert!(rendered.contains(" 40"));
    assert!(rendered.contains(" 30"));
    assert!(rendered.contains(" 20"));
    assert!(rendered.contains(" 15"));
    assert!(rendered.contains(" 10"));
    assert!(rendered.contains("  5"));
    assert!(rendered.contains("  0"));
    assert!(rendered.contains("█"));
    assert!(rendered.contains("●"));
}

#[test]
fn preamp_visual_stacks_observed_meter_and_gain_sliders() {
    let mut input = PreampInputState::from_raw(0x14, 0x10);
    input.observed_meter = Some(0x30);

    let rendered = render_buffer(Rect::new(0, 0, 44, 5), |area, buffer| {
        render::render_preamp_visual_widget(area, buffer, "Preamp 1", input, true, None);
    });

    assert!(rendered.contains("Preamp 1"));
    assert!(rendered.contains("GAIN 20 dB"));
    assert!(rendered.contains("OBS -48 dB"));
    assert!(rendered.contains("░"));
    assert!(rendered.contains("─"));
    assert!(rendered.contains("●"));
    assert!(!rendered.contains("48V:"));
    assert!(!rendered.contains("PH:"));
    assert!(!rendered.contains("raw "));
}

#[test]
fn mixer_strip_widget_uses_reserved_meter_width_for_silence() {
    let mut state = AppState::default();
    state.focus = FocusArea::Mixer;
    state.selected_channel = 0;
    state.mixer_channels[0][0].level = Some(0x00);
    state.mixer_channels[0][0].meter = Some(0x60);

    let rendered = render_buffer(
        Rect::new(0, 0, 72, layouts::mixer_strip_height()),
        |area, buffer| {
            render::render_mixer_strip_widget(area, buffer, &state, 0, &state.mixer_channels[0][0]);
        },
    );

    assert!(rendered.contains(" -∞ dB"));
}

#[test]
fn mixer_strip_widget_keeps_db_scale_markers_in_wide_area() {
    let mut state = AppState::default();
    state.focus = FocusArea::Mixer;
    state.selected_channel = 0;
    state.mixer_channels[0][0].level = Some(0x00);
    state.mixer_channels[0][0].meter = Some(0x10);

    let rendered = render_buffer(
        Rect::new(0, 0, 120, layouts::mixer_strip_height()),
        |area, buffer| {
            render::render_mixer_strip_widget(area, buffer, &state, 0, &state.mixer_channels[0][0]);
        },
    );

    assert!(rendered.contains("60"));
    assert!(rendered.contains("30"));
    assert!(rendered.contains("LVL 0 dB"));
}

#[test]
fn labeled_level_slider_keeps_handle_visible_at_maximum() {
    let area = Rect::new(0, 0, 24, 1);
    let mut buffer = Buffer::empty(area);

    render::render_labeled_slider(
        area,
        &mut buffer,
        "LVL   0 dB",
        Some(1.0),
        Color::Yellow,
        true,
    );

    assert_eq!(buffer[(23, 0)].symbol(), "●");
}

#[test]
fn status_strip_surfaces_message_surface_and_output() {
    let mut state = AppState::default();
    state.surface = Surface::Hp2;
    state.selected_output = 1;
    state.last_message = "Applied dim change".to_string();

    let rendered = render::render_status_strip(&state).to_string();

    assert!(!rendered.contains("STATUS"));
    assert!(!rendered.contains("Applied dim change"));
    assert_eq!(
        rendered,
        render::render_experimental_pair_state_line(&state)
    );
}

#[test]
fn mix_meter_extracts_mix1_lane_pair() {
    let mut state = AppState::default();
    let mut frame = vec![0_u8; 320];
    frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    frame[0x10 + 0x6a] = 0x0f;
    frame[0x10 + 0xda] = 0x0a;
    frame[0x10 + 0xdb] = 0x05;
    state.latest_raw_73 = Some(frame);

    assert_eq!(mouse::mix_meter(&state), Some(("MIX 1", 0x0a, 0x05)));
}

#[test]
fn mixer_strip_panel_layout_reserves_two_rows_for_embedded_mix_meter() {
    let layout = layouts::mixer_strip_panel_layout(Rect::new(0, 0, 80, 14), true);

    assert_eq!(layout[1].height, 2);
    assert_eq!(
        layout[0].height + layout[1].height,
        layouts::inner_area(Rect::new(0, 0, 80, 14)).height
    );
}

#[test]
fn mixer_list_mouse_action_ignores_embedded_mix_meter_rows() {
    let mut state = AppState::default();
    let mut frame = vec![0_u8; 320];
    frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    frame[0x10 + 0x6a] = 0x0f;
    frame[0x10 + 0xda] = 0x0a;
    frame[0x10 + 0xdb] = 0x05;
    state.latest_raw_73 = Some(frame);

    let mixer = layouts::mixer_layout(Rect::new(0, 0, 100, 20));
    let meter_area = layouts::mixer_strip_panel_layout(mixer[1], true)[1];

    assert_eq!(
        mouse::mixer_list_mouse_action(mixer[1], &state, (meter_area.x + 1, meter_area.y)),
        None
    );
}

#[test]
fn mix_meter_widget_renders_two_row_stereo_bar_and_fixed_db_labels() {
    let rendered = render_buffer(Rect::new(0, 0, 56, 2), |area, buffer| {
        render::render_mix_meter_widget(area, buffer, 0x00, 0x3c);
    });

    assert!(rendered.contains("L"));
    assert!(rendered.contains("R"));
    assert!(rendered.contains("  0 dB"));
    assert!(rendered.contains("-60 dB"));
    assert!(rendered.contains("█"));
    assert!(rendered.contains("░"));
    let lines = rendered.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("L"));
    assert!(lines[1].contains("R"));
}

#[test]
fn device_header_surfaces_serial_and_hw_without_duplicate_status_line() {
    let mut state = AppState::default();
    state.device.metadata = Some(antelope_protocol::DeviceMetadata {
        product_name: "Zen Go Synergy Core".to_string(),
        serial: "1234567890".to_string(),
        hardware_version: "6.6".to_string(),
    });
    state.device.sample_rate = Some(SampleRate::Hz48000);
    state.device.clock_source = Some(ClockSource::Internal);
    state.device.lock_known = true;
    state.device.locked = Some(true);

    let rendered = render::render_device_header(&state).to_string();
    let metadata = render::render_device_metadata(&state).to_string();

    assert!(metadata.contains("1234567890"));
    assert!(metadata.contains("6.6"));
    assert!(metadata.contains(" HW  6.6 "));
    assert!(!rendered.contains("SURFACE"));
    assert!(!rendered.contains("PAGE"));
    assert!(!rendered.contains("Last"));
    assert!(!rendered.contains('\n'));
}

#[test]
fn device_panel_layout_reserves_full_width_for_serial_and_hw_chips() {
    let mut state = AppState::default();
    state.device.metadata = Some(antelope_protocol::DeviceMetadata {
        product_name: "Zen Go Synergy Core".to_string(),
        serial: "1234567890".to_string(),
        hardware_version: "6.6".to_string(),
    });

    let device = layouts::titlebar_layout(Rect::new(0, 0, 90, 3))[0];
    let metadata = layouts::device_panel_layout(device, &state)[1];

    assert!(
        metadata.width >= styles::chip_width("SN 1234567890") + 1 + styles::chip_width("HW 6.6")
    );
}

#[test]
fn device_header_prefers_live_sample_rate_readout_over_configured_rate() {
    let mut state = AppState::default();
    state.device.sample_rate = Some(SampleRate::Hz96000);
    state.device.sample_rate_hz = Some(44_100);

    let rendered = render::render_device_header(&state).to_string();

    assert!(rendered.contains("44.1 kHz"));
    assert!(!rendered.contains("96000 Hz"));
}

#[test]
fn afx_page_renders_usb_recording_pairs_from_mixer_assignments() {
    let mut state = AppState::default();
    state.mixer_channels[MixerSurface::Mix1.index()][0].assignment =
        Some(MixerAssignment::Preamp(1));
    state.mixer_channels[MixerSurface::Mix1.index()][1].assignment =
        Some(MixerAssignment::Preamp(2));
    for channel in 2..8 {
        state.mixer_channels[MixerSurface::Mix1.index()][channel].assignment =
            Some(MixerAssignment::Mute);
    }

    let rendered = render::render_afx_routing_text(&state).to_string();

    assert!(rendered.contains("Zen Go USB recordings mirror mixer strip assignments"));
    assert!(rendered.contains("USB 1/2  Zen Go Recording 1 <- Preamp 1"));
    assert!(rendered.contains("Zen Go Recording 2 <- Preamp 2"));
    assert!(rendered.contains("USB 7/8  Zen Go Recording 7 <- Mute"));
    assert!(rendered.contains("Zen Go Recording 8 <- Mute"));
}

#[test]
fn titlebar_renders_system_panel_with_raw_and_options() {
    let state = AppState::default();
    let rendered = render::render_system_summary(&state).to_string();

    assert!(rendered.contains("RAW"));
    assert!(rendered.contains("OPTNS"));
    assert!(!rendered.contains('\n'));
}

#[test]
fn connection_badge_uses_green_orange_and_red_states() {
    let mut state = AppState::default();
    assert_eq!(
        render::connection_badge_color(&state),
        Color::Rgb(255, 165, 0)
    );

    state.connection.connected = true;
    assert_eq!(render::connection_badge_color(&state), Color::LightGreen);

    state.connection.connected = false;
    state.connection.last_snapshot_at = Some(Instant::now() - std::time::Duration::from_secs(3));
    assert_eq!(render::connection_badge_color(&state), Color::LightRed);
}

#[test]
fn mixer_strip_rendering_includes_solo_state() {
    let mut state = AppState::default();
    state.focus = crate::app::FocusArea::Mixer;
    state.selected_channel = 0;
    state.mixer_channels[MixerSurface::Mix1.index()][0].soloed = Some(true);

    let line = render::render_mixer_strip_line(
        &state,
        0,
        &state.mixer_channels[MixerSurface::Mix1.index()][0],
    );
    let controls = render::render_mixer_strip_controls(
        &state,
        0,
        &state.mixer_channels[MixerSurface::Mix1.index()][0],
    );

    assert!(line.contains("solo=on"));
    assert!(controls.contains("[Solo on]"));
}

#[test]
fn hex_dump_renders_offset_and_ascii() {
    let dump = render::render_full_packet_dump(&[0x83, 0x00, 0x41, 0x42, 0x0a], None);
    let first = &dump.lines[0];
    let rendered: String = first
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert!(rendered.contains("0000:"));
    assert!(rendered.contains("83 00 41 42 0a"));
    assert!(rendered.contains("|..AB.|"));
}

#[test]
fn zero_bytes_are_dimmed_and_offsets_are_bold() {
    let dump = render::render_full_packet_dump(&[0x00], None);
    let first = &dump.lines[0];
    assert!(first.spans[0].style.add_modifier.contains(Modifier::BOLD));
    assert!(first.spans[1].style.add_modifier.contains(Modifier::DIM));
}

#[test]
fn query_reply_panel_includes_recent_reply_log() {
    let mut state = AppState::default();
    state.recent_query_reply_entries = vec![
        crate::app::QueryReplyLogEntry {
            summary: "0x75 03/05 [64 bytes] 05 00 00 00 01 01 00 01".to_string(),
            raw: vec![0x75, 0x05],
        },
        crate::app::QueryReplyLogEntry {
            summary: "0x75 03/06 [64 bytes] 06 03 00 03 01 03 02 03".to_string(),
            raw: vec![0x75, 0x06],
        },
    ];
    state.selected_query_reply_entry = Some(1);

    let text = render::render_query_reply_panel(&[0x75, 0x00, 0x00, 0x00], &state).to_string();

    assert!(text.contains("0000: 75 06"));
}

#[test]
fn query_request_panel_includes_recent_request_log() {
    let mut state = AppState::default();
    state.recent_query_request_log = vec!["0x74 03/05".to_string(), "0x74 03/06".to_string()];

    let text = render::render_query_request_panel(&[0x74, 0x00, 0x00, 0x00], &state).to_string();

    assert!(text.contains("Recent 0x74 requests:"));
    assert!(text.contains("0x74 03/05"));
    assert!(text.contains("0x74 03/06"));
}

#[test]
fn mouse_action_hits_status_raw_view_toggle() {
    let area = Rect::new(0, 0, 120, 50);
    let system_panel = layouts::titlebar_layout(layouts::root_chunks(area)[0])[1];
    let inner = layouts::inner_area(system_panel);
    let point = (inner.x, inner.y);

    assert_eq!(
        mouse_action(area, &AppState::default(), point.0, point.1),
        Some(Intent::ToggleRawView)
    );
}

#[test]
fn mouse_action_selects_raw_packet_tab_when_raw_view_is_open() {
    let area = Rect::new(0, 0, 120, 50);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);
    let tabs = layouts::raw_tab_hit_areas(layout[1]);
    let point = (tabs[3].x + tabs[3].width / 2, tabs[3].y);
    let mut state = AppState::default();
    state.raw_view_open = true;

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::SelectRawPacketTab(RawPacketTab::Query75))
    );
}

#[test]
fn mouse_action_opens_routing_popup_from_mixer_surface_button() {
    let area = Rect::new(0, 0, 120, 50);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let button = layouts::mixer_header_button_rects(mixer[0])[1];
    let point = (button.x + button.width / 2, button.y);

    assert_eq!(
        mouse_action(area, &AppState::default(), point.0, point.1),
        Some(Intent::OpenRoutingPopup)
    );
}

#[test]
fn mouse_action_opens_profiles_popup_from_mixer_surface_button() {
    let area = Rect::new(0, 0, 120, 50);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let button = layouts::mixer_header_button_rects(mixer[0])[0];
    let point = (button.x + button.width / 2, button.y);

    assert_eq!(
        mouse_action(area, &AppState::default(), point.0, point.1),
        Some(Intent::OpenProfilesPopup)
    );
}

#[test]
fn mouse_action_pages_mixer_strips_left_from_panel_button() {
    let area = Rect::new(0, 0, 120, 50);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let button = layouts::mixer_strip_page_button_rects(mixer[1])[0];

    assert_eq!(
        mouse_action(
            area,
            &AppState::default(),
            button.x + button.width / 2,
            button.y
        ),
        Some(Intent::PageMixerStripsLeft)
    );
}

#[test]
fn mouse_action_pages_mixer_strips_right_from_panel_button() {
    let area = Rect::new(0, 0, 120, 50);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let button = layouts::mixer_strip_page_button_rects(mixer[1])[1];

    assert_eq!(
        mouse_action(
            area,
            &AppState::default(),
            button.x + button.width / 2,
            button.y
        ),
        Some(Intent::PageMixerStripsRight)
    );
}

#[test]
fn mouse_action_opens_assignment_picker_from_afx_routing_source_chip() {
    let area = Rect::new(0, 0, 120, 50);
    let mut state = AppState::default();
    state.routing_popup_open = true;
    state.focus = FocusArea::Mixer;
    state.mixer_channels[MixerSurface::Mix1.index()][0].assignment =
        Some(MixerAssignment::Preamp(1));
    state.mixer_channels[MixerSurface::Mix1.index()][1].assignment =
        Some(MixerAssignment::Preamp(2));
    let row_area = layouts::afx_routing_layout(layouts::routing_popup_area(area))[4];
    let rects = layouts::afx_routing_row_rects(row_area, &state, 0);
    let point = (rects[2].x + rects[2].width / 2, rects[2].y);

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::OpenAssignmentPicker(1))
    );
}

#[test]
fn afx_routing_source_columns_stay_aligned_for_different_label_lengths() {
    let area = Rect::new(0, 0, 120, 1);
    let mut state = AppState::default();
    state.mixer_channels[MixerSurface::Mix1.index()][0].assignment = Some(MixerAssignment::Mute);
    state.mixer_channels[MixerSurface::Mix1.index()][1].assignment =
        Some(MixerAssignment::Preamp(2));
    state.mixer_channels[MixerSurface::Mix1.index()][2].assignment =
        Some(MixerAssignment::ComputerPlay(8));
    state.mixer_channels[MixerSurface::Mix1.index()][3].assignment =
        Some(MixerAssignment::Oscillator(1));

    let first = layouts::afx_routing_row_rects(area, &state, 0);
    let second = layouts::afx_routing_row_rects(area, &state, 1);

    assert_eq!(first[2].x, second[2].x);
    assert_eq!(first[4].x, second[4].x);
}

#[test]
fn mouse_action_opens_sample_rate_selector_from_device_chip() {
    let area = Rect::new(0, 0, 120, 50);
    let mut state = AppState::default();
    state.device.clock_source = Some(ClockSource::Internal);
    let chips = layouts::device_header_hit_areas(
        layouts::titlebar_layout(layouts::root_chunks(area)[0])[0],
        &state,
    );

    assert_eq!(
        mouse_action(area, &state, chips[1].x + 1, chips[1].y),
        Some(Intent::OpenSampleRateSelector)
    );
}

#[test]
fn mouse_action_does_not_open_sample_rate_selector_when_clock_is_external() {
    let area = Rect::new(0, 0, 120, 50);
    let mut state = AppState::default();
    state.device.clock_source = Some(ClockSource::Usb);
    let chips = layouts::device_header_hit_areas(
        layouts::titlebar_layout(layouts::root_chunks(area)[0])[0],
        &state,
    );

    assert_eq!(mouse_action(area, &state, chips[1].x + 1, chips[1].y), None);
}

#[test]
fn mouse_action_hits_visible_surface_tab_position() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let tabs = layouts::surface_tab_hit_areas(mixer[0]);

    assert_eq!(
        mouse_action(area, &AppState::default(), tabs[1].x + 1, tabs[1].y),
        Some(Intent::SelectSurface(Surface::Hp2))
    );
}

#[test]
fn mouse_action_hits_visible_output_dim_chip_position() {
    let area = Rect::new(0, 0, 120, 50);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let list_inner = layouts::inner_area(page[1]);
    let row_area = layouts::output_card_areas(list_inner)[0];
    let state = AppState::default();
    let dim = layouts::output_control_rects(row_area)[2];

    assert_eq!(
        mouse_action(area, &state, dim.x + dim.width / 2, dim.y),
        Some(Intent::ToggleOutputDim(0))
    );
}

#[test]
fn mouse_action_hits_visible_output_mute_chip_position_on_hp1() {
    let area = Rect::new(0, 0, 120, 50);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let list_inner = layouts::inner_area(page[1]);
    let row_area = layouts::output_card_areas(list_inner)[1];
    let state = AppState::default();
    let mute = layouts::output_control_rects(row_area)[3];

    assert_eq!(
        mouse_action(area, &state, mute.x + mute.width / 2, mute.y),
        Some(Intent::ToggleOutputMute(1))
    );
}

#[test]
fn mouse_action_selects_recent_query_reply_entry_when_raw_query_tab_is_open() {
    let area = Rect::new(0, 0, 120, 50);
    let layout = layouts::raw_page_layout(area);
    let sections = layouts::query_reply_history_layout(layout[2]);
    let inner = layouts::inner_area(sections[0]);
    let mut state = AppState::default();
    state.raw_view_open = true;
    state.selected_raw_packet = RawPacketTab::Query75;
    state.recent_query_reply_entries = vec![
        crate::app::QueryReplyLogEntry {
            summary: "0x75 03/05".to_string(),
            raw: vec![0x75, 0x05],
        },
        crate::app::QueryReplyLogEntry {
            summary: "0x75 03/06".to_string(),
            raw: vec![0x75, 0x06],
        },
    ];
    let point = (inner.x + 1, inner.y + 1);

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::SelectQueryReplyEntry(1))
    );
}

#[test]
fn mouse_action_hits_preamp_gain_up_button() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let cards = layouts::preamp_bar_layout(main[0]);
    let buttons = layouts::preamp_button_rects(cards[0], AppState::default().preamp.input1);
    let point = (buttons[1].x + buttons[1].width / 2, buttons[1].y);

    assert_eq!(
        mouse_action(area, &AppState::default(), point.0, point.1),
        Some(Intent::AdjustPreampGain {
            input: 0,
            increase: true,
        })
    );
}

#[test]
fn output_card_renders_arrow_adjust_buttons() {
    let rendered = styles::render_output_card(&AppState::default().outputs[0], true).to_string();

    assert!(rendered.contains(" ↑ "));
    assert!(rendered.contains(" ↓ "));
    assert!(!rendered.contains(" + "));
    assert!(!rendered.contains(" - "));
}

#[test]
fn preamp_controls_render_arrow_adjust_buttons() {
    let rendered =
        render::render_preamp_controls_text(AppState::default().preamp.input1).to_string();

    assert!(rendered.contains(" ↑ "));
    assert!(rendered.contains(" ↓ "));
    assert!(!rendered.contains(" + "));
    assert!(!rendered.contains(" - "));
}

#[test]
fn slider_wheel_action_adjusts_output_level_one_step() {
    let area = Rect::new(0, 0, 120, 50);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let card = layouts::output_card_areas(layouts::inner_area(page[1]))[0];
    let track = layouts::output_level_slider_rect(card);

    assert_eq!(
        slider_wheel_action(area, &AppState::default(), track.x, track.y, true),
        Some(Intent::AdjustOutputLevel {
            index: 0,
            increase: true,
        })
    );
}

#[test]
fn slider_wheel_action_adjusts_preamp_gain_one_step() {
    let area = Rect::new(0, 0, 120, 50);
    let card = layouts::preamp_bar_layout(
        layouts::mixer_main_layout(layouts::mixer_page_layout(layouts::root_chunks(area)[1])[0])[0],
    )[0];
    let track = layouts::preamp_gain_slider_rect(card);

    assert_eq!(
        slider_wheel_action(area, &AppState::default(), track.x, track.y, true),
        Some(Intent::AdjustPreampGain {
            input: 0,
            increase: true,
        })
    );
}

#[test]
fn slider_wheel_action_adjusts_mixer_pan_inside_strip_panel() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let list_inner = layouts::mixer_strip_panel_layout(mixer[1], false)[0];
    let card = layouts::mixer_strip_card_area(list_inner, 0);
    let track = layouts::mixer_pan_slider_rect(card);

    assert_eq!(
        slider_wheel_action(area, &AppState::default(), track.x, track.y, true),
        Some(Intent::AdjustMixerPan {
            index: 0,
            right: true,
        })
    );
}

#[test]
fn slider_wheel_action_adjusts_mixer_level_inside_strip_panel() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let list_inner = layouts::mixer_strip_panel_layout(mixer[1], false)[0];
    let card = layouts::mixer_strip_card_area(list_inner, 0);
    let track = layouts::mixer_level_slider_rect(card);

    assert_eq!(
        slider_wheel_action(area, &AppState::default(), track.x, track.y, true),
        Some(Intent::AdjustMixerLevel {
            index: 0,
            increase: true,
        })
    );
}

#[test]
fn slider_wheel_action_uses_wider_hitbox_for_thin_mixer_level_slider() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let list_inner = layouts::mixer_strip_panel_layout(mixer[1], false)[0];
    let card = layouts::mixer_strip_card_area(list_inner, 0);
    let track = layouts::mixer_level_slider_rect(card);

    assert_eq!(
        slider_wheel_action(
            area,
            &AppState::default(),
            track.x.saturating_sub(1),
            track.y,
            true
        ),
        Some(Intent::AdjustMixerLevel {
            index: 0,
            increase: true,
        })
    );
}

#[test]
fn mouse_action_hits_visible_output_level_slider_position() {
    let area = Rect::new(0, 0, 120, 50);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let card = layouts::output_card_areas(layouts::inner_area(page[1]))[0];
    let slider_row = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(card)[1];
    let slider_area = layouts::bounded_signal_area(slider_row);
    let label_width = layouts::SIGNAL_LABEL_WIDTH
        .min(slider_area.width.saturating_sub(1))
        .max(1);
    let track = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(label_width), Constraint::Min(1)])
        .split(slider_area)[1];

    assert_eq!(
        mouse_action(
            area,
            &AppState::default(),
            track.x + track.width.saturating_sub(1),
            track.y
        ),
        Some(Intent::SetOutputLevel { index: 0, step: 0 })
    );
}

#[test]
fn mouse_action_hits_visible_preamp_gain_slider_position() {
    let area = Rect::new(0, 0, 120, 50);
    let card = layouts::preamp_bar_layout(
        layouts::mixer_main_layout(layouts::mixer_page_layout(layouts::root_chunks(area)[1])[0])[0],
    )[0];
    let signal_area = layouts::preamp_card_inner_layout(card)[0];
    let gain_row = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(signal_area)[1];
    let slider_area = layouts::bounded_signal_area(gain_row);
    let label_width = layouts::SIGNAL_LABEL_WIDTH
        .min(slider_area.width.saturating_sub(1))
        .max(1);
    let track = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(label_width), Constraint::Min(1)])
        .split(slider_area)[1];

    assert_eq!(
        mouse_action(
            area,
            &AppState::default(),
            track.x + track.width.saturating_sub(1),
            track.y
        ),
        Some(Intent::SetPreampGain {
            input: 0,
            raw: 0x41
        })
    );
}

#[test]
fn mouse_action_hits_visible_mixer_pan_slider_position() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let list_inner = layouts::mixer_strip_panel_layout(mixer[1], false)[0];
    let card = layouts::mixer_strip_card_area(list_inner, 0);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(layouts::mixer_strip_inner_area(card));

    assert_eq!(
        mouse_action(
            area,
            &AppState::default(),
            rows[2].x + rows[2].width.saturating_sub(1),
            rows[2].y
        ),
        Some(Intent::SetMixerPan {
            index: 0,
            pan: PanState::right(),
        })
    );
}

#[test]
fn mouse_action_hits_visible_mixer_level_slider_position() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let list_inner = layouts::mixer_strip_panel_layout(mixer[1], false)[0];
    let card = layouts::mixer_strip_card_area(list_inner, 0);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(layouts::mixer_strip_inner_area(card));
    let combo = rows[5];
    let content_width = 6.min(combo.width);
    let content_area = Rect::new(
        combo.x + combo.width.saturating_sub(content_width) / 2,
        combo.y,
        content_width,
        combo.height,
    );
    let level = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(content_area)[2];

    assert_eq!(
        mouse_action(area, &AppState::default(), level.x, level.y),
        Some(Intent::SetMixerLevel { index: 0, level: 0 })
    );
}

#[test]
fn mouse_action_hits_visible_preamp_mode_chip_position() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let cards = layouts::preamp_bar_layout(main[0]);
    let state = AppState::default();
    let mode = layouts::preamp_button_rects(cards[0], state.preamp.input1)[2];

    assert_eq!(
        mouse_action(area, &state, mode.x + mode.width / 2, mode.y),
        Some(Intent::OpenPreampModeSelector(0))
    );
}

#[test]
fn mouse_action_picks_preamp_mode_from_selector_popup() {
    let area = Rect::new(0, 0, 120, 50);
    let mut state = AppState::default();
    state.selector_popup = Some(SelectorPopupState {
        kind: SelectorPopupKind::PreampMode { input: 0 },
    });
    let popup = layouts::assignment_picker_area(area);
    let inner = layouts::popup_list_inner_area(popup, "Preamp Mode");

    assert_eq!(
        mouse_action(area, &state, inner.x + 1, inner.y + 1),
        Some(Intent::PickPreampMode {
            input: 0,
            mode: PreampMode::Line,
        })
    );
}

#[test]
fn mouse_action_picks_first_assignment_from_first_popup_row() {
    let area = Rect::new(0, 0, 120, 50);
    let popup = layouts::assignment_picker_area(area);
    let inner = layouts::popup_list_inner_area(popup, "Assign CH 11");
    let mut state = AppState::default();
    state.assignment_picker = Some(AssignmentPickerState { strip: 11 });

    assert_eq!(
        mouse_action(area, &state, inner.x + 1, inner.y),
        Some(Intent::PickAssignment {
            strip: 11,
            assignment: MixerAssignment::Mute,
        })
    );
}

#[test]
fn preamp_control_row_keeps_leading_chip_padding_when_rendered() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 1));
    ratatui::widgets::Paragraph::new(render::render_preamp_controls_text(
        AppState::default().preamp.input1,
    ))
    .render(Rect::new(0, 0, 40, 1), &mut buffer);

    assert_eq!(buffer[(0, 0)].symbol(), " ");
    assert_eq!(buffer[(1, 0)].symbol(), "↓");
}

#[test]
fn mouse_action_hits_mixer_link_button_on_odd_strip() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let list_inner = layouts::mixer_strip_panel_layout(mixer[1], false)[0];
    let card = layouts::mixer_strip_card_area(list_inner, 0);
    let buttons = mouse::mixer_control_button_rects(card, true);
    let point = (buttons[0].x + buttons[0].width / 2, buttons[0].y);

    assert_eq!(
        mouse_action(area, &AppState::default(), point.0, point.1),
        Some(Intent::ToggleMixerLink(1))
    );
}

#[test]
fn mouse_action_hits_mixer_solo_button() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let list_inner = layouts::mixer_strip_panel_layout(mixer[1], false)[0];
    let card = layouts::mixer_strip_card_area(list_inner, 0);
    let buttons = mouse::mixer_control_button_rects(card, true);
    let point = (buttons[1].x + buttons[1].width / 2, buttons[1].y);

    assert_eq!(
        mouse_action(area, &AppState::default(), point.0, point.1),
        Some(Intent::ToggleMixerSolo(1))
    );
}

#[test]
fn mouse_action_hits_visible_mixer_solo_chip_position() {
    let area = Rect::new(0, 0, 120, 50);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let list_inner = layouts::mixer_strip_panel_layout(mixer[1], false)[0];
    let card = layouts::mixer_strip_card_area(list_inner, 0);
    let state = AppState::default();
    let buttons = mouse::mixer_control_button_rects(card, true);
    let point = (buttons[1].x + buttons[1].width / 2, buttons[1].y);

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::ToggleMixerSolo(1))
    );
}

#[test]
fn mouse_action_opens_assignment_picker_from_src_button() {
    let area = Rect::new(0, 0, 120, 60);
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout(page[0]);
    let mixer = layouts::mixer_layout(main[1]);
    let list_inner = layouts::mixer_strip_panel_layout(mixer[1], false)[0];
    let mut state = AppState::default();
    state.selected_channel = 3;
    state.mixer_channels[0][3].assignment = Some(MixerAssignment::ComputerPlay(2));
    let card = layouts::mixer_strip_card_area(list_inner, 3);
    let (_, source_rect) = layouts::mixer_header_chip_rects(card, "C2");
    let point = (source_rect.x + source_rect.width / 2, source_rect.y);

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::OpenAssignmentPicker(4))
    );
}

#[test]
fn mouse_action_picks_assignment_from_modal() {
    let area = Rect::new(0, 0, 120, 50);
    let popup = layouts::assignment_picker_area(area);
    let inner = layouts::popup_list_inner_area(popup, "Assign CH 11");
    let mut state = AppState::default();
    state.assignment_picker = Some(AssignmentPickerState { strip: 11 });

    assert_eq!(
        mouse_action(area, &state, inner.x + inner.width / 2, inner.y + 4),
        Some(Intent::PickAssignment {
            strip: 11,
            assignment: MixerAssignment::ComputerPlay(2),
        })
    );
}

#[test]
fn status_panel_surfaces_grounded_non_metadata_startup_queries() {
    let mut state = AppState::default();
    state.device.startup_query_summaries[1] =
        Some("Capability/default block: 3 bytes [aa bb cc]".to_string());
    state.device.startup_query_summaries[2] =
        Some("Status/capability value: 1 bytes [12]".to_string());

    let lines = vec![
        Line::from(format!(
            "Startup: {}",
            state.startup_query_summary(0x00).unwrap_or_default()
        )),
        Line::from(format!(
            "         {}",
            state.startup_query_summary(0x11).unwrap_or_default()
        )),
    ];
    let rendered = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(rendered.contains("Capability/default block: 3 bytes [aa bb cc]"));
    assert!(rendered.contains("Status/capability value: 1 bytes [12]"));
}

#[test]
fn mixer_strip_line_includes_assignment_pan_and_link() {
    let mut state = AppState::default();
    state.mixer_channels[0][10].assignment = Some(MixerAssignment::ComputerPlay(8));
    state.mixer_channels[0][10].pan = PanState::from_raw(0x3e);
    state.mixer_channels[0][10].linked = Some(true);
    state.mixer_channels[0][10].level = Some(0x10);
    state.mixer_channels[0][10].meter = Some(0x08);
    state.mixer_channels[0][10].muted = Some(false);

    let channel = &state.mixer_channels[0][10];
    let line = render::render_mixer_strip_line(&state, 10, channel);

    assert!(line.contains("Computer Play 8"));
    assert!(line.contains("pan=R30"));
    assert!(line.contains("link=on"));
    assert!(line.contains("meter="));
}

#[test]
fn mixer_strip_line_renders_meter_separately_from_level_value() {
    let mut state = AppState::default();
    state.mixer_channels[0][0].level = Some(0x00);
    state.mixer_channels[0][0].meter = Some(0x30);
    state.mixer_channels[0][0].muted = Some(false);

    let line = render::render_mixer_strip_line(&state, 0, &state.mixer_channels[0][0]);

    assert!(line.contains("level=0 dB"));
    assert!(line.contains("meter=-48 dB"));
}

#[test]
fn mixer_strip_line_hides_meter_value_below_ui_floor() {
    let mut state = AppState::default();
    state.mixer_channels[0][0].level = Some(0x00);
    state.mixer_channels[0][0].meter = Some(0x60);
    state.mixer_channels[0][0].muted = Some(false);

    let line = render::render_mixer_strip_line(&state, 0, &state.mixer_channels[0][0]);

    assert!(line.contains("meter= mute=off"));
}

#[test]
fn mixer_strip_line_renders_newly_grounded_pair_link() {
    let mut state = AppState::default();
    let target = MixerLinkTarget::from_channel(MixerSurface::Mix1, 7).expect("grounded pair");
    state.mixer_channels[target.mixer.index()][target.left_channel as usize - 1].linked =
        Some(true);
    state.mixer_channels[target.mixer.index()][target.left_channel as usize - 1].assignment =
        Some(MixerAssignment::SpdifIn(1));

    let line = render::render_mixer_strip_line(
        &state,
        target.left_channel as usize - 1,
        &state.mixer_channels[target.mixer.index()][target.left_channel as usize - 1],
    );

    assert!(line.contains("CH 07"));
    assert!(line.contains("SPDIF In 1"));
    assert!(line.contains("link=on"));
}

#[test]
fn experimental_pair_state_line_surfaces_mix1_mirrored_lanes() {
    let mut state = AppState::default();
    let mut frame = vec![0_u8; 320];
    frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    frame[0x10 + 0x6a] = 0x0f;
    frame[0x10 + 0xda] = 0x0a;
    frame[0x10 + 0xdb] = 0x05;
    frame[0x10 + 0xdc] = 0x0a;
    frame[0x10 + 0xdd] = 0x05;
    frame[0x10 + 0xe0] = 0x60;
    frame[0x10 + 0xe1] = 0x60;
    state.latest_raw_73 = Some(frame);

    let line = render::render_experimental_pair_state_line(&state);

    assert!(line.contains("MIX 1"));
    assert!(line.contains("L ███████░ -10 dB"));
    assert!(line.contains("R ███████░  -5 dB"));
}

#[test]
fn experimental_pair_state_line_surfaces_mix2_compact_lanes() {
    let mut state = AppState::default();
    let mut frame = vec![0_u8; 320];
    frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    frame[0x10 + 0x6a] = 0x0c;
    frame[0x10 + 0xde] = 0x00;
    frame[0x10 + 0xdf] = 0x06;
    frame[0x10 + 0xe0] = 0x60;
    frame[0x10 + 0xe1] = 0x60;
    state.latest_raw_73 = Some(frame);

    let line = render::render_experimental_pair_state_line(&state);

    assert!(line.contains("MIX 2"));
    assert!(line.contains("L ████████   0 dB"));
    assert!(line.contains("R ███████░  -6 dB"));
}

#[test]
fn experimental_pair_state_line_surfaces_no_signal_family_as_pending_meter() {
    let mut state = AppState::default();
    let mut frame = vec![0_u8; 320];
    frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    frame[0x10 + 0x6a] = 0x0c;
    frame[0x10 + 0xde] = 0x5a;
    frame[0x10 + 0xdf] = 0x5a;
    frame[0x10 + 0x6e] = 0x60;
    frame[0x10 + 0x8e] = 0x60;
    frame[0x10 + 0xe2] = 0x60;
    state.latest_raw_73 = Some(frame);

    let line = render::render_experimental_pair_state_line(&state);

    assert!(line.contains("MIX 2"));
    assert!(line.contains("L ░░░░░░░░  -∞ dB"));
    assert!(line.contains("R ░░░░░░░░  -∞ dB"));
}

#[test]
fn experimental_pair_state_line_keeps_unknown_meter_bytes_visible() {
    let mut state = AppState::default();
    let mut frame = vec![0_u8; 320];
    frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
    frame[0x10 + 0x6a] = 0x0c;
    frame[0x10 + 0xde] = 0x12;
    frame[0x10 + 0xdf] = 0x34;
    state.latest_raw_73 = Some(frame);

    let line = render::render_experimental_pair_state_line(&state);

    assert!(line.contains("L ██████░░ -18 dB"));
    assert!(line.contains("R █░░░░░░░ -52 dB"));
}

#[test]
fn observed_meter_label_mentions_raw_value() {
    let mut input = PreampInputState::from_raw(0x2a, 0x00);
    input.observed_meter = Some(0x30);

    assert_eq!(render::observed_meter_label(input), "obs meter -48 dB");
}

#[test]
fn observed_meter_label_mentions_pending_state() {
    assert_eq!(
        render::observed_meter_label(PreampInputState::from_raw(0x2a, 0x00)),
        ""
    );
}

#[test]
fn observed_meter_label_hides_values_below_ui_floor() {
    let mut input = PreampInputState::from_raw(0x2a, 0x00);
    input.observed_meter = Some(0x60);

    assert_eq!(render::observed_meter_label(input), "");
}

#[test]
#[ignore = "benchmark"]
fn perf_draw_full_frame() {
    const FRAMES: usize = 2_000;

    let backend = TestBackend::new(140, 42);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut state = AppState::default();
    state.connection.connected = true;
    state.device.metadata = Some(antelope_protocol::DeviceMetadata {
        product_name: "Zen Go Synergy Core".to_string(),
        serial: "4502721001300".to_string(),
        hardware_version: "6.6".to_string(),
    });
    state.device.sample_rate = Some(SampleRate::Hz48000);
    state.device.clock_source = Some(ClockSource::Internal);
    state.selected_channel = 7;
    state.focus = FocusArea::Mixer;
    state.mixer_channels[MixerSurface::Mix1.index()][7].level = Some(0x18);
    state.mixer_channels[MixerSurface::Mix1.index()][7].meter = Some(0x24);
    state.mixer_channels[MixerSurface::Mix1.index()][7].assignment =
        Some(MixerAssignment::ComputerPlay(4));
    state.mixer_channels[MixerSurface::Mix1.index()][7].soloed = Some(true);
    state.mixer_channels[MixerSurface::Mix1.index()][7].linked = Some(true);

    let started = Instant::now();
    for _ in 0..FRAMES {
        terminal
            .draw(|frame| draw(frame, &state))
            .expect("draw full frame");
    }
    let elapsed = started.elapsed();

    println!(
        "draw full frame: frames={FRAMES} elapsed_ms={} ns_per_frame={}",
        elapsed.as_millis(),
        elapsed.as_nanos() / FRAMES as u128
    );
}
