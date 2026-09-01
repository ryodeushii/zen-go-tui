# Multi-device Antelope support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enumerate Antelope hardware safely, generate Rust device definitions from Antelope-Ctl profiles, preserve Zen Go behavior, and add full Orion Studio III support.

**Architecture:** A generated static catalog separates profile data from runtime drivers. HID discovery returns metadata-rich candidates and opens selected paths exactly. A `DeviceDriver` seam converts normalized application actions/events to device-specific frames; dynamic runtime state and UI consume generated capabilities rather than fixed Zen Go arrays.

**Tech Stack:** Rust 2021, Cargo build-time/generated source, Python standard-library profile generator, `hidapi` 2.6, `serde`, `ratatui`, existing captured-packet tests.

**Spec:** `docs/superpowers/specs/2026-08-31-multi-device-antelope.md`

## Global Constraints

- Antelope-Ctl `profiles/*.json` is canonical; exclude `mic_models.json`.
- Ordinary Cargo builds must not require an Antelope-Ctl checkout.
- Generated Rust records source profile SHA-256 and generator version; generator check detects drift.
- Every hardware profile appears in catalog; only validated runtime support is selectable.
- Never send commands to unsupported, incomplete, or ambiguous candidates.
- Zen Go `0x23e5:0xa015` remains supported; Orion `0x23e5:0xa221` becomes fully supported; Discrete profiles remain visible/disabled until validated.
- HID selection opens exact selected path and does not blindly choose first VID/PID match.
- Full Orion model includes 12 physical inputs, 16 ADAT inputs, 2 S/PDIF inputs, 6 buses, Mix 1-4, and 32 strips per mix.
- JSON profile fields with unconfirmed status are represented but unavailable to normal commands.
- No commits are created unless user requests them.
- Every production behavior change follows red, green, refactor testing; generated output is exempt from test-first code authoring but generator behavior is test-first.

---

### Task 1: Define typed device schema and profile generator

**Files:**
- Create: `src/device/mod.rs`
- Create: `src/device/definition.rs`
- Create: `tools/generate_device_catalog.py`
- Create: `tools/test_generate_device_catalog.py`
- Modify: `src/lib.rs`
- Modify: `.gitignore` only if generated temporary files require it
- Create: `src/device/generated.rs`

**Interfaces:**
- Produces `DeviceDefinition`, `DeviceEntry`, `SupportLevel`, `TransportDefinition`, `AddressSpaceDefinition`, `InputDefinition`, `OutputDefinition`, `MixerDefinition`, `FrameDefinition`, `ParamDefinition`, `ConstraintDefinition`, and `HazardDefinition`.
- Produces `pub static DEVICE_CATALOG: &[DeviceEntry]` in `src/device/generated.rs`.
- Generator CLI: `python3 tools/generate_device_catalog.py --profiles-dir PATH --output PATH` and `--check PATH --generated PATH`.

- [ ] **Step 1: Write failing generator tests**

Test canonical-profile discovery, exclusion of `mic_models.json`, hex/int parsing, source hash output, readiness classification, and rejection of incomplete required profile data. Assert generated Rust contains Orion, Zen Go, Discrete 8 Pro, both Discrete 4 entries, and no mic-model catalogue.

- [ ] **Step 2: Run generator tests and verify expected failures**

Run: `python3 -m unittest tools/test_generate_device_catalog.py -v`
Expected: FAIL because generator and typed output do not exist.

- [ ] **Step 3: Implement typed schema and generator**

Parse loose JSON into normalized typed records. Emit Rust literals for every required profile section, preserving status, constraints, hazards, frame layouts, decoder metadata, and profile SHA-256. Mark readiness independently from profile existence: Zen Go supported, Orion supported only after driver registration is present, Discrete 8 partial/disabled, Discrete 4 profiles unverified/disabled. Generator must fail on malformed identity or transport fields instead of inventing defaults.

- [ ] **Step 4: Generate catalog from Antelope-Ctl profiles**

Run: `python3 tools/generate_device_catalog.py --profiles-dir /home/ryodeushii/repos/Antelope-Ctl/profiles --output src/device/generated.rs`
Expected: generated Rust compiles and includes five hardware profiles, source hashes, and no `mic_models.json` entry.

