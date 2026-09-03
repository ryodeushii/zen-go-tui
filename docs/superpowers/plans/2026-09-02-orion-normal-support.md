# Plan: Promote Orion Studio III to Normal Runtime Support

Status: architecture approved, implementation pending.

## Goal

Promote `orion_studio_3` to normal runtime support using the existing generic `ProfileDriver`,
including accurate per-channel meters from Orion state reports. Keep canonical Antelope-Ctl evidence
unchanged. Make generated runtime data carry one explicit non-numbered HID framing assumption.
Preserve all unresolved evidence and defer physical hardware verification without claiming it happened.

## Architecture

Use an Orion-only policy in `tools/generate_device_catalog.py`.

- Match Orion by profile id `orion_studio_3` and VID/PID `0x23e5:0xa221`.
- Resolve missing Orion `uses_numbered_reports` to `false` in normalized runtime output.
- Keep an explicit conflicting value of `true` blocking support rather than masking it.
- Remove the physical/ADAT link blocker only when no physical/ADAT link action is emitted.
- Keep confirmed mixer link domain space 3 with 16 pairs as the only exposed link domain.
- Classify Orion as `Supported` only after every other existing readiness check passes.
- Map supported Orion to `RuntimeDriverKind::Profile`.
- Keep profile validation strict. Allow the generic driver to select either a confirmed meter-report
  source or a confirmed state-report meter source, with equivalent finite layout validation.
- Add a generated Orion `physical_meter` indexed operation to `state_report` at base 157, stride 1,
  and physical-channel max index 11.
- Populate state snapshot input meters from that operation.
- Treat Orion `meter_report` as superseded for per-channel values. Ignore its response instead of
  decoding bytes 33+ as physical channels.
- Reuse existing discovery, picker, session, controller, UI, and transport paths.
- Do not add descriptor probing, framing fallback, blind probing, a dedicated Orion driver, or a
  physical/ADAT link action.

The generated support reason and generated hazard metadata must state that non-numbered HID framing
is a generator policy assumption pending descriptor and hardware verification. They must also retain
the superseded per-channel meter warning. Identity status and raw source evidence remain unchanged.

## Tech Stack

- Python 3 generator and `unittest` tests.
- Rust workspace with Cargo, Serde, HIDAPI, and the existing profile codec.
- Generated Rust catalog at `src/device/generated.rs`.
- Generated normalized JSON pack at `src/device/generated_profiles.json`.
- Ratatui runtime and existing picker/session integration.

## Spec

Approved design:
`docs/superpowers/specs/2026-09-02-orion-normal-support-design.md`

## Global Constraints

- Do not edit `modules/Antelope-Ctl/profiles/orion_studio_3.json`.
- Do not change canonical source hashes or raw evidence strings.
- Do not invent physical/ADAT input-link semantics.
- Do not expose any link domain except confirmed mixer space 3.
- Do not weaken generic `ProfileDriver` validation. A state-meter fallback must validate confirmed
  source, width, finite count, and complete physical-input coverage.
- Do not decode Orion `meter_report` bytes 33+ as per-channel meters.
- Do not change behavior for Zen Go, Discrete 8 Pro, Discrete 4 Pro, or other profiles.
- Keep malformed, incomplete, conflicting, formula-bearing, superseded-source, and out-of-bounds
  Orion data disabled or unsupported.
- A wrong framing assumption must surface as a transport error and use existing reconnect behavior.
- Invalid profile mappings must fail before HID open.
- Unsupported physical/ADAT link requests must produce no write.
- Hardware validation is future work. Do not report it as completed.
- Follow RED, GREEN, then refactor. Observe one failing test before production edits.
- Run formatters and checks on touched files before completion.
- Do not commit unrelated changes.

## Repository map

### Generator and policy tests

- `tools/generate_device_catalog.py`
  - `orion_readiness_blockers(profile)` at lines 760-856.
  - `classify_readiness(profile)` at lines 859-874.
  - `_runtime_driver_kind(profile, readiness)` around line 3073.
  - `_normalized_profile_record(profile)` around lines 3459-3616.
  - `_section_status(section, fallback)` at lines 1064-1085.
  - `normalize_profile(data, path=None, profiles_dir=None, source_bytes=None)` and
    `render_profile_pack(profiles)`.
