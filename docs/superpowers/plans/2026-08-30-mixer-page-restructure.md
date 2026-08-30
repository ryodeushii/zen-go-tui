# Mixer Page Restructuring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure the Slint GUI into four useful tabs with Mixer as a vertically ordered, complete audio-control page while preserving Rust, controller, transport, protocol, TUI, and mixer-strip behavior.

**Architecture:** Keep Rust `GuiSnapshot` authoritative and add a GUI-only page-index seam around the existing five-variant `GuiPage`, including Rust-only `Routing`. Move preamp composition into `MixerPage`, keep its existing horizontal strip bank intact, place shared-gain stereo outputs below it, and move sample-rate and clock-source selection into reusable header pickers. Remove only GUI routing artifacts and leave controller routing state untouched.

**Tech Stack:** Rust edition 2021, Slint 1.16.0, `slint-build` 1.16.0, Cargo workspace, mock transport, existing Warm Hardware Slint primitives.

**Spec:** `docs/superpowers/specs/2026-08-30-mixer-page-restructure-design.md`

## Global Constraints

- Keep Rust `GuiPage::Routing`, `GuiPage::ALL`, `GuiPage::index`, and `GuiPage::from_index` unchanged for TUI and controller compatibility.
- Add GUI-only mapping `0 -> Mixer`, `1 -> Profiles`, `2 -> Raw`, `3 -> Settings`; tab commands must use it.
- Keep `GuiSnapshot` as the authoritative input to Slint; rejected values return through the next snapshot.
- Use existing Rust command constructors and validation for every header, preamp, mixer, output, profile, raw, and settings callback.
- Do not add independent left/right output gain or meter fields and do not decode new protocol fields.
- Render output gain as one shared value and one bounded gain-position indicator, not as a signal meter.
- Keep the existing mixer-strip markup, bindings, callbacks, direct-control behavior, and horizontal scrolling unchanged.
- Keep all 16 mixer strips reachable through horizontal scrolling and keep the whole Mixer page vertically scrollable.
- Reuse `ZenSourcePicker` and its Slint 1.16 `PopupWindow`; do not duplicate picker popup behavior.
- Disable header and write controls when `controls-available` is false; picker reset tokens must close transient popups.
- Keep the Warm Hardware visual direction and existing shared primitives.
- Do not modify transport, protocol encoding, controller validation, worker ownership, or TUI behavior.
- Do not add Material Slint, another component dependency, a second UI framework, or a new UI crate.
- Do not create commits unless the user explicitly requests them.

---

## File Map

### Rust state and command boundary

- Modify: `zen-go-slint/src/models.rs`
  - Add `GuiPage::gui_index(self) -> Option<i32>` and `GuiPage::from_gui_index(i32) -> Option<GuiPage>`.
  - Leave `index`, `from_index`, `ALL`, and `Routing` unchanged.
- Modify: `zen-go-slint/src/commands.rs`
  - Make `GuiCommand::set_page_from_index(i32)` use `from_gui_index`.
  - Update page command tests for four GUI indexes.
- Modify: `zen-go-slint/src/ui_bridge.rs`
  - Map active Rust pages through `gui_index().unwrap_or(0)`.
  - Remove the unused Slint `selected-mixer-channel` view property and setter after Route UI removal.
  - Keep snapshot/controller selected-channel state intact.
- Verify only: `zen-go-slint/src/runtime.rs`
  - Existing callback names and validated command calls must continue compiling; no behavior change is planned.

### Slint shell and page files

- Modify: `zen-go-slint/ui/main.slint`
  - Replace sample-rate and clock status chips with `ZenSourcePicker` instances.
  - Change tabs to Mixer, Profile, Raw, Settings.
  - Remove Route import, Route branch, and selected-channel property.
  - Pass preamps and existing callbacks into `MixerPage`.
- Modify: `zen-go-slint/ui/pages/mixer.slint`
  - Add preamp models and callback forwarding.
  - Move preamp composition into a two-column row.
  - Wrap page content in one vertical `ScrollView`.
  - Preserve the current nested horizontal strip bank verbatim.
  - Place outputs below strips in Monitor/HP1/HP2 grid order.
- Delete: `zen-go-slint/ui/pages/route.slint`
  - Delete only after no Slint import or reference remains.
- Modify: `zen-go-slint/ui/domain/output-card.slint`
  - Remove signal-meter rendering.
  - Add literal `STEREO L / R` identity and shared gain-position indicator.
  - Preserve output callbacks and shared gain label.
- Reuse unchanged: `zen-go-slint/ui/domain/preamp-card.slint`
- Reuse unchanged: `zen-go-slint/ui/primitives/zen-source-picker.slint`
- Do not modify: `zen-go-slint/ui/domain/mixer-strip.slint`
- Do not modify: `zen-go-slint/ui/view-models.slint` output fields.

