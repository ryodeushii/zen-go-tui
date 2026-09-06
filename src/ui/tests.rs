use std::time::Instant;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use ratatui::Terminal;

use crate::app::{
    AppState, AssignmentPickerState, Controller, FocusArea, Intent, RawMapScope, RawPacketTab,
    SelectorPopupKind, SelectorPopupState,
};
use antelope_protocol::{
    ClockSource, DynamicMeterState, MixerAddress, MixerAssignment, MixerChannelState,
    MixerLinkTarget, MixerSurface, OutputMode, OutputState, OutputTarget, PanState,
    PreampInputState, PreampMode, RuntimeMeterTarget, SampleRate, Surface, OFFSET_MIX1_LANE_A,
    OFFSET_MIX1_LANE_B, OFFSET_MIX2_LANE_A, OFFSET_MIX2_LANE_B, OFFSET_SURFACE_SELECTOR,
    SNAPSHOT_PAYLOAD_OFFSET, SURFACE_CODE_HP2, SURFACE_CODE_MONITOR_HP1,
};

use crate::device::ProfileCatalog;
use crate::transport::MockTransport;

use super::*;

fn zen_go_state() -> AppState {
    let catalog = ProfileCatalog::builtin();
    let entry = catalog
        .entries()
        .iter()
        .find(|entry| entry.id == "zen_go_sc")
        .expect("Zen Go profile");
    AppState::from_entry(entry)
}

fn zen_mixer_controls(
    area: Rect,
    state: &AppState,
    index: usize,
) -> layouts::DynamicMixerControlRects {
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout_for_state(page[0], state);
    let mixer = layouts::mixer_layout(main[1]);
    let inner = layouts::mixer_strip_panel_layout(mixer[1], false)[0];
    let (start, end) = layouts::mixer_strip_visible_bounds(inner, state);
    let strip_area = layouts::mixer_input_strip_area(inner, state);
    let card = layouts::dynamic_mixer_strip_card_area(
        strip_area,
        state,
        index.saturating_sub(start),
        end.saturating_sub(start),
    );
    let surface = state.mixers()[state.mixer.surface_index].surface;
    layouts::dynamic_mixer_control_rects(
        card,
        state,
        MixerAddress {
            surface,
            strip: state.mixers()[state.mixer.surface_index].strips[index].strip,
        },
    )
    .expect("mixer controls")
}

fn afx_routing_source_label(assignment: Option<MixerAssignment>) -> String {
    assignment
        .map(|a| a.label().to_string())
        .unwrap_or_else(|| "?".to_string())
}

fn render_afx_routing_text(state: &AppState) -> Text<'static> {
    let assignments = &state.mixer.channels[MixerSurface::Mix1.index()];
    let mut lines = vec![
        Line::from(vec![styles::chip("ROUTING", Color::Black, Color::LightMagenta)]),
        Line::from(""),
        Line::from("Zen Go USB recordings mirror mixer strip assignments instead of using a separate routing matrix."),
        Line::from("This view reformats shared CH 01-08 assignments into the 4 stereo recording pairs exposed to the host."),
        Line::from(""),
        Line::from(vec![
            Span::styled("PAIR    ", styles::subdued_style()),
            Span::styled("LEFT", styles::strong_style(Color::LightCyan)),
            Span::styled("                           ", styles::subdued_style()),
            Span::styled("RIGHT", styles::strong_style(Color::LightCyan)),
        ]),
    ];

    for pair in 0..4 {
        let left = &assignments[pair * 2];
        let right = &assignments[pair * 2 + 1];
        lines.push(Line::from(format!(
            "USB {:>1}/{:>1}  Zen Go Recording {:>1} <- {:<18}  Zen Go Recording {:>1} <- {}",
            left.channel,
            right.channel,
            left.channel,
            afx_routing_source_label(left.assignment),
            right.channel,
            afx_routing_source_label(right.assignment),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("STATUS ", styles::subdued_style()),
        Span::styled(
            state.ui.last_message.clone(),
            styles::strong_style(Color::LightCyan),
        ),
    ]));
    Text::from(lines)
}

fn render_mixer_strip_controls(
    _state: &AppState,
    _index: usize,
    channel: &antelope_protocol::MixerChannelState,
) -> String {
    let mute = channel
        .muted
        .map(|value| if value { "on" } else { "off" })
        .unwrap_or("?");
    let src = channel
        .assignment
        .map(|value| value.label().to_string())
        .unwrap_or_else(|| "assignment?".to_string());
    let solo = channel
        .soloed
        .map(|value| if value { "on" } else { "off" })
        .unwrap_or("?");
    let link = if channel.channel % 2 == 1 {
        let value = channel
            .linked
            .map(|flag| if flag { "on" } else { "off" })
            .unwrap_or("?");
        format!(" [Link {}]", value)
    } else {
        String::new()
    };
    format!("    [Mute {}] [Solo {}]{} [Src {}]", mute, solo, link, src)
}

fn observed_meter_label(input: PreampInputState) -> String {
    match input.observed_meter {
        Some(_) => input
            .observed_meter_db()
            .map(|value| format!("obs meter {} dB", value))
            .unwrap_or_default(),
        None => String::new(),
    }
}

fn render_mixer_strip_line(
    state: &AppState,
    index: usize,
    channel: &antelope_protocol::MixerChannelState,
) -> String {
    let selected = state.ui.focus == FocusArea::Mixer && state.mixer.selected_channel == index;
    let bar = channel
        .meter_ratio()
        .or_else(|| channel.gain_ratio())
        .map(|ratio| styles::render_symbol_bar(ratio, 8, '|', '.'))
        .unwrap_or_else(|| "........".to_string());
    let assignment = channel
        .assignment
        .map(|value| value.label().to_string())
        .unwrap_or_else(|| "assignment?".to_string());
    let pan = channel.pan.display_percent();
    let pan_label = if pan < 0 {
        format!("L{}", pan.unsigned_abs())
    } else if pan > 0 {
        format!("R{}", pan)
    } else {
        "C".to_string()
    };
    format!(
        "CH {:02} {:<8} src={:<16} level={} meter={} mute={} solo={} pan={} link={} {}",
        channel.channel,
        bar,
        assignment,
        channel
            .display_db()
            .map(|value| format!("{} dB", value))
            .unwrap_or_else(|| "undecoded".to_string()),
        channel
            .meter_db()
            .map(|value| format!("{} dB", value))
            .or_else(|| channel.meter.map(|_| String::new()))
            .unwrap_or_else(|| "undecoded".to_string()),
        channel
            .muted
            .map(|value| if value { "on" } else { "off" })
            .unwrap_or("undecoded"),
        channel
            .soloed
            .map(|value| if value { "on" } else { "off" })
            .unwrap_or("undecoded"),
        pan_label,
        channel
            .linked
            .map(|value| if value { "on" } else { "off" })
            .unwrap_or("unknown"),
        if selected { "←" } else { "" }
    )
}

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
    state.ui.focus = FocusArea::Mixer;
    state.mixer.selected_channel = 10;
    state.mixer.channels[0][10] = MixerChannelState {
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
                &state.mixer.channels[0][10],
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
    state.ui.focus = FocusArea::Mixer;
    state.mixer.selected_channel = 0;
    state.mixer.channels[0][0].level = Some(0x00);
    state.mixer.channels[0][0].meter = Some(0x60);

    let rendered = render_buffer(
        Rect::new(0, 0, 72, layouts::mixer_strip_height()),
        |area, buffer| {
            render::render_mixer_strip_widget(area, buffer, &state, 0, &state.mixer.channels[0][0]);
        },
    );

    assert!(rendered.contains(" -∞ dB"));
}

