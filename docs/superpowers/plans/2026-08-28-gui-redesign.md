# Zen-Go Slint GUI Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current monolithic Slint screen with a Warm Hardware design system and a mixer-first GUI that keeps direct controls on every strip.

**Architecture:** Keep Rust `Controller`, `WorkerCore`, `GuiCommand`, `GuiSnapshot`, mapper, bridge, transport, and protocol ownership unchanged. Split Slint into shared view models, theme tokens, primitives, audio domain modules, page modules, and a shell that composes horizontal tabs and pages. Send direct mixer values through one-based channel callbacks and validate them in Rust.

**Tech Stack:** Rust edition 2021, Slint 1.16.0, `slint-build` 1.16.0, Ratatui 0.30, Cargo workspace, mock transport.

**Spec:** `docs/superpowers/specs/2026-08-28-gui-redesign-design.md`

## Global Constraints

- Do not add Material Slint, another component dependency, or a separate UI crate.
- Keep Rust `WorkerCore`, `Controller`, transport ownership, protocol behavior, and TUI behavior stable.
- Keep `GuiSnapshot` as the authoritative input to Slint and use the next snapshot to restore rejected values.
- Use one-based device channels at the Slint callback interface and convert them in one Rust validation seam.
- Accept direct mixer levels from `0` through `0x5a` (`90`).
- Accept direct pan values from `PanState::MIN` (`0x02`) through `PanState::MAX` (`0x3e`) without mute or solo flag bits.
- Expose mixer link only for linkable channels. Preserve the current odd-numbered-channel rule in Rust mapping and command validation.
- Keep meter and level values independent.
- Keep all 16 strips reachable through horizontal scrolling at the existing 1280x820 window size.
- Apply Warm Hardware tokens to Mixer, Route, Profile, Raw, and Settings.
- Emit no more than one direct-control event for each distinct rounded integer value during one gesture.
- Do not create commits unless the user requests them.

## File Map

### Rust contract and state files

- Modify `zen-go-slint/src/commands.rs` to validate direct level, pan, link, and channel-aware assignment callbacks.
- Modify `zen-go-slint/src/models.rs` to add `linkable` to `MixerStripSnapshot`.
- Modify `zen-go-slint/src/mapper.rs` to map channel linkability into each snapshot.
- Modify `zen-go-slint/src/ui_bridge.rs` to copy `linkable` into generated Slint view models.
- Modify `zen-go-slint/src/runtime.rs` to wire direct mixer, link, and channel-aware assignment callbacks.
- Keep Rust tests beside their existing command, mapper, bridge, and worker tests.

### Slint files

- Create `zen-go-slint/ui/view-models.slint` for exported structs consumed by Rust and Slint modules.
- Create `zen-go-slint/ui/theme.slint` for Warm Hardware colors, typography, spacing, radii, borders, and control sizes.
- Create `zen-go-slint/ui/primitives/zen-button.slint` for shared actions.
- Create `zen-go-slint/ui/primitives/zen-tab-bar.slint` for horizontal page tabs.
- Create `zen-go-slint/ui/primitives/zen-panel.slint` for shared panel surfaces and headings.
- Create `zen-go-slint/ui/primitives/zen-status-chip.slint` for header status indicators.
- Create `zen-go-slint/ui/primitives/zen-meter.slint` for independent meter rendering.
- Create `zen-go-slint/ui/primitives/zen-level-fader.slint` for direct level dragging.
- Create `zen-go-slint/ui/primitives/zen-pan-slider.slint` for direct pan dragging.
- Create `zen-go-slint/ui/primitives/zen-toggle-chip.slint` for link, solo, mute, and autosave states.
- Create `zen-go-slint/ui/primitives/zen-source-picker.slint` for inline assignment choices.
- Create `zen-go-slint/ui/domain/output-card.slint` for output controls.
- Create `zen-go-slint/ui/domain/mixer-strip.slint` for the direct-control mixer strip.
- Create `zen-go-slint/ui/domain/preamp-card.slint` for preamp controls.
- Create `zen-go-slint/ui/pages/mixer.slint` for output cards and the horizontal strip bank.
- Create `zen-go-slint/ui/pages/route.slint` for preamps, sample rate, and clock source.
- Create `zen-go-slint/ui/pages/profile.slint` for profile actions and selection.
- Create `zen-go-slint/ui/pages/raw.slint` for raw rows and baseline actions.
- Create `zen-go-slint/ui/pages/settings.slint` for settings labels and autosave.
- Modify `zen-go-slint/ui/main.slint` so it contains only `AppWindow`, shell layout, top tabs, shared status, footer, and page composition.