- [ ] **Step 5: Run generator and Rust tests**

Run: `python3 -m unittest tools/test_generate_device_catalog.py -v && cargo test -p antelope-protocol device`
Expected: PASS.

**Completion criterion:** Typed generated catalog compiles without Antelope-Ctl present, all five hardware profiles are represented, and drift check reports changed canonical inputs.

---

### Task 2: Add HID discovery, classification, and exact-path opening

**Files:**
- Create: `src/device/discovery.rs`
- Modify: `src/device/mod.rs`
- Modify: `src/transport.rs`
- Modify: `src/cli.rs`
- Create or modify: `src/device/discovery_tests.rs` or module tests

**Interfaces:**
- Produces `DeviceCandidate`, `CandidateStatus`, `enumerate_antelope_devices(&HidApi, catalog)`, `classify_candidate`, and `sort_candidates`.
- Produces `HidTransport::open_path(candidate: &DeviceCandidate) -> Result<Self>`.
- Keeps `HidTransport::open(vid, pid)` only as a tested compatibility wrapper for existing Zen Go callers until Task 4 removes its use.

- [ ] **Step 1: Write failing pure discovery tests**

Test matching VID, metadata extraction, known supported classification, incomplete-profile disabled classification, unsupported-PID classification, supported-first stable sorting, and duplicate interface diagnostics. Use pure candidate records for classification; do not mock HID internals.

- [ ] **Step 2: Run discovery tests and verify expected failures**

Run: `cargo test device::discovery -p zen-go-tui`
Expected: FAIL because discovery module and classification API do not exist.

- [ ] **Step 3: Implement candidate classification and sorting**

Use `HidApi::device_list()` filtered by VID `0x23e5`. Capture path, PID, serial, product, usage page, usage, and interface number. Keep ambiguous same-device interfaces as diagnostics until profile metadata resolves control interface. Sort by readiness, then product/name, then path.

- [ ] **Step 4: Implement exact-path transport opening**

Convert candidate path to HIDAPI C string and call `HidApi::open_path`. Allocate read buffer from selected profile report size. Reject unsupported or ambiguous candidates before opening. Preserve reconnect identity with exact path first and validated identity fallback.

- [ ] **Step 5: Run discovery, transport, and existing tests**

Run: `cargo test device::discovery transport:: -p zen-go-tui`
Expected: PASS.

**Completion criterion:** VID-only enumeration returns safe metadata-rich candidates, supported candidates sort first, disabled candidates carry reasons, and selected transport opens only exact path.

---

### Task 3: Introduce protocol-driver seam and extract Zen Go backend

**Files:**
- Create: `antelope-protocol/src/driver.rs`
- Create: `antelope-protocol/src/zen_go.rs`
- Modify: `antelope-protocol/src/lib.rs`
- Modify: `src/app/controller.rs`
- Modify: `src/app/state.rs`
- Modify: `src/cli.rs`
- Modify: `src/runtime.rs`
- Modify: affected existing tests

**Interfaces:**
- Produces `DeviceDriver` with `definition()`, `startup_requests()`, `encode(Action)`, and `decode(&[u8])` methods.
- Produces normalized `Action`, `CommandBatch`, `DeviceEvent`, and `DynamicDeviceState` types.
- `Controller::new(transport: Box<dyn Transport>, driver: Box<dyn DeviceDriver>)` owns selected driver and transport.

- [ ] **Step 1: Write failing driver contract tests**

Test Zen Go driver identity, startup request count/bytes, representative output/preamp/mixer encoding, snapshot decoding, and preservation of existing raw packet behavior. Test controller rejects unsupported driver before writes.

- [ ] **Step 2: Run driver tests and verify expected failures**

Run: `cargo test -p antelope-protocol zen_go_driver && cargo test controller_driver -p zen-go-tui`
Expected: FAIL because driver interface and normalized actions do not exist.

- [ ] **Step 3: Implement driver interface and Zen Go adapter**

Move existing fixed encoding/decoding behind `ZenGoDriver` without changing frame bytes. Translate existing `Command`/`Frame` behavior into normalized batches/events. Keep Zen Go-specific decoder code in adapter module, not in controller.