- `tools/test_generate_device_catalog.py`
  - Existing Orion readiness, topology, startup, raw-source, and generated-artifact tests.
  - Existing Discrete readiness regression tests.

### Generated runtime data

- `src/device/generated.rs`
  - Generated Orion `RuntimeEntry`, transport mode, support reason, and driver kind.
- `src/device/generated_profiles.json`
  - Generated normalized pack consumed by runtime profile loading.
- `antelope-protocol/tests/fixtures/orion/profile_driver_pack.json`
  - Single-entry fixture used by profile driver and profile pack tests.

### Runtime codec and transport consumers

- `antelope-protocol/src/profile.rs`
  - `RuntimeReadiness`, `RuntimeDriverKind`, `RuntimeEntry`, and pack validation.
- `antelope-protocol/src/profile_driver.rs`
  - Generic driver source-selection extension. Existing validation remains strict for both meter
    source forms.
- `antelope-protocol/tests/profile_driver.rs`
  - Profile encode/decode and fixture behavior tests.
- `antelope-protocol/tests/profile_pack.rs`
  - Normalized pack loading and validation tests.
- `src/device/session.rs`
  - Existing `driver_for_entry` maps `RuntimeDriverKind::Profile` to `ProfileDriver`.
- `src/transport.rs`
  - Existing non-numbered report preparation and exact-write validation.

### Documentation

- `docs/device-support.md`
  - Support matrix, generated-source policy, selection behavior, evidence, and hardware-validation
    notes.
- `docs/superpowers/specs/2026-09-02-orion-normal-support-design.md`
  - Approved architecture record.
- `docs/superpowers/plans/2026-09-02-orion-normal-support.md`
  - This implementation plan.

## Data flow and ownership

1. Canonical Orion JSON remains the evidence owner.
2. Generator applies only the declared Orion runtime policy and emits normalized JSON and Rust.
3. Runtime catalog validation checks the generated entry before discovery and HID open.
4. Candidate classification sees `Supported` plus `Profile` and makes Orion selectable.
5. Session construction creates `ProfileDriver` before opening HID.
6. Existing transport applies generated non-numbered framing and validates write length.
7. Existing generic codec encodes confirmed controls and decodes confirmed state, meter, mixer,
   routing, and readback mappings. State snapshots include meters when a confirmed state-meter
   source exists. Superseded meter responses are not decoded as channel meters.
8. Unknown or unsupported actions fail without a write, or surface through existing device-error
   reconnect handling.

## Implementation steps

### Task 0: Integrate reviewed baseline

- [ ] Confirm root is `/home/ryodeushii/repos/zen-go-tui` on `feature/multi-device-antelope`.
- [ ] Confirm root has no unrelated changes and preserve the untracked approved spec.
- [ ] Fast-forward root to reviewed commit `04a79e2` with `git merge --ff-only 04a79e2`.
- [ ] Confirm the baseline profile-support files and approved spec both exist.
- [ ] Treat this fast-forward as baseline integration. The first Orion promotion change remains a
      test-only RED change.

### Task 1: RED: specify Orion promotion behavior

- [ ] Add `test_orion_normal_support_policy` to `tools/test_generate_device_catalog.py`.
- [ ] Load canonical `orion_studio_3.json` through existing generator helpers.
- [ ] Assert readiness is `Supported`, driver kind is `Profile`, and normalized transport framing
      is `false`.
- [ ] Assert the transport framing blocker and ambiguous physical/ADAT link blocker are absent.
- [ ] Assert identity status remains `unknown` and the canonical source hash remains unchanged.
- [ ] Run:
      `python3 -m unittest tools.test_generate_device_catalog.GeneratorTests.test_orion_normal_support_policy`
- [ ] Confirm the test fails against the current unconditional Orion `Disabled` policy.
- [ ] Do not edit production code until this failure is observed.

### Task 2: GREEN: implement Orion-only generator policy

- [ ] Add one Orion identity predicate and one effective framing helper near the existing readiness
      policy helpers in `tools/generate_device_catalog.py`.