## Task 1: Add direct mixer command validation

**Files:**
- Modify: `zen-go-slint/src/commands.rs`
- Test: `zen-go-slint/src/commands.rs` test module

**Interfaces:**
- Consumes existing `GuiCommand`, `PanState`, `MixerAssignment`, and one-based channel conventions.
- Produces `set_mixer_level_by_channel(channel: i32, level: i32) -> Option<GuiCommand>`.
- Produces `set_mixer_pan_by_channel(channel: i32, raw: i32) -> Option<GuiCommand>`.
- Produces `toggle_mixer_link(channel: u8) -> Option<GuiCommand>` with odd-channel validation.
- Produces `toggle_mixer_link_by_channel(channel: i32) -> Option<GuiCommand>` for signed Slint callback values.
- Produces `pick_assignment_from_indices(channel: i32, choice_index: i32) -> Option<GuiCommand>`.

- [ ] **Step 1: Write failing command tests.**

Add tests that exercise callback-shaped signed integers before conversion to unsigned types.

```rust
#[test]
fn direct_mixer_events_validate_channels_and_ranges() {
    assert_eq!(
        GuiCommand::set_mixer_level_by_channel(1, 0),
        Some(GuiCommand::SetMixerLevel { index: 0, level: 0 })
    );
    assert_eq!(
        GuiCommand::set_mixer_level_by_channel(16, 90),
        Some(GuiCommand::SetMixerLevel {
            index: 15,
            level: 90
        })
    );
    assert_eq!(GuiCommand::set_mixer_level_by_channel(0, 20), None);
    assert_eq!(GuiCommand::set_mixer_level_by_channel(17, 20), None);
    assert_eq!(GuiCommand::set_mixer_level_by_channel(1, -1), None);
    assert_eq!(GuiCommand::set_mixer_level_by_channel(1, 91), None);

    assert_eq!(
        GuiCommand::set_mixer_pan_by_channel(1, i32::from(PanState::MIN)),
        Some(GuiCommand::SetMixerPan {
            index: 0,
            pan: PanState::from_raw(PanState::MIN)
        })
    );
    assert_eq!(
        GuiCommand::set_mixer_pan_by_channel(16, i32::from(PanState::MAX)),
        Some(GuiCommand::SetMixerPan {
            index: 15,
            pan: PanState::from_raw(PanState::MAX)
        })
    );
    assert_eq!(GuiCommand::set_mixer_pan_by_channel(1, 1), None);
    assert_eq!(GuiCommand::set_mixer_pan_by_channel(1, 0x3f), None);
    assert_eq!(GuiCommand::set_mixer_pan_by_channel(-1, 0x20), None);
}

#[test]
fn link_and_assignment_events_use_device_channels() {
    assert!(GuiCommand::toggle_mixer_link(1).is_some());
    assert_eq!(GuiCommand::toggle_mixer_link(2), None);
    assert_eq!(GuiCommand::toggle_mixer_link(17), None);
    assert!(GuiCommand::toggle_mixer_link_by_channel(1).is_some());
    assert_eq!(GuiCommand::toggle_mixer_link_by_channel(2), None);
    assert_eq!(GuiCommand::toggle_mixer_link_by_channel(-1), None);

    assert!(GuiCommand::pick_assignment_from_indices(16, 0).is_some());
    assert_eq!(GuiCommand::pick_assignment_from_indices(0, 0), None);
    assert_eq!(GuiCommand::pick_assignment_from_indices(1, -1), None);
}
```

- [ ] **Step 2: Run the focused tests and verify failure.**

Run: `cargo test -p zen-go-slint commands::tests::direct_mixer_events_validate_channels_and_ranges commands::tests::link_and_assignment_events_use_device_channels`

Expected: FAIL because direct callback helpers do not exist and even mixer channels still pass link validation.

- [ ] **Step 3: Add one safe conversion and validation seam.**

Add a private channel converter and direct callback helpers. Keep existing index-based helpers for internal command construction.