- [ ] **Step 4: Inject driver into controller and runtime**

Controller startup, writes, polling, and reconnect refresh use driver methods. Preserve saved-state profile behavior separately from generated device definitions. Existing mock tests select Zen Go driver explicitly.

- [ ] **Step 5: Run full existing test suite**

Run: `cargo test --workspace`
Expected: PASS with existing Zen Go behavior unchanged.

**Completion criterion:** Controller and runtime no longer import fixed protocol operations directly; Zen Go tests pass byte-for-byte.

---

### Task 4: Build dynamic device state and UI model

**Files:**
- Modify: `src/app/state.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/app/controller.rs`
- Modify: `src/profile.rs` only for naming separation if required
- Modify: `src/ui/layouts.rs`
- Modify: `src/ui/render/*.rs`, `src/ui/widgets/*.rs`, and UI tests as needed

**Interfaces:**
- Produces dynamic state collections keyed by generated input/output/surface IDs.
- Existing intents remain stable where possible; unsupported controls return explicit no-op/error status.
- Saved-state TOML profiles remain compatible and are not used as protocol definitions.

- [ ] **Step 1: Write failing state/UI tests**

Test state construction from Zen Go and Orion definitions, 12 Orion preamps, 6 Orion buses, four 32-strip mixer surfaces, capability-based panel visibility, generated names, and no out-of-bounds indexing. Test existing Zen Go rendering snapshots/interaction behavior.

- [ ] **Step 2: Run tests and verify expected failures**

Run: `cargo test dynamic_state ui:: -p zen-go-tui`
Expected: FAIL because state remains fixed-size Zen Go data.

- [ ] **Step 3: Replace fixed arrays with capability-driven collections**

Use generated definitions to allocate inputs, outputs, mixer surfaces, strips, meters, and address spaces. Preserve IDs and selected indices through refresh. Remove hardcoded Zen Go header text from dynamic paths.

- [ ] **Step 4: Adapt layout and render paths**

Render dynamic labels/counts and paginate or viewport larger Orion collections. Show only capabilities exposed by selected definition. Display device name, serial, readiness, and disabled diagnostics in status/header views.

- [ ] **Step 5: Run workspace tests**

Run: `cargo test --workspace`
Expected: PASS.

**Completion criterion:** Dynamic model renders Zen Go unchanged and can represent full Orion dimensions without fixed-array assumptions.

---

### Task 5: Implement Orion protocol driver and full capability mapping

**Files:**
- Create: `antelope-protocol/src/orion.rs`
- Modify: `antelope-protocol/src/driver.rs`
- Modify: `antelope-protocol/src/lib.rs`
- Modify: `src/device/generated.rs` support registration if needed
- Create: `antelope-protocol/tests/orion_golden.rs`
- Add captured fixtures under `antelope-protocol/tests/fixtures/orion/`

**Interfaces:**
- Produces `OrionDriver` implementing `DeviceDriver` for PID `0xa221`.
- Supports generated Orion command/frame definitions, startup topology, state reports, meter reports, preamp/input spaces, ADAT, S/PDIF, buses, Mix 1-4 with 32 strips, routing, and confirmed profile-defined controls.

- [ ] **Step 1: Write failing Orion golden tests**

Add tests for profile identity, startup request bytes, physical preamp mode/gain/phantom/phase, global settings, all six buses, mixer frame including send byte, link spaces, routing group frames, state/meter decoding, and rejection of unconfirmed fields. Fixtures must assert exact bytes and decoded normalized events.

- [ ] **Step 2: Run Orion tests and verify expected failures**

Run: `cargo test -p antelope-protocol --test orion_golden`
Expected: FAIL because Orion driver and fixtures are absent.

- [ ] **Step 3: Implement shared profile-driven command encoding**

Use generated frame/parameter definitions for confirmed generic command shapes. Validate opcode, target space, enum, range, report length, and min write interval before producing frames. Return an explicit unsupported error for incomplete or unconfirmed definitions.

- [ ] **Step 4: Implement Orion-specific startup/state/meter decoder**