- [ ] Make the effective framing helper return raw framing for non-Orion profiles, return `false`
      only for Orion with missing framing, and preserve explicit Orion `true`.
- [ ] Make `orion_readiness_blockers` evaluate effective framing for the canonical missing value.
- [ ] Add Orion-only effective status for source-backed generic mappings. Promote only
      `command`, `global_command`, `mix_command`, `link_command`, `state_report`, and `readback`
      when their finite operation geometry and confirmed parameter or decoded evidence are present.
- [ ] Recognize explicit `DECODED` readback evidence as confirmed without changing negative or
      superseded statuses. Do not promote a frame from Orion identity alone.
- [ ] Add the Orion state-meter mapping from confirmed `state_report.channel_meter_notes`: emit one
      `physical_meter` indexed operation at base 157, stride 1, and max index physical-count minus
      one. Reject missing, ambiguous, or out-of-range state-meter evidence.
- [ ] Replace the unconditional ambiguous `adat_channel_link` note blocker with a check for an
      emitted actionable physical/ADAT link. Keep the blocker when such an action or domain exists.
- [ ] Remove the unconditional Orion `Disabled` return from `classify_readiness`.
- [ ] Let existing strict Orion geometry, source-backed frame status, formula, readback, startup,
      routing, link, and state-meter checks determine whether the result is `Supported`.
- [ ] Exclude unsupported `auraverb_command` and superseded `meter_report` from required actionable
      frame sources while requiring the confirmed state-report meter mapping.
- [ ] Add the Orion `Supported` branch to `_runtime_driver_kind` and return `Profile`.
- [ ] Preserve Zen Go mapping and `None` for every unsupported or non-supported profile.
- [ ] Make normalized transport output use effective Orion framing rather than raw `null`.
- [ ] Extend generated support reason and hazard metadata with the non-numbered framing assumption
      and pending hardware verification. Keep raw hazard entries intact.
- [ ] Run the focused RED test again and confirm it passes.
- [ ] Run:
      `python3 -m unittest tools.test_generate_device_catalog.GeneratorTests.test_readiness_requires_registered_driver_and_complete_profile`
- [ ] Confirm existing expectations now cover Orion `Supported` and `Profile` without changing
      Zen Go or Discrete behavior.

### Task 3: Add fail-closed readiness regression coverage

- [ ] Add or update Python tests for an Orion entry with explicit `uses_numbered_reports=true`.
- [ ] Assert it remains disabled with the unrepresentable framing blocker.
- [ ] Add or update a Python test for an actionable physical/ADAT link parameter or domain.
- [ ] Assert it remains disabled and the ambiguous-link blocker remains present.
- [ ] Keep tests for missing command operations, unsupported formulas, unsafe startup bounds, and
      incomplete geometry disabled.
- [ ] Treat these as regression tests for the guards implemented in Task 2. If a guard is already
      correct, record its existing green result. If a new production change is required, write and
      run its failing test before that production change.
- [ ] Run:
      ```bash
      python3 -m unittest \
        tools.test_generate_device_catalog.GeneratorTests.\
        test_orion_field_counts_alone_and_one_missing_operation_never_enable
      ```
- [ ] Run:
      ```bash
      python3 -m unittest \
        tools.test_generate_device_catalog.GeneratorTests.\
        test_orion_unsupported_formula_and_unsafe_startup_bound_are_blockers
      ```
- [ ] Run the new conflicting-framing and actionable-link tests.
- [ ] Run:
      `python3 -m unittest tools.test_generate_device_catalog.GeneratorTests.test_discrete_readiness_policy_is_unchanged`
- [ ] Confirm all strict boundaries pass and non-Orion behavior is unchanged.

### Task 4: Regenerate runtime artifacts

- [ ] Run:
      ```bash
      python3 tools/generate_device_catalog.py \
        --profiles-dir modules/Antelope-Ctl/profiles \
        --output src/device/generated.rs \
        --pack-output src/device/generated_profiles.json
      ```
- [ ] Update `antelope-protocol/tests/fixtures/orion/profile_driver_pack.json` from the generated
      Orion entry while retaining its single-entry pack shape.
- [ ] Confirm generated Orion metadata contains `readiness: supported`, `driver_kind: profile`,
      and `uses_numbered_reports: false`.