### Contract tests and documentation checks

- Create: `zen-go-slint/tests/ui_contracts.rs`
  - Test source-level tab, picker, layout, output identity, and strip-bank contracts.
  - Assert the Route Slint file is deleted.
- Modify: inline tests in `models.rs`, `commands.rs`, and `ui_bridge.rs`.
- Inspect: `README.md` and GUI usage documentation with the docs-drift workflow; CLI behavior remains unchanged.

---

## Task 1: Freeze Rust and Slint contracts with RED tests

**Files:**
- Create: `zen-go-slint/tests/ui_contracts.rs`
- Modify: `zen-go-slint/src/models.rs` test module
- Modify: `zen-go-slint/src/commands.rs` test module

**Interfaces:**
- Consumes current `GuiPage`, `GuiCommand`, and Slint source files through `include_str!`.
- Produces failing tests that define the four-tab GUI mapping, header picker wiring, Mixer section order, stereo output contract, and unchanged strip-bank markers.

- [ ] **Step 1: Add failing GUI page mapping tests.**

Append this test to `zen-go-slint/src/models.rs` without changing the existing old-index test:

```rust
#[test]
fn gui_page_index_round_trips_visible_pages() {
    assert_eq!(GuiPage::Mixer.gui_index(), Some(0));
    assert_eq!(GuiPage::Profiles.gui_index(), Some(1));
    assert_eq!(GuiPage::Raw.gui_index(), Some(2));
    assert_eq!(GuiPage::Settings.gui_index(), Some(3));
    assert_eq!(GuiPage::Routing.gui_index(), None);

    assert_eq!(GuiPage::from_gui_index(0), Some(GuiPage::Mixer));
    assert_eq!(GuiPage::from_gui_index(1), Some(GuiPage::Profiles));
    assert_eq!(GuiPage::from_gui_index(2), Some(GuiPage::Raw));
    assert_eq!(GuiPage::from_gui_index(3), Some(GuiPage::Settings));
    assert_eq!(GuiPage::from_gui_index(4), None);
}
```

Replace the page assertions in `zen-go-slint/src/commands.rs` test `command_rejects_invalid_page_index` with:

```rust
assert_eq!(
    GuiCommand::set_page_from_index(0),
    Some(GuiCommand::SetPage(GuiPage::Mixer))
);
assert_eq!(
    GuiCommand::set_page_from_index(1),
    Some(GuiCommand::SetPage(GuiPage::Profiles))
);
assert_eq!(
    GuiCommand::set_page_from_index(3),
    Some(GuiCommand::SetPage(GuiPage::Settings))
);
assert_eq!(GuiCommand::set_page_from_index(4), None);
```

- [ ] **Step 2: Add source-level UI contract tests.**

Create `zen-go-slint/tests/ui_contracts.rs` with these exact tests:

```rust
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
    assert!(MAIN.contains("ZenSourcePicker"));
    assert!(MAIN.contains("selected-label: root.sample-rate-label;"));
    assert!(MAIN.contains("choices: root.sample-rate-choices;"));
    assert!(MAIN.contains("choice-requested(index) => { root.sample-rate-requested(index); }"));
    assert!(MAIN.contains("selected-label: root.clock-source-label;"));
    assert!(MAIN.contains("choices: root.clock-source-choices;"));
    assert!(MAIN.contains("choice-requested(index) => { root.clock-source-requested(index); }"));
    assert!(MAIN.contains("reset-token: root.active-page;"));
    assert!(MAIN.contains("enabled: root.controls-available;"));
}

#[test]
fn mixer_orders_preamps_strips_and_outputs_and_keeps_strip_markers() {
    assert!(position(MIXER, "title: \"Preamps\";") < position(MIXER, "title: root.mixer-label + \" Mixer\";"));
    assert!(position(MIXER, "title: root.mixer-label + \" Mixer\";") < position(MIXER, "title: \"Stereo Outputs\";"));
    assert!(MIXER.contains("width: 2006px;"));
    assert!(MIXER.contains("height: max(438px, parent.height);"));
    assert!(MIXER.contains("width: 116px;"));
    assert!(MIXER.contains("context-token: root.picker-reset-token;"));
    assert!(MIXER.contains("assignment-choices: root.assignment-choices;"));
    assert!(MIXER.contains("colspan: output.index == 0 ? 2 : 1;"));
}

#[test]
fn output_card_states_shared_stereo_gain_without_signal_meter() {
    assert!(OUTPUT.contains("title: root.output.name;"));
    assert!(OUTPUT.contains("STEREO L / R"));
    assert!(OUTPUT.contains("root.output.level-percent"));
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
}

#[test]
fn route_page_source_is_deleted() {
    let route_page = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/pages/route.slint");
    assert!(!route_page.exists());
}
```