#[test]
fn mixer_strip_widget_keeps_db_scale_markers_in_wide_area() {
    let mut state = AppState::default();
    state.ui.focus = FocusArea::Mixer;
    state.mixer.selected_channel = 0;
    state.mixer.channels[0][0].level = Some(0x00);
    state.mixer.channels[0][0].meter = Some(0x10);

    let rendered = render_buffer(
        Rect::new(0, 0, 120, layouts::mixer_strip_height()),
        |area, buffer| {
            render::render_mixer_strip_widget(area, buffer, &state, 0, &state.mixer.channels[0][0]);
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
    state.mixer.surface = Surface::Hp2;
    state.output.selected = 1;
    state.ui.last_message = "Applied dim change".to_string();

    let rendered = render::render_status_strip(&state).to_string();

    assert!(!rendered.contains("STATUS"));
    assert!(!rendered.contains("Applied dim change"));
    assert_eq!(rendered, render::render_mix_meter_state_line(&state));
}

#[test]
fn mix_meter_extracts_selected_profile_lanes() {
    let mut state = zen_go_state();
    state.meters = vec![
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 0,
            value: 0x0a,
        },
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 1,
            value: 0x05,
        },
    ];

    let meter = mouse::mix_meter(&state).expect("selected profile meter");
    assert_eq!(meter.name, state.mixers()[0].name);
    assert_eq!(
        meter
            .lanes
            .iter()
            .map(|lane| (lane.lane, lane.value))
            .collect::<Vec<_>>(),
        vec![(0, 0x0a), (1, 0x05)]
    );
}

#[test]
fn zen_go_mix_meter_follows_local_surface_and_ignores_stale_snapshot_selector() {
    let mut state = zen_go_state();
    state.meters = vec![
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 0,
            value: 0x0a,
        },
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 1,
            value: 0x05,
        },
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 1,
            lane: 0,
            value: 0x3c,
        },
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 1,
            lane: 1,
            value: 0x2a,
        },
    ];
    let mut frame = [0_u8; 320];
    frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_SURFACE_SELECTOR] = SURFACE_CODE_HP2;
    frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX1_LANE_A] = 0xaa;
    frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX1_LANE_B] = 0xab;
    frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX2_LANE_A] = 0xba;
    frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX2_LANE_B] = 0xbb;
    state.raw_view.latest_raw_73 = Some(frame.to_vec());

    for (surface_index, stale_snapshot_selector, expected_values) in [
        (0, SURFACE_CODE_HP2, vec![(0, 0x0a), (1, 0x05)]),
        (1, SURFACE_CODE_MONITOR_HP1, vec![(0, 0x3c), (1, 0x2a)]),
    ] {
        state.mixer.surface_index = surface_index;
        state.raw_view.latest_raw_73.as_mut().unwrap()
            [SNAPSHOT_PAYLOAD_OFFSET + OFFSET_SURFACE_SELECTOR] = stale_snapshot_selector;

        let meter = mouse::mix_meter(&state).expect("selected profile meter");
        assert_eq!(
            meter
                .lanes
                .iter()
                .map(|lane| (lane.lane, lane.value))
                .collect::<Vec<_>>(),
            expected_values
        );
        let line = render::render_mix_meter_state_line(&state);
        assert!(line.starts_with(&meter.name));
        for lane in &meter.lanes {
            assert!(line.contains(&super::widgets::signals::render_mix_meter(lane.value)));
        }
    }
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
fn mixer_strip_panel_layout_reserves_only_available_meter_lanes() {
    let layout = layouts::mixer_strip_panel_layout_for_meter_lanes(Rect::new(0, 0, 80, 14), 1);

    assert_eq!(layout[1].height, 1);
    assert_eq!(
        layout[0].height + layout[1].height,
        layouts::inner_area(Rect::new(0, 0, 80, 14)).height
    );
}

#[test]
fn mixer_list_mouse_action_ignores_embedded_mix_meter_rows() {
    let mut state = zen_go_state();
    state.meters = vec![
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 0,
            value: 0x0a,
        },
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 1,
            value: 0x05,
        },
    ];

    let mixer = layouts::mixer_layout(Rect::new(0, 0, 100, 20));
    let meter_area = layouts::mixer_strip_panel_layout_for_meter_lanes(mixer[1], 2)[1];

    assert_eq!(
        mouse::mixer_list_mouse_action(mixer[1], &state, (meter_area.x + 1, meter_area.y)),
        None
    );
}

