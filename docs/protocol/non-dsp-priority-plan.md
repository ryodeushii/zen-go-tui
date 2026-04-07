# Non-DSP Protocol Backlog Plan

## Discovery

### Original Request
- "so aside from dsp only few things left, right? solo read and write per channel, metering per channel, per amp and combo for mix (for output basically e.g. one for monitor+hp1, another for hp2)"
- "please do"

### Interview Summary
- Scope: prioritize remaining non-DSP protocol work only.
- Confirmed priority order: mixer strip metering, output/master combo metering, preamp-vs-mixer metering split, solo passive decode, early-strip capability bounds.
- Constraint: keep working grounded behavior intact; do not regress source assignment, pan, mute, link, or current metering display separation.

### Research Findings
- `docs/zen-go-tui.md:157-165`: per-channel source assignment is grounded and implemented across `CH1..16`; startup assignment readback is seeded from `0x75`.
- `docs/zen-go-tui.md:175-181`: full passive mixer-state decode is still unresolved; metering is known to live in `0x73`, but exact strip/master parsing is still out of scope.
- `docs/protocol/open-questions.md:170-186`: solo write encoding is grounded, but passive solo-state decode is still unresolved.
- `docs/protocol/open-questions.md:208-233`: exact meter-field mapping is still unresolved; `0x73` is the likely meter plane, `0x83` is not.
- `src/protocol.rs:1287-1337`: current passive mixer decode only handles a narrow active-surface subset and a single observed shared meter path.
- `src/app.rs:173-222`: app state already preserves separate observed preamp meter state and applies passive mixer decode without overwriting assigned levels.
- `src/ui.rs:942-1100`: UI already has separate strip meter rendering and a dedicated preamp observed-meter widget, so protocol decode work can plug into existing display paths.

---

## Non-Goals (What we're NOT building)
- DSP/editor-page reverse engineering outside the already grounded preamp controls.
- Broad UI redesign.
- Refactoring unrelated protocol families.
- Commit or release work in this plan.

---

## Tasks

### 1. Ground Per-Channel Mixer Metering Decode

**Depends on**: none

**Files:**
- Modify: `docs/protocol/open-questions.md:208-233`
- Modify: `docs/protocol/mixer-protocol.md`
- Modify: `src/protocol.rs:1287-1337`
- Modify: `src/app.rs:190-208`
- Modify: `src/ui.rs:942-988`
- Test: `src/protocol.rs` tests
- Test: `src/app.rs` tests
- Test: `src/ui.rs` tests

**What to do**:
- Step 1: Write a failing protocol test for one grounded strip meter lane from the best passive capture.
- Step 2: Run the targeted test to verify it fails.
  - Run: `cargo test decodes_passive_mix1_strip1_meter_from_late_row_cluster`
  - Expected: FAIL if the new generalized meter target is not implemented yet.
- Step 3: Add one new failing test for a second strip / second mix based on the best capture pair.
- Step 4: Run the targeted test to verify it fails.
  - Run: `cargo test meter`
  - Expected: FAIL in the new test only.
- Step 5: Implement the minimal `0x73` strip-meter decode in `src/protocol.rs` without changing level decode semantics.
- Step 6: Thread the decoded strip meter into `AppState::apply_passive_mixer_decode()`.
- Step 7: Keep the existing UI contract: meter display remains separate from stored level.
- Step 8: Run targeted tests to verify they pass.
  - Run: `cargo test meter`
  - Expected: PASS for the newly added strip-meter tests.

**Must NOT do**:
- Do not mix master/output metering into this task.
- Do not overwrite stored assigned level with live meter values.

**References**:
- `docs/protocol/open-questions.md:208-233` — current meter-field uncertainty and best known candidate windows.
- `src/protocol.rs:1287-1337` — current narrow meter decode entry point.
- `src/ui.rs:942-988` — current strip meter presentation contract.