- [ ] **Step 3: Run RED tests and record expected failures.**

Run:

```bash
cargo test -p zen-go-slint models::tests::gui_page_index_round_trips_visible_pages
cargo test -p zen-go-slint --test ui_contracts
```

Expected:

- The Rust test fails to compile because `gui_index` and `from_gui_index` do not exist.
- The source contract test fails because current shell still has Route, current Mixer has outputs above strips, current output card imports `ZenMeter`, and `ui/pages/route.slint` still exists.

Do not change production files to weaken these tests.

---

## Task 2: Add the GUI page mapping and bridge fallback

**Files:**
- Modify: `zen-go-slint/src/models.rs`
- Modify: `zen-go-slint/src/commands.rs`
- Modify: `zen-go-slint/src/ui_bridge.rs`
- Test: inline tests in the three files above

**Interfaces:**
- Consumes `GuiPage::{Mixer, Routing, Profiles, Raw, Settings}` and existing old page-index behavior.
- Produces `GuiPage::gui_index(self) -> Option<i32>`.
- Produces `GuiPage::from_gui_index(index: i32) -> Option<GuiPage>`.
- Produces `GuiCommand::set_page_from_index(index: i32) -> Option<GuiCommand>` using the GUI mapping.
- Keeps `AppViewModel.active_page: i32` valid for all snapshots by mapping Rust-only Routing to Mixer index `0`.
- Removes `AppViewModel.selected_mixer_channel` and `AppWindow::set_selected_mixer_channel` usage after no Slint consumer remains.

- [ ] **Step 1: Implement GUI-only page methods without changing old methods.**

Add these methods below `from_index` in `impl GuiPage`:

```rust
pub fn gui_index(self) -> Option<i32> {
    match self {
        Self::Mixer => Some(0),
        Self::Routing => None,
        Self::Profiles => Some(1),
        Self::Raw => Some(2),
        Self::Settings => Some(3),
    }
}

pub fn from_gui_index(index: i32) -> Option<Self> {
    match index {
        0 => Some(Self::Mixer),
        1 => Some(Self::Profiles),
        2 => Some(Self::Raw),
        3 => Some(Self::Settings),
        _ => None,
    }
}
```

Leave this existing mapping unchanged:

```rust
pub fn index(self) -> i32 {
    match self {
        Self::Mixer => 0,
        Self::Routing => 1,
        Self::Profiles => 2,
        Self::Raw => 3,
        Self::Settings => 4,
    }
}
```

- [ ] **Step 2: Route tab clicks through the GUI mapping.**

Change only `GuiCommand::set_page_from_index` in `zen-go-slint/src/commands.rs`:

```rust
pub fn set_page_from_index(index: i32) -> Option<Self> {
    GuiPage::from_gui_index(index).map(Self::SetPage)
}
```

Do not change `SetPage`, controller intents, or any non-GUI page conversion.

- [ ] **Step 3: Remove selected-channel UI state and map active pages safely.**

In `AppViewModel`, remove this field:

```rust
pub selected_mixer_channel: i32,
```

In `snapshot_to_view_model`, replace the old active-page and selected-channel initializers with:

```rust
active_page: snapshot.header.active_page.gui_index().unwrap_or(0),
```

Delete the iterator that derives a selected channel. In `apply_snapshot`, delete this setter call:

```rust
window.set_selected_mixer_channel(view_model.selected_mixer_channel);
```

Keep `GuiSnapshot` mixer strip selection and controller routing state unchanged. Add this bridge test:

```rust
#[test]
fn routing_snapshot_falls_back_to_mixer_for_gui_page() {
    let snapshot = GuiSnapshot::disconnected(GuiPage::Routing);
    assert_eq!(snapshot_to_view_model(&snapshot).active_page, 0);
}
```

Remove only the existing bridge assertion for `view_model.selected_mixer_channel`; retain assertions for selected strip data and all models.

- [ ] **Step 4: Run Rust mapping and bridge tests.**

Run:

```bash
cargo test -p zen-go-slint models::tests::gui_page_index_round_trips_visible_pages
cargo test -p zen-go-slint commands::tests::command_rejects_invalid_page_index
cargo test -p zen-go-slint ui_bridge::tests
```

Expected: PASS, with old `page_index_round_trips_known_pages` still proving the five-value Rust/TUI mapping.

---

## Task 3: Present outputs as shared-gain stereo cards

**Files:**
- Modify: `zen-go-slint/ui/domain/output-card.slint`
- Test: `zen-go-slint/tests/ui_contracts.rs`