#[test]
fn mix_meter_widget_renders_two_row_stereo_bar_and_fixed_db_labels() {
    let meter = mouse::MixMeterState {
        name: "MIX 1".into(),
        lanes: vec![
            DynamicMeterState {
                target: RuntimeMeterTarget::MixMaster,
                target_index: 0,
                lane: 0,
                value: 0x00,
            },
            DynamicMeterState {
                target: RuntimeMeterTarget::MixMaster,
                target_index: 0,
                lane: 1,
                value: 0x3c,
            },
        ],
    };
    let rendered = render_buffer(Rect::new(0, 0, 56, 2), |area, buffer| {
        render::render_mix_meter_widget(area, buffer, &meter);
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
fn mix_meter_widget_renders_only_available_lane() {
    let meter = mouse::MixMeterState {
        name: "Mix 1".into(),
        lanes: vec![DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 0,
            value: 0x3c,
        }],
    };
    let rendered = render_buffer(Rect::new(0, 0, 56, 2), |area, buffer| {
        render::render_mix_meter_widget(area, buffer, &meter);
    });

    assert!(rendered.contains("Lane 1"));
    assert!(!rendered.contains(" R "));
    assert_eq!(
        rendered
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        1
    );
}

#[test]
fn device_header_keeps_selected_runtime_profile_name_over_reported_product() {
    let mut state = AppState::default();
    state.ui_profile.device_name = "External Runtime Profile".to_string();
    state.device.status.metadata = Some(antelope_protocol::DeviceMetadata {
        product_name: "Reported HID Product".to_string(),
        serial: "EXT-1".to_string(),
        hardware_version: "1.0".to_string(),
    });

    let rendered = render::render_device_header(&state).to_string();

    assert!(rendered.contains("External Runtime Profile"));
}

#[test]
fn device_header_surfaces_serial_and_hw_without_duplicate_status_line() {
    let mut state = AppState::default();
    state.device.status.metadata = Some(antelope_protocol::DeviceMetadata {
        product_name: "Zen Go Synergy Core".to_string(),
        serial: "1234567890".to_string(),
        hardware_version: "6.6".to_string(),
    });
    state.device.status.sample_rate = Some(SampleRate::Hz48000);
    state.device.status.clock_source = Some(ClockSource::Internal);
    state.device.status.lock_known = true;
    state.device.status.locked = Some(true);

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
    state.device.status.metadata = Some(antelope_protocol::DeviceMetadata {
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
    state.device.status.sample_rate = Some(SampleRate::Hz96000);
    state.device.status.sample_rate_hz = Some(44_100);

    let rendered = render::render_device_header(&state).to_string();

    assert!(rendered.contains("44.1 kHz"));
    assert!(!rendered.contains("96000 Hz"));
}

#[test]
fn afx_page_renders_usb_recording_pairs_from_mixer_assignments() {
    let mut state = AppState::default();
    state.mixer.channels[MixerSurface::Mix1.index()][0].assignment =
        Some(MixerAssignment::Preamp(1));
    state.mixer.channels[MixerSurface::Mix1.index()][1].assignment =
        Some(MixerAssignment::Preamp(2));
    for channel in 2..8 {
        state.mixer.channels[MixerSurface::Mix1.index()][channel].assignment =
            Some(MixerAssignment::Mute);
    }

    let rendered = render_afx_routing_text(&state).to_string();

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

    state.device.connection.connected = true;
    assert_eq!(render::connection_badge_color(&state), Color::LightGreen);

    state.device.connection.connected = false;
    state.device.connection.last_snapshot_at =
        Some(Instant::now() - std::time::Duration::from_secs(3));
    assert_eq!(render::connection_badge_color(&state), Color::LightRed);
}

#[test]
fn mixer_strip_rendering_includes_solo_state() {
    let mut state = AppState::default();
    state.ui.focus = crate::app::FocusArea::Mixer;
    state.mixer.selected_channel = 0;
    state.mixer.channels[MixerSurface::Mix1.index()][0].soloed = Some(true);

    let line = render_mixer_strip_line(
        &state,
        0,
        &state.mixer.channels[MixerSurface::Mix1.index()][0],
    );
    let controls = render_mixer_strip_controls(
        &state,
        0,
        &state.mixer.channels[MixerSurface::Mix1.index()][0],
    );

    assert!(line.contains("solo=on"));
    assert!(controls.contains("[Solo on]"));
}

#[test]
fn semantic_dump_uses_coverage_color_and_composes_baseline_marker() {
    let bytes = [0x42; 320];
    let map = raw_map::build_raw_packet_map(RawPacketTab::State73, &bytes);
    let mut baseline = bytes;
    baseline[0x13] = 0x00;

    let dump = render::render_full_packet_dump(&bytes, Some(&baseline), &map, RawMapScope::Base);
    let clock_span = &dump.lines[1].spans[1 + (0x13 - 0x10)];

    let expected_style = crate::terminal::adapt_style(
        Style::default()
            .fg(Color::Green)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    );
    assert_eq!(clock_span.style, expected_style);
}

#[test]
fn all_map_scope_emphasizes_mapped_entries_and_subdues_unknown_entries() {
    let map = raw_map::build_raw_packet_map(RawPacketTab::State73, &[0x42; 320]);
    let entry_for = |needle: &str| {
        map.entries_for_scope(RawMapScope::All)
            .into_iter()
            .find(|entry| entry.label.contains(needle))
            .unwrap_or_else(|| panic!("missing map entry {needle:?}"))
    };

    let mapped = render::raw_map_entry_style(entry_for("status flags 0-1"), RawMapScope::All);
    assert_eq!(mapped.fg, Some(Color::Green));
    assert!(!mapped.add_modifier.contains(Modifier::DIM));

    let unmapped = render::raw_map_entry_style(entry_for("unmapped report"), RawMapScope::All);
    assert_eq!(unmapped.fg, Some(Color::Red));
    assert!(unmapped.add_modifier.contains(Modifier::DIM));
    assert!(!unmapped.add_modifier.contains(Modifier::BOLD));

    let padding =
        render::raw_map_entry_style(entry_for("fixed snapshot padding"), RawMapScope::All);
    assert_eq!(padding.fg, Some(Color::DarkGray));
    assert!(padding.add_modifier.contains(Modifier::DIM));

    let selected = render::raw_map_entry_style(entry_for("unmapped report"), RawMapScope::Unmapped);
    assert_eq!(selected.fg, Some(Color::LightRed));
    assert!(!selected.add_modifier.contains(Modifier::DIM));
}

#[test]
fn all_and_unmapped_scopes_emphasize_only_their_selected_coverage() {
    let bytes = [0x42; 320];
    let map = raw_map::build_raw_packet_map(RawPacketTab::State73, &bytes);
    let all_unmapped = map.classify(0x18, RawMapScope::All);
    let all_padding = map.classify(0xf6, RawMapScope::All);
    let selected_unmapped = map.classify(0x18, RawMapScope::Unmapped);
    let excluded_output = map.classify(0x1c, RawMapScope::Base);

    assert_eq!(all_unmapped.coverage, raw_map::Coverage::Unmapped);
    assert!(!all_unmapped.selected);
    assert_eq!(
        styles::raw_coverage_color(all_unmapped.coverage, all_unmapped.selected),
        Color::Red
    );
    assert_eq!(all_padding.coverage, raw_map::Coverage::Padding);
    assert!(!all_padding.selected);
    assert!(selected_unmapped.selected);
    assert_eq!(
        styles::raw_coverage_color(selected_unmapped.coverage, selected_unmapped.selected),
        Color::LightRed
    );
    assert!(!excluded_output.selected);

    let mut baseline = bytes;
    baseline[0x18] = 0;
    let all_dump = render::render_full_packet_dump(&bytes, Some(&baseline), &map, RawMapScope::All);
    let all_unmapped_hex = &all_dump.lines[1].spans[1 + (0x18 - 0x10) + 1];
    let all_unmapped_ascii = &all_dump.lines[1].spans[19 + (0x18 - 0x10)];
    let expected_subdued = crate::terminal::adapt_style(
        Style::default()
            .fg(Color::Red)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::DIM | Modifier::UNDERLINED),
    );
    assert_eq!(all_unmapped_hex.style, expected_subdued);
    assert_eq!(all_unmapped_ascii.style, expected_subdued);

    let unmapped_dump = render::render_full_packet_dump(&bytes, None, &map, RawMapScope::Unmapped);
    let selected_ascii = &unmapped_dump.lines[1].spans[19 + (0x18 - 0x10)];
    let expected_selected = crate::terminal::adapt_style(
        Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
    );
    assert_eq!(selected_ascii.style, expected_selected);
}

#[test]
fn map_text_contains_exact_offsets_labels_and_overlap_note() {
    let map = raw_map::build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
    let text = render::render_raw_map_text(&map, RawMapScope::Mixer, false).to_string();

    assert!(text.contains("report 0x9f"));
    assert!(text.contains("payload 0x8f"));
    assert!(text.contains("active mixer CH01/CH02 link correlation"));
    assert!(text.contains("OVERLAP"));
}

#[test]
fn hex_dump_renders_offset_and_ascii() {
    let bytes = [0x83, 0x00, 0x41, 0x42, 0x0a];
    let map = raw_map::build_raw_packet_map(RawPacketTab::Auxiliary, &bytes);
    let dump = render::render_full_packet_dump(&bytes, None, &map, RawMapScope::All);
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
    let bytes = [0x00];
    let map = raw_map::build_raw_packet_map(RawPacketTab::State73, &bytes);
    let dump = render::render_full_packet_dump(&bytes, None, &map, RawMapScope::All);
    let first = &dump.lines[0];
    assert!(first.spans[0].style.add_modifier.contains(Modifier::BOLD));
    let expected_zero_style = crate::terminal::adapt_style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::DIM | Modifier::BOLD),
    );
    assert_eq!(first.spans[1].style, expected_zero_style);
}

#[test]
fn compact_map_truncates_notes_without_truncating_offsets_or_labels() {
    let map = raw_map::build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
    let text = render::render_raw_map_text(&map, RawMapScope::Mixer, true).to_string();
    let line = text
        .lines()
        .find(|line| line.contains("active mixer CH01/CH02 link correlation"))
        .expect("link-correlation map line");

    assert!(line.starts_with("report 0x9f,0xdf,0xea..0xf0 / payload 0x8f,0xcf,0xda..0xe0"));
    assert!(line.contains("active mixer CH01/CH02 link correlation"));
    assert!(line.ends_with("OVERLAP"));
}

#[test]
fn selected_query_reply_bytes_prefers_selected_history_entry() {
    let fallback = [0x75; 320];
    let selected = [0x42; 320];
    let mut state = AppState::default();
    state.raw_view.recent_query_reply_entries = vec![crate::app::QueryReplyLogEntry {
        summary: "selected".to_string(),
        raw: selected.to_vec(),
    }];
    state.raw_view.selected_query_reply_entry = Some(0);

    assert_eq!(
        render::selected_query_reply_bytes(&fallback, &state),
        selected.as_slice()
    );
}

#[test]
fn raw_page_renders_selected_query_reply_when_latest_reply_is_absent() {
    let mut state = AppState::default();
    state.popup.raw_view_open = true;
    state.raw_view.selected_tab = RawPacketTab::Query75;
    state.raw_view.recent_query_reply_entries = vec![crate::app::QueryReplyLogEntry {
        summary: "selected reply".to_string(),
        raw: [0x42; 320].to_vec(),
    }];
    state.raw_view.selected_query_reply_entry = Some(0);

    let mut terminal = Terminal::new(TestBackend::new(120, 50)).unwrap();
    terminal.draw(|frame| render::draw(frame, &state)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut rendered = String::new();
    for y in 0..50 {
        for x in 0..120 {
            rendered.push_str(buffer[(x, y)].symbol());
        }
        rendered.push('\n');
    }

    assert!(rendered.contains("0000: 42"));
    assert!(!rendered.contains("Waiting for first 0x75 query reply"));
}

#[test]
fn query_reply_panel_includes_recent_reply_log() {
    let mut state = AppState::default();
    let mut raw1 = [0_u8; 320];
    raw1[..2].copy_from_slice(&[0x75, 0x05]);
    let mut raw2 = [0_u8; 320];
    raw2[..2].copy_from_slice(&[0x75, 0x06]);
    state.raw_view.recent_query_reply_entries = vec![
        crate::app::QueryReplyLogEntry {
            summary: "0x75 03/05 [64 bytes] 05 00 00 00 01 01 00 01".to_string(),
            raw: raw1.to_vec(),
        },
        crate::app::QueryReplyLogEntry {
            summary: "0x75 03/06 [64 bytes] 06 03 00 03 01 03 02 03".to_string(),
            raw: raw2.to_vec(),
        },
    ];
    state.raw_view.selected_query_reply_entry = Some(1);

    let text = render::render_query_reply_panel(&[0x75, 0x00, 0x00, 0x00], &state).to_string();

    assert!(text.contains("0000: 75 06"));
}

#[test]
fn query_request_panel_includes_recent_request_log() {
    let mut state = AppState::default();
    state.raw_view.recent_query_request_log =
        vec!["0x74 03/05".to_string(), "0x74 03/06".to_string()];

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
fn selecting_packet_resets_unsupported_scope_and_scroll() {
    let mut state = AppState::default();
    state.raw_view.raw_map_scope = RawMapScope::Mixer;
    state.raw_view.raw_dump_scroll = 7;
    state.raw_view.raw_map_scroll = 4;

    state.raw_view.select_tab(RawPacketTab::DeviceNotification);

    assert_eq!(state.raw_view.raw_map_scope, RawMapScope::All);
    assert_eq!(state.raw_view.raw_dump_scroll, 0);
    assert_eq!(state.raw_view.raw_map_scroll, 0);
}

#[test]
fn raw_layout_has_packet_and_scope_rows() {
    let rows = layouts::raw_page_layout(Rect::new(0, 0, 140, 40));
    assert_eq!(rows.len(), 5);
    assert!(rows[1].height >= 3);
    assert!(rows[2].height >= 3);
    assert!(rows[3].width >= 140);
}

#[test]
fn raw_scope_mouse_action_selects_unmapped_scope() {
    let area = Rect::new(0, 0, 140, 40);
    let rows = layouts::raw_page_layout(area);
    let scopes = layouts::raw_scope_hit_areas(rows[2], RawPacketTab::State73);
    let point = (scopes[5].x + 1, scopes[5].y);
    let mut state = AppState::default();
    state.popup.raw_view_open = true;
    state.raw_view.selected_tab = RawPacketTab::State73;

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::SelectRawMapScope(RawMapScope::Unmapped))
    );
}

#[test]
fn narrow_raw_content_reserves_dump_after_compact_map() {
    let content = layouts::raw_content_layout(Rect::new(0, 0, 80, 13), false);
    assert!(content.map().height > 0);
    assert!(content.dump().height > 0);
}

#[test]
fn raw_page_keeps_selected_query_map_and_dump_in_sync() {
    fn query_reply(body_byte: u8, query_id: u8, sub_id: u8, body_len: usize) -> [u8; 320] {
        let mut raw = [0_u8; 320];
        raw[0..4].copy_from_slice(&0x75_u32.to_le_bytes());
        raw[4..8].copy_from_slice(&(16_u32 + body_len as u32).to_le_bytes());
        raw[8] = query_id;
        raw[12] = sub_id;
        raw[16..16 + body_len].fill(body_byte);
        raw
    }

    let area = Rect::new(0, 0, 140, 40);
    let mut state = AppState::default();
    state.popup.raw_view_open = true;
    state.raw_view.selected_tab = RawPacketTab::Query75;
    state.raw_view.recent_query_reply_entries = vec![
        crate::app::QueryReplyLogEntry {
            summary: "valid assignment".to_string(),
            raw: query_reply(0x05, 0x03, 0x05, 9).to_vec(),
        },
        crate::app::QueryReplyLogEntry {
            summary: "unresolved query".to_string(),
            raw: query_reply(0x42, 0x99, 0x01, 1).to_vec(),
        },
    ];

    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    state.raw_view.selected_query_reply_entry = Some(0);
    terminal.draw(|frame| render::draw(frame, &state)).unwrap();
    let first =
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .fold(String::new(), |mut output, cell| {
                output.push_str(cell.symbol());
                output
            });
    assert!(first.contains("CH01 assignment"));
    assert!(first.contains("0010: 05"));

    state.raw_view.selected_query_reply_entry = Some(1);
    terminal.draw(|frame| render::draw(frame, &state)).unwrap();
    let second =
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .fold(String::new(), |mut output, cell| {
                output.push_str(cell.symbol());
                output
            });
    assert!(second.contains("unresolved query body"));
    assert!(second.contains("0010: 42"));
}

#[test]
fn raw_content_layout_keeps_map_and_dump_visible_at_target_sizes() {
    let wide =
        layouts::raw_content_layout(layouts::raw_page_layout(Rect::new(0, 0, 140, 40))[3], false);
    assert!(wide.map().width > 0);
    assert!(wide.map().height > 0);
    assert!(wide.dump().width > 0);
    assert!(wide.dump().height > 0);
    assert!(wide.history().is_none());
    assert!(!wide.compact_map());

    let narrow =
        layouts::raw_content_layout(layouts::raw_page_layout(Rect::new(0, 0, 80, 24))[3], false);
    assert!(narrow.map().width > 0);
    assert!(narrow.map().height > 0);
    assert!(narrow.dump().width > 0);
    assert!(narrow.dump().height > 0);
    assert!(narrow.map().y < narrow.dump().y);
    assert!(narrow.compact_map());

    let query =
        layouts::raw_content_layout(layouts::raw_page_layout(Rect::new(0, 0, 140, 40))[3], true);
    assert!(query.history().is_some());
    assert!(query.map().width > 0);
    assert!(query.dump().width > 0);
}

#[test]
fn raw_scroll_intent_moves_map_and_dump_by_one_page() {
    let mut controller = Controller::new(
        Box::new(MockTransport::default()),
        Box::new(crate::device::builtin_zen_go_driver().expect("Zen Go driver")),
    )
    .expect("Zen Go controller");
    controller.state.popup.raw_view_open = true;

    controller
        .apply_intent(
            Intent::ScrollRawDump {
                increase: true,
                page: true,
            },
            Rect::new(0, 0, 80, 24),
        )
        .unwrap();

    assert_eq!(controller.state.raw_view.raw_dump_scroll, 10);
    assert_eq!(controller.state.raw_view.raw_map_scroll, 10);
}

#[test]
fn raw_page_renders_borders_legend_and_footer_at_narrow_size() {
    let mut state = AppState::default();
    state.popup.raw_view_open = true;
    state.raw_view.latest_raw_73 = Some([0x73; 320].to_vec());

    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| render::draw(frame, &state)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut rendered = String::new();
    for y in 0..24 {
        for x in 0..80 {
            rendered.push_str(buffer[(x, y)].symbol());
        }
        rendered.push('\n');
    }

    assert!(rendered.contains("Field Map"));
    assert!(rendered.contains("0x73 State"));
    assert!(rendered.contains("USED green"));
    assert!(rendered.contains("PageUp/PageDown"));
    assert!(rendered.contains("0000:"));
}

#[test]
fn raw_scroll_clamp_matches_ratatui_wrapped_map_lines() {
    let map = raw_map::build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
    let actual_max_scroll = |text: &Text<'_>, viewport: Rect| {
        let mut expected = 0_u16;
        loop {
            let scroll = expected.saturating_add(1);
            let mut buffer = Buffer::empty(viewport);
            Paragraph::new(text.clone())
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0))
                .render(viewport, &mut buffer);
            if !buffer.content().iter().any(|cell| cell.symbol() != " ") {
                break expected;
            }
            expected = scroll;
        }
    };

    let text = render::render_raw_map_text(&map, RawMapScope::All, false);
    let map_viewport = Rect::new(0, 0, 20, 1);
    let expected_map = actual_max_scroll(&text, map_viewport);
    assert!(expected_map > 0);
    assert_eq!(
        render::raw_scroll_offset_for_test(usize::MAX, &text, map_viewport, true),
        expected_map
    );

    let dump = render::render_full_packet_dump(&[0x42; 320], None, &map, RawMapScope::All);
    let dump_viewport = Rect::new(0, 0, 80, 1);
    assert_eq!(
        render::raw_scroll_offset_for_test(usize::MAX, &dump, dump_viewport, true),
        actual_max_scroll(&dump, dump_viewport)
    );
}