```rust
const MAX_DIRECT_MIXER_LEVEL: u8 = 0x5a;

fn channel_index(channel: i32) -> Option<usize> {
    let channel = u8::try_from(channel).ok()?;
    valid_channel(channel).then_some(usize::from(channel - 1))
}

pub fn set_mixer_level_by_channel(channel: i32, level: i32) -> Option<Self> {
    let index = channel_index(channel)?;
    let level = u8::try_from(level).ok()?;
    (level <= MAX_DIRECT_MIXER_LEVEL).then_some(Self::SetMixerLevel { index, level })
}

pub fn set_mixer_pan_by_channel(channel: i32, raw: i32) -> Option<Self> {
    let index = channel_index(channel)?;
    let raw = u8::try_from(raw).ok()?;
    (PanState::MIN..=PanState::MAX)
        .contains(&raw)
        .then_some(Self::SetMixerPan {
            index,
            pan: PanState::from_raw(raw),
        })
}

pub fn pick_assignment_from_indices(channel: i32, choice_index: i32) -> Option<Self> {
    let channel = u8::try_from(channel).ok()?;
    let choice_index = usize::try_from(choice_index).ok()?;
    Self::pick_assignment_from_index(channel, choice_index)
}
```

Update `toggle_mixer_link` so it accepts only valid odd channels.

```rust
pub fn toggle_mixer_link(channel: u8) -> Option<Self> {
    (valid_channel(channel) && channel % 2 == 1).then_some(Self::ToggleMixerLink(channel))
}

pub fn toggle_mixer_link_by_channel(channel: i32) -> Option<Self> {
    let channel = u8::try_from(channel).ok()?;
    Self::toggle_mixer_link(channel)
}
```

- [ ] **Step 4: Run command tests and the formatter.**

Run: `cargo test -p zen-go-slint commands::tests`

Expected: PASS for existing command tests and new direct-control tests.

Run: `cargo fmt --check`

Expected: PASS.

## Task 2: Add linkability to snapshots and mapper output

**Files:**
- Modify: `zen-go-slint/src/models.rs`
- Modify: `zen-go-slint/src/mapper.rs`
- Test: `zen-go-slint/src/models.rs` and `zen-go-slint/src/mapper.rs` test modules

**Interfaces:**
- Consumes `MixerChannelState.channel` and existing `MixerStripSnapshot` construction.
- Produces `MixerStripSnapshot.linkable: bool` for every mixer strip.
- Keeps the current device rule in Rust with `channel % 2 == 1`.

- [ ] **Step 1: Write failing model and mapper assertions.**

Add the model default assertion and extend `maps_active_mixer_surface_and_strips` with both odd and even channels.

```rust
#[test]
fn empty_mixer_strips_expose_linkability() {
    assert!(MixerStripSnapshot::empty(1).linkable);
    assert!(!MixerStripSnapshot::empty(2).linkable);
}
```

Extend the existing mapper test after the selected channel assertions.

```rust
assert!(snapshot.mixer.strips[2].linkable);
assert!(!snapshot.mixer.strips[1].linkable);
```

- [ ] **Step 2: Run focused tests and verify failure.**

Run: `cargo test -p zen-go-slint mapper::tests::maps_active_mixer_surface_and_strips models::tests::empty_mixer_strips_expose_linkability`

Expected: FAIL because `MixerStripSnapshot` has no `linkable` field or test yet.

- [ ] **Step 3: Add the field and map the device capability.**

Add `linkable: bool` to `MixerStripSnapshot`. Set the fallback value in `empty` and the mapped value in `mixer_strip_from_state`.

```rust
pub struct MixerStripSnapshot {
    pub channel: u8,
    pub name: String,
    pub assignment_label: String,
    pub assignment_short_label: String,
    pub level: u8,
    pub level_ratio: f32,
    pub meter_ratio: f32,
    pub pan_raw: u8,
    pub pan_ratio: f32,
    pub pan_display: i16,
    pub muted: bool,
    pub soloed: bool,
    pub linked: bool,
    pub linkable: bool,
    pub selected: bool,
}
```

```rust
linkable: channel.channel % 2 == 1,
```

Use the same expression in `MixerStripSnapshot::empty` so disconnected snapshots expose capability without putting the rule in Slint.

