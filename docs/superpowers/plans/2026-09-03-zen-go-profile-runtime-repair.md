# Zen Go Profile Runtime Repair: Remaining Dynamic-State Defects

## Overview

Repair profile-driven Zen Go runtime behavior without replacing the current custom driver or weakening profile safety. Current submodule work already supplies profile-backed startup queries, sparse codec operations, Zen Go `0x73` snapshots, `0x74` meters, `0x75` query replies, `0x83` auxiliary handling, and candidate `0x73` preamp meter offsets. Do not repeat that work.

The remaining defects cross four boundaries:

1. Dynamic mixer pan uses raw wire positions in some paths and semantic positions in others.
2. Dynamic output levels use raw attenuation values but render and step them as direct values.
3. Profile-derived input spaces include output topology, which forces a clipped compact layout.
4. Existing live-state tests do not prove q04 fader readbacks survive snapshot merges or appear in dynamic widgets.

The implementation must use profile metadata at each boundary. Raw wire values may exist inside adapters and compatibility conversion only. UI and dynamic actions must use semantic values.

## Current baseline

- Parent HEAD: `dced48f`.
- Antelope-Ctl submodule HEAD: `ede7bfd2bb1dfa9fb2252749be6742148a259096`.
- The nested profile repository already contains the Zen Go runtime query and readback improvements described above.
- The old checked-in plan repeated work that is present at current HEAD. This file replaces it.
- No source files change until an implementation task starts.
- Current workspace baseline: 345 tests pass, 1 fails, and 2 are ignored.
- Current failing Rust test: `device::profile::tests::builtin_entries_match_checked_in_normalized_pack_profiles` for `orion_studio_3`.
- Current generator baseline: 1 failure and 23 errors caused by Orion `frame.routing_command.source_banks.0x02` non-contiguous range handling.
- The Orion source and packed profile share SHA-256 `de969294cf7d261536469127b72c4fb6b9e29597e55058e8cabb083a2a83954f`, generator version `1.4.0`, readiness `supported`, and driver kind `profile`. The failure is normalized-content drift, not stale provenance.

## File map and responsibilities

- `modules/Antelope-Ctl/profiles/zen_go_sc.json`: canonical Zen Go profile metadata and evidence.
- `tools/generate_device_catalog.py`: validates profile metadata and emits Rust definitions and the runtime pack.
- `tools/test_generate_device_catalog.py`: generator and profile-validation tests.
- `src/device/definition.rs`: compile-time generated-definition types.
- `src/device/generated.rs`: checked-in compile-time catalog output.
- `src/device/generated_profiles.json`: checked-in normalized runtime pack.
- `src/device/profile.rs`: generated-definition to runtime-profile conversion and pack consistency tests.
- `antelope-protocol/src/profile.rs`: owned runtime profile types and profile-aware semantic conversion helpers.
- `antelope-protocol/src/zen_go.rs`: Zen Go wire decoding, q04/q18 patch construction, and raw command encoding.
- `antelope-protocol/src/profile_driver.rs`: generic profile-driver decoding and encoding.
- `src/app/types.rs`: dynamic intents and pending mutation state.
- `src/app/mod.rs`: profile-shaped app state, dynamic-state merging, compatibility projection, and range lookup.
- `src/app/controller.rs`: keyboard and action handling.
- `src/ui/layouts.rs`: dynamic input, output, and mixer geometry plus ratio conversion.
- `src/ui/widgets.rs`: dynamic labels, fader readouts, pan labels, and meter rendering.
- `src/ui/mouse.rs`: pointer-to-semantic control conversion.
- `src/runtime.rs`: dynamic keyboard dispatch and repaint flow.

## Global constraints