#[test]
fn narrow_raw_footer_shows_complete_legend_and_navigation_help() {
    let area = Rect::new(0, 0, 80, 24);
    let mut state = AppState::default();
    state.popup.raw_view_open = true;
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
    terminal.draw(|frame| render::draw(frame, &state)).unwrap();
    let footer_area = layouts::raw_page_layout(area)[4];
    let buffer = terminal.backend().buffer();
    let mut footer = String::new();
    for y in footer_area.y..footer_area.y + footer_area.height {
        for x in footer_area.x..footer_area.x + footer_area.width {
            footer.push_str(buffer[(x, y)].symbol());
        }
        footer.push('\n');
    }

    for token in [
        "USED green",
        "READBACK blue",
        "OBSERVED amber",
        "PARSER cyan",
        "UNMAPPED red",
        "PADDING gray",
    ] {
        assert!(
            footer.contains(token),
            "missing footer token {token:?}: {footer:?}"
        );
    }
    assert!(footer.contains("[/] scope"));
    assert!(footer.contains("PageUp/PageDown scroll"));
    assert!(footer.contains("map 0"));
    assert!(footer.contains("dump 0"));
}

#[test]
fn raw_scope_hit_areas_match_every_packet_scope_label() {
    let area = Rect::new(0, 0, 140, 40);
    let rows = layouts::raw_page_layout(area);
    for tab in [
        RawPacketTab::Query74,
        RawPacketTab::State73,
        RawPacketTab::Auxiliary,
        RawPacketTab::Query75,
        RawPacketTab::DeviceNotification,
    ] {
        let scopes = layouts::raw_scope_hit_areas(rows[2], tab);
        assert_eq!(scopes.len(), RawMapScope::options_for(tab).len());
        assert!(scopes
            .iter()
            .all(|scope| scope.width > 0 && scope.height > 0));
    }
}

