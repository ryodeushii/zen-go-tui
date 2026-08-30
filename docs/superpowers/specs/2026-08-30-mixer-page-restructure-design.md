# Mixer page restructuring

**Date:** 2026-08-30  
**Status:** Approved  
**Related design:** `docs/superpowers/specs/2026-08-28-gui-redesign-design.md`

## Context

`zen-go-slint` currently separates mixer controls from preamps and device selectors. The Mixer page shows three output cards above the mixer strips. The Route page shows two preamps, sample-rate controls, clock-source controls, and a selected-mixer-channel label.

The Route page has no routing matrix or other route-specific behavior. Moving its useful controls to Mixer would leave an empty page. The TUI already presents preamps and outputs as part of its broader control flow.

The protocol models Monitor, HP1, and HP2 as stereo destinations. Each destination has one shared gain and one mode. The current state does not contain independent left and right output meter values. The GUI must not invent them.

The existing mixer strip is an approved boundary. This restructuring must not change its visual design, markup, bindings, callbacks, or interaction behavior.

## Goals

1. Make Mixer the complete audio-control page.
2. Put A1 and A2 preamps above the unchanged mixer strips.
3. Put stereo output cards below the mixer strips in a TUI-like order.
4. Move sample-rate and clock-source selection into header dropdowns.
5. Remove the empty Route tab and Route page from the GUI.
6. Show each output as a stereo L/R destination with one shared gain value.
7. Keep Rust as the authority for device state, validation, commands, and protocol behavior.
8. Keep the Warm Hardware visual direction and existing shared primitives.

## Non-goals

- Do not modify the appearance or behavior of existing mixer strips.
- Do not add independent left and right output gain or meter fields.
- Do not decode new protocol fields in this change.
- Do not build a routing matrix.
- Do not remove Rust routing state or TUI routing behavior.
- Do not add a second GUI component framework.
- Do not change transport, protocol encoding, or controller validation rules.

## Options considered

### TUI-order Mixer page

Move preamps into a two-column row above the mixer. Keep the existing strip bank unchanged. Move outputs below the mixer. Put Monitor across both output columns and put HP1 and HP2 in the next row. Move sample-rate and clock-source selectors into header dropdowns. Remove Route from the GUI.

This option matches the user-requested order and the TUI control flow. It uses existing state and callbacks. It is the selected option.

### Unified top control deck

Combine outputs and preamps in a compact grid above the mixer. This uses less vertical space, but it changes the visual grouping and does not match the TUI order. The user rejected this option.

### Keep a Route placeholder

Move preamps and device selectors to Mixer while retaining a Route tab with a future-routing message. This leaves dead navigation in the primary shell. The user rejected this option because Route has no current function.

## Approved page structure

Use this order inside the Mixer page:

```text
Application header
  connection status
  sample-rate dropdown
  clock-source dropdown
  profile status
  auto-save status

Tabs
  Mixer | Profile | Raw | Settings

Mixer page, vertically scrollable
  Preamps
    A1 | A2

  Existing mixer strip bank
    horizontal scrolling remains unchanged

  Stereo outputs
    Monitor, spanning two columns
    HP1 | HP2
```

The page owns one vertical scroll area. The mixer strip bank keeps its existing horizontal scroll area so all 16 strips remain reachable.

## Architecture and ownership

### Application shell

`zen-go-slint/ui/main.slint` owns the four GUI tabs, header layout, page selection, shared models, and callback forwarding.

Keep `GuiPage::Routing` in Rust for TUI and controller compatibility. Add a GUI-specific page-index mapping so GUI indexes are contiguous:

```text
GUI index 0 -> Mixer
GUI index 1 -> Profiles
GUI index 2 -> Raw
GUI index 3 -> Settings
```

The GUI command helper must use this mapping for tab clicks. Existing Rust page and controller behavior outside the GUI remains unchanged.

Remove the Slint `RoutePage` import, Route tab entry, Route page branch, and selected-mixer-channel display. Delete `zen-go-slint/ui/pages/route.slint` after no imports remain.

### Header selectors

Replace the noninteractive sample-rate and clock-source status chips with dropdown triggers in the header. Reuse `ZenSourcePicker` and its Slint 1.16 `PopupWindow` implementation. Do not duplicate popup positioning, scrolling, or outside-click behavior.