**Verify**:
- [ ] Run: `cargo test meter` → new strip-meter tests pass.
- [ ] Run: `cargo test passive_meter_does_not_override_known_level_value` → PASS.
- [ ] Run: `cargo test mixer_strip_line_renders_meter_separately_from_level_value` → PASS.

### 2. Ground Output/Master Combo Metering

**Depends on**: 1

**Files:**
- Modify: `docs/protocol/open-questions.md:208-233`
- Modify: `docs/protocol/mixer-protocol.md`
- Modify: `src/protocol.rs`
- Modify: `src/app.rs`
- Modify: `src/ui.rs`
- Test: `src/protocol.rs` tests
- Test: `src/ui.rs` tests

**What to do**:
- Step 1: Write a failing protocol test for `Monitor+HP1` master meter decoding from the current best isolated capture.
- Step 2: Run it to verify it fails.
  - Run: `cargo test master_meter`
  - Expected: FAIL in the new test.
- Step 3: Write a second failing protocol test for `HP2` master meter decoding.
- Step 4: Run it to verify it fails.
  - Run: `cargo test hp2`
  - Expected: FAIL in the new test.
- Step 5: Implement the minimal passive decode needed to separate `Monitor+HP1` meter behavior from `HP2`.
- Step 6: Surface those meters in state without inventing unsupported output semantics.
- Step 7: Render the meters in the smallest existing UI location that matches current output panel structure.
- Step 8: Run targeted tests to verify they pass.
  - Run: `cargo test meter`
  - Expected: PASS for new master/output meter tests.

**Must NOT do**:
- Do not infer unsupported output mute/dim semantics from meter activity.
- Do not change output volume control behavior.

**References**:
- `docs/protocol/open-questions.md:215-233` — current best isolated master-meter evidence.
- `docs/zen-go-tui.md:181` — current out-of-scope statement to retire once grounded.

**Verify**:
- [ ] Run: `cargo test meter` → master/output meter tests pass.
- [ ] Run: `cargo test ui::tests` → output panel rendering stays green.

### 3. Separate Preamp Direct Metering From Mixer Strip Metering

**Depends on**: 1, 2

**Files:**
- Modify: `docs/protocol/open-questions.md:208-233`
- Modify: `src/protocol.rs:1287-1337`
- Modify: `src/app.rs:173-222`
- Modify: `src/ui.rs:1036-1100`
- Test: `src/protocol.rs` tests
- Test: `src/app.rs` tests
- Test: `src/ui.rs` tests

**What to do**:
- Step 1: Write a failing test that proves preamp direct meter and mixer-strip meter can coexist without aliasing.
- Step 2: Run it to verify it fails.
  - Run: `cargo test preamp.*meter`
  - Expected: FAIL in the new coexistence test.
- Step 3: Implement the minimal decode/state split needed so preamp direct meter and strip meter do not share one bucket accidentally.
- Step 4: Preserve the existing `observed_meter` contract for preamp UI.
- Step 5: Run targeted tests to verify they pass.
  - Run: `cargo test preamp`
  - Expected: PASS for the new coexistence test and existing preamp meter tests.

**Must NOT do**:
- Do not rework preamp controls or DSP cluster handling.

**References**:
- `docs/protocol/open-questions.md:212-214` — coexistence boundary already documented.
- `src/app.rs:184-221` — current observed preamp meter preservation path.
- `src/ui.rs:1036-1100` — existing preamp observed meter widget.

**Verify**:
- [ ] Run: `cargo test preamp_pending_updates_preserve_observed_input2_meter` → PASS.
- [ ] Run: `cargo test observed_meter` → PASS.

### 4. Ground Passive Solo-State Decode Per Channel

**Depends on**: none

**Files:**
- Modify: `docs/protocol/open-questions.md:170-186`
- Modify: `docs/protocol/mixer-protocol.md`
- Modify: `src/protocol.rs`
- Modify: `src/app.rs:190-208`
- Modify: `src/ui.rs:965-987`
- Test: `src/protocol.rs` tests
- Test: `src/app.rs` tests