#[test]
fn raw_wheel_scrolls_map_and_dump_but_not_query_history() {
    let area = Rect::new(0, 0, 140, 40);
    let mut state = AppState::default();
    state.popup.raw_view_open = true;

    let content = layouts::raw_content_layout(layouts::raw_page_layout(area)[3], false);
    let point = (content.map().x, content.map().y);
    assert_eq!(
        mouse::raw_dump_wheel_action(area, &state, point, true),
        Some(Intent::ScrollRawDump {
            increase: true,
            page: false,
        })
    );

    state.raw_view.selected_tab = RawPacketTab::Query75;
    state.raw_view.recent_query_reply_entries = (0..9)
        .map(|index| crate::app::QueryReplyLogEntry {
            summary: format!("reply {index}"),
            raw: [index as u8; 320].to_vec(),
        })
        .collect();
    let query = layouts::raw_content_layout(layouts::raw_page_layout(area)[3], true);
    let history = query.history().expect("query history area");
    assert_eq!(
        slider_wheel_action(area, &state, history.x + 1, history.y + 1, true),
        Some(Intent::ScrollQueryReplyList { increase: true })
    );
    assert_eq!(
        slider_wheel_action(area, &state, query.dump().x, query.dump().y, true),
        Some(Intent::ScrollRawDump {
            increase: true,
            page: false,
        })
    );
}

#[test]
fn raw_scope_cycle_resets_both_scroll_offsets() {
    let mut state = AppState::default();
    state.raw_view.raw_dump_scroll = 9;
    state.raw_view.raw_map_scroll = 4;

    state.raw_view.cycle_scope(true);

    assert_eq!(state.raw_view.raw_map_scope, RawMapScope::Base);
    assert_eq!(state.raw_view.raw_dump_scroll, 0);
    assert_eq!(state.raw_view.raw_map_scroll, 0);
}

#[test]
fn mouse_action_selects_raw_packet_tab_when_raw_view_is_open() {
    let area = Rect::new(0, 0, 120, 50);
    let layout = layouts::raw_page_layout(area);
    let tabs = layouts::raw_tab_hit_areas(layout[1]);
    let point = (tabs[3].x + tabs[3].width / 2, tabs[3].y);
    let mut state = AppState::default();
    state.popup.raw_view_open = true;

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::SelectRawPacketTab(RawPacketTab::Query75))
    );
}

#[test]
fn mouse_action_opens_routing_popup_from_mixer_surface_button() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout_for_state(page[0], &state);
    let mixer = layouts::mixer_layout(main[1]);
    let button = layouts::mixer_header_button_rects(mixer[0])[1];
    let point = (button.x + button.width / 2, button.y);

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::OpenRoutingPopup)
    );
}

#[test]
fn mouse_action_opens_profiles_popup_from_mixer_surface_button() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout_for_state(page[0], &state);
    let mixer = layouts::mixer_layout(main[1]);
    let button = layouts::mixer_header_button_rects(mixer[0])[0];
    let point = (button.x + button.width / 2, button.y);

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::OpenProfilesPopup)
    );
}

#[test]
fn mouse_action_pages_mixer_strips_left_from_panel_button() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout_for_state(page[0], &state);
    let mixer = layouts::mixer_layout(main[1]);
    let button = layouts::mixer_strip_page_button_rects(mixer[1])[0];

    assert_eq!(
        mouse_action(area, &state, button.x + button.width / 2, button.y),
        Some(Intent::PageMixerStripsLeft)
    );
}

#[test]
fn mouse_action_pages_mixer_strips_right_from_panel_button() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout_for_state(page[0], &state);
    let mixer = layouts::mixer_layout(main[1]);
    let button = layouts::mixer_strip_page_button_rects(mixer[1])[1];

    assert_eq!(
        mouse_action(area, &state, button.x + button.width / 2, button.y),
        Some(Intent::PageMixerStripsRight)
    );
}

#[test]
fn mouse_action_opens_assignment_picker_from_afx_routing_source_chip() {
    let area = Rect::new(0, 0, 120, 50);
    let mut state = AppState::default();
    state.popup.routing_open = true;
    state.ui.focus = FocusArea::Mixer;
    state.mixer.channels[MixerSurface::Mix1.index()][0].assignment =
        Some(MixerAssignment::Preamp(1));
    state.mixer.channels[MixerSurface::Mix1.index()][1].assignment =
        Some(MixerAssignment::Preamp(2));
    let row_area = layouts::afx_routing_layout(layouts::routing_popup_area(area))[4];
    let rects = layouts::afx_routing_row_rects(row_area, &state, 0);
    let point = (rects[2].x + rects[2].width / 2, rects[2].y);

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::OpenAssignmentPickerAt {
            address: MixerAddress {
                surface: 0,
                strip: 1,
            },
        })
    );
}

#[test]
fn afx_routing_source_columns_stay_aligned_for_different_label_lengths() {
    let area = Rect::new(0, 0, 120, 1);
    let mut state = AppState::default();
    state.mixer.channels[MixerSurface::Mix1.index()][0].assignment = Some(MixerAssignment::Mute);
    state.mixer.channels[MixerSurface::Mix1.index()][1].assignment =
        Some(MixerAssignment::Preamp(2));
    state.mixer.channels[MixerSurface::Mix1.index()][2].assignment =
        Some(MixerAssignment::ComputerPlay(8));
    state.mixer.channels[MixerSurface::Mix1.index()][3].assignment =
        Some(MixerAssignment::Oscillator(1));

    let first = layouts::afx_routing_row_rects(area, &state, 0);
    let second = layouts::afx_routing_row_rects(area, &state, 1);

    assert_eq!(first[2].x, second[2].x);
    assert_eq!(first[4].x, second[4].x);
}

