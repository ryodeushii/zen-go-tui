# Zen Go Profile UI Parity Repair Plan

> **For coding agents:** Execute each task with red-green-refactor. Run each focused test before and after its production change.

**Goal:** Restore capture-backed Zen Go behavior and master UI parity without rolling back the profile runtime.

**Architecture:** Keep device facts in `modules/Antelope-Ctl/profiles/zen_go_sc.json`. Keep raw wire values inside protocol adapters. Use semantic scalar and pan domains in application and UI code. Use the existing Zen Go compatibility state only for assignment, link, and routing behavior that the generic runtime model cannot represent yet.

**Stack:** Rust, Python, JSON profiles, Ratatui, Cargo, unittest, tmux.

**Spec:** `docs/superpowers/specs/2026-09-03-zen-go-profile-driven-runtime-repair.md`

**Global constraints:** Preserve the capture-backed 47 startup queries and q04 mixer layouts. Do not infer mute-plus-dim composition. Keep meter values above 60 as unknown. Do not expose observed storage routing groups as user controls. Do not commit the Antelope-Ctl submodule changes during this task.

## Task 1: Correct Zen Go profile presentation metadata

**Files:**
- Modify: `modules/Antelope-Ctl/profiles/zen_go_sc.json`
- Modify: `tools/generate_device_catalog.py`
- Modify: `tools/test_generate_device_catalog.py`
- Regenerate: `src/device/generated.rs`
- Regenerate: `src/device/generated_profiles.json`

1. Add a generator test named `test_zen_go_profile_uses_capture_confirmed_output_and_mixer_names`.
2. Assert output names `Monitor`, `HP1`, and `HP2`.
3. Assert mixer names `MIX 1 / Monitor-HP1` and `MIX 2 / HP2`.
4. Assert `bus_level` uses attenuation direction, unity `0`, and accurate encoding text.
5. Run the focused test and confirm it fails on placeholder names or stale encoding text.
6. Update the local Zen Go profile with master and capture evidence.
7. Regenerate both catalog artifacts.
8. Run the focused test and artifact drift check.

Completion criterion: Generated runtime contains confirmed user-facing names and internally consistent output scalar semantics.

## Task 2: Exclude output-only spaces from application input state

**Files:**
- Modify: `src/app/mod.rs`
- Modify: `src/app/dynamic_state_tests.rs`

1. Add a test named `profile_output_space_does_not_create_empty_input_bank`.
2. Build a runtime profile with one populated input space and one output-only space.
3. Assert `AppState::from_profile` creates one `InputSpaceState` and retains all outputs.
4. Run the focused test and confirm the extra input bank fails the assertion.
5. Filter projected input spaces by actual runtime inputs.
6. Run the focused test and related application tests.

Completion criterion: Zen Go selects the two-card preamp layout and no empty `Outputs` input heading remains.

## Task 3: Render profile enum labels for input modes

**Files:**
- Modify: `src/app/state.rs`
- Modify: `src/ui/render/mod.rs`
- Modify: `src/ui/dynamic_tests.rs`

1. Add a test named `dynamic_preamp_uses_profile_mode_label`.
2. Supply mode value `0` with profile label `Mic`.
3. Assert rendered text contains `Mic` and excludes `M0`.
4. Run the focused test and confirm it fails.
5. Add a narrow `UiProfileState` lookup for input control value labels.
6. Use the lookup in dynamic input rendering with a numeric fallback.
7. Run the focused UI test.

Completion criterion: Dynamic preamp cards show `Mic`, `Line`, or `HiZ` from profile metadata.

## Task 4: Use typed output scalar semantics

**Files:**
- Modify: `src/app/state.rs`
- Modify: `src/ui/widgets/mixer.rs`
- Modify: `src/ui/mouse.rs`
- Modify: `src/ui/tests.rs`