**What to do**:
- Step 1: Write a failing passive-solo decode test from the grounded strip-1 / strip-2 captures.
- Step 2: Run it to verify it fails.
  - Run: `cargo test solo`
  - Expected: FAIL in the new passive-solo test.
- Step 3: Implement the minimal passive solo decode without changing the already grounded solo write path.
- Step 4: Thread the decoded solo flag into app state.
- Step 5: Surface it in existing mixer strip rendering if the state is available.
- Step 6: Run targeted tests to verify they pass.
  - Run: `cargo test solo`
  - Expected: PASS for both existing and new solo tests.

**Must NOT do**:
- Do not invent exclusive-solo policy if captures do not prove it.
- Do not change solo write encoding.

**References**:
- `docs/protocol/open-questions.md:170-186` — current exact solo boundary.
- `src/protocol.rs:405` — solo bit helper already exists on pan/state bytes.

**Verify**:
- [ ] Run: `cargo test solo` → PASS.
- [ ] Run: `cargo test pan_state_decodes_mute_and_solo_flags_from_state_code` → PASS.

### 5. Bound Early-Strip Source Capability Matrix

**Depends on**: none

**Files:**
- Modify: `docs/protocol/open-questions.md:7-30`
- Modify: `docs/protocol/mixer-protocol.md:236-241`
- Modify: `src/protocol.rs`
- Modify: `src/ui.rs`
- Test: `src/protocol.rs` tests
- Test: `src/main.rs` tests

**What to do**:
- Step 1: Write failing tests for any newly proven invalid early-strip source combinations.
- Step 2: Run them to verify they fail.
  - Run: `cargo test assignment`
  - Expected: FAIL in the new early-strip capability tests.
- Step 3: Implement the minimal capability guard only if captures prove a restriction.
- Step 4: If no restriction is proven, document the remaining uncertainty and keep behavior unchanged.
- Step 5: Run targeted tests to verify they pass.
  - Run: `cargo test assignment`
  - Expected: PASS.

**Must NOT do**:
- Do not add speculative UI restrictions without capture proof.

**References**:
- `docs/protocol/open-questions.md:17-21` — current unresolved early-strip capability scope.
- `docs/protocol/mixer-protocol.md:236-241` — placeholder-vs-direct-early encoding boundary.

**Verify**:
- [ ] Run: `cargo test assignment` → PASS.
- [ ] Run: `cargo test mouse_assignment_picker_sends_selected_assignment_for_ordinary_strip` → PASS.

### 6. Final Documentation Sweep For Remaining Non-DSP State

**Depends on**: 1, 2, 3, 4, 5

**Files:**
- Modify: `docs/zen-go-tui.md`
- Modify: `docs/protocol/mixer-protocol.md`
- Modify: `docs/protocol/open-questions.md`
- Modify: `docs/protocol/pcap-analysis.md`

**What to do**:
- Step 1: Update user-facing support docs to reflect what is now grounded.
- Step 2: Move resolved items out of `open-questions.md` and tighten remaining unknowns.
- Step 3: Add exact capture/file references for any newly grounded meter or solo fields.
- Step 4: Run doc-focused grep checks for stale wording.
  - Run: `rg -n "out of scope|unresolved|deferred|partial|only grounded" docs`
  - Expected: only genuinely unresolved items remain.

**Must NOT do**:
- Do not claim fields are solved without test evidence and capture anchors.

**References**:
- `docs/zen-go-tui.md:175-181` — current intentionally limited meter statements.
- `docs/protocol/open-questions.md` — canonical remaining-unknowns tracker.

**Verify**:
- [ ] Run: `rg -n "out of scope|unresolved|deferred|partial|only grounded" docs` → stale solved-item wording removed.

---

## Execution Handoff

Two execution options:
1. Subagent-Driven (this session) — implement task-by-task in priority order with review between tasks.
2. Parallel Session (separate) — open a fresh session and execute this plan with the `executing-plans` skill.