- [ ] **Step 4: Run mapper and model tests.**

Run: `cargo test -p zen-go-slint mapper::tests models::tests`

Expected: PASS.

## Task 3: Create shared Slint view models, theme, and basic primitives

**Files:**
- Create: `zen-go-slint/ui/view-models.slint`
- Create: `zen-go-slint/ui/theme.slint`
- Create: `zen-go-slint/ui/primitives/zen-button.slint`
- Create: `zen-go-slint/ui/primitives/zen-tab-bar.slint`
- Create: `zen-go-slint/ui/primitives/zen-panel.slint`
- Create: `zen-go-slint/ui/primitives/zen-status-chip.slint`
- Modify: `zen-go-slint/ui/main.slint`
- Modify: `zen-go-slint/src/ui_bridge.rs`
- Test: `zen-go-slint/src/ui_bridge.rs` test module

**Interfaces:**
- Consumes Rust `OutputView`, `MixerStripView`, `PreampView`, and `ChoiceView` generated types.
- Produces shared exported Slint structs and `MixerStripView.linkable`.
- Produces `ZenTheme`, `ZenButton`, `ZenTabBar`, `ZenPanel`, and `ZenStatusChip` interfaces for later page modules.

- [ ] **Step 1: Define the shared Slint view contract.**

Create `view-models.slint` and move the four exported structs from `main.slint` into it. Keep field names and types unchanged except for `linkable`.

```slint
export struct MixerStripView {
    channel: int,
    name: string,
    assignment: string,
    level-step: int,
    level-db: int,
    meter-percent: int,
    pan-raw: int,
    pan: int,
    muted: bool,
    soloed: bool,
    linked: bool,
    linkable: bool,
    selected: bool,
}
```

Import the structs from `view-models.slint` at the top of `main.slint` and remove their duplicate declarations.

- [ ] **Step 2: Add the bridge assertion before implementing the bridge field.**

Extend `bridge_converts_complete_snapshot_to_slint_models`.

```rust
assert!(view_model.mixer_strips[0].linkable);
assert!(!view_model.mixer_strips[1].linkable);
```

- [ ] **Step 3: Run the bridge test and verify the generated contract failure.**

Run: `cargo test -p zen-go-slint ui_bridge::tests::bridge_converts_complete_snapshot_to_slint_models`

Expected: FAIL to compile because `strip_to_view` does not initialize the new generated `linkable` field.

- [ ] **Step 4: Copy linkability through the bridge.**

Update `strip_to_view`.

```rust
linked: strip.linked,
linkable: strip.linkable,
selected: strip.selected,
```

- [ ] **Step 5: Add Warm Hardware tokens.**

Create `theme.slint` with one exported global. Keep shared visual values in this global instead of literal values in domain modules.

```slint
export global ZenTheme {
    in property <color> surface-deep: #171314;
    in property <color> surface-panel: #241b1c;
    in property <color> surface-raised: #302324;
    in property <color> surface-inset: #100d0e;
    in property <color> accent-primary: #d97868;
    in property <color> accent-active: #f0a05f;
    in property <color> text-primary: #fff2ec;
    in property <color> text-muted: #b8a49d;
    in property <color> state-connected: #78bf8b;
    in property <color> state-error: #d65b5b;
    in property <color> border: #5a4541;
    in property <length> control-height: 30px;
    in property <length> panel-radius: 10px;
    in property <length> spacing-small: 6px;
    in property <length> spacing-medium: 10px;
    in property <length> spacing-large: 16px;
}
```

- [ ] **Step 6: Create basic shared primitives.**

Use `ZenTheme` in each primitive. Keep interfaces small.

```slint
export component ZenButton inherits Rectangle {
    in property <string> label;
    in property <bool> active: false;
    in property <bool> destructive: false;
    in property <bool> enabled: true;
    callback clicked;
}

export component ZenTabBar inherits Rectangle {
    in property <[string]> labels;
    in property <int> active-index;
    callback tab-requested(int);
}

export component ZenPanel inherits Rectangle {
    in property <string> title;
}

export component ZenStatusChip inherits Rectangle {
    in property <string> label;
    in property <bool> active;
}
```