**Interfaces:**
- Consumes existing `OutputView { index, name, level-step, level-db, level-percent, muted, dimmed }`.
- Preserves callbacks `mute-requested(int)`, `dim-requested(int)`, and `level-adjusted(int, int, int)`.
- Produces destination title, literal `STEREO L / R`, shared dB value, one bounded gain-position indicator, MUTE, and DIM.
- Does not add fields to `OutputView` and does not render `ZenMeter`.

- [ ] **Step 1: Remove the output signal-meter import and add the stereo identity.**

Delete this import from `output-card.slint`:

```slint
import { ZenMeter } from "../primitives/zen-meter.slint";
```

Keep `ZenPanel`, `ZenButton`, and `ZenToggleChip` imports. Keep the existing panel title:

```slint
ZenPanel {
    width: parent.width;
    height: parent.height;
    title: root.output.name;
}
```

- [ ] **Step 2: Replace the meter column with the bounded shared-gain indicator.**

Use this content inside the existing card body, retaining the existing output adjustment and toggle callbacks:

```slint
VerticalLayout {
    x: 16px;
    y: 40px;
    width: parent.width - 32px;
    height: 68px;
    spacing: 5px;

    Text {
        text: "STEREO L / R";
        color: ZenTheme.text-muted;
        font-size: ZenTheme.text-size-small;
        horizontal-alignment: left;
    }

    Rectangle {
        width: parent.width;
        height: 6px;
        border-radius: 3px;
        background: ZenTheme.surface-inset;

        Rectangle {
            width: (root.width - 32px) * max(0, min(100, root.output.level-percent)) / 100;
            height: parent.height;
            border-radius: 3px;
            background: ZenTheme.accent-active;
        }
    }

    HorizontalLayout {
        spacing: ZenTheme.spacing-small;

        Text {
            text: root.output.level-db + " dB";
            color: ZenTheme.accent-active;
            font-size: ZenTheme.text-size-body;
            horizontal-stretch: 1;
            vertical-alignment: center;
        }

        ZenButton {
            label: "−";
            min-width: 30px;
            enabled: root.controls-enabled;
            clicked => { root.level-adjusted(root.output.index, root.output.level-step, 4); }
        }
        ZenButton {
            label: "+";
            min-width: 30px;
            enabled: root.controls-enabled;
            clicked => { root.level-adjusted(root.output.index, root.output.level-step, -4); }
        }
        ZenToggleChip {
            label: "MUTE";
            active: root.output.muted;
            enabled: root.controls-enabled;
            clicked => { root.mute-requested(root.output.index); }
        }
        ZenToggleChip {
            label: "DIM";
            active: root.output.dimmed;
            enabled: root.controls-enabled;
            clicked => { root.dim-requested(root.output.index); }
        }
    }
}
```

The indicator is a position visualization of the shared output gain. It must not be labelled or treated as a live signal meter.

- [ ] **Step 3: Run the source contract and Slint compiler.**

Run:

```bash
cargo test -p zen-go-slint --test ui_contracts
cargo check -p zen-go-slint
```

Expected: The output-card source assertions pass. `cargo check` compiles generated Slint bindings without `ZenMeter` references in `OutputCard`.

---

## Task 4: Move preamps into Mixer and preserve the strip bank

**Files:**
- Modify: `zen-go-slint/ui/pages/mixer.slint`
- Reuse unchanged: `zen-go-slint/ui/domain/preamp-card.slint`
- Do not modify: `zen-go-slint/ui/domain/mixer-strip.slint`
- Test: `zen-go-slint/tests/ui_contracts.rs`

**Interfaces:**
- Adds `in property <[PreampView]> preamps` to `MixerPage`.
- Adds `preamp-phase-requested(int)`, `preamp-phantom-requested(int)`, `preamp-gain-adjusted(int, int, int)`, and `preamp-mode-requested(int, int)` callbacks to `MixerPage`.
- Produces private `MixerPreamp` with the same callback meanings as the deleted Route wrapper.
- Keeps existing `MixerPage` output and mixer callback signatures unchanged.
- Keeps the current 16-strip `ScrollView`, bank width, strip width, context token, assignment model, and callback forwarding unchanged.

- [ ] **Step 1: Add preamp imports, property, and callbacks.**

Update the imports at the top of `mixer.slint`:

```slint
import { ScrollView } from "std-widgets.slint";
import { OutputView, MixerStripView, PreampView, ChoiceView } from "../view-models.slint";
import { ZenTheme } from "../theme.slint";
import { ZenButton } from "../primitives/zen-button.slint";
import { ZenPanel } from "../primitives/zen-panel.slint";
import { OutputCard } from "../domain/output-card.slint";
import { MixerStrip } from "../domain/mixer-strip.slint";
import { PreampCard } from "../domain/preamp-card.slint";
```