Decode Orion `0x73`, `0x75`, `0x74`, and other profile-defined reports into normalized events. Keep Zen Go decoder separate for differing magic/layout/mixer semantics. Include six output buses, 12 physical channels, 16 ADAT channels, two S/PDIF channels, 4×32 mixer state, and profile-defined routing state.

- [ ] **Step 5: Enable Orion only after golden tests pass**

Run: `cargo test --workspace`
Expected: PASS; catalog marks Orion selectable only when driver registration and all required tests compile.

**Completion criterion:** Orion can construct full runtime state, encode confirmed controls, decode captured state/meter reports, and never falls back to Zen Go frame layouts.

---

### Task 6: Add device picker and safe runtime switching

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/runtime.rs`
- Modify: `src/app/state.rs`
- Modify: `src/ui/layouts.rs` and relevant render/input modules
- Add picker tests in existing UI/runtime test modules

**Interfaces:**
- Produces startup picker with supported candidates first and disabled diagnostics.
- Adds deterministic `--device PATH|VID:PID|SERIAL` selection for headless/automation where valid.
- Recreates transport/controller/driver on switch; no worker transport is mutated underneath an active controller.

- [ ] **Step 1: Write failing picker tests**

Test empty discovery, supported-first order, disabled entries, selecting Orion by path, rejecting disabled selection, and replacing active controller without sending through old transport. Test mock mode selects Zen Go explicitly.

- [ ] **Step 2: Run picker tests and verify expected failures**

Run: `cargo test picker runtime_device_selection -p zen-go-tui`
Expected: FAIL because startup still opens fixed Zen Go PID before UI creation.

- [ ] **Step 3: Implement picker lifecycle**

Move real-device discovery before controller construction. Show picker/wait screen when no candidates exist. Open exact path, construct driver from catalog, then construct controller. Reopen picker on disconnect or explicit device-switch action.

- [ ] **Step 4: Implement safe reconnect and switching**

Close old worker before creating new one. Prefer exact path, then validated serial/VID/PID/interface identity. Report path changes and ambiguity instead of guessing.

- [ ] **Step 5: Run full tests**

Run: `cargo test --workspace`
Expected: PASS.

**Completion criterion:** User can select supported Zen Go or Orion candidate from picker, see all known disabled devices, and no unsupported candidate can cause a write.

---

### Task 7: Disable incomplete devices, add drift checks, docs, and final verification

**Files:**
- Modify: `README.md`
- Modify: `docs/zen-go-tui.md`
- Create: `docs/device-support.md`
- Modify: CI/configuration files only if repository has an existing CI workflow
- Modify: generator tests and golden fixtures as needed

**Interfaces:**
- Documents canonical profile workflow, generated Rust output, support/readiness matrix, picker behavior, and hardware validation steps.
- Provides profile drift check command that runs without changing generated output.

- [ ] **Step 1: Write failing drift/support documentation checks**

Test generator `--check` detects modified profile hash and passes against current generated output. Test catalog support statuses match documented matrix.

- [ ] **Step 2: Run checks and verify expected failures**

Run: `python3 tools/generate_device_catalog.py --check /home/ryodeushii/repos/Antelope-Ctl/profiles --generated src/device/generated.rs`
Expected: PASS for current files after Task 1; add regression test that fails for altered hash.

- [ ] **Step 3: Implement checks and documentation**

Document all five hardware entries, Orion dimensions, unsupported rationale, exact-path safety, generator invocation, provenance hashes, and hardware validation requirements. Keep current saved-state profile documentation distinct.

- [ ] **Step 4: Run formatter, diagnostics, tests, and drift validation**

Run: `cargo fmt --all -- --check && cargo test --workspace && python3 -m unittest discover -s tools -p 'test_*.py' -v && python3 tools/generate_device_catalog.py --check /home/ryodeushii/repos/Antelope-Ctl/profiles --generated src/device/generated.rs`
Expected: all commands pass.

- [ ] **Step 5: Review final diff and diagnostics**

Run: `git diff --check`; `lens_diagnostics` with `mode=all` for edited Rust/Python files; inspect `git diff --stat` and changed files. Record any unavailable hardware checks without claiming hardware validation.

**Completion criterion:** Documentation, generator drift checks, golden tests, workspace tests, formatting, and diagnostics pass; unsupported profiles remain visible but disabled.