`ZenButton` must expose pressed, hover, disabled, active, and destructive states. `ZenTabBar` must render `Mixer`, `Route`, `Profile`, `Raw`, and `Settings` labels supplied by its caller. `ZenPanel` must render a shared heading and background. `ZenStatusChip` must render active and inactive indicators.

- [ ] **Step 7: Compile the shared contract and bridge.**

Run: `cargo test -p zen-go-slint ui_bridge::tests`

Expected: PASS with generated Slint bindings containing `MixerStripView.linkable`.

## Task 4: Create direct-control primitives and audio domain modules

**Files:**
- Create: `zen-go-slint/ui/primitives/zen-meter.slint`
- Create: `zen-go-slint/ui/primitives/zen-level-fader.slint`
- Create: `zen-go-slint/ui/primitives/zen-pan-slider.slint`
- Create: `zen-go-slint/ui/primitives/zen-toggle-chip.slint`
- Create: `zen-go-slint/ui/primitives/zen-source-picker.slint`
- Create: `zen-go-slint/ui/domain/output-card.slint`
- Create: `zen-go-slint/ui/domain/mixer-strip.slint`
- Create: `zen-go-slint/ui/domain/preamp-card.slint`

**Interfaces:**
- Consumes `ZenTheme`, `OutputView`, `MixerStripView`, `PreampView`, and `ChoiceView`.
- Produces direct level and pan callbacks with integer values.
- Produces `MixerStrip` callbacks: `selected(int)`, `level-set(int, int)`, `pan-set(int, int)`, `link-requested(int)`, `mute-requested(int)`, `solo-requested(int)`, and `assignment-requested(int, int)`.
- Produces independent meter rendering and local source-picker open state.

- [ ] **Step 1: Define direct-control primitive interfaces.**

Create interfaces with these exact properties and callbacks.

```slint
export component ZenLevelFader inherits Rectangle {
    in-out property <int> value;
    in property <int> maximum: 90;
    in property <bool> enabled: true;
    callback value-changed(int);
    callback value-released(int);
}

export component ZenPanSlider inherits Rectangle {
    in-out property <int> value;
    in property <int> minimum-raw: 2;
    in property <int> maximum-raw: 62;
    in property <bool> enabled: true;
    callback value-changed(int);
    callback value-released(int);
}

export component ZenMeter inherits Rectangle {
    in property <int> percent;
    in property <bool> peak: false;
}

export component ZenToggleChip inherits Rectangle {
    in property <string> label;
    in property <bool> active;
    in property <bool> visible: true;
    in property <bool> enabled: true;
    callback clicked;
}

export component ZenSourcePicker inherits Rectangle {
    in property <string> selected-label;
    in property <[ChoiceView]> choices;
    in property <bool> enabled: true;
    callback choice-requested(int);
}
```

- [ ] **Step 2: Implement level dragging with bounded integer events.**

Cover the fader with a `TouchArea`. Map `mouse-y` to a clamped `0..maximum` value with the top at maximum and the bottom at zero. Track the last emitted value. Emit `value-changed` only when the rounded value changes. Use `pointer-event` and `PointerEventKind.up` for the final release event because `TouchArea` has no dedicated release callback.

```slint
private property <int> last-emitted: -1;

TouchArea {
    enabled: root.enabled;
    moved => {
        let next = round((1.0 - self.mouse-y / root.height) * root.maximum);
        let bounded = max(0, min(root.maximum, next));
        if (bounded != root.last-emitted) {
            root.value = bounded;
            root.last-emitted = bounded;
            root.value-changed(bounded);
        }
    }
    pointer-event(event) => {
        if (event.kind == PointerEventKind.up && root.value != root.last-emitted) {
            root.last-emitted = root.value;
            root.value-released(root.value);
        }
    }
}
```

Set `root.value` from snapshots before the next gesture. Do not couple fader position to meter height.

- [ ] **Step 3: Implement pan dragging with position-only raw values.**

Map `mouse-x` to a normalized ratio and then to `minimum-raw..maximum-raw`. Clamp before rounding. Emit only distinct raw values. Do not add flag bits to the emitted value.

```slint
let ratio = max(0.0, min(1.0, self.mouse-x / root.width));
let next = round(root.minimum-raw + ratio * (root.maximum-raw - root.minimum-raw));
```

- [ ] **Step 4: Implement meter, toggle, and source picker behavior.**