Add these members to `MixerPage`:

```slint
in property <[PreampView]> preamps;
callback preamp-phase-requested(int);
callback preamp-phantom-requested(int);
callback preamp-gain-adjusted(int, int, int);
callback preamp-mode-requested(int, int);
```

- [ ] **Step 2: Move the Route preamp composition into a Mixer-local component.**

Add this private component before `MixerPage`:

```slint
component MixerPreamp inherits Rectangle {
    in property <PreampView> preamp;
    in property <bool> controls-enabled: false;
    callback gain-adjusted(int, int, int);
    callback mode-requested(int, int);
    callback phantom-requested(int);
    callback phase-requested(int);

    width: 240px;
    height: 164px;

    PreampCard {
        preamp: root.preamp;
        controls-enabled: root.controls-enabled;
        gain-adjusted(input, current, delta) => { root.gain-adjusted(input, current, delta); }
        mode-requested(input, mode) => { root.mode-requested(input, mode); }
        phantom-requested(input) => { root.phantom-requested(input); }
        phase-requested(input) => { root.phase-requested(input); }
    }

    HorizontalLayout {
        y: 132px;
        width: parent.width;
        height: ZenTheme.control-height;
        spacing: 4px;
        ZenButton {
            label: "Mic";
            enabled: root.controls-enabled;
            active: root.preamp.mode == "Mic";
            min-width: 0px;
            horizontal-stretch: 1;
            clicked => { root.mode-requested(root.preamp.input, 0); }
        }
        ZenButton {
            label: "Line";
            enabled: root.controls-enabled;
            active: root.preamp.mode == "Line";
            min-width: 0px;
            horizontal-stretch: 1;
            clicked => { root.mode-requested(root.preamp.input, 1); }
        }
        ZenButton {
            label: "Hi-Z";
            enabled: root.controls-enabled;
            active: root.preamp.mode == "Hi-Z";
            min-width: 0px;
            horizontal-stretch: 1;
            clicked => { root.mode-requested(root.preamp.input, 2); }
        }
    }
}
```

Do not edit `PreampCard` callback names or Rust preamp command handling.

- [ ] **Step 3: Replace only the outer Mixer layout.**

Replace the current top-level output layout and absolute strip-panel placement with one vertical scroll area. The section order must be:

```slint
ScrollView {
    x: 0px;
    y: 0px;
    width: parent.width;
    height: parent.height;
    horizontal-scrollbar-policy: always-off;
    vertical-scrollbar-policy: as-needed;

    VerticalLayout {
        width: max(parent.visible-width, 760px);
        spacing: ZenTheme.spacing-large;

        ZenPanel {
            width: parent.width;
            height: 220px;
            title: "Preamps";

            GridLayout {
                x: ZenTheme.spacing-medium;
                y: 46px;
                width: parent.width - 32px;
                height: 164px;
                column-gap: ZenTheme.spacing-medium;
                row-gap: 0px;

                for preamp in root.preamps: MixerPreamp {
                    row: 0;
                    col: preamp.input - 1;
                    preamp: preamp;
                    controls-enabled: root.controls-enabled;
                    gain-adjusted(input, current, delta) => { root.preamp-gain-adjusted(input, current, delta); }
                    mode-requested(input, mode) => { root.preamp-mode-requested(input, mode); }
                    phantom-requested(input) => { root.preamp-phantom-requested(input); }
                    phase-requested(input) => { root.preamp-phase-requested(input); }
                }
            }
        }

        ZenPanel {
            width: parent.width;
            height: 480px;
            title: root.mixer-label + " Mixer";

            ScrollView {
                x: ZenTheme.spacing-medium;
                y: 46px;
                width: parent.width - 20px;
                height: parent.height - 56px;
                horizontal-scrollbar-policy: as-needed;
                vertical-scrollbar-policy: as-needed;

                HorizontalLayout {
                    width: 2006px;
                    height: max(438px, parent.height);
                    spacing: ZenTheme.spacing-medium;
                    for strip in root.mixer-strips: VerticalLayout {
                        width: 116px;
                        height: parent.height;
                        alignment: start;
                        MixerStrip {
                            strip: strip;
                            controls-enabled: root.controls-enabled;
                            context-token: root.picker-reset-token;
                            assignment-choices: root.assignment-choices;
                            level-set(channel, value) => { root.mixer-level-set(channel, value); }
                            pan-set(channel, value) => { root.mixer-pan-set(channel, value); }
                            link-requested(channel) => { root.mixer-link-requested(channel); }
                            mute-requested(channel) => { root.mixer-mute-requested(channel); }
                            solo-requested(channel) => { root.mixer-solo-requested(channel); }
                            assignment-requested(channel, index) => { root.assignment-requested(channel, index); }
                        }
                    }
                }
            }
        }

        ZenPanel {
            width: parent.width;
            height: 280px;
            title: "Stereo Outputs";

            GridLayout {
                x: ZenTheme.spacing-medium;
                y: 46px;
                width: parent.width - 32px;
                height: 220px;
                column-gap: ZenTheme.spacing-medium;
                row-gap: ZenTheme.spacing-medium;

                for output in root.outputs: OutputCard {
                    row: output.index == 0 ? 0 : 1;
                    col: output.index == 0 ? 0 : output.index - 1;
                    colspan: output.index == 0 ? 2 : 1;
                    output: output;
                    controls-enabled: root.controls-enabled;
                    level-adjusted(index, current, delta) => { root.output-level-adjusted(index, current, delta); }
                    mute-requested(index) => { root.output-mute-requested(index); }
                    dim-requested(index) => { root.output-dim-requested(index); }
                }
            }
        }
    }
}
```

