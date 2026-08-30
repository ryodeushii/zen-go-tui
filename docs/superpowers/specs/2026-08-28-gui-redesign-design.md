# Zen-Go Slint GUI redesign

**Date:** 2026-08-28  
**Status:** Approved design for review before implementation

## Context

`zen-go-slint` provides a native Slint front end for the existing Zen-Go controller. Its current UI uses one 602-line `main.slint` file with custom controls and a compact dark layout. Its Rust worker owns transport access, controller state, command execution, polling, and snapshot mapping.

The redesign keeps that ownership model. It changes the visual language and Slint module structure. It does not change the device protocol, transport, controller, or TUI behavior.

The approved product direction is:

- Warm Hardware visual language
- horizontal top tabs
- mixer-first Top Deck layout
- direct controls on every mixer strip
- inline source and assignment picker
- no context drawer or inspector
- all existing pages remain available

Material Slint is not part of this design. The application will use an internal Slint design system.

## Goals

1. Give the GUI a coherent warm hardware-console identity.
2. Keep mixer controls visible and directly manipulable.
3. Preserve functional parity with the TUI for mixer interaction.
4. Make shared visual behavior local to reusable Slint modules.
5. Keep Rust as the authoritative source for device state and command validation.
6. Make later visual changes local to theme, primitive, or domain modules.

## Non-goals

- Do not add Material Slint or another component dependency.
- Do not create a separately packaged UI crate.
- Do not redesign the Rust controller or transport.
- Do not move device rules into Slint.
- Do not replace the TUI.
- Do not add a generic component framework beyond current application needs.

## Options considered

### Monolithic redesign

Keep all redesigned controls in `main.slint`. This has the smallest initial file change, but it keeps visual behavior shallow and spreads future fixes across one large module.

### Internal layered Slint design system

Separate theme tokens, primitives, domain controls, pages, and shell composition. This gives the application a small internal interface and keeps visual behavior local. It is the selected option.

### Separate reusable crate

Package the design system for reuse outside this application. One current consumer does not justify package boundaries, versioning, or a second public interface.

## Architecture

Use this module layout:

```text
zen-go-slint/ui/
  theme.slint
  primitives/
    zen-button.slint
    zen-tab-bar.slint
    zen-panel.slint
    zen-status-chip.slint
    zen-meter.slint
    zen-level-fader.slint
    zen-pan-slider.slint
    zen-toggle-chip.slint
    zen-source-picker.slint
  domain/
    output-card.slint
    mixer-strip.slint
    preamp-card.slint
  pages/
    mixer.slint
    route.slint
    profile.slint
    raw.slint
    settings.slint
  main.slint
```

`theme.slint` owns colors, typography, spacing, radii, borders, control sizes, and state colors. All visual modules use these tokens instead of local color literals for shared behavior.

Primitive modules provide reusable interaction and visual behavior. Domain modules compose primitives around audio concepts. Page modules compose domain modules and expose page-specific properties and callbacks. `main.slint` owns only the application shell, top tabs, shared status areas, page selection, and page composition.

The external seam remains the generated `AppWindow` interface. Slint properties and models receive `GuiSnapshot` data through `ui_bridge`. Slint callbacks send semantic events to `runtime`. Rust converts those events into `GuiCommand` values and controller intents.

## Visual system

Use these token groups:

- deep, panel, raised, and inset surfaces in warm charcoal and brown tones
- terracotta for primary selection and active controls
- amber for level values and attention states
- cream primary text and muted warm-gray secondary text
- green for connected or healthy state
- red for mute, error, and peak state
- consistent panel borders, shallow hardware-like radii, and restrained shadows

The top tab bar replaces the left rail. Tabs are `Mixer`, `Route`, `Profile`, `Raw`, and `Settings`. The header keeps connection, sample-rate, sync, and autosave status visible. The footer keeps the current notice and error surface.

The theme owns control metrics. The mixer must remain usable at the existing 1280x820 window size. The strip bank must keep all 16 channels accessible through horizontal scrolling. Exact token values can be tuned during visual verification without changing this structure or visual direction.

## Mixer design

The Mixer page contains an output-card row followed by a horizontally scrollable strip bank.

Each `MixerStrip` shows, in top-to-bottom order:

1. channel label and source label
2. inline source or assignment picker
3. pan value
4. horizontal pan slider and scale
5. independent meter with meter value
6. vertical level fader
7. level value
8. inline link, solo, and mute controls

Selecting a strip changes its highlight only. It does not open a drawer, inspector, or modal. The strip preserves its position and controls after selection.

The link control appears only for link-capable channels. The current TUI rule exposes link for odd-numbered channels. The mapper must expose a `linkable` field in `MixerStripSnapshot` and `MixerStripView` so Slint does not own this device rule. The Rust command path must validate the same constraint.

