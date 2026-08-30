use std::path::Path;

const MAIN: &str = include_str!("../ui/main.slint");
const MIXER: &str = include_str!("../ui/pages/mixer.slint");
const OUTPUT: &str = include_str!("../ui/domain/output-card.slint");
const STRIP: &str = include_str!("../ui/domain/mixer-strip.slint");

fn position(source: &str, marker: &str) -> usize {
    source
        .find(marker)
        .unwrap_or_else(|| panic!("missing source marker: {marker}"))
}

fn source_blocks<'a>(source: &'a str, marker: &str) -> Vec<&'a str> {
    let mut blocks = Vec::new();
    let mut search_from = 0;

    while let Some(relative_start) = source[search_from..].find(marker) {
        let start = search_from + relative_start;
        let open_brace = start + marker.len() - 1;
        let mut depth = 0;
        let mut end = None;

        for (offset, character) in source[open_brace..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open_brace + offset + character.len_utf8());
                        break;
                    }
                }
                _ => {}
            }
        }

        let end = end.unwrap_or_else(|| panic!("unterminated source block: {marker}"));
        blocks.push(&source[start..end]);
        // Continue after marker, not balanced block, so nested matching blocks are found.
        // Each marker occurrence is visited once, preventing duplicate extraction.
        search_from = start + marker.len();
    }

    blocks
}

#[test]
fn shell_has_four_tabs_and_no_route_surface() {
    assert!(MAIN.contains("labels: [\"Mixer\", \"Profile\", \"Raw\", \"Settings\"]"));
    assert!(!MAIN.contains("RoutePage"));
    assert!(!MAIN.contains("selected-mixer-channel"));
    assert!(MAIN.contains("visible: root.active-page == 0;"));
    assert!(MAIN.contains("visible: root.active-page == 1;"));
    assert!(MAIN.contains("visible: root.active-page == 2;"));
    assert!(MAIN.contains("visible: root.active-page == 3;"));
}

#[test]
fn header_uses_reusable_source_pickers() {
    let pickers = source_blocks(MAIN, "ZenSourcePicker {");
    assert_eq!(
        pickers.len(),
        2,
        "header must contain separate rate and clock pickers"
    );

    let sample_rate_picker = pickers
        .iter()
        .find(|picker| picker.contains("selected-label: root.sample-rate-label;"))
        .expect("missing sample-rate picker");
    assert!(sample_rate_picker.contains("choices: root.sample-rate-choices;"));
    assert!(sample_rate_picker
        .contains("choice-requested(index) => { root.sample-rate-requested(index); }"));
    assert!(sample_rate_picker.contains("reset-token: root.active-page;"));
    assert!(sample_rate_picker.contains("enabled: root.controls-available;"));

    let clock_picker = pickers
        .iter()
        .find(|picker| picker.contains("selected-label: root.clock-source-label;"))
        .expect("missing clock-source picker");
    assert!(clock_picker.contains("choices: root.clock-source-choices;"));
    assert!(
        clock_picker.contains("choice-requested(index) => { root.clock-source-requested(index); }")
    );
    assert!(clock_picker.contains("reset-token: root.active-page;"));
    assert!(clock_picker.contains("enabled: root.controls-available;"));
}