The inner strip bank above is the existing bank with only its containing panel moved into the outer vertical layout. Do not rename its callbacks, alter `width: 2006px`, alter `width: 116px`, or replace its nested horizontal `ScrollView`.

- [ ] **Step 4: Forward Mixer preamps from the shell laterally through explicit callbacks.**

Before editing `main.slint`, compile the page after adding the callback declarations:

```bash
cargo check -p zen-go-slint
```

Expected: Slint may report missing `MixerPage` property assignments only after `main.slint` is compiled. Do not resolve that by changing callback signatures; Task 5 supplies the shell bindings.

- [ ] **Step 5: Run layout contract checks.**

Run:

```bash
cargo test -p zen-go-slint --test ui_contracts
```

Expected: The test still fails only for shell Route, output-card, and Route-file assertions. Mixer section-order and strip-bank assertions pass.

---

## Task 5: Move selectors into the header and remove GUI Route

**Files:**
- Modify: `zen-go-slint/ui/main.slint`
- Delete: `zen-go-slint/ui/pages/route.slint`
- Verify: `zen-go-slint/src/runtime.rs`
- Test: `zen-go-slint/tests/ui_contracts.rs`

**Interfaces:**
- Consumes `ZenSourcePicker` properties `selected-label`, `choices`, `enabled`, `reset-token`, and callback `choice-requested(int)`.
- Consumes `MixerPage` preamp/output/mixer callback signatures from Task 4.
- Produces four GUI tab indexes: Mixer `0`, Profile `1`, Raw `2`, Settings `3`.
- Preserves AppWindow callback names used by `runtime::wire_callbacks`.
- Leaves Rust `GuiPage::Routing` and controller routing behavior intact.

- [ ] **Step 1: Update shell imports and remove the selected-channel property.**

Use these imports at the top of `main.slint`:

```slint
import { ZenTheme } from "theme.slint";
import { OutputView, MixerStripView, PreampView, ChoiceView } from "view-models.slint";
import { ZenSourcePicker } from "primitives/zen-source-picker.slint";
import { ZenStatusChip } from "primitives/zen-status-chip.slint";
import { ZenTabBar } from "primitives/zen-tab-bar.slint";
import { MixerPage } from "pages/mixer.slint";
import { ProfilePage } from "pages/profile.slint";
import { RawPage } from "pages/raw.slint";
import { SettingsPage } from "pages/settings.slint";
```

Remove this AppWindow property:

```slint
in property <int> selected-mixer-channel: 1;
```

Keep `sample-rate-label`, `clock-source-label`, their choice models, semantic callbacks, and all non-Route page properties.

- [ ] **Step 2: Replace header sample-rate and clock chips with source pickers.**

Replace the current sample-rate and clock `ZenStatusChip` instances with:

```slint
ZenSourcePicker {
    selected-label: root.sample-rate-label;
    choices: root.sample-rate-choices;
    enabled: root.controls-available;
    reset-token: root.active-page;
    choice-requested(index) => { root.sample-rate-requested(index); }
}
ZenSourcePicker {
    selected-label: root.clock-source-label;
    choices: root.clock-source-choices;
    enabled: root.controls-available;
    reset-token: root.active-page;
    choice-requested(index) => { root.clock-source-requested(index); }
}
```

Keep connection, profile, and auto-save status chips. Do not duplicate `PopupWindow` code in `main.slint`.

- [ ] **Step 3: Reduce tabs to four and bind the Mixer page.**

Replace the tab labels with:

```slint
labels: ["Mixer", "Profile", "Raw", "Settings"];
```

Use this complete `MixerPage` branch inside the existing page container:

```slint
MixerPage {
    visible: root.active-page == 0;
    width: parent.width;
    height: parent.height;
    mixer-label: root.mixer-label;
    outputs: root.outputs;
    preamps: root.preamps;
    controls-enabled: root.controls-available;
    picker-reset-token: root.active-page;
    mixer-strips: root.mixer-strips;
    assignment-choices: root.assignment-choices;
    output-level-adjusted(index, current, delta) => { root.output-level-adjusted(index, current, delta); }
    output-mute-requested(index) => { root.output-mute-requested(index); }
    output-dim-requested(index) => { root.output-dim-requested(index); }
    mixer-level-set(channel, value) => { root.mixer-level-set(channel, value); }
    mixer-pan-set(channel, value) => { root.mixer-pan-set(channel, value); }
    mixer-link-requested(channel) => { root.mixer-link-requested(channel); }
    mixer-mute-requested(channel) => { root.mixer-mute-requested(channel); }
    mixer-solo-requested(channel) => { root.mixer-solo-requested(channel); }
    assignment-requested(channel, index) => { root.assignment-requested(channel, index); }
    preamp-phase-requested(input) => { root.preamp-phase-requested(input); }
    preamp-phantom-requested(input) => { root.preamp-phantom-requested(input); }
    preamp-gain-adjusted(input, current, delta) => { root.preamp-gain-adjusted(input, current, delta); }
    preamp-mode-requested(input, mode) => { root.preamp-mode-requested(input, mode); }
}
```

- [ ] **Step 4: Remove the Route branch and renumber remaining page visibility.**

Delete the entire `RoutePage` instance, including its selected-channel, preamp, sample-rate, clock-source, and Route callback bindings. Set remaining page visibility exactly as follows:

```slint
ProfilePage {
    visible: root.active-page == 1;
    width: parent.width;
    height: parent.height;
    profiles: root.profiles;
    controls-enabled: root.controls-available;
    profile-selected(index) => { root.profile-selected(index); }
    profile-load-requested => { root.profile-load-requested(); }
    profile-save-requested => { root.profile-save-requested(); }
    profile-rename-requested => { root.profile-rename-requested(); }
    profile-delete-requested => { root.profile-delete-requested(); }
}

RawPage {
    visible: root.active-page == 2;
    width: parent.width;
    height: parent.height;
    raw-tab-label: root.raw-tab-label;
    controls-enabled: root.controls-available;
    raw-summary: root.raw-summary;
    raw-rows: root.raw-rows;
    raw-capture-baseline-requested => { root.raw-capture-baseline-requested(); }
    raw-clear-baseline-requested => { root.raw-clear-baseline-requested(); }
    refresh-requested => { root.refresh-requested(); }
}

SettingsPage {
    visible: root.active-page == 3;
    width: parent.width;
    height: parent.height;
    refresh-rate-label: root.refresh-rate-label;
    peak-threshold-label: root.peak-threshold-label;
    peak-hold-label: root.peak-hold-label;
    peak-enabled: root.peak-enabled;
    controls-enabled: root.controls-available;
    auto-save: root.auto-save;
    refresh-rate-requested(index) => { root.settings-refresh-rate-requested(index); }
    peak-threshold-adjusted(increase) => { root.settings-peak-threshold-adjusted(increase); }
    peak-enabled-toggled => { root.settings-peak-enabled-toggled(); }
    peak-hold-requested(index) => { root.settings-peak-hold-requested(index); }
    auto-save-toggled => { root.auto-save-toggled(); }
}
```

Preserve each existing page's property and callback bindings. Only page indexes and the removed Route block change.

- [ ] **Step 5: Delete the obsolete Route page after reference search.**

Run:

```bash
rg -n "RoutePage|RoutePreamp|selected-mixer-channel" zen-go-slint/ui
```

Expected: no output. Then delete `zen-go-slint/ui/pages/route.slint`.

Run the Rust/Slint compile:

```bash
cargo check -p zen-go-slint
```

Expected: PASS. `runtime::wire_callbacks` continues to compile against the same AppWindow callback names and argument types. If generated bindings report a missing callback, restore the corresponding AppWindow callback declaration rather than changing `runtime.rs` command semantics.

- [ ] **Step 6: Run all focused contract tests.**

Run:

```bash
cargo test -p zen-go-slint --test ui_contracts
cargo test -p zen-go-slint
```

Expected: PASS. The UI source contracts prove four tabs, header pickers, Mixer order, output identity, unchanged strip markers, and deleted Route source. Package tests prove old Rust page indexing, new GUI page indexing, bridge fallback, command routing, and existing control behavior.

---

## Task 6: Run final verification and GUI smoke checks

**Files:**
- Verify final diff for all files listed in the File Map.
- No additional source file is expected to change in this task.