#[test]
fn device_header_mouse_actions_follow_visible_status_chip_positions() {
    let area = Rect::new(0, 0, 180, 50);
    let mut state = AppState::default();
    state.device.status.clock_source = Some(ClockSource::Internal);
    let titlebar = layouts::titlebar_layout(layouts::root_chunks(area)[0])[0];
    let header = layouts::device_panel_layout(titlebar, &state)[0];
    let rendered = render_buffer(header, |area, buffer| {
        Paragraph::new(render::render_device_header(&state))
            .wrap(Wrap { trim: false })
            .render(area, buffer);
    });

    let rendered_offset = |text: &str| {
        rendered
            .find(text)
            .map(|offset| rendered[..offset].chars().count() as u16)
            .expect("visible device header chip")
    };
    let clock_x = header.x + rendered_offset(" Internal ") + 1;
    let sample_x = header.x + rendered_offset(" rate ? ") + 1;
    assert_eq!(
        mouse_action(area, &state, clock_x, header.y),
        Some(Intent::OpenClockSourceSelector)
    );
    assert_eq!(
        mouse_action(area, &state, sample_x, header.y),
        Some(Intent::OpenSampleRateSelector)
    );

    let supported_offset = rendered_offset(" SUPPORTED ");
    let supported_x = header.x + supported_offset + 1;
    let adjacent_x = header.x + supported_offset + styles::chip_width("SUPPORTED") + 1;
    assert_eq!(mouse_action(area, &state, supported_x, header.y), None);
    assert_eq!(mouse_action(area, &state, adjacent_x, header.y), None);
    assert!(!device_header_name_hit(area, &state, clock_x, header.y));
}

#[test]
fn mouse_action_opens_sample_rate_selector_from_device_chip() {
    let area = Rect::new(0, 0, 120, 50);
    let mut state = AppState::default();
    state.device.status.clock_source = Some(ClockSource::Internal);
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
    state.device.status.clock_source = Some(ClockSource::Usb);
    let chips = layouts::device_header_hit_areas(
        layouts::titlebar_layout(layouts::root_chunks(area)[0])[0],
        &state,
    );

    assert_eq!(mouse_action(area, &state, chips[1].x + 1, chips[1].y), None);
}

#[test]
fn mouse_action_hits_visible_surface_tab_position() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let chunks = layouts::root_chunks(area);
    let page = layouts::mixer_page_layout(chunks[1]);
    let main = layouts::mixer_main_layout_for_state(page[0], &state);
    let mixer = layouts::mixer_layout(main[1]);
    let tabs = layouts::dynamic_surface_tab_hit_areas(mixer[0], &state);

    assert_eq!(
        mouse_action(area, &state, tabs[1].x + 1, tabs[1].y),
        Some(Intent::SelectMixerSurface { surface: 1 })
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
fn narrow_query_history_mouse_rows_match_list_inner_bounds() {
    let area = Rect::new(0, 0, 80, 24);
    let layout = layouts::raw_page_layout(area);
    let history = layouts::raw_content_layout(layout[3], true)
        .history()
        .expect("query history area");
    let inner = styles::section_block("Recent 0x75 Replies", true).inner(history);
    let mut state = AppState::default();
    state.popup.raw_view_open = true;
    state.raw_view.selected_tab = RawPacketTab::Query75;
    state.raw_view.recent_query_reply_entries = vec![
        crate::app::QueryReplyLogEntry {
            summary: "oldest".to_string(),
            raw: [0x75; 320].to_vec(),
        },
        crate::app::QueryReplyLogEntry {
            summary: "newest".to_string(),
            raw: [0x75; 320].to_vec(),
        },
    ];

    assert_eq!(
        mouse_action(area, &state, inner.x + 1, inner.y),
        Some(Intent::SelectQueryReplyEntry(1))
    );
    assert_eq!(
        mouse_action(area, &state, inner.x + 1, inner.y + 1),
        Some(Intent::SelectQueryReplyEntry(0))
    );
    assert_eq!(mouse_action(area, &state, inner.x + 1, history.y), None);
    assert_eq!(
        mouse_action(area, &state, inner.x + 1, history.y + history.height - 1),
        None
    );
    assert_eq!(
        mouse_action(area, &state, inner.x + 1, history.y + history.height),
        None
    );
}

#[test]
fn mouse_action_selects_recent_query_reply_entry_when_raw_query_tab_is_open() {
    let area = Rect::new(0, 0, 120, 50);
    let layout = layouts::raw_page_layout(area);
    let history = layouts::raw_content_layout(layout[3], true)
        .history()
        .expect("query history area");
    let inner = styles::section_block("Recent 0x75 Replies", true).inner(history);
    let mut state = AppState::default();
    state.popup.raw_view_open = true;
    state.raw_view.selected_tab = RawPacketTab::Query75;
    state.raw_view.recent_query_reply_entries = vec![
        crate::app::QueryReplyLogEntry {
            summary: "0x75 03/05".to_string(),
            raw: {
                let mut r = [0_u8; 320];
                r[..2].copy_from_slice(&[0x75, 0x05]);
                r
            }
            .to_vec(),
        },
        crate::app::QueryReplyLogEntry {
            summary: "0x75 03/06".to_string(),
            raw: {
                let mut r = [0_u8; 320];
                r[..2].copy_from_slice(&[0x75, 0x06]);
                r
            }
            .to_vec(),
        },
    ];
    let point = (inner.x + 1, inner.y);

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::SelectQueryReplyEntry(1))
    );
}

#[test]
fn mouse_action_hits_preamp_gain_control() {
    let area = Rect::new(0, 0, 120, 50);
    let mut state = zen_go_state();
    state.input_spaces[0].inputs[0].mode = Some(0);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout_for_state(page[0], &state);
    let (_, _, row) = layouts::dynamic_input_rows(main[0], &state)
        .into_iter()
        .find(|(_, index, _)| *index == 0)
        .expect("first input row");
    let controls = layouts::dynamic_input_control_rects(row, &state, 0, 0).expect("controls");
    let gain = controls.gain.expect("gain control");
    let point = (gain.x + gain.width.saturating_sub(1), gain.y);
    let address = state.input_spaces[0].inputs[0].address;

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::SetInputGainAt {
            address,
            raw: state.input_range(address, Some(0)).expect("input range").1,
        })
    );
}

#[test]
fn output_card_renders_arrow_adjust_buttons() {
    let rendered =
        styles::render_output_card(&AppState::default().output.states[0], true).to_string();

    assert!(rendered.contains(" ↑ "));
    assert!(rendered.contains(" ↓ "));
    assert!(!rendered.contains(" + "));
    assert!(!rendered.contains(" - "));
}

#[test]
fn preamp_controls_render_arrow_adjust_buttons() {
    let rendered =
        render::render_preamp_controls_text(AppState::default().preamp.state.input1).to_string();

    assert!(rendered.contains(" ↑ "));
    assert!(rendered.contains(" ↓ "));
    assert!(!rendered.contains(" + "));
    assert!(!rendered.contains(" - "));
}

#[test]
fn slider_wheel_action_adjusts_output_level_one_step() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let row =
        layouts::dynamic_output_card_areas(layouts::inner_area(page[1]), state.outputs().len())[0];
    let controls = layouts::dynamic_output_control_rects(row, &state, 0).expect("output controls");
    let track = controls.level.expect("output level control");

    assert_eq!(
        slider_wheel_action(area, &state, track.x, track.y, true),
        Some(Intent::AdjustOutputLevel {
            index: 0,
            increase: true,
        })
    );
}