1. Add tests for attenuation values `0`, `10`, `27`, and `96`.
2. Assert display values `0`, `-10`, `-27`, and `-96` dB.
3. Assert slider and mouse conversions preserve attenuation direction.
4. Run focused tests and confirm current `96 - raw` behavior fails.
5. Expose output `FaderSemantics` from `RuntimeParam::scalar_semantics()`.
6. Replace range-only output helpers with semantics-aware helpers.
7. Use the same helpers for rendering and mouse intents.
8. Run focused UI and mouse tests.

Completion criterion: Live raw levels `10/27/15` render as `-10/-27/-15 dB`, and pointer writes map back to correct raw attenuation.

## Task 5: Keep mixer pan semantic through UI and controller

**Files:**
- Modify: `src/app/types.rs`
- Modify: `src/runtime.rs`
- Modify: `src/app/controller.rs`
- Modify: `src/ui/mouse.rs`
- Modify: `src/app/dynamic_state_tests.rs`
- Modify: `src/ui/tests.rs`

1. Change pan tests to use semantic range `-30..=30`.
2. Assert center ratio maps to `0`, left maps to `-30`, and right maps to `30`.
3. Assert keyboard steps increment and decrement semantic values.
4. Assert controller sends semantic pan values to the driver.
5. Run focused tests and confirm raw-centered code fails.
6. Change `Intent::SetMixerPanAt` to carry `i32` semantic pan.
7. Keep conversion to raw `PanState` inside protocol drivers.
8. Run focused runtime, controller, and UI tests.

Completion criterion: Pan text, slider position, keyboard writes, and mouse writes use one semantic domain.

## Task 6: Restore active assignment and link compatibility

**Files:**
- Modify: `src/app/mod.rs`
- Modify: `src/app/state.rs`
- Modify: `src/ui/widgets/mixer.rs`
- Modify: `src/app/zen_go_dynamic_tests.rs`

1. Add an active-path test named `query_reply_event_updates_zen_go_assignments_and_links`.
2. Feed q03 and q11 data through `AppState::observe_event`, not legacy `observe_frame`.
3. Assert source assignments and link state update in visible mixer state.
4. Run the test and confirm the active path fails.
5. Apply Zen Go raw query compatibility parsing inside `observe_event(QueryReply)`.
6. Let assignment controls use Zen Go compatibility when generic routing groups do not model mixer assignment.
7. Let dynamic strips display compatibility link state when normalized link state is absent.
8. Run focused application and widget tests.

Completion criterion: Live strips show source chips and link state, and assignment/link controls remain writable.

## Task 7: Restore user-facing Zen Go routing popup

**Files:**
- Modify: `src/ui/render/mod.rs`
- Modify: `src/ui/dynamic_tests.rs`

1. Add a test named `zen_go_routing_popup_prefers_recording_assignment_view`.
2. Populate observed low-level routing groups and Zen Go compatibility assignments.
3. Assert popup contains `Zen Go USB recordings mirror mixer strip assignments` and excludes `destination_6`.
4. Run the test and confirm current generic summary fails.
5. Prefer the existing Zen Go compatibility popup for Zen Go profiles.
6. Keep generic routing summary for other profiles.
7. Run focused UI tests.

Completion criterion: Routing popup matches the PATH/master recording-assignment view without mislabeling storage groups as mixer surfaces.

## Task 8: Verify profile/runtime/UI parity

**Files:**
- Verify all modified files.

1. Run `python -m unittest tools.test_generate_device_catalog`.
2. Run generator artifact drift check with explicit worktree paths.
3. Run `cargo fmt --all -- --check`.
4. Run `cargo test --workspace`.
5. Run `git diff --check`.
6. Run primary LSP and `lens_diagnostics(mode="all")` on edited source files.
7. Start PATH and worktree TUIs in separate 200x55 tmux sessions.
8. Compare preamp cards, output names and levels, mixer pan sliders, source chips, links, and routing popup.
9. Exercise only read-only navigation unless the user approves device mutations.

Completion criterion: Automated checks pass and live worktree output matches master for every audited field.