**Interfaces:**
- Consumes the complete implementation from Tasks 1 through 5.
- Produces validated Rust, generated Slint bindings, source contracts, workspace tests, build artifacts, diagnostics, and one mock GUI smoke run.

- [ ] **Step 1: Run package formatting and focused tests.**

Run from `/home/ryodeushii/repos/zen-go-tui/.worktrees/gui-redesign`:

```bash
cargo fmt --package zen-go-slint -- --check
cargo test -p zen-go-slint --test ui_contracts
cargo test -p zen-go-slint
```

Expected: all commands pass. Do not run a workspace-wide formatter that changes unrelated packages.

- [ ] **Step 2: Run workspace tests, build, and whitespace validation.**

Run:

```bash
cargo test --workspace
cargo build -p zen-go-slint
git diff --check
```

Expected: workspace tests pass, `zen-go-slint` builds, and `git diff --check` emits no whitespace errors. If an unrelated pre-existing workspace formatting failure remains, report its exact files and do not modify it as part of this feature.

- [ ] **Step 3: Run edited-file diagnostics.**

Invoke:

```text
lens_diagnostics(mode="all", paths=[
  "zen-go-slint/src/models.rs",
  "zen-go-slint/src/commands.rs",
  "zen-go-slint/src/ui_bridge.rs",
  "zen-go-slint/ui/main.slint",
  "zen-go-slint/ui/pages/mixer.slint",
  "zen-go-slint/ui/domain/output-card.slint",
  "zen-go-slint/ui/pages/route.slint"
])
```

Expected: no blocking diagnostics for edited or deleted paths. If the deleted path is ignored by diagnostics, verify deletion with `test ! -e zen-go-slint/ui/pages/route.slint`.

- [ ] **Step 4: Check documentation drift.**

Run the `docs-drift` skill against the final diff and README. Confirm CLI invocation and mock-mode documentation remain accurate. If drift reports the removed Route tab or old tab list, update only the affected documentation before rerunning package tests and `git diff --check`.

- [ ] **Step 5: Launch exactly one fresh mock GUI instance.**

Build already completed in Step 2. From the worktree, launch one process:

```bash
cargo run -p zen-go-slint -- --mock --no-bootstrap
```

Verify there is one visible Zen Go window and no second application instance. Keep process lifetime limited to this smoke check and exit it after captures. Do not start another GUI process while the first is running.

- [ ] **Step 6: Perform compact and tall GUI checks.**

With the one mock window visible, capture one compact and one tall window state using Spectacle when available. Check each state manually:

1. Header contains connection status, interactive sample-rate picker, interactive clock-source picker, profile status, and auto-save status.
2. Tabs read `Mixer | Profile | Raw | Settings`; no Route tab or selected-channel label appears.
3. Mixer order reads Preamps, existing mixer strip bank, Stereo Outputs.
4. A1 and A2 mode, gain, 48V, and PHASE controls work while connected and are disabled while disconnected.
5. Inner mixer strip area scrolls horizontally and reaches all 16 strips.
6. Output cards read destination name and `STEREO L / R`, show one shared dB value and gain-position indicator, and expose MUTE/DIM.
7. Monitor spans the two output columns; HP1 and HP2 occupy the next row.
8. Header picker popup closes on selection, outside click, page change, and disconnect.
9. Footer continues to show command or polling notices.

If Spectacle or an interactive display is unavailable, record that manual capture check as skipped while retaining all automated evidence.

- [ ] **Step 7: Inspect final diff and status.**

Run:

```bash
git status --short
git diff --stat
git diff -- zen-go-slint/src/models.rs zen-go-slint/src/commands.rs zen-go-slint/src/ui_bridge.rs zen-go-slint/ui/main.slint zen-go-slint/ui/pages/mixer.slint zen-go-slint/ui/domain/output-card.slint zen-go-slint/ui/pages/route.slint zen-go-slint/tests/ui_contracts.rs
```

Confirm only approved files changed, Route deletion is intentional, no protocol/controller/TUI files changed, no dependency changed, and no files are staged. Do not commit; project instructions require explicit user request before committing.

---

## Plan completion criteria

The implementation is ready for final review only when:

- Four GUI tabs map contiguously without changing Rust's five-page/TUI mapping.
- Header sample-rate and clock controls use `ZenSourcePicker` and existing popup behavior.
- Mixer contains preamps above the unchanged horizontal strip bank and shared-gain stereo outputs below it.
- Route Slint source and all GUI Route references are gone while Rust Routing remains.
- Output cards contain no fake independent L/R data or signal meter.
- Focused tests, package tests, workspace tests, build, formatting, whitespace, diagnostics, docs-drift, and available GUI checks have recorded results.
- Final diff contains no unrelated changes and no commit is created without user request.