#[test]
fn mixer_orders_preamps_strips_and_outputs_and_keeps_strip_markers() {
    assert!(
        position(MIXER, "title: \"Preamps\";")
            < position(MIXER, "title: root.mixer-label + \" Mixer\";")
    );
    assert!(
        position(MIXER, "title: root.mixer-label + \" Mixer\";")
            < position(MIXER, "title: \"Stereo Outputs\";")
    );

    let scroll_views = source_blocks(MIXER, "ScrollView {");
    assert_eq!(
        scroll_views.len(),
        2,
        "Mixer needs outer and strip-bank scrolling"
    );
    let outer_scroll = scroll_views[0];
    assert!(outer_scroll.contains("x: 0px;"));
    assert!(outer_scroll.contains("y: 0px;"));
    assert!(outer_scroll.contains("horizontal-scrollbar-policy: always-off;"));
    assert!(outer_scroll.contains("vertical-scrollbar-policy: as-needed;"));
    assert!(outer_scroll.contains("VerticalLayout {"));

    let strip_scroll = scroll_views[1];
    assert!(strip_scroll.contains("horizontal-scrollbar-policy: as-needed;"));
    assert!(strip_scroll.contains("VerticalLayout {"));
    assert!(strip_scroll.contains("HorizontalLayout {"));

    let panels = source_blocks(MIXER, "ZenPanel {");
    assert_eq!(
        panels.len(),
        3,
        "Mixer needs preamp, strip, and output panels"
    );
    let preamp_panel = panels
        .iter()
        .find(|panel| panel.contains("title: \"Preamps\";"))
        .expect("missing preamp panel");
    assert!(preamp_panel.contains("GridLayout {"));
    assert!(preamp_panel.contains("for preamp in root.preamps: MixerPreamp {"));
    assert!(preamp_panel.contains("spacing-horizontal: ZenTheme.spacing-medium;"));
    assert!(preamp_panel.contains("spacing-vertical: 0px;"));
    assert!(preamp_panel.contains("row: 0;"));
    assert!(preamp_panel.contains("col: preamp.input - 1;"));
    assert_eq!(MIXER.matches("PreampCard {").count(), 1);

    let output_panel = panels
        .iter()
        .find(|panel| panel.contains("title: \"Stereo Outputs\";"))
        .expect("missing stereo output panel");
    assert!(output_panel.contains("GridLayout {"));
    assert!(output_panel.contains("for output in root.outputs: OutputCard {"));
    assert!(output_panel.contains("row: output.index == 0 ? 0 : 1;"));
    assert!(output_panel.contains("col: output.index == 0 ? 0 : output.index - 1;"));
    assert!(output_panel.contains("colspan: output.index == 0 ? 2 : 1;"));
    assert!(output_panel.contains("spacing-horizontal: ZenTheme.spacing-medium;"));
    assert!(output_panel.contains("spacing-vertical: ZenTheme.spacing-medium;"));

    assert!(MIXER.contains("width: 2006px;"));
    assert!(MIXER.contains("height: max(438px, parent.height);"));
    assert!(MIXER.contains("width: 116px;"));
    assert!(MIXER.contains("context-token: root.picker-reset-token;"));
    assert!(MIXER.contains("assignment-choices: root.assignment-choices;"));
}

#[test]
fn output_card_states_shared_stereo_gain_without_signal_meter() {
    assert!(OUTPUT.contains("title: root.output.name;"));
    assert!(OUTPUT.contains("STEREO L / R"));
    assert!(OUTPUT.contains("root.output.level-percent"));
    assert!(OUTPUT.contains(
        "width: (root.width - 32px) * max(0, min(100, root.output.level-percent)) / 100;"
    ));
    assert!(OUTPUT.contains("root.output.level-db + \" dB\""));
    assert!(!OUTPUT.contains("ZenMeter"));
    assert!(OUTPUT.contains("mute-requested(int)"));
    assert!(OUTPUT.contains("dim-requested(int)"));
    assert!(OUTPUT.contains("level-adjusted(int, int, int)"));
}

#[test]
fn mixer_strip_domain_still_exposes_existing_controls() {
    assert!(STRIP.contains("level-set(int, int)"));
    assert!(STRIP.contains("pan-set(int, int)"));
    assert!(STRIP.contains("link-requested(int)"));
    assert!(STRIP.contains("assignment-requested(int, int)"));
    assert!(STRIP.contains("mute-requested(int)"));
    assert!(STRIP.contains("solo-requested(int)"));
    assert!(STRIP.contains("ZenLevelFader {"));
    assert!(STRIP.contains("ZenPanSlider {"));
    assert!(STRIP.contains("ZenMeter {"));
    assert!(STRIP.contains("percent: root.strip.meter-percent;"));
    assert!(STRIP.contains("value-changed(value) => { root.pan-set(root.strip.channel, value); }"));
    assert!(STRIP.contains("value-released(value) => { root.pan-set(root.strip.channel, value); }"));
    assert!(
        STRIP.contains("value-changed(value) => { root.level-set(root.strip.channel, value); }")
    );
    assert!(
        STRIP.contains("value-released(value) => { root.level-set(root.strip.channel, value); }")
    );
    assert!(MIXER.contains("horizontal-scrollbar-policy: as-needed;"));
}

#[test]
fn route_page_source_is_deleted() {
    let route_page = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/pages/route.slint");
    assert!(!route_page.exists());
}