#[test]
fn slider_wheel_action_adjusts_preamp_gain_one_step() {
    let area = Rect::new(0, 0, 120, 50);
    let mut state = zen_go_state();
    state.input_spaces[0].inputs[0].mode = Some(0);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout_for_state(page[0], &state);
    let (_, _, row) = layouts::dynamic_input_rows(main[0], &state)
        .into_iter()
        .find(|(_, index, _)| *index == 0)
        .expect("first input row");
    let address = state.input_spaces[0].inputs[0].address;
    let controls = layouts::dynamic_input_control_rects(row, &state, 0, 0).expect("controls");
    let gain = controls.gain.expect("gain control");

    assert_eq!(
        slider_wheel_action(area, &state, gain.x, gain.y, true),
        Some(Intent::AdjustInputGainAt {
            address,
            increase: true,
        })
    );
}

#[test]
fn slider_wheel_action_adjusts_mixer_pan_inside_strip_panel() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let controls = zen_mixer_controls(area, &state, 0);
    let track = controls.pan.expect("mixer pan control");
    let address = MixerAddress {
        surface: 0,
        strip: 1,
    };

    assert_eq!(
        slider_wheel_action(area, &state, track.x, track.y, true),
        Some(Intent::AdjustMixerPanAt {
            address,
            right: true,
        })
    );
}

#[test]
fn slider_wheel_action_adjusts_mixer_level_inside_strip_panel() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let controls = zen_mixer_controls(area, &state, 0);
    let track = controls.fader.expect("mixer fader control");
    let address = MixerAddress {
        surface: 0,
        strip: 1,
    };

    assert_eq!(
        slider_wheel_action(area, &state, track.x, track.y, true),
        Some(Intent::AdjustMixerLevelAt {
            address,
            increase: true,
        })
    );
}

#[test]
fn slider_wheel_action_uses_wider_hitbox_for_thin_mixer_level_slider() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let controls = zen_mixer_controls(area, &state, 0);
    let track = controls.fader.expect("mixer fader control");
    let address = MixerAddress {
        surface: 0,
        strip: 1,
    };

    assert_eq!(
        slider_wheel_action(area, &state, track.x.saturating_sub(1), track.y, true),
        Some(Intent::AdjustMixerLevelAt {
            address,
            increase: true,
        })
    );
}

#[test]
fn mouse_action_hits_visible_output_level_slider_position() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let row =
        layouts::dynamic_output_card_areas(layouts::inner_area(page[1]), state.outputs().len())[0];
    let controls = layouts::dynamic_output_control_rects(row, &state, 0).expect("output controls");
    let track = controls.level.expect("output level control");

    assert_eq!(
        mouse_action(
            area,
            &state,
            track.x + track.width.saturating_sub(1),
            track.y
        ),
        Some(Intent::SetOutputLevel { index: 0, step: 0 })
    );
}

#[test]
fn mouse_action_hits_visible_preamp_gain_slider_position() {
    let area = Rect::new(0, 0, 120, 50);
    let mut state = zen_go_state();
    state.input_spaces[0].inputs[0].mode = Some(0);
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout_for_state(page[0], &state);
    let (_, _, row) = layouts::dynamic_input_rows(main[0], &state)
        .into_iter()
        .find(|(_, index, _)| *index == 0)
        .expect("first input row");
    let controls = layouts::dynamic_input_control_rects(row, &state, 0, 0).expect("controls");
    let track = controls.gain.expect("gain control");
    let address = state.input_spaces[0].inputs[0].address;

    assert_eq!(
        mouse_action(
            area,
            &state,
            track.x + track.width.saturating_sub(1),
            track.y
        ),
        Some(Intent::SetInputGainAt {
            address,
            raw: state.input_range(address, Some(0)).expect("input range").1,
        })
    );
}

#[test]
fn mouse_action_hits_visible_mixer_pan_slider_position() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let controls = zen_mixer_controls(area, &state, 0);
    let track = controls.pan.expect("mixer pan control");
    let address = MixerAddress {
        surface: 0,
        strip: 1,
    };

    assert_eq!(
        mouse_action(
            area,
            &state,
            track.x + track.width.saturating_sub(1),
            track.y
        ),
        Some(Intent::SetMixerPanAt { address, pan: 30 })
    );
}

#[test]
fn dynamic_mixer_fader_renders_only_ratio_positioned_handle() {
    let area = Rect::new(0, 0, 200, 55);
    let mut state = zen_go_state();
    state.mixer.surfaces[0].strips[0].fader = Some(0);
    state.mixer.surfaces[0].strips[0].meter = None;
    let controls = zen_mixer_controls(area, &state, 0);
    let track = controls.fader.expect("mixer fader control");
    let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");

    terminal
        .draw(|frame| render::draw(frame, &state))
        .expect("render mixer");

    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(track.x, track.y)].symbol(), "●");
    assert_ne!(buffer[(track.x, track.y + track.height / 2)].symbol(), "●");
}

#[test]
fn mouse_action_hits_visible_mixer_level_slider_position() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let controls = zen_mixer_controls(area, &state, 0);
    let track = controls.fader.expect("mixer fader control");
    let address = MixerAddress {
        surface: 0,
        strip: 1,
    };

    assert_eq!(
        mouse_action(area, &state, track.x, track.y),
        Some(Intent::SetMixerLevelAt { address, level: 0 })
    );
}

#[test]
fn mouse_action_hits_visible_preamp_mode_chip_position() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let page = layouts::mixer_page_layout(layouts::root_chunks(area)[1]);
    let main = layouts::mixer_main_layout_for_state(page[0], &state);
    let (_, _, row) = layouts::dynamic_input_rows(main[0], &state)
        .into_iter()
        .find(|(_, index, _)| *index == 0)
        .expect("first input row");
    let controls = layouts::dynamic_input_control_rects(row, &state, 0, 0).expect("controls");
    let mode = controls.mode.expect("mode control");
    let address = state.input_spaces[0].inputs[0].address;

    assert_eq!(
        mouse_action(area, &state, mode.x + mode.width / 2, mode.y),
        Some(Intent::CycleInputModeAt { address })
    );
}

#[test]
fn mouse_action_picks_preamp_mode_from_selector_popup() {
    let area = Rect::new(0, 0, 120, 50);
    let mut state = zen_go_state();
    state.popup.selector_popup = Some(SelectorPopupState {
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
    state.popup.assignment_picker = Some(AssignmentPickerState { strip: 11 });

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
        AppState::default().preamp.state.input1,
    ))
    .render(Rect::new(0, 0, 40, 1), &mut buffer);

    assert_eq!(buffer[(0, 0)].symbol(), " ");
    assert_eq!(buffer[(1, 0)].symbol(), "↓");
}

#[test]
fn mouse_action_hits_mixer_link_button_on_odd_strip() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let controls = zen_mixer_controls(area, &state, 0);
    let link = controls.link.expect("mixer link control");

    assert_eq!(
        mouse_action(area, &state, link.x, link.y),
        Some(Intent::ToggleMixerLinkAt {
            address: MixerAddress {
                surface: 0,
                strip: 1
            },
        })
    );
}

#[test]
fn mouse_action_hits_mixer_solo_button() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let controls = zen_mixer_controls(area, &state, 0);
    let solo = controls.solo.expect("mixer solo control");

    assert_eq!(
        mouse_action(area, &state, solo.x, solo.y),
        Some(Intent::ToggleMixerSoloAt {
            address: MixerAddress {
                surface: 0,
                strip: 1
            },
        })
    );
}

#[test]
fn mouse_action_hits_visible_mixer_solo_chip_position() {
    let area = Rect::new(0, 0, 120, 50);
    let state = zen_go_state();
    let controls = zen_mixer_controls(area, &state, 0);
    let solo = controls.solo.expect("mixer solo control");

    assert_eq!(
        mouse_action(area, &state, solo.x, solo.y),
        Some(Intent::ToggleMixerSoloAt {
            address: MixerAddress {
                surface: 0,
                strip: 1
            },
        })
    );
}