- Use the Antelope-Ctl profile as the source of truth for device geometry, ranges, semantics, startup queries, and readback layouts.
- Keep Zen Go selected through `RuntimeDriverKind::ZenGo`. Do not route Zen Go through `ProfileDriver`.
- Preserve the profile's 47 safe-query entries, including order and duplicates.
- Preserve raw fader attenuation. Raw `0` means unity and raw `90` means `-90 dB`.
- Convert dynamic mixer pan to profile semantic degrees. Convert to `PanState` raw positions only in adapters and compatibility projection.
- Keep fader conversion in one profile-aware helper. Do not invert values in both UI and driver layers.
- Do not add a typed `0x83` channel map. Keep `0x83` as auxiliary state until capture evidence proves its channel layout.
- Do not add Orion category counts, Zen Go `0x83` channel offsets, unverified physical output names, or untested Zen Go routing readback semantics.
- Do not change saved-state TOML schema.
- Do not reset, detach, or rewrite the Antelope-Ctl submodule history.
- Hardware validation must use read-only launch, resize, navigation, and quit flows. Do not send adjustment, mute, routing, or save actions during visual checks.
- Do not claim all tests pass while the Orion normalized-content failure or generator blocker remains unresolved.

## Implementation tasks

### Task 1: Add profile-backed scalar semantics for output levels

**Files:**

- `modules/Antelope-Ctl/profiles/zen_go_sc.json`
- `tools/generate_device_catalog.py`
- `tools/test_generate_device_catalog.py`
- `src/device/definition.rs`
- `src/device/generated.rs`
- `src/device/generated_profiles.json`
- `src/device/profile.rs`
- `antelope-protocol/src/profile.rs`

**Tests first:**

1. Add generator assertions that Zen Go `params.bus_level` emits `direction: "attenuation"` and `unity: 0`.
2. Add generator rejection tests for a parameter with only one domain field and for a unity value outside its range.
3. Add a Rust profile test that a runtime parameter with range `(0, 96)`, attenuation direction, and unity `0` returns `FaderSemantics { min: 0, max: 96, direction: Attenuation, unity: 0 }`.
4. Run `python -m unittest tools.test_generate_device_catalog` and the focused Rust profile test. Confirm new assertions fail before implementation.

**Implementation:**

1. Add `direction: Option<FaderDirectionDefinition>` and `unity: Option<i32>` to generated `ParamDefinition`.
2. Parse optional parameter scalar-domain fields in `_build_params`. Require both fields when either is present. Accept only `direct` and `attenuation`. Require unity to fit the normalized parameter range.
3. Render the new fields in each generated `ParamDefinition` initializer. Render absent fields as `None`.
4. Add `direction: Option<FaderDirection>` and `unity: Option<i32>` to `RuntimeParam` with serde defaults for older packs.
5. Map generated direction values in `src/device/profile.rs::convert_entry`.
6. Add `RuntimeParam::scalar_semantics() -> Option<FaderSemantics>` in `antelope-protocol/src/profile.rs`. Return `None` when range, direction, or unity is absent.
7. Update `zen_go_sc.json` `params.bus_level` with the confirmed raw attenuation domain: range `[0, 96]`, direction `attenuation`, and unity `0`. Keep output names generic because physical bus names remain unverified.
8. Regenerate both checked-in artifacts with:

   ```bash
   python tools/generate_device_catalog.py \
     --profiles-dir modules/Antelope-Ctl/profiles \
     --output src/device/generated.rs \
     --pack-output src/device/generated_profiles.json
   ```

9. Preserve the existing Orion parser guard when regeneration reaches `frame.routing_command.source_banks.0x02`. Do not broaden source-bank parsing to accept non-contiguous ranges.

**Verify:**

```bash
python -m unittest tools.test_generate_device_catalog
cargo test -p zen-go-tui device::profile::tests::builtin_entries_match_checked_in_normalized_pack_profiles
cargo test -p antelope-protocol runtime_param_scalar_semantics
```

**Commit:**

```bash
git -C modules/Antelope-Ctl add profiles/zen_go_sc.json
git -C modules/Antelope-Ctl commit -m "feat: declare Zen Go output attenuation"
git add tools/generate_device_catalog.py tools/test_generate_device_catalog.py \
  src/device/definition.rs src/device/generated.rs src/device/generated_profiles.json \
  src/device/profile.rs antelope-protocol/src/profile.rs modules/Antelope-Ctl
git commit -m "feat: carry profile scalar semantics"
```

### Task 2: Normalize dynamic mixer pan at the adapter boundary

**Files:**