Meter and level remain independent. Meter rendering can show a peak state without changing the level fader position.

The inline source picker uses the existing assignment choices from `GuiSnapshot`. Opening and closing the picker is local UI state. Selecting an item sends the strip channel and choice index. The picker closes after selection, page change, or strip selection change.

## Direct control behavior

Use Slint `TouchArea` handling inside `ZenLevelFader` and `ZenPanSlider`. Do not add pointer-coordinate conversion to Rust.

`ZenLevelFader` maps its vertical position to an integer level from `0` through `90`. It emits a level event only when the rounded integer changes. It emits the final value on release when that value differs from the last emitted value.

`ZenPanSlider` maps its horizontal position to a normalized ratio and then to the position range `PanState::MIN` through `PanState::MAX`. It emits a position-only raw pan value. It must never include the upper mute or solo flag bits in the pan value.

Rust remains responsible for validating channel, level, assignment, and pan ranges. External snapshots remain authoritative. A rejected command or device update can restore the displayed value on the next snapshot.

## Rust and Slint interface

Keep this data flow:

```text
GuiSnapshot
    -> ui_bridge
    -> AppWindow properties and models
    -> Slint controls

Slint semantic event
    -> runtime callback wiring
    -> GuiCommand
    -> Controller intent
    -> next GuiSnapshot
```

Use one-based device channels at the Slint callback interface. Convert to existing zero-based indexes in Rust command helpers. Keep conversion in one place.

Add or update these mixer callbacks:

- `mixer-level-set(channel, level)` with an integer level from `0` through `90`
- `mixer-pan-set(channel, raw)` with a position-only pan raw value
- `mixer-link-requested(channel)`
- `assignment-requested(channel, choice-index)`

Keep existing mixer mute, solo, selection, output, preamp, profile, raw, refresh, and autosave behavior. Retain delta callbacks for controls that still use step buttons, such as output and preamp nudges.

`src/commands.rs` remains the command validation seam. Existing `SetMixerLevel`, `SetMixerPan`, `ToggleMixerLink`, and `PickAssignment` commands must receive the new direct-control events without protocol changes.

## Other pages

Apply the same theme and primitives to all existing pages.

- **Route:** use shared panels, tabs, buttons, and source selectors for routing choices.
- **Profile:** use shared panels and actions for profile selection, load, save, rename, and delete.
- **Raw:** use shared panels and table-like rows for raw values, baseline capture, baseline clearing, and refresh.
- **Settings:** use shared selectors, toggle chips, panels, and action buttons for sample rate, clock source, and autosave.

These pages keep their existing snapshot fields, callbacks, and command behavior. The redesign changes presentation and module composition only.

## Failure handling

- Disable or visibly mark controls when snapshots report a disconnected or unavailable state.
- Show command and polling errors through the existing footer notice.
- Let the next authoritative snapshot restore values after a failed direct-control command.
- Emit at most one event per distinct integer level or pan raw value during a gesture.
- Keep all channels reachable when the strip bank overflows the viewport.
- Close transient source-picker state when the page or selected strip changes.

## Migration constraints

Implement the redesign in these stages:

1. Add theme tokens and primitive modules.
2. Replace the left rail with the horizontal top-tab shell.
3. Implement the redesigned output cards and direct-control mixer strip.
4. Add link and channel-aware assignment callback wiring.
5. Migrate Route, Profile, Raw, and Settings to shared modules.
6. Remove obsolete UI paths after replacement behavior passes verification.

Keep the Rust worker, mapper, bridge, controller, transport, and protocol interfaces stable except for the explicitly listed GUI callback and snapshot additions.

## Verification

Run these checks before completion:

1. Compile the Slint UI and generated Rust bindings.
2. Run existing `zen-go-slint` tests.
3. Add command tests for direct level, direct pan, link, and channel-aware assignment events.
4. Add or update mapper and bridge tests for `linkable`, level ratio, pan ratio, selection, meter independence, and assignments.
5. Run `cargo fmt --check`.
6. Run workspace and package tests.
7. Run `lens_diagnostics` with `mode=all` for edited source files.
8. Run a manual GUI smoke test with the mock transport.

The manual test must cover connected, disconnected, and error states. It must also cover tab navigation, strip selection, level drag endpoints, pan drag endpoints, independent meter updates, link/solo/mute actions, inline assignment, profile actions, Raw actions, Settings actions, and horizontal access to all 16 strips.

Success means the redesigned GUI compiles, preserves existing command behavior, exposes direct strip controls, keeps Rust authoritative, and presents one consistent Warm Hardware design system across all pages.