Render `ZenMeter` from `percent` only. Render a distinct peak color when `peak` is true. Make `ZenToggleChip` show active and disabled states. Make `ZenSourcePicker` keep `open` as local state, render `choices` when open, emit the selected choice index, and close after selection.

- [ ] **Step 5: Write the mixer domain interface before its body.**

Use one-based `strip.channel` values in every domain callback.

```slint
export component MixerStrip inherits Rectangle {
    in property <MixerStripView> strip;
    in property <[ChoiceView]> assignment-choices;
    callback selected(int);
    callback level-set(int, int);
    callback pan-set(int, int);
    callback link-requested(int);
    callback mute-requested(int);
    callback solo-requested(int);
    callback assignment-requested(int, int);
}
```

- [ ] **Step 6: Compose the direct-control strip.**

Render channel and source labels, source picker, pan value and slider, independent meter, level fader, level value, and inline link, solo, and mute chips. Render link only when `strip.linkable` is true. Use `strip.selected` for highlight only. Do not place a full-strip `TouchArea` above the fader or pan slider.

Connect direct controls as follows.

```slint
ZenLevelFader {
    value: min(strip.level-step, 90);
    value-changed(value) => { root.level-set(strip.channel, value); }
    value-released(value) => { root.level-set(strip.channel, value); }
}
ZenPanSlider {
    value: strip.pan-raw;
    value-changed(value) => { root.pan-set(strip.channel, value); }
    value-released(value) => { root.pan-set(strip.channel, value); }
}
```

Use a small selectable header or empty strip area for `selected(strip.channel)`. Keep the strip width compact enough to show several channels and let `MixerPage` provide horizontal scrolling.

- [ ] **Step 7: Compose output and preamp domain modules.**

Move current output and preamp behavior into `OutputCard` and `PreampCard`. Preserve existing delta callbacks for output and preamp step buttons. Use `ZenPanel`, `ZenButton`, `ZenMeter`, and `ZenToggleChip` rather than local color literals.

## Task 5: Build pages and the horizontal-tab shell

**Files:**
- Create: `zen-go-slint/ui/pages/mixer.slint`
- Create: `zen-go-slint/ui/pages/route.slint`
- Create: `zen-go-slint/ui/pages/profile.slint`
- Create: `zen-go-slint/ui/pages/raw.slint`
- Create: `zen-go-slint/ui/pages/settings.slint`
- Modify: `zen-go-slint/ui/main.slint`

**Interfaces:**
- Consumes all generated `AppWindow` properties and shared primitives/domain modules.
- Produces the existing page callback names plus new direct mixer callback names.
- Keeps five page indexes unchanged: Mixer `0`, Route `1`, Profile `2`, Raw `3`, Settings `4`.

- [ ] **Step 1: Define page interfaces.**

Keep page modules presentational. Each page receives snapshot properties and emits callbacks to `AppWindow`.

```slint
export component MixerPage inherits Rectangle {
    in property <string> mixer-label;
    in property <[OutputView]> outputs;
    in property <[MixerStripView]> mixer-strips;
    in property <[ChoiceView]> assignment-choices;
    callback output-level-adjusted(int, int, int);
    callback output-mute-requested(int);
    callback output-dim-requested(int);
    callback mixer-channel-selected(int);
    callback mixer-level-set(int, int);
    callback mixer-pan-set(int, int);
    callback mixer-link-requested(int);
    callback mixer-mute-requested(int);
    callback mixer-solo-requested(int);
    callback assignment-requested(int, int);
}
```

Apply the same pattern to Route, Profile, Raw, and Settings with their existing data and callback contracts.

- [ ] **Step 2: Implement the Mixer page.**

Render output cards at the top. Render the 16 `MixerStrip` instances inside a horizontal `ScrollView`. Pass `assignment-choices` to each strip. Forward every strip callback without changing its channel or value.

- [ ] **Step 3: Implement the remaining pages.**

Use shared panels, buttons, toggles, status colors, and choice controls.

- Keep Route preamp, sample-rate, and clock-source controls.
- Move assignment selection to each mixer strip and remove the duplicate selected-channel assignment grid from Route.
- Keep Profile load, save, rename, delete, and profile selection callbacks.
- Keep Raw baseline, clear, refresh, summary, and row rendering.
- Keep Settings refresh, peak, and autosave presentation.