- `antelope-protocol/src/profile.rs`
- `antelope-protocol/src/zen_go.rs`
- `antelope-protocol/src/profile_driver.rs`
- `antelope-protocol/src/driver.rs`
- `src/app/types.rs`
- `src/app/mod.rs`
- `src/app/controller.rs`
- `src/ui/layouts.rs`
- `src/ui/mouse.rs`
- `src/runtime.rs`

**Tests first:**

1. Add `RuntimeMixer` tests for raw `PanState::left()`, `center()`, and `right()` mapping to profile values `-30`, `0`, and `30`.
2. Add inverse tests for semantic values `-30`, `0`, and `30`, including rejection of values outside the profile range.
3. Add a Zen Go fixture test that `0x73` and q04 pan fields enter `DynamicMixerStrip.pan` as `-30..30`, not `2..62`.
4. Add an app test that `mixer_range(MixerControl::Pan)` returns the profile semantic range.
5. Add mouse and controller tests for left, center, and right pan actions. Run them before implementation and confirm failure.

**Implementation:**

1. Add `RuntimeMixer::pan_value_from_raw(PanState) -> Option<i32>` and `RuntimeMixer::pan_raw_from_value(i32) -> Option<PanState>`. Use `pan_range` and `pan_center`. Reject values outside either semantic or valid raw bounds.
2. Document `DynamicMixerStrip.pan` and `Action::SetMixerStripState.pan` as profile semantic pan values.
3. Convert Zen Go snapshot and q04 pan fields with `pan_value_from_raw`. Keep raw `PanState` inside wire decoding and command assembly.
4. Convert Zen Go complete-strip actions with `pan_raw_from_value` before building state-code bytes.
5. Replace hard-coded raw-center arithmetic in `ProfileDriver` decode and encode paths with the same `RuntimeMixer` helpers.
6. Change `src/app/mod.rs::mixer_range` to return semantic `pan_range` directly. Remove center addition from UI-facing range lookup.
7. Change dynamic `SetMixerPanAt` and pending mutations to carry semantic `i32` values. Keep legacy `Action::SetMixerPan` compatibility paths raw until their adapter boundary.
8. Rename `pan_from_ratio` to `pan_value_from_ratio` and make it return a semantic `i32`.
9. Make `compatibility_channel` convert semantic dynamic pan back through the selected runtime mixer's `pan_raw_from_value` before creating legacy `MixerChannelState`.
10. Keep fader values raw attenuation values throughout this task.

**Verify:**

```bash
cargo test -p antelope-protocol runtime_mixer_pan_conversion
cargo test -p antelope-protocol zen_go_dynamic_pan_is_semantic
cargo test -p zen-go-tui dynamic_pan_uses_semantic_profile_range
cargo test -p zen-go-tui compatibility_projection_converts_pan
```

**Commit:**

```bash
git add antelope-protocol/src/profile.rs antelope-protocol/src/zen_go.rs \
  antelope-protocol/src/profile_driver.rs antelope-protocol/src/driver.rs \
  src/app/types.rs src/app/mod.rs src/app/controller.rs src/ui/layouts.rs \
  src/ui/mouse.rs src/runtime.rs
git commit -m "fix: normalize profile mixer pan values"
```

### Task 3: Shape input topology and dynamic control geometry from the profile

**Files:**

- `src/app/mod.rs`
- `src/ui/layouts.rs`
- `src/ui/widgets.rs`
- `src/ui/mouse.rs`
- `src/app/mod.rs` tests
- `src/ui/layouts.rs` tests
- `src/ui/widgets.rs` tests

**Tests first:**

1. Add a profile fixture with physical inputs and an output address space. Assert `AppState::from_profile` places only address spaces referenced by `profile.inputs` in `input_spaces`.
2. Add a layout test at `140x40` that checks complete input chips for `GAIN 43`, `48V`, and `PH`.
3. Add a layout test at `80x24` that allows controls to be hidden but rejects partial chip rectangles and clipped control tokens.
4. Add an output render test that feeds raw level `10` and expects `-10 dB`, not `-86 dB`.
5. Run the new tests before implementation and confirm failure.

**Implementation:**