- [ ] Confirm generated Orion exposes only mixer link space 3 with 16 pairs and no physical/ADAT
      link action.
- [ ] Confirm generated state-report operations include `physical_meter` at base 157, stride 1,
      max index 11, while superseded `meter_report` remains non-actionable for per-channel data.
- [ ] Confirm generated identity status is still `unknown` and raw source content is retained.
- [ ] Review generated changes with targeted `rg` and `git diff --stat`, not a full raw-profile
      line dump.
- [ ] Run:
      ```bash
      python3 tools/generate_device_catalog.py \
        --check modules/Antelope-Ctl/profiles \
        --generated src/device/generated.rs \
        --pack-generated src/device/generated_profiles.json
      ```
- [ ] Confirm the generated-artifact check is clean.

### Task 5: Extend and verify generic ProfileDriver construction and actions

- [ ] Confirm Task 4 updated `antelope-protocol/tests/fixtures/orion/profile_driver_pack.json` from the
      promoted generated entry. Do not perform a second fixture rewrite.
- [ ] Add an Orion fixture test in `antelope-protocol/tests/profile_pack.rs` that loads the promoted
      fixture, validates the pack, finds `orion_studio_3`, and constructs `ProfileDriver`.
- [ ] Run:
      `cargo test -p antelope-protocol --test profile_pack`
- [ ] Extend generic `ProfileDriver::new` to accept either a confirmed meter-report source or a
      confirmed state-report `physical_meter` source, with strict finite-layout validation.
- [ ] Make meter-report decoder, discriminator, and zero-report checks conditional on a confirmed
      meter-report source. Keep that response-meter path unchanged when present.
- [ ] Make `decode_state` validate the state `physical_meter` layout against finite physical-input
      count and populate each `DynamicInputState.meter` from the selected state source.
- [ ] Make `decode` emit `DeviceEvent::Meter` only when a confirmed meter-report source exists.
      With only a state source, ignore non-readback superseded meter responses.
- [ ] Confirm promoted Orion constructs through existing `ProfileDriver::new` with strict validation
      of its confirmed state-meter source. Task 1 is the watched RED gate for this promotion.
- [ ] Add focused `antelope-protocol/tests/profile_driver.rs` coverage using
      `state_report_73.hex`, `readback_75.hex`, and `startup_requests.txt`.
- [ ] Verify profile input, output, global, mixer, confirmed link, routing, and query actions use
      existing codec mappings and expected report bytes.
- [ ] Verify state snapshots populate physical input meters from state-report offset 157.
- [ ] Verify malformed state-meter width, count, or range fails driver construction.
- [ ] Verify superseded meter responses are ignored and never decode bytes 33+ as channels.
- [ ] Verify profiles with confirmed meter-report sources retain their existing response-meter path.
- [ ] Verify no physical/ADAT link action is available or encoded.
- [ ] Run:
      `cargo test -p antelope-protocol --test profile_driver`
- [ ] Confirm state, meter, mixer, routing, readback, and startup behavior passes with the generic
      source-selection extension.

### Task 6: Verify runtime selection and transport integration

- [ ] Add or update the generated-pack assertion in `antelope-protocol/tests/profile_pack.rs` that
      classifies promoted Orion as `Supported` and selects `RuntimeDriverKind::Profile`.
- [ ] Confirm `src/device/session.rs::driver_for_entry` uses existing `ProfileDriver::new` for that
      driver kind. Add a focused session test in `src/device/session.rs` only if existing coverage
      does not exercise this branch.
- [ ] Confirm `src/transport.rs` keeps existing non-numbered behavior: logical 320-byte reports
      receive the zero report-id prefix and writes require the exact HID length.
- [ ] Keep the existing unknown-framing rejection test for all profiles that have no policy value.
- [ ] Confirm invalid driver/profile mappings fail before HID open.
- [ ] Do not claim physical discovery, descriptor verification, or hardware round-trip success.

### Task 7: Update documentation

- [ ] Update `docs/device-support.md` support matrix from Orion `Disabled/None` to
      `Supported/Profile`.