#[test]
fn mouse_action_opens_assignment_picker_from_src_button() {
    let area = Rect::new(0, 0, 120, 60);
    let mut state = AppState::default();
    state.mixer.selected_channel = 3;
    state.mixer.channels[0][3].assignment = Some(MixerAssignment::ComputerPlay(2));
    let source_rect = zen_mixer_controls(area, &state, 3)
        .source
        .expect("assignment source control");
    let point = (source_rect.x + source_rect.width / 2, source_rect.y);

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::OpenAssignmentPickerAt {
            address: MixerAddress {
                surface: 0,
                strip: 4,
            },
        })
    );
}

#[test]
fn mouse_action_picks_assignment_from_modal() {
    let area = Rect::new(0, 0, 120, 50);
    let popup = layouts::assignment_picker_area(area);
    let inner = layouts::popup_list_inner_area(popup, "Assign CH 11");
    let mut state = AppState::default();
    state.popup.assignment_picker = Some(AssignmentPickerState { strip: 11 });

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
    state.device.status.startup_query_summaries[1] =
        Some("Capability/default block: 3 bytes [aa bb cc]".to_string());
    state.device.status.startup_query_summaries[2] =
        Some("Status/capability value: 1 bytes [12]".to_string());

    let lines = [
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
    state.mixer.channels[0][10].assignment = Some(MixerAssignment::ComputerPlay(8));
    state.mixer.channels[0][10].pan = PanState::from_raw(0x3e);
    state.mixer.channels[0][10].linked = Some(true);
    state.mixer.channels[0][10].level = Some(0x10);
    state.mixer.channels[0][10].meter = Some(0x08);
    state.mixer.channels[0][10].muted = Some(false);

    let channel = &state.mixer.channels[0][10];
    let line = render_mixer_strip_line(&state, 10, channel);

    assert!(line.contains("Computer Play 8"));
    assert!(line.contains("pan=R30"));
    assert!(line.contains("link=on"));
    assert!(line.contains("meter="));
}

#[test]
fn mixer_strip_line_renders_meter_separately_from_level_value() {
    let mut state = AppState::default();
    state.mixer.channels[0][0].level = Some(0x00);
    state.mixer.channels[0][0].meter = Some(0x30);
    state.mixer.channels[0][0].muted = Some(false);

    let line = render_mixer_strip_line(&state, 0, &state.mixer.channels[0][0]);

    assert!(line.contains("level=0 dB"));
    assert!(line.contains("meter=-48 dB"));
}

#[test]
fn mixer_strip_line_hides_meter_value_below_ui_floor() {
    let mut state = AppState::default();
    state.mixer.channels[0][0].level = Some(0x00);
    state.mixer.channels[0][0].meter = Some(0x60);
    state.mixer.channels[0][0].muted = Some(false);

    let line = render_mixer_strip_line(&state, 0, &state.mixer.channels[0][0]);

    assert!(line.contains("meter= mute=off"));
}

#[test]
fn mixer_strip_line_renders_newly_grounded_pair_link() {
    let mut state = AppState::default();
    let target = MixerLinkTarget::from_channel(MixerSurface::Mix1, 7).expect("grounded pair");
    state.mixer.channels[target.mixer.index()][target.left_channel as usize - 1].linked =
        Some(true);
    state.mixer.channels[target.mixer.index()][target.left_channel as usize - 1].assignment =
        Some(MixerAssignment::SpdifIn(1));

    let line = render_mixer_strip_line(
        &state,
        target.left_channel as usize - 1,
        &state.mixer.channels[target.mixer.index()][target.left_channel as usize - 1],
    );

    assert!(line.contains("CH 07"));
    assert!(line.contains("SPDIF In 1"));
    assert!(line.contains("link=on"));
}

#[test]
fn profile_meter_state_line_surfaces_decoded_stereo_lanes() {
    let mut state = zen_go_state();
    state.meters = vec![
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 0,
            value: 0x0a,
        },
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 0,
            lane: 1,
            value: 0x05,
        },
    ];

    let line = render::render_mix_meter_state_line(&state);

    assert!(line.contains("MIX 1 / Monitor-HP1"));
    assert!(line.contains("L ███████░ -10 dB"));
    assert!(line.contains("R ███████░  -5 dB"));
}

#[test]
fn profile_meter_state_line_surfaces_zero_and_silence_values() {
    let mut state = zen_go_state();
    state.meters = vec![
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 1,
            lane: 0,
            value: 0x00,
        },
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 1,
            lane: 1,
            value: 0x06,
        },
    ];
    state.mixer.surface_index = 1;

    let line = render::render_mix_meter_state_line(&state);

    assert!(line.contains("MIX 2 / HP2"));
    assert!(line.contains("L ████████   0 dB"));
    assert!(line.contains("R ███████░  -6 dB"));
}

#[test]
fn profile_meter_state_line_uses_decoded_values_not_raw_bytes() {
    let mut state = zen_go_state();
    state.meters = vec![
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 1,
            lane: 0,
            value: 0x12,
        },
        DynamicMeterState {
            target: RuntimeMeterTarget::MixMaster,
            target_index: 1,
            lane: 1,
            value: 0x34,
        },
    ];
    state.mixer.surface_index = 1;
    state.raw_view.latest_raw_73 = Some(vec![0; 320]);

    let line = render::render_mix_meter_state_line(&state);

    assert!(line.contains("L ██████░░ -18 dB"));
    assert!(line.contains("R █░░░░░░░ -52 dB"));
}

#[test]
fn profile_meter_state_line_renders_only_declared_lane() {
    let mut state = AppState::from_entry(
        &ProfileCatalog::builtin()
            .entries()
            .iter()
            .find(|entry| entry.id == "orion_studio_3")
            .expect("Orion profile"),
    );
    state.meters = vec![DynamicMeterState {
        target: RuntimeMeterTarget::MixMaster,
        target_index: 0,
        lane: 0,
        value: 0x3c,
    }];

    let line = render::render_mix_meter_state_line(&state);

    assert!(line.contains("Lane 1"));
    assert!(line.contains("-60 dB"));
    assert!(!line.contains(" R "));
}

#[test]
fn observed_meter_label_mentions_raw_value() {
    let mut input = PreampInputState::from_raw(0x2a, 0x00);
    input.observed_meter = Some(0x30);

    assert_eq!(observed_meter_label(input), "obs meter -48 dB");
}

#[test]
fn observed_meter_label_mentions_pending_state() {
    assert_eq!(
        observed_meter_label(PreampInputState::from_raw(0x2a, 0x00)),
        ""
    );
}

#[test]
fn observed_meter_label_hides_values_below_ui_floor() {
    let mut input = PreampInputState::from_raw(0x2a, 0x00);
    input.observed_meter = Some(0x60);

    assert_eq!(observed_meter_label(input), "");
}

#[test]
#[ignore = "benchmark"]
fn perf_draw_full_frame() {
    const FRAMES: usize = 2_000;

    let backend = TestBackend::new(140, 42);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut state = AppState::default();
    state.device.connection.connected = true;
    state.device.status.metadata = Some(antelope_protocol::DeviceMetadata {
        product_name: "Zen Go Synergy Core".to_string(),
        serial: "4502721001300".to_string(),
        hardware_version: "6.6".to_string(),
    });
    state.device.status.sample_rate = Some(SampleRate::Hz48000);
    state.device.status.clock_source = Some(ClockSource::Internal);
    state.mixer.selected_channel = 7;
    state.ui.focus = FocusArea::Mixer;
    state.mixer.channels[MixerSurface::Mix1.index()][7].level = Some(0x18);
    state.mixer.channels[MixerSurface::Mix1.index()][7].meter = Some(0x24);
    state.mixer.channels[MixerSurface::Mix1.index()][7].assignment =
        Some(MixerAssignment::ComputerPlay(4));
    state.mixer.channels[MixerSurface::Mix1.index()][7].soloed = Some(true);
    state.mixer.channels[MixerSurface::Mix1.index()][7].linked = Some(true);

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