- [ ] **Step 4: Replace the shell with Warm Hardware layout.**

Remove the left rail and old inline components from `main.slint`. Import `ZenTheme`, `ZenTabBar`, `ZenStatusChip`, and all page modules. Keep the window title, minimum size, header status values, footer notice, and snapshot properties.

Use this callback contract in `AppWindow`.

```slint
callback mixer-channel-selected(int);
callback mixer-mute-requested(int);
callback mixer-solo-requested(int);
callback mixer-level-set(int, int);
callback mixer-pan-set(int, int);
callback mixer-link-requested(int);
callback assignment-requested(int, int);
```

Remove `mixer-level-adjusted`, `mixer-pan-adjusted`, and the one-argument `assignment-requested` callback. Keep output, preamp, profile, raw, refresh, autosave, and page callbacks.

- [ ] **Step 5: Compose top tabs and page visibility.**

Use labels `Mixer`, `Route`, `Profile`, `Raw`, and `Settings`. Forward `tab-requested(index)` to `page-requested(index)`. Show one page from `active-page`. Keep status chips visible in the header and the footer notice visible across page changes.

- [ ] **Step 6: Compile the complete Slint surface.**

Run: `cargo check -p zen-go-slint`

Expected: PASS with generated `on_mixer_level_set`, `on_mixer_pan_set`, `on_mixer_link_requested`, and two-argument `on_assignment_requested` methods available to Rust.

## Task 6: Wire direct callbacks through runtime

**Files:**
- Modify: `zen-go-slint/src/runtime.rs`
- Test: existing `zen-go-slint/src/commands.rs` tests and generated Slint compile surface

**Interfaces:**
- Consumes generated `AppWindow` callbacks from Task 5.
- Produces validated `GuiCommand` values through the helpers from Task 1.
- Keeps worker thread, polling, snapshot delivery, and error handling unchanged.

- [ ] **Step 1: Run the compile test before changing runtime.**

Run: `cargo check -p zen-go-slint`

Expected: FAIL because `runtime.rs` still calls `on_mixer_level_adjusted`, `on_mixer_pan_adjusted`, and the one-argument assignment callback removed by Task 5.

- [ ] **Step 2: Wire direct level and pan callbacks.**

Replace the old adjusted mixer callbacks in `wire_callbacks`.

```rust
let command_worker = Rc::clone(&worker);
app.on_mixer_level_set(move |channel, level| {
    if let Some(command) = GuiCommand::set_mixer_level_by_channel(channel, level) {
        command_worker.send_command(command);
    }
});

let command_worker = Rc::clone(&worker);
app.on_mixer_pan_set(move |channel, raw| {
    if let Some(command) = GuiCommand::set_mixer_pan_by_channel(channel, raw) {
        command_worker.send_command(command);
    }
});
```

- [ ] **Step 3: Wire link and channel-aware assignment callbacks.**

Remove the selected-channel lookup from assignment wiring. Use the signed-input validation helper for link events.

```rust
let command_worker = Rc::clone(&worker);
app.on_mixer_link_requested(move |channel| {
    if let Some(command) = GuiCommand::toggle_mixer_link_by_channel(channel) {
        command_worker.send_command(command);
    }
});

let command_worker = Rc::clone(&worker);
app.on_assignment_requested(move |channel, choice_index| {
    if let Some(command) = GuiCommand::pick_assignment_from_indices(channel, choice_index) {
        command_worker.send_command(command);
    }
});
```

Keep all other callback wiring unchanged.

- [ ] **Step 4: Run runtime and package tests.**

Run: `cargo check -p zen-go-slint`

Expected: PASS with generated direct callback methods.

Run: `cargo test -p zen-go-slint runtime::tests`

Expected: PASS.

Run: `cargo test -p zen-go-slint`

Expected: PASS.

## Task 7: Remove obsolete UI paths and verify the redesign

**Files:**
- Modify: `zen-go-slint/ui/main.slint`
- Modify: any new Slint module that still contains replaced inline behavior
- Modify: `zen-go-slint/src/commands.rs`, `zen-go-slint/src/models.rs`, `zen-go-slint/src/mapper.rs`, `zen-go-slint/src/ui_bridge.rs`, or `zen-go-slint/src/runtime.rs` only when cleanup follows the tested interface

