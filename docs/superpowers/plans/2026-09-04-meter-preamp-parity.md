# Meter and Physical Preamp Parity Implementation Plan

> **Goal:** Match parent `master` meter and physical-preamp behavior while keeping all supported devices profile-driven.
>
> **Architecture:** Preserve raw meter bytes in protocol adapters. Render semantic meter and gain state through one rich physical-preamp card. Use profile address-space kind to select rich physical cards. Keep ADAT and S/PDIF rows compact.
>
> **Tech stack:** Rust, Ratatui, Python profile generator, JSON Antelope-Ctl profiles, tmux TUI verification.
>
> **Spec:** `docs/superpowers/specs/2026-09-03-zen-go-profile-runtime-repair-design.md`

## Global constraints

- Keep Antelope-Ctl Python and runtime code unchanged.
- Keep `modules/Antelope-Ctl` uncommitted.
- Preserve the capture-backed 47 startup queries and q04 layouts.
- Keep raw meter values inside protocol and state adapters.
- Render physical analog preamps as rich cards on every supported device.
- Keep ADAT and S/PDIF inputs compact.
- Render capture-backed no-signal meter values as `-∞ dB`.
- Render absent or unsupported strip meters as `?`.
- Use mode-specific profile gain ranges for sliders and writes.
- Run PATH and worktree TUIs one at a time to prevent HID contention.

## Task 1: Correct Zen Go physical-preamp names

**Files:**

- Modify: `modules/Antelope-Ctl/profiles/zen_go_sc.json`
- Modify: `tools/test_generate_device_catalog.py`
- Regenerate: `src/device/generated.rs`
- Regenerate: `src/device/generated_profiles.json`

1. Add a generator test that expects `Preamp 1` and `Preamp 2`.
2. Run the focused test and observe the name mismatch.
3. Change only the two local profile input names.
4. Regenerate both parent artifacts.
5. Run the focused generator test and artifact drift check.

## Task 2: Preserve address-space kind and build an adaptive physical-preamp grid

**Files:**

- Modify: `src/app/state.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/ui/layouts.rs`
- Modify: `src/ui/dynamic_tests.rs`

1. Add `kind: String` to `InputSpaceState`.
2. Populate it from `RuntimeAddressSpace.kind` in `AppState::from_profile`.
3. Add a Zen Go render test for two half-width rich card areas.
4. Add an Orion render/layout test that covers all 12 physical preamps.
5. Run both tests and observe compact-row or missing-card failures.
6. Add shared physical-preamp grid helpers with five-row cards and adaptive columns.
7. Derive input-panel height from rich-card rows and compact nonphysical rows.
8. Keep mixer layout height nonzero at supported terminal dimensions.
9. Run the focused layout tests.

## Task 3: Render one rich card for every physical preamp

**Files:**

- Modify: `src/ui/widgets/mixer.rs`
- Modify: `src/ui/render/mod.rs`
- Modify: `src/ui/layouts.rs`
- Modify: `src/ui/dynamic_tests.rs`
- Modify: `src/ui/tests.rs`

1. Add a production-render test for `OBS -∞ dB`, `GAIN 43 dB`, gain slider, `Mic`, `48V`, and phase controls.
2. Add a test that verifies rich cards appear only in `physical_inputs` spaces.
3. Run both tests and observe shallow-row output.
4. Add `render_dynamic_preamp_card_widget` using `DynamicInputState` and profile control semantics.
5. Reuse `preamp_card_inner_layout` and `render_stacked_signal_rows`.
6. Render meter and gain sliders from current dynamic values.
7. Render only declared mode, phantom, phase, and link controls.
8. Keep dynamic gain mouse rectangles aligned with rendered slider tracks.
9. Run focused render and mouse tests.

## Task 4: Render strip meter values without the redundant prefix

**Files:**

- Modify: `src/ui/widgets/mixer.rs`
- Modify: `src/ui/dynamic_tests.rs`

1. Add a dynamic strip test with active, no-signal, and absent meter states.
2. Require `-N dB`, `-∞ dB`, or `?` without the `MTR` prefix.
3. Require different rendered output for active and absent meter states.
4. Run the test and observe the redundant `MTR` prefix failure.
5. Remove only the `MTR` prefix from the dynamic strip's value paragraph.
6. Keep the existing meter threshold, ratio, and vertical meter rendering.
7. Run focused meter tests.

## Task 5: Verify gain controls and live parity

**Files:**

- Modify as needed: `src/ui/mouse.rs`
- Modify: `src/ui/tests.rs`

1. Add a geometry-derived mouse test for a physical-preamp gain slider.
2. Verify left and right slider endpoints use the current mode-specific profile range.
3. Run the test and fix only confirmed geometry or intent defects.
4. Run `python -m unittest discover -s tools -p 'test_generate_device_catalog.py'`.
5. Run the generated-artifact drift check.
6. Run `cargo fmt --all -- --check`.
7. Run `cargo test --workspace`.
8. Run `git diff --check`.
9. Run primary LSP and active lens diagnostics on edited source files.
10. Launch PATH and worktree TUIs sequentially in dedicated 200x55 tmux panes.
11. Compare preamp names, OBS meters, gain labels, gain sliders, declared controls, strip no-signal labels, and active numeric strip meters.
12. Stop both verification processes.

## Completion criteria

- Zen Go preamp cards match parent `master` at 200x55.
- Orion renders a rich card for each of its 12 physical preamps.
- ADAT and S/PDIF inputs remain compact.
- Strip no-signal lanes show `-∞ dB`.
- Active strip lanes show numeric dB values.
- Physical-preamp gain sliders use profile mode ranges.
- Full automated checks pass.
- Live PATH and worktree captures match for requested behavior.