1. Filter `RuntimeProfile.address_spaces` in `AppState::from_profile` to spaces referenced by at least one `RuntimeInput`. Do not infer input topology from every address-space entry.
2. Keep output address spaces available to output lookup and dynamic output rendering. Do not rename generic output buses.
3. Add one chip-width helper in `src/ui/layouts.rs` that counts the rendered label plus both chip padding spaces.
4. Make `dynamic_input_control_rects` reserve complete chip widths for the current labels. Return no rectangle when the remaining row width cannot fit a complete chip. Never allocate a partial chip rectangle.
5. Use the same labels and widths in `render_dynamic_input_row` so layout and rendering cannot disagree.
6. Add `AppState::output_level_semantics()` using the profile parameter selected for `OutputControl::Level`. Do not hard-code Zen Go output inversion in widgets.
7. Render output level with `fader_display_db` and `fader_ratio` from `FaderSemantics`. Raw `10` must render as `-10 dB`. Raw `0` must render as `0 dB` and the top of the slider. Raw `96` must render as `-96 dB` and the bottom.
8. Make output mouse conversion and keyboard stepping use the same profile semantics helper. Remove direct `value - 96` and direct-range slider inversion.
9. Keep the compact layout safe at narrow widths by omitting controls that do not fit, rather than clipping their labels.

**Verify:**

```bash
cargo test -p zen-go-tui input_spaces_exclude_output_topology
cargo test -p zen-go-tui dynamic_input_chip_rects_are_complete
cargo test -p zen-go-tui dynamic_output_uses_attenuation_semantics
cargo test -p zen-go-tui output_ratio_maps_unity_to_top
```

**Commit:**

```bash
git add src/app/mod.rs src/ui/layouts.rs src/ui/widgets.rs src/ui/mouse.rs
git commit -m "fix: derive dynamic UI geometry from profile topology"
```

### Task 4: Prove q04 fader and candidate-meter state flow

**Files:**

- `antelope-protocol/src/zen_go.rs`
- `antelope-protocol/src/driver.rs`
- `src/app/mod.rs`
- `src/ui/widgets.rs`
- `src/ui/layouts.rs`
- existing Zen Go and app test modules

**Tests first:**

1. Add a q04 integration test using existing readback fixtures. Assert decoded faders preserve distinct raw values such as `0`, `18`, `30`, and `90`.
2. Add an app merge test that applies a snapshot with `fader: None`, applies the q04 mixer patch, then applies another snapshot with `fader: None`. Assert the q04 fader values remain present.
3. Add a widget test that renders an unknown fader as `LVL ?` and raw `18` as `LVL -18 dB`.
4. Add a candidate preamp meter test that uses the profile's candidate offset and confirms the decoded value reaches the matching dynamic input.
5. Keep the existing `0x83` auxiliary test and assert it does not populate dynamic mixer meters.
6. Run the focused tests before implementation and confirm failure where current coverage lacks the contract.

**Implementation:**

1. Preserve optional fields during dynamic snapshot merges. An incoming `None` must not replace a known q04 fader, pan, mute, solo, or meter value.
2. Keep q04 fader values as raw profile-domain values. Do not default absent q04 values to zero.
3. Render fader readouts through the shared profile-aware fader helper. Render `?` until a value exists.
4. Keep candidate preamp meter decoding tied to `RuntimeStateReport.candidate_preamp_meters`.
5. Keep `0x83` storage as auxiliary bytes without typed mixer-channel merging.
6. Update only the smallest adapter or merge seam required by failing tests. Do not repeat already-working startup-query or q18 work.

**Verify:**

```bash
cargo test -p antelope-protocol zen_go_q04_faders_preserve_raw_values
cargo test -p zen-go-tui q04_faders_survive_snapshot_merge
cargo test -p zen-go-tui dynamic_mixer_fader_readout
cargo test -p antelope-protocol candidate_preamp_meter_uses_profile_offset
cargo test -p antelope-protocol auxiliary_0x83_does_not_merge_as_mixer_meter
```

**Commit:**

```bash
git add antelope-protocol/src/zen_go.rs antelope-protocol/src/driver.rs \
  src/app/mod.rs src/ui/widgets.rs src/ui/layouts.rs
git commit -m "test: cover Zen Go dynamic state preservation"
```

### Task 5: Reconcile generated artifacts and close Orion drift