The header sends selected choice indexes through semantic callbacks. Runtime callback wiring converts indexes through existing validated `GuiCommand` constructors before sending controller intents.

Connection, profile, and auto-save remain status content. Their layout stays within the existing centered header row.

### Mixer page

`zen-go-slint/ui/pages/mixer.slint` owns the vertical page composition. It receives the existing output, preamp, and choice models and forwards existing callbacks.

Move the existing preamp composition from Route into a two-column row in Mixer. Reuse `PreampCard` and its mode-control composition. Arrange gain, phantom, phase, and mode controls in a compact card without changing their callback meanings.

Keep the existing mixer strip markup and bindings unchanged. The migration must move its containing layout only.

Place outputs after the strip bank in a two-column layout. Monitor spans both columns. HP1 and HP2 occupy the next row.

### Output cards

`zen-go-slint/ui/domain/output-card.slint` keeps the existing shared output callbacks and `OutputView` fields. Each card must show:

- destination name
- `STEREO L / R` identity
- one shared gain value
- one shared gain-position indicator
- MUTE
- DIM

The gain-position indicator is not a signal meter. Bind it to the existing shared gain ratio. Do not render separate L/R bars or claim independent signal levels.

Keep output level, mute, and dim command validation in the existing Rust command path.

### Rust data flow

Keep this flow:

```text
GuiSnapshot
    -> ui_bridge
    -> AppWindow properties and models
    -> header, MixerPage, and output/preamp controls

Slint semantic callback
    -> runtime callback wiring
    -> GuiCommand
    -> Controller intent
    -> next GuiSnapshot
```

No new protocol field is required. No new output meter field is allowed without protocol evidence.

## Failure handling

- Disable header dropdowns and write controls when snapshots report a disconnected state.
- Keep the existing mock mode connected state for safe local interaction testing.
- Keep command and polling errors in the existing footer notice.
- Close header dropdowns on page change, reset-token change, disable, selection, and outside click.
- Leave the selected choice visible when a dropdown is disabled.
- Keep all 16 mixer strips reachable through horizontal scrolling.
- Keep the page vertically scrollable when preamps, strips, and outputs exceed the window height.
- Treat shared output gain as one value. Do not display missing left/right values.
- Preserve the existing Rust `GuiPage::Routing` and controller routing behavior even though the GUI no longer exposes Route.

## Migration sequence

1. Add RED contract tests for four GUI tabs, GUI page-index mapping, header dropdown callbacks, section order, two-column preamps, stereo output identity, and unchanged mixer-strip source.
2. Add the GUI-specific page-index mapping and update tab command tests.
3. Move preamp composition into Mixer without changing preamp callbacks.
4. Move sample-rate and clock-source pickers into the header and wire their callbacks.
5. Remove the Route tab, Route page branch, selected-channel display, and unused Route page file.
6. Move output composition below the mixer and implement the two-column stereo output presentation.
7. Run Slint compilation and focused tests after each migration stage.
8. Run full workspace verification and inspect the final diff.
9. Capture the GUI at compact and tall sizes. Verify page order, header dropdown placement, strip preservation, and access to all 16 strips.

## Verification

Run these checks from `/home/ryodeushii/repos/zen-go-tui/.worktrees/gui-redesign`:

1. Run focused RED/GREEN tests for GUI tab mapping and new layout contracts.
2. Run `cargo fmt --package zen-go-slint -- --check`.
3. Run `cargo test -p zen-go-slint`.
4. Run `cargo test --workspace`.
5. Run `cargo build -p zen-go-slint`.
6. Run `git diff --check`.
7. Run `lens_diagnostics` with `mode=all` for edited Rust files and Slint files when available.
8. Run a docs-drift scan for tab and GUI usage documentation.
9. Build one fresh mock GUI binary and launch exactly one instance.
10. Capture compact and tall windows with Spectacle when the target window is visible.
11. Manually verify header dropdowns, preamp controls, output controls, vertical page scrolling, horizontal strip scrolling, and unchanged strip appearance.

Success means the GUI has four useful tabs, Mixer follows the approved TUI-like order, preamps and outputs are directly available on Mixer, header selectors are interactive, output cards communicate shared stereo identity without fake meter data, and existing mixer strips remain unchanged.