- [ ] Document the generator-only non-numbered framing assumption and pending hardware verification.
- [ ] Document Orion per-channel meters come from `state_report` offset 157 and that superseded
      `meter_report` bytes 33+ are not interpreted as channels.
- [ ] Document that only confirmed mixer link space 3 is exposed.
- [ ] Document that physical/ADAT link semantics remain non-actionable.
- [ ] Keep canonical/raw profile ownership, generated-artifact commands, selection behavior, and
      hardware-validation limitations accurate.
- [ ] Keep approved design decisions unchanged. Update only its implementation-status line after
      verification if the status would otherwise be stale.

### Task 8: Full verification

- [ ] Run the complete Python generator suite:
      `python3 -m unittest discover -s tools -p 'test_*.py'`
- [ ] Run workspace Rust tests:
      `cargo test --workspace`
- [ ] Run formatting:
      `cargo fmt --all -- --check`
- [ ] Run whitespace validation:
      `git diff --check`
- [ ] Run the generator drift check from step 4.
- [ ] Run primary LSP diagnostics on edited Rust files, including
      `antelope-protocol/src/profile.rs`, `antelope-protocol/src/profile_driver.rs`,
      `antelope-protocol/tests/profile_driver.rs`, `antelope-protocol/tests/profile_pack.rs`,
      and `src/device/session.rs` if edited.
- [ ] Run `lens_diagnostics` with `mode=all` for every edited source file.
- [ ] Run the docs-drift review and apply only required documentation updates.
- [ ] Inspect `git diff --stat`, `git diff --check`, and final status.
- [ ] Confirm no hardware-validation claim appears in code or docs.

### Task 9: Commit and push

- [ ] Stage only Orion promotion code, generated artifacts, tests, fixture, documentation, approved
      spec, and this plan.
- [ ] Commit with:
      `git commit -m "feat: enable Orion Studio III runtime support"`
- [ ] Confirm the commit contains the reviewed baseline ancestor and the Orion promotion changes.
- [ ] Push the requested branch:
      `git push origin HEAD:feature/multi-device-antelope`
- [ ] Confirm push output and clean working tree.
- [ ] Report commit id, pushed branch, verification commands, and deferred hardware verification.

## Verification matrix

| Area | Required result |
| --- | --- |
| Canonical source | Orion JSON unchanged, source hash and raw evidence preserved |
| Generator policy | Missing Orion framing normalizes to `false` only for Orion |
| Readiness | Complete canonical Orion is `Supported`, malformed or unsafe variants stay disabled |
| Driver selection | Supported Orion maps to `RuntimeDriverKind::Profile` |
| Codec | Generic `ProfileDriver` constructs Orion and validates its state-meter source |
| Meter source | State-report `physical_meter` base 157, stride 1, max index 11; superseded response ignored |
| Links | Mixer space 3 only, physical/ADAT linking absent and non-actionable |
| Transport | Existing non-numbered prefix and exact-write checks remain active |
| Regression | Zen Go and Discrete readiness behavior unchanged |
| Documentation | Matrix and framing assumption match generated runtime behavior |
| Safety | No probing, fallback framing, descriptor claim, or hardware-validation claim |
| Delivery | Commit pushed to `feature/multi-device-antelope`, working tree clean |

## Acceptance criteria

- Canonical Orion source remains unchanged.
- Generated Orion entry is `Supported` with `Profile` driver kind and effective framing `false`.
- Existing generic `ProfileDriver` validates and constructs Orion with a confirmed state-meter source
  without weakening validation for normal meter-report profiles.
- Confirmed Orion state, state-meter, mixer, routing, readback, startup, and supported control tests
  pass.
- Physical/ADAT link actions are absent and no unsupported link write is emitted.
- Superseded Orion meter responses never produce per-channel values from bytes 33+.
- Malformed, conflicting, formula-bearing, superseded-source, and unsafe Orion profiles remain
  disabled or unsupported.
- Zen Go and Discrete behavior remains unchanged.
- Python tests, workspace Cargo tests, formatting, generator drift, diff checks, LSP diagnostics,
  lens diagnostics, and documentation review pass.
- Commit is pushed to `feature/multi-device-antelope`.
- Hardware verification remains explicitly deferred.