**Files:**

- `tools/generate_device_catalog.py`
- `tools/test_generate_device_catalog.py`
- `src/device/profile.rs`
- `src/device/generated.rs`
- `src/device/generated_profiles.json`

**Tests first:**

1. Run the exact existing assertion:

   ```bash
   cargo test -p zen-go-tui device::profile::tests::builtin_entries_match_checked_in_normalized_pack_profiles -- --nocapture
   ```

2. Add field-by-field mismatch reporting to the test helper before changing expected data. Report the first differing Orion field by name.
3. Add a focused Orion regression assertion for the differing field. Keep source SHA, generator version, readiness, driver kind, source-bank data, and link-domain data unchanged.
4. Run `python -m unittest tools.test_generate_device_catalog` and record the existing `source_banks.0x02` parser diagnostics separately from scalar metadata tests.

**Implementation:**

1. Compare the built-in Orion `RuntimeEntry` and packed Orion `RuntimeEntry` field by field.
2. Align only the normalized representation responsible for the reported content drift. Do not update Orion source-bank parsing, category counts, readiness, or routing semantics.
3. Regenerate checked-in artifacts after scalar metadata changes when the generator can process the profile set.
4. If full generation stops at the known Orion non-contiguous source-bank guard, preserve that guard and apply the smallest equivalent generated-artifact update needed for new optional scalar fields. Keep generated output deterministic and add a test that prevents silent Orion field loss.
5. Remove temporary field-diff diagnostics after the regression test identifies the stable mismatch, unless the helper improves future failure messages without changing behavior.

**Verify:**

```bash
python tools/generate_device_catalog.py \
  --check modules/Antelope-Ctl/profiles \
  --generated src/device/generated.rs \
  --pack-generated src/device/generated_profiles.json
python -m unittest tools.test_generate_device_catalog
cargo test -p zen-go-tui device::profile::tests::builtin_entries_match_checked_in_normalized_pack_profiles
```

**Commit:**

```bash
git add tools/generate_device_catalog.py tools/test_generate_device_catalog.py \
  src/device/profile.rs src/device/generated.rs src/device/generated_profiles.json
git commit -m "test: keep generated profiles synchronized"
```

### Task 6: Run repository and read-only TUI verification

**Tests and checks:**

1. Run `cargo fmt --all -- --check`.
2. Run `python -m py_compile tools/generate_device_catalog.py tools/test_generate_device_catalog.py`.
3. Run `cargo check --workspace`.
4. Run `cargo test --workspace`.
5. Run the generator drift check.
6. Run `lens_diagnostics` with `mode=all` for every edited source file.
7. Review `git diff --check` and `git status --short`.
8. Require zero unrelated source changes and zero unresolved blocking diagnostics.
9. Treat any remaining Orion failure as an open blocker. Do not mark this task complete until its narrow remediation is verified.

**TUI procedure:**

1. Start the current branch from the worktree in a tmux session at `140x40`.
2. Start the installed legacy binary in a separate tmux session at `140x40`.
3. Capture text panes without pressing adjustment, mute, routing, save, or quit actions until capture is complete.
4. Confirm the branch shows input controls without an `Outputs` header inside the preamp pane.
5. Confirm branch input chips show complete `GAIN`, `48V`, and `PH` tokens at `140x40`.
6. Confirm dynamic mixer labels show semantic pan values such as `PAN 0` and profile-aware fader values such as `LVL -18 dB` when q04 supplies raw `18`.
7. Confirm output raw `10` displays `-10 dB` and the level slider direction matches the legacy path.
8. Repeat at `80x24`. Confirm narrow layout hides whole controls instead of clipping labels or pan values.
9. Quit both sessions and record the captured output in the implementation handoff.
10. Use a targeted Kitty screenshot only if text capture cannot establish clipping or border geometry. Capture the app window, not the desktop.

**Final review:**

- Review the final diff against this plan.
- Confirm no saved-state TOML changes.
- Confirm no unverified physical output names were introduced.
- Confirm no typed `0x83` channel map was introduced.
- Confirm the Antelope-Ctl submodule pointer records its committed profile change.
- Report exact test counts, exact remaining Orion status, changed files, and skipped hardware checks.