**Interfaces:**
- Consumes the complete page and callback surface from Tasks 1 through 6.
- Produces a single Warm Hardware Slint composition with no obsolete rail, strip button-only level controls, old adjusted mixer callbacks, or duplicate assignment path.

- [ ] **Step 1: Remove replaced inline components.**

Delete old `RailButton`, `StatusChip`, `OutputCard`, `StripCard`, `SmallPanel`, `ChoiceButton`, and `PreampMini` definitions from `main.slint` after each replacement module is imported and compiled.

- [ ] **Step 2: Search for obsolete callback and component names.**

Run:

```bash
git grep -nE 'mixer-level-adjusted|mixer-pan-adjusted|assignment-requested\(int\)|RailButton|StripCard|ChoiceButton|PreampMini|SmallPanel' -- zen-go-slint
```

Expected: no obsolete product definitions or callback declarations remain. Any remaining match must be a deliberate compatibility reference in a test or documentation comment and must be removed if it no longer serves the new interface.

- [ ] **Step 3: Run formatting and package verification.**

Run: `cargo fmt --check`

Expected: PASS.

Run: `cargo test -p zen-go-slint`

Expected: PASS.

Run: `cargo test --workspace`

Expected: PASS.

Run: `cargo check -p zen-go-slint`

Expected: PASS.

- [ ] **Step 4: Run edited-file diagnostics.**

Run `lens_diagnostics` with `mode=all` and these paths:

```text
zen-go-slint/src/commands.rs
zen-go-slint/src/models.rs
zen-go-slint/src/mapper.rs
zen-go-slint/src/ui_bridge.rs
zen-go-slint/src/runtime.rs
zen-go-slint/ui/main.slint
zen-go-slint/ui/view-models.slint
zen-go-slint/ui/theme.slint
zen-go-slint/ui/primitives/
zen-go-slint/ui/domain/
zen-go-slint/ui/pages/
```

Expected: no blocking errors for edited files.

- [ ] **Step 5: Run the GUI smoke test with mock transport.**

Run: `cargo run -p zen-go-slint -- --mock --no-bootstrap`

Verify manually:

1. Open each top tab and confirm only its page is visible.
2. Confirm connected, disconnected, and error status states remain visible.
3. Select multiple mixer strips and confirm selection changes highlight only.
4. Drag level to bottom and top and confirm values stay within `0..90`.
5. Drag pan to both ends and center and confirm values stay within `0x02..0x3e`.
6. Confirm meter height changes do not move level fader position.
7. Confirm link appears only on odd-numbered channels.
8. Exercise link, solo, mute, and inline assignment controls.
9. Scroll horizontally and reach all 16 mixer strips.
10. Exercise Profile, Raw, Route, and Settings actions.
11. Confirm a command error appears in the footer and a later snapshot restores authoritative values.

Success means the redesigned GUI compiles, preserves existing command behavior, exposes direct strip controls, keeps Rust authoritative, and uses shared Warm Hardware modules across all five pages.

## Plan self-review checklist

- Spec coverage: Tasks 1 and 2 cover command validation and linkable snapshot data. Tasks 3 and 4 cover view contracts, tokens, primitives, direct controls, meters, source picker, and domain modules. Task 5 covers shell, tabs, mixer layout, all other pages, and assignment relocation. Task 6 covers generated callback wiring and worker behavior. Task 7 covers cleanup and every required verification command.
- Type consistency: Slint callbacks use `int` values. Runtime receives `i32`. Direct command helpers validate signed values before conversion. `MixerStripView.linkable` matches `MixerStripSnapshot.linkable` and `strip_to_view`.
- Range consistency: direct level uses `0..0x5a`. Direct pan uses `PanState::MIN..=PanState::MAX`. The command layer rejects values outside these ranges.
- Ownership consistency: Slint owns presentation and gesture-to-value mapping. Rust owns channel validation, pan construction, linkability, commands, controller intents, and snapshots.
- Scope consistency: no protocol, transport, controller, TUI, Material dependency, or separate crate changes are planned.
- Failure coverage: disconnected and error states, failed command restoration, picker closure, event deduplication, independent meters, and horizontal strip access appear in Task 7 verification.
