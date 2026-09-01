# Multi-device Antelope support implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `zen-go-tui` control validated Antelope profiles through one generic driver, with built-in generated data and optional normalized JSON profile packs.

**Architecture:** Antelope-Ctl remains the canonical source. The Python normalizer emits the checked-in Rust catalog and a versioned normalized JSON pack. Both loaders produce the same owned typed profile model. `ProfileDriver` consumes that model and hides frame encoding, readback bounds, and decoding behind normalized actions and events. Dynamic application state and UI consume profile addresses and capabilities rather than Zen Go-sized arrays.

**Tech Stack:** Rust 2021, Cargo workspace, `serde`, `serde_json`, `thiserror`, `hidapi` 2.6, Python standard library, `ratatui`, existing transport and mock-test harness.

**Spec:** `docs/superpowers/specs/2026-08-31-multi-device-antelope.md`

## Global constraints

- Antelope-Ctl `profiles/*.json` is canonical. `mic_models.json` is not hardware and is excluded.
- The runtime loader accepts normalized profile-pack JSON. It does not reimplement canonical-profile normalization in Rust.
- Ordinary Cargo builds must not require an Antelope-Ctl checkout.
- Built-in generated data and provenance stay checked in.
- Profile data is parsed once. HID command and decode paths use typed in-memory records.
- The driver never evaluates formula text, executes profile code, guesses offsets, or queries an index without a confirmed bound.
- Every valid hardware profile appears in catalog data. Profile existence does not imply control support.
- Unsupported, unverified, ambiguous, malformed, or unconfirmed profiles cannot open a controlling driver or cause a write.
- `src/profile.rs` remains saved-state TOML. It must not become a hardware protocol definition.
- Zen Go frame bytes and existing Zen Go behavior must remain unchanged.
- Orion support uses `ProfileDriver` unless a confirmed frame cannot fit the typed operation set.
- No hardware validation claim may come from fixture, simulator, or unit-test results.
- Do not create commits unless the user requests them. When commits are requested, use one focused commit per completed task.

## Completed groundwork

The current branch already contains these completed tasks. Keep them green while implementing remaining tasks.

- [x] Profile generator and checked-in catalog in `src/device/generated.rs`.
- [x] HID discovery, candidate classification, exact-path opening, reconnect identity checks, and threaded transport error handling.
- [x] `DeviceDriver`, normalized event storage, `ZenGoDriver`, controller driver injection, and explicit Zen Go construction at all current runtime callers.

---

### Task 1: Add normalized JSON profile packs and owned profile loading

**Files:**
- Modify: `antelope-protocol/Cargo.toml`
- Create: `antelope-protocol/src/profile.rs`
- Create: `antelope-protocol/tests/profile_pack.rs`
- Modify: `antelope-protocol/src/lib.rs`
- Modify: `tools/generate_device_catalog.py`
- Modify: `tools/test_generate_device_catalog.py`
- Create: `src/device/profile.rs`
- Modify: `src/device/mod.rs`
- Modify: `src/lib.rs`
- Create: `src/device/generated_profiles.json`
- Create: `antelope-protocol/tests/fixtures/profile_pack_v1.json`

**Interfaces:**
- `antelope-protocol/src/profile.rs` owns `ProfilePack`, `RuntimeProfile`, `RuntimeEntry`, `ReadbackDefinition`, typed operation records, and `ProfileLoadError`.
- `src/device/profile.rs` owns `ProfileCatalog`, built-in catalog conversion, and external-pack merging.
- The generator adds `--pack-output PATH` and `--pack-generated PATH` while preserving `--output PATH` and `--generated PATH`.
- `load_profile_pack(bytes: &[u8]) -> Result<ProfilePack, ProfileLoadError>` parses one normalized pack.
- `load_profile_pack_file(path: &Path) -> Result<ProfilePack, ProfileLoadError>` reads and parses one pack.
- `ProfileCatalog { entries: Vec<RuntimeEntry> }` owns the merged runtime entries.
- `ProfileCatalog::builtin() -> Self` converts `DEVICE_CATALOG` into owned `RuntimeEntry` values without filesystem access.
- `ProfileCatalog::add_external(&mut self, pack: ProfilePack) -> Result<(), ProfileLoadError>` rejects duplicate profile IDs and duplicate VID/PID identities.
- `ProfileCatalog::entries(&self) -> &[RuntimeEntry]` returns deterministic catalog order.
- `ProfileCatalog::find(&self, vid: u16, pid: u16) -> Option<&RuntimeEntry>` finds one identity without opening HID.
- `ProfilePack::validate(pack: ProfilePack) -> Result<ProfilePack, ProfileLoadError>` validates one parsed pack and returns it unchanged when valid.
- `ProfilePack::schema_version(&self) -> u16`, `ProfilePack::profiles(&self) -> &[RuntimeEntry]`, and `RuntimeEntry::profile(&self) -> &RuntimeProfile` expose read-only test and driver access.
- `RuntimeProfile::identity(&self) -> &RuntimeIdentity`, `RuntimeProfile::inputs_in(&self, space: &str) -> usize`, `RuntimeProfile::outputs(&self) -> &[RuntimeOutput]`, and `RuntimeProfile::mixers(&self) -> &[RuntimeMixer]` expose stable capability queries.
- `RuntimeReadiness` is protocol-owned with `Supported`, `Partial`, `Unverified`, and `Disabled` variants. `src/device/profile.rs` converts it to the catalog-facing `Readiness` enum.

The Rust test module defines `fixture_pack() -> ProfilePack`, `valid_pack_with_two_same_id() -> ProfilePack`, and `pack_with_readback_index_outside_count() -> ProfilePack`. Each helper starts from `fixtures/profile_pack_v1.json`, clones the owned records, changes only the named field, and returns the candidate without calling validation.

- [ ] **Step 1: Write failing loader and generator tests**

Add Python tests for a normalized pack containing one supported profile and one disabled profile. Assert the pack has `schema_version`, `generator_version`, `profiles`, stable profile IDs, `startup_queries`, `readback`, and typed frame operations. Assert canonical Orion normalization produces physical, ADAT, S/PDIF, output, mixer, routing, parameter, and hazard sections.

Add Rust tests with concrete cases:

```rust
#[test]
fn loads_version_one_profile_pack() {
    let pack = load_profile_pack(include_bytes!("fixtures/profile_pack_v1.json"))
        .expect("fixture must load");
    assert_eq!(pack.schema_version(), 1);
    assert_eq!(pack.profiles().len(), 1);
    assert_eq!(pack.profiles()[0].profile().identity().pid, 0xa221);
}

#[test]
fn rejects_unknown_schema_version() {
    let error = load_profile_pack(br#"{"schema_version":99,"profiles":[]}"#)
        .expect_err("unknown schema must fail");
    assert!(matches!(error, ProfileLoadError::UnsupportedSchemaVersion { .. }));
}

#[test]
fn rejects_duplicate_identity_and_profile_id() {
    let pack = valid_pack_with_two_same_id();
    let error = ProfilePack::validate(pack).expect_err("duplicate identity must fail");
    assert!(matches!(error, ProfileLoadError::DuplicateProfileId { .. }));
}

#[test]
fn rejects_unsafe_readback_bounds_and_unconfirmed_commands() {
    let pack = pack_with_readback_index_outside_count();
    let error = ProfilePack::validate(pack).expect_err("unsafe query must fail");
    assert!(matches!(error, ProfileLoadError::InvalidReadbackBounds { .. }));
}
```

Use a checked-in fixture with explicit fields. Do not construct the fixture through a permissive `serde_json::Value` path in the test.

- [ ] **Step 2: Run tests and verify they fail**

Run:

```bash
python3 -m unittest tools/test_generate_device_catalog.py -v
cargo test -p antelope-protocol profile_pack
```

Expected result: Python tests fail because pack rendering is absent. Rust tests fail because `ProfilePack`, `ProfileLoadError`, and the loader do not exist.

- [ ] **Step 3: Define owned typed profile records**

Add `serde` and `serde_json` to `antelope-protocol/Cargo.toml`. Define owned records with `String` and `Vec` fields. Keep field names stable with `#[serde(rename = "schema_version")]` where needed. Include these records:

```rust
pub const PROFILE_PACK_SCHEMA_VERSION: u16 = 1;

pub struct ProfilePack {
    schema_version: u16,
    generator_version: String,
    profiles: Vec<RuntimeEntry>,
}

pub struct RuntimeEntry {
    pub id: String,
    pub profile: RuntimeProfile,
    pub readiness: RuntimeReadiness,
    pub support_reason: String,
}

pub struct RuntimeProfile {
    pub identity: RuntimeIdentity,
    pub transport: RuntimeTransport,
    pub address_spaces: Vec<RuntimeAddressSpace>,
    pub inputs: Vec<RuntimeInput>,
    pub outputs: Vec<RuntimeOutput>,
    pub mixers: Vec<RuntimeMixer>,
    pub frames: Vec<RuntimeFrame>,
    pub decoders: Vec<RuntimeDecoder>,
    pub params: Vec<RuntimeParam>,
    pub constraints: Vec<RuntimeConstraint>,
    pub hazards: Vec<RuntimeHazard>,
    pub startup_queries: Vec<QueryRequest>,
    pub readback: ReadbackDefinition,
    pub provenance: RuntimeProvenance,
}

pub struct ReadbackDefinition {
    pub request_magic: u8,
    pub request_subcommand: u32,
    pub response_magic: u8,
    pub response_discriminator_offset: u16,
    pub response_discriminator: u8,
    pub category_offset: u16,
    pub index_offset: u16,
    pub data_offset: u16,
    pub category_counts: Vec<ReadbackCategory>,
}

pub struct ReadbackCategory {
    pub category: u8,
    pub count: u16,
}

pub struct RuntimeIdentity {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub bcd_device: Option<String>,
    pub status: String,
    pub status_text: String,
    pub notes: String,
    pub evidence: String,
}

pub struct RuntimeTransport {
    pub kind: String,
    pub report_size: Option<u16>,
    pub out_endpoint: Option<u8>,
    pub in_endpoint: Option<u8>,
    pub poll_interval_ms: Option<u16>,
    pub uses_numbered_reports: Option<bool>,
    pub expected_interface_number: Option<i32>,
    pub expected_usage_page: Option<u16>,
    pub expected_usage: Option<u16>,
    pub status: String,
    pub status_text: String,
    pub notes: String,
    pub evidence: String,
}

pub struct RuntimeAddressSpace { pub id: String, pub kind: String, pub count: Option<u16>, pub addressing: String, pub status: String, pub metadata: String }
pub struct RuntimeInput { pub id: String, pub space: String, pub index: u16, pub name: String, pub hiz_capable: bool, pub status: String, pub metadata: String }
pub struct RuntimeOutput { pub id: u16, pub name: String, pub aliases: Vec<String>, pub verified: bool, pub status: String, pub metadata: String }
pub struct RuntimeMixer { pub id: String, pub name: String, pub mix_index: u8, pub strip_count: u16, pub fader_range: Option<(i32, i32)>, pub pan_range: Option<(i32, i32)>, pub pan_center: Option<i32>, pub send_range: Option<(i32, i32)>, pub status: String, pub metadata: String }
pub struct RuntimeFrame { pub id: String, pub kind: String, pub status: String, pub report_size: u16, pub operations: Vec<FrameOperation>, pub metadata: String }
pub struct RuntimeDecoder { pub id: String, pub frame_id: String, pub kind: String, pub status: String, pub metadata: String }
pub struct RuntimeParam { pub name: String, pub id: Option<u16>, pub value_type: String, pub status: String, pub range: Option<(i32, i32)>, pub values: Vec<(i32, String)>, pub frame: ParamReference, pub readback: ParamReference, pub metadata: String }
pub struct RuntimeConstraint { pub name: String, pub status: String, pub range: Option<(i32, i32)>, pub values: Vec<i32>, pub metadata: String }
pub struct RuntimeHazard { pub name: String, pub status: String, pub rule: String, pub effect: String, pub opcodes: Vec<u8>, pub metadata: String }
pub struct RuntimeProvenance { pub source_path: String, pub source_sha256: String, pub generator_version: String }
pub struct ParamReference { pub text: String, pub formula: String, pub offsets: Vec<(String, u16)> }

pub enum FrameOperation {
    FixedByte { offset: u16, value: u8 },
    Scalar { offset: u16, width: u8 },
    Indexed { base: u16, stride: u16, index_field: String, width: u8 },
    BitField { offset: u16, mask: u8, shift: u8 },
    PairIndex { base: u16, stride: u16, pair_field: String },
    AllowedValues { values: Vec<i32> },
}

pub enum RuntimeReadiness { Supported, Partial, Unverified, Disabled }
```

`RuntimeFrame` must own typed field trees. Represent fixed bytes, scalar offsets, indexed offsets, strides, masks, allowed values, and pair-index operations as enum variants. Store formula text only as display metadata. Return `ProfileLoadError::UncompiledFormula` when a command depends on a formula the generator did not compile.

- [ ] **Step 4: Implement strict pack validation**

Implement `ProfilePack::validate` and call it from both public loader functions. Reject unknown schema versions, empty IDs, duplicate IDs, duplicate VID/PID identities, unsupported transport kinds, missing report sizes for selectable entries, offsets outside report size, invalid masks and shifts, duplicate parameter IDs, invalid enum values, missing startup query layout, category counts below referenced indexes, and command operations whose status is not `Confirmed`.

Map every validation failure to a named `ProfileLoadError` variant. Include profile ID and field path in error text. Do not turn absent data into zero, empty, or Zen Go defaults.

- [ ] **Step 5: Emit and check normalized packs**

Extend `normalize_profile` so existing builders produce one serializable normalized record. Add `render_profile_pack` and write it with sorted profile IDs and stable compact JSON. Update the CLI so this command writes both artifacts:

```bash
python3 tools/generate_device_catalog.py \
  --profiles-dir /home/ryodeushii/repos/Antelope-Ctl/profiles \
  --output src/device/generated.rs \
  --pack-output src/device/generated_profiles.json
```

Add `--pack-generated` to `--check` and compare exact bytes after the generator normalizes line endings. Add a Python test that edits one source profile byte and asserts the Rust and JSON checks fail with the source path and SHA-256 mismatch.

- [ ] **Step 6: Convert built-in static records and merge external packs**

Implement `src/device/profile.rs`. Convert every borrowed generated record into the owned protocol record. Preserve static readiness and provenance. Load `src/device/generated_profiles.json` only in tests and explicit external-pack mode. `ProfileCatalog::add_external` must reject identity collisions instead of silently shadowing built-ins.

Sort catalog entries by readiness rank, product name, and profile ID. Do not access the filesystem from `ProfileCatalog::builtin`.

- [ ] **Step 7: Run loader and generator tests**

Run:

```bash
python3 -m unittest tools/test_generate_device_catalog.py -v
cargo test -p antelope-protocol profile_pack
cargo test -p zen-go-tui device::
```

Expected result: all new tests and existing catalog/discovery tests pass.

---

### Task 2: Generalize protocol actions/events and implement `ProfileDriver`

**Files:**
- Modify: `antelope-protocol/src/driver.rs`
- Create: `antelope-protocol/src/profile_driver.rs`
- Create: `antelope-protocol/src/profile_codec.rs`
- Modify: `antelope-protocol/src/zen_go.rs`
- Modify: `antelope-protocol/src/lib.rs`
- Create: `antelope-protocol/tests/profile_driver.rs`
- Create: `antelope-protocol/tests/fixtures/orion/profile_driver_pack.json`
- Create: `antelope-protocol/tests/fixtures/orion/startup_requests.txt`
- Create: `antelope-protocol/tests/fixtures/orion/state_report_73.hex`
- Create: `antelope-protocol/tests/fixtures/orion/readback_75.hex`
- Modify: `src/app/controller.rs`
- Modify: `src/command_queue.rs`
- Modify: `src/app/types.rs`
- Modify: `src/app/state.rs`

**Interfaces:**

Use numeric profile addresses so actions do not encode Zen Go dimensions:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InputAddress { pub space: u16, pub index: u16 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputAddress { pub id: u16 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MixerAddress { pub surface: u8, pub strip: u16 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutingSource { pub bank: u8, pub index: u16 }

pub enum ControlValue { Bool(bool), Int(i32), Enum(i32) }
pub enum InputControl { Mode, Gain, Phantom, Phase, Parameter(u16) }
pub enum OutputControl { Level, Mute, Dim, Parameter(u16) }
pub enum MixerControl { Fader, Pan, Send, Mute, Solo, Parameter(u16) }
pub enum GlobalControl { SampleRate, ClockSource, Surface, Parameter(u16) }

pub enum Action {
    SetInput { address: InputAddress, control: InputControl, value: ControlValue },
    SetOutput { address: OutputAddress, control: OutputControl, value: ControlValue },
    SetMixer { address: MixerAddress, control: MixerControl, value: ControlValue },
    SetLink { surface: u8, pair: u16, enabled: bool },
    SetRouting { destination: u16, channel: u16, source: RoutingSource },
    SetGlobal { control: GlobalControl, value: ControlValue },
    Query(QueryRequest),
}
```

Replace the Zen Go-sized event state with owned dynamic records:

```rust
pub struct DynamicDeviceState {
    pub globals: Vec<DynamicGlobalState>,
    pub inputs: Vec<DynamicInputState>,
    pub outputs: Vec<DynamicOutputState>,
    pub mixers: Vec<DynamicMixerSurface>,
    pub routing: Vec<DynamicRoutingGroup>,
}

pub struct DynamicMixerSurface {
    pub surface: u8,
    pub name: String,
    pub master: Option<DynamicMixerStrip>,
    pub strips: Vec<DynamicMixerStrip>,
}

pub enum DynamicStatePatch {
    Inputs(Vec<DynamicInputState>),
    Outputs(Vec<DynamicOutputState>),
    Mixer(DynamicMixerSurface),
    Routing(DynamicRoutingGroup),
    Globals(Vec<DynamicGlobalState>),
}
```

`DeviceEvent` keeps `Snapshot`, `QueryReply`, `Auxiliary`, and `Notification`, but each variant carries owned raw bytes. `QueryReply` also carries `patch: Option<DynamicStatePatch>`. `DriverDefinition` owns `String` identity fields. `DeviceDriver` keeps four methods: `definition`, `startup_requests`, `encode`, and `decode`.

`ProfileDriver` has this interface:

```rust
use std::collections::HashMap;

pub struct ProfileDriver {
    definition: DriverDefinition,
    profile: RuntimeProfile,
    startup_requests: Vec<QueryRequest>,
    parameter_index: HashMap<(String, String), usize>,
    frame_index: HashMap<String, usize>,
}

impl ProfileDriver {
    pub fn new(entry: RuntimeEntry) -> Result<Self, DriverError>;
}
```

`ProfileDriver::new` rejects every entry readiness other than `RuntimeReadiness::Supported`, validates all required typed operations, builds parameter/frame indexes, and copies startup requests. `ProfileDriver::encode` returns `CommandBatch` and `ProfileDriver::decode` distinguishes 0x73 state reports, 0x75 readback responses, 0x75 meter reports, and unknown frames using profile-defined magic and discriminator values. Readback decode fills `QueryReply.patch` for confirmed mixer and routing records, while preserving `body` and `raw` for every response.

The test module defines these fixture helpers before the tests:

```rust
fn fixture_entry() -> RuntimeEntry;
fn disabled_orion_entry() -> RuntimeEntry;
fn profile_driver_from_fixture() -> ProfileDriver;
fn decode_orion_mixer_record() -> DynamicMixerSurface;
```

`fixture_entry` loads `fixtures/orion/profile_driver_pack.json`. `disabled_orion_entry` clones that entry and changes only its readiness to `RuntimeReadiness::Disabled`. `profile_driver_from_fixture` calls `ProfileDriver::new(fixture_entry())`. `decode_orion_mixer_record` calls `profile_driver_from_fixture().decode` with the bytes from `fixtures/orion/readback_75.hex`, matches `DeviceEvent::QueryReply { patch: Some(DynamicStatePatch::Mixer(surface)), .. }`, and returns that surface.

- [ ] **Step 1: Write failing normalized contract tests**

Add tests for numeric addresses, dynamic master strips, unsupported controls, and profile-driver construction:

```rust
#[test]
fn profile_driver_rejects_disabled_profile_before_encoding() {
    let entry = disabled_orion_entry();
    let error = ProfileDriver::new(entry).expect_err("disabled profile must not load");
    assert!(matches!(error, DriverError::UnsupportedAction(_)));
}

#[test]
fn profile_driver_rejects_unconfirmed_parameter() {
    let driver = profile_driver_from_fixture();
    let error = driver
        .encode(Action::SetOutput {
            address: OutputAddress { id: 5 },
            control: OutputControl::Parameter(99),
            value: ControlValue::Int(1),
        })
        .expect_err("unconfirmed parameter must be rejected");
    assert!(matches!(error, DriverError::UnsupportedAction(_)));
}

#[test]
fn dynamic_mixer_state_keeps_master_outside_input_strip_vector() {
    let state = decode_orion_mixer_record();
    assert!(state.master.is_some());
    assert_eq!(state.strips.len(), 32);
}
```

Update Zen Go tests to assert byte-for-byte output through normalized actions. Keep a small private conversion adapter for existing unit fixtures until controller migration finishes.

- [ ] **Step 2: Run contract tests and verify they fail**

Run:

```bash
cargo test -p antelope-protocol profile_driver
cargo test -p zen-go-tui controller_driver
```

Expected result: compilation fails because numeric actions, dynamic event records, `ProfileDriver`, and owned `DriverDefinition` do not exist.

- [ ] **Step 3: Implement typed action/event records**

Add address, control, and value types to `driver.rs`. Keep `Action` free of `Vec`, `String`, fixed arrays, and device-specific enums in its normal control variants. Use `QueryRequest` for category/index queries because its two `u8` fields map to Orion category and index after profile validation.

Add dynamic state records with vectors and optional master state. Preserve raw report bytes on every event. Update `ZenGoDriver::state_from_snapshot` to populate dynamic input, output, mixer, and routing records without changing decoded values.

- [ ] **Step 4: Implement generic frame codec**

Implement `profile_codec.rs` with pure functions that receive a validated `RuntimeFrame`, target address, and `ControlValue`. The codec must:

1. allocate exactly `transport.report_size` bytes;
2. write profile-defined magic and opcode;
3. resolve only typed fixed, indexed, stride, bit-mask, pair-index, and allowed-value operations;
4. validate channel, output, mixer, strip, destination, category, and index bounds;
5. validate enum membership and numeric ranges;
6. reject unconfirmed fields and unknown operations;
7. return `DriverError::InvalidAction` with profile field context.

Implement readback query encoding as a 0x74 request with profile-defined magic, subcommand, category offset, and index offset. Reject every category/index pair outside `ReadbackDefinition.category_counts`.

- [ ] **Step 5: Implement `ProfileDriver`**

Construct lookup tables once in `ProfileDriver::new`. Derive startup requests from `RuntimeProfile.startup_queries`, which the generator creates from the profile’s explicit readback category counts and emission order. Do not query categories without confirmed counts.

Implement profile-driven encode paths for input, output, mixer, link, routing, global, and query actions. Implement decode paths for state, meter, and readback frames. Return `Ok(None)` only for explicitly ignored frame kinds. Return protocol errors for malformed report length, wrong magic, invalid discriminator, and truncated fields.

- [ ] **Step 6: Adapt Zen Go and controller callers**

Translate existing controller methods to numeric addresses and semantic controls. Move Zen Go assignment-table construction into `ZenGoDriver` while the controller emits a normalized `Action::SetRouting` sequence. Keep routing batches immediate because they can contain multiple frames. Keep single-frame fader changes coalescible in `CommandQueue`.

Update queue coalescing keys to use `InputAddress`, `OutputAddress`, and `MixerAddress`. `CommandQueue::flush` must call the injected driver for every action and recursively dispatch refresh requests through the same path.

Propagate driver encoding and decoding errors from `Controller::write_query`, `write_batch`, `send`, `flush_commands`, and `poll_device`. Do not silently discard a `DriverError`.

- [ ] **Step 7: Add Orion profile-derived fixtures and driver tests**

Build fixture bytes from the normalized Orion profile operations. Store one startup request sequence, one 0x73 state report, and one 0x75 response in the named fixture paths. Tests must assert:

- profile identity is VID `0x23e5`, PID `0xa221`;
- startup requests contain 0x74, subcommand 0x10, category at byte 8, and index at byte 12;
- category 0x04 indexes 0 through 3 encode, while index 4 is rejected;
- a confirmed physical input control writes its profile-defined parameter ID and value;
- output bus IDs 0, 1, 2, 3, 4, and 5 use profile offsets, not Zen Go output enums;
- a mixer frame can encode master strip 0 and input strip 32;
- routing frames preserve every source pair in a destination group;
- 0x75 discriminator 0 decodes as readback and discriminator 0x1f decodes as meter data;
- decoded state contains 12 physical, 16 ADAT, and 2 S/PDIF inputs, six outputs, four mixers, one master, and 32 input strips per mixer.

- [ ] **Step 8: Run protocol and controller tests**

Run:

```bash
cargo fmt --all -- --check
cargo test -p antelope-protocol profile_driver
cargo test -p zen-go-tui controller_driver command_queue
```

Expected result: Zen Go contract tests and new profile-driver tests pass. Orion remains disabled in catalog until Task 5 enables its readiness after these tests are present.

---

### Task 3: Replace fixed application state with dynamic profile state

**Files:**
- Modify: `src/app/state.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/app/types.rs`
- Modify: `src/app/controller.rs`
- Modify: `src/command_queue.rs`
- Modify: `src/profile_ops.rs`
- Create: `src/app/dynamic_state_tests.rs`

**Interfaces:**
- `AppState::from_profile(profile: &RuntimeProfile) -> Self` allocates profile-defined collections.
- `AppState::apply_dynamic_state(&mut self, state: DynamicDeviceState, raw: Vec<u8>) -> bool` updates visible state and raw packet views.
- `AppState::observe_event(&mut self, event: DeviceEvent) -> bool` consumes normalized events without converting through `DeviceSnapshot`.
- `AppState::active_mixer_surface(&self) -> Option<usize>` returns a safe surface index.
- `AppState::visible_mixer_strip_bounds(&self) -> Range<usize>` clamps viewport state to the active surface.
- Saved profile code continues to read/write `src/profile.rs` types and maps only controls exposed by the selected runtime profile.

The test module defines `orion_profile() -> RuntimeProfile`, `zen_go_profile() -> RuntimeProfile`, `discrete_4_entry() -> RuntimeEntry`, `controller_for_profile(entry: RuntimeEntry) -> Controller`, and `unsupported_input_action() -> Action`. The profile helpers load `fixtures/orion/profile_driver_pack.json` or the checked-in Zen Go pack, clone the owned entry, and return the requested profile. `controller_for_profile` constructs `ProfileDriver::new(entry)` and injects it into `Controller::new` with `MockTransport`.

- [ ] **Step 1: Write failing dynamic-state tests**

Add tests with a validated Orion profile and a Zen Go profile:

```rust
#[test]
fn orion_state_allocates_full_io_and_mixer_geometry() {
    let state = AppState::from_profile(&orion_profile());
    assert_eq!(state.inputs_for_space("physical_inputs").len(), 12);
    assert_eq!(state.inputs_for_space("adat_inputs").len(), 16);
    assert_eq!(state.inputs_for_space("spdif_inputs").len(), 2);
    assert_eq!(state.outputs().len(), 6);
    assert_eq!(state.mixers().len(), 4);
    assert!(state.mixers().iter().all(|mixer| mixer.strips.len() == 32));
}

#[test]
fn dynamic_selection_clamps_after_profile_refresh() {
    let mut state = AppState::from_profile(&orion_profile());
    state.select_mixer_strip(31);
    state.reconfigure_for_profile(&zen_go_profile());
    assert!(state.selected_mixer_strip() < 16);
}

#[test]
fn missing_capability_returns_no_action() {
    let mut controller = controller_for_profile(discrete_4_entry());
    let error = controller.send(unsupported_input_action(), None)
        .expect_err("unconfirmed control must not write");
    assert!(error.to_string().contains("unsupported"));
}
```

Add an event test that applies a dynamic Orion snapshot and asserts raw bytes remain available without constructing a fixed `[u8; 320]` snapshot.

- [ ] **Step 2: Run dynamic-state tests and verify they fail**

Run:

```bash
cargo test -p zen-go-tui dynamic_state
```

Expected result: tests fail because `MixerState`, `OutputData`, `PreampData`, and `AppState` still assume two surfaces, three outputs, two preamps, and 16 strips.

- [ ] **Step 3: Add profile-owned application collections**

Replace fixed application collections with vectors keyed by profile address. Keep selection, viewport, peak, and pending-mutation indexes separate from collection storage. Use `u16` profile indexes and convert to `usize` only after bounds checks.

Store input spaces separately so physical, ADAT, S/PDIF, and future spaces cannot collide on display index. Store each mixer surface with an optional master and dynamic input strips. Store routing destination groups and source pairs as vectors.

Keep `DeviceStatus` metadata, connection, raw packet buffers, and saved settings. Remove the old `legacy_snapshot_from_dynamic` path from normal event handling. Retain `observe_frame` only for existing test fixtures until UI migration is complete, and mark it private.

- [ ] **Step 4: Migrate intents, pending mutations, controller, and queue**

Change `Intent` and `PendingMutation` to carry numeric profile addresses. Convert output, input, mixer, link, routing, and global actions at the controller seam. Guard every selection and mutation with `get` or `checked_sub` before indexing.

Update profile application to iterate profile-defined controls. Skip controls with unavailable status and report the first skipped control. Do not synthesize a second output or preamp for profiles with different geometry.

- [ ] **Step 5: Preserve Zen Go behavior through the dynamic model**

Map Zen Go’s two mixer surfaces, 16 strips, three outputs, and two preamps into the new vectors. Keep existing labels and raw packet behavior. Compare pre-refresh and post-refresh structural state using normalized values, treating missing pan as equivalent to center only in the compatibility comparison.

- [ ] **Step 6: Run state and controller tests**

Run:

```bash
cargo test -p zen-go-tui dynamic_state app::tests:: controller_driver command_queue
```

Expected result: Orion dimension tests and existing Zen Go application tests pass without out-of-bounds panics or writes from unavailable capabilities.

---

### Task 4: Render dynamic controls, mixer pages, and routing

**Files:**
- Modify: `src/ui/layouts.rs`
- Modify: `src/ui/render/mod.rs`
- Modify: `src/ui/render/text.rs`
- Modify: `src/ui/widgets/mixer.rs`
- Modify: `src/ui/widgets/signals.rs`
- Modify: `src/ui/raw_map.rs`
- Modify: `src/ui/mouse.rs`
- Modify: `src/ui/tests.rs`
- Create: `src/ui/dynamic_tests.rs`

**Interfaces:**
- Layout functions accept `&AppState` where collection length or capability affects geometry.
- Mixer viewport uses `MIXER_STRIP_PAGE_SIZE` and returns checked visible bounds.
- Renderer gets labels from `RuntimeProfile` input, output, mixer, and routing definitions.
- Unsupported controls render disabled text and never create `Intent` values.

The UI test module defines `orion_ui_state() -> AppState`, `discrete_4_ui_state() -> AppState`, `test_terminal(width: u16, height: u16) -> Terminal<TestBackend>`, `draw_page(terminal: &mut Terminal<TestBackend>, state: &AppState)`, `terminal_text(terminal: &Terminal<TestBackend>) -> String`, `render_orion_screen() -> String`, `available_intents(state: &AppState) -> Vec<Intent>`, and `render_to_string(state: &AppState) -> String`. These helpers construct state from the same normalized fixtures used by Task 3 and use `ratatui::backend::TestBackend`; they do not open HID.

- [ ] **Step 1: Write failing UI tests**

Add tests that render a profile-backed `AppState` with a `TestBackend`. Assert text contains dynamic device and control names and that every page can render without panic:

```rust
#[test]
fn orion_mixer_pages_render_all_thirty_two_input_strips() {
    let state = orion_ui_state();
    for page in 0..4 {
        let mut terminal = test_terminal(140, 48);
        draw_page(&mut terminal, state.with_mixer_page(page));
        let text = terminal_text(&terminal);
        assert!(text.contains(&format!("CH {:02}", page * 8 + 1)));
        assert!(text.contains(&format!("CH {:02}", page * 8 + 8)));
    }
}

#[test]
fn orion_header_and_output_panel_use_profile_names() {
    let text = render_orion_screen();
    assert!(text.contains("Orion Studio III"));
    assert!(text.contains("monitor_2"));
    assert!(text.contains("Input 12"));
}

#[test]
fn unavailable_controls_are_visible_but_not_actionable() {
    let state = discrete_4_ui_state();
    assert!(render_to_string(&state).contains("unverified"));
    assert!(available_intents(&state).iter().all(|intent| !intent.writes_hardware()));
}
```

Add routing tests that render all profile-defined destinations and verify source pairs use bounds-checked rows. Keep existing raw-view tests unchanged except for `Vec<u8>` storage.

- [ ] **Step 2: Run UI tests and verify they fail**

Run:

```bash
cargo test -p zen-go-tui dynamic ui::
```

Expected result: tests fail because layouts, renderers, mouse hit areas, and mixer widgets use fixed output, preamp, surface, and strip counts.

- [ ] **Step 3: Make layouts profile-aware**

Update `device_panel_layout`, `mixer_strip_visible_bounds`, `mixer_header_button_rects`, preamp layouts, output card layouts, and routing layouts to use state collection lengths. Preserve current geometry for Zen Go. Clamp selected indexes before calculating rectangles.

Keep four Orion mixer pages accessible through existing left/right paging. Render master strip separately from input strip pages. Use the profile’s strip names and mix names when available, then fall back to `CH NN` only for confirmed unnamed channels.

- [ ] **Step 4: Make renderers capability-aware**

Update title, status, output, preamp, mixer, and routing render paths. Show device name, serial, readiness, and support reason. Render all six Orion output buses through scrolling or paging. Render 12 physical, 16 ADAT, and 2 S/PDIF inputs in their profile-defined sections. Do not label an unverified output as writable.

Move repeated labels and numeric formatting into `src/ui/render/text.rs`. Do not add Zen Go-specific constants to dynamic paths.

- [ ] **Step 5: Make mouse and keyboard paths dynamic**

Calculate hit areas from the same visible bounds used by rendering. Reject clicks outside vector length. Resolve output, input, mixer, and routing actions through profile addresses. Hide or disable controls when the selected profile marks them unavailable.

- [ ] **Step 6: Run UI tests and existing snapshots**

Run:

```bash
cargo fmt --all -- --check
cargo test -p zen-go-tui ui:: dynamic
cargo test -p zen-go-tui app::tests::
```

Expected result: dynamic Orion rendering tests pass and Zen Go interaction tests remain green.

---

### Task 5: Enable Orion through generic profile-driver validation

**Files:**
- Modify: `tools/generate_device_catalog.py`
- Modify: `tools/test_generate_device_catalog.py`
- Modify: `src/device/generated.rs`
- Modify: `src/device/generated_profiles.json`
- Modify: `antelope-protocol/tests/profile_driver.rs`
- Modify: `antelope-protocol/tests/fixtures/orion/profile_driver_pack.json`
- Modify: `antelope-protocol/tests/fixtures/orion/startup_requests.txt`
- Modify: `antelope-protocol/tests/fixtures/orion/state_report_73.hex`
- Modify: `antelope-protocol/tests/fixtures/orion/readback_75.hex`
- Modify: `src/device/profile.rs`

**Interfaces:**
- Orion uses `ProfileDriver`. Do not create `OrionDriver` unless a confirmed operation cannot be represented by `RuntimeFrame` and its typed operations.
- `ProfileCatalog::find(0x23e5, 0xa221)` returns a selectable entry only after pack validation and profile-driver capability validation pass.
- `ProfileDriver::new(orion_entry)` must validate the complete required operation set before returning `Ok`.

The Rust test module defines `orion_entry() -> RuntimeEntry` by loading the normalized Orion fixture and setting its readiness to `RuntimeReadiness::Supported`, `required_orion_actions() -> Vec<Action>` with one action per confirmed Orion control family, and `fixture_with_unconfirmed_orion_command() -> NormalizedProfile` in the Python test module.

- [ ] **Step 1: Write the failing Orion enablement tests**

Add tests for every required profile area:

```rust
#[test]
fn orion_profile_covers_required_geometry() {
    let profile = orion_profile();
    assert_eq!(profile.inputs_in("physical_inputs"), 12);
    assert_eq!(profile.inputs_in("adat_inputs"), 16);
    assert_eq!(profile.inputs_in("spdif_inputs"), 2);
    assert_eq!(profile.outputs().len(), 6);
    assert_eq!(profile.mixers().len(), 4);
    assert!(profile.mixers().iter().all(|mixer| mixer.strip_count == 32));
}

#[test]
fn orion_profile_driver_covers_required_controls() {
    let driver = ProfileDriver::new(orion_entry()).expect("Orion profile driver");
    for action in required_orion_actions() {
        driver.encode(action).expect("confirmed Orion action");
    }
}

#[test]
fn orion_profile_rejects_unbounded_category() {
    let driver = ProfileDriver::new(orion_entry()).expect("driver");
    let error = driver.encode(Action::Query(QueryRequest { query_id: 0x04, sub_id: 4 }))
        .expect_err("category 0x04 index 4 is unsafe");
    assert!(error.to_string().contains("readback"));
}

#[test]
fn generator_does_not_enable_orion_from_field_count_alone() {
    let profile = fixture_with_unconfirmed_orion_command();
    assert_ne!(classify_readiness(&profile), Readiness::Supported);
}
```

- [ ] **Step 2: Run Orion tests and verify they fail**

Run:

```bash
cargo test -p antelope-protocol profile_driver
python3 -m unittest tools/test_generate_device_catalog.py -v
```

Expected result: the required action coverage test fails for missing generic operations, and Orion readiness remains disabled.

- [ ] **Step 3: Add missing normalized operations from the Orion profile**

Use the canonical profile and existing generator builders to compile these confirmed operations:

- 0x74/0x10 category-index request layout;
- 0x75 response magic, discriminator, category, index, and data offsets;
- readback category counts and bounds;
- 0x73 gain, status, ADAT, S/PDIF, and bus offsets;
- mixer fader, pan/mute/solo, send, wet, space, and pair-index offsets;
- routing destination and source-pair layout;
- global and confirmed AuraVerb command layouts;
- physical input mode, gain, phantom, and phase parameters;
- output bus IDs 0, 1, 2, 3, 4, and 5.

For every operation, keep the source status and evidence metadata. If the profile contains a formula, compile only the known mixer pair-index formula into a `PairIndex` operation. Mark any other formula unavailable.

- [ ] **Step 4: Add exact fixture assertions**

Populate the Orion fixture files from profile-derived operation values and captured report layouts. Tests must compare full frame bytes for startup queries, representative input/output/mixer/link/routing/global writes, and full decoded address/value sets for state and readback reports. Include a test that the meter discriminator does not enter the readback decoder.

- [ ] **Step 5: Enable Orion readiness**

After all required tests pass, change the Orion readiness policy from `Disabled` to `Supported` in the generator policy and regenerate both `src/device/generated.rs` and `src/device/generated_profiles.json`. Keep Discrete 8 Pro `Partial`, Discrete 4 `Unverified`, and Discrete 4 Pro `Unverified`.

`ProfileCatalog` must still call `ProfileDriver::new` before selecting Orion. A malformed or incomplete external Orion pack remains disabled.

- [ ] **Step 6: Run Orion and workspace tests**

Run:

```bash
cargo fmt --all -- --check
cargo test -p antelope-protocol profile_driver
cargo test -p zen-go-tui dynamic_state ui::
cargo test --workspace
```

Expected result: Orion profile-driver and dynamic state/UI tests pass. No unsupported profile writes occur.

---

### Task 6: Add catalog-driven picker and safe runtime switching

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/runtime.rs`
- Modify: `src/app/picker.rs`
- Modify: `src/app/state.rs`
- Modify: `src/app/controller.rs`
- Modify: `src/transport.rs`
- Modify: `src/ui/layouts.rs`
- Modify: `src/ui/render/mod.rs`
- Modify: `src/ui/mouse.rs`
- Create: `src/device/session.rs`
- Create: `src/app/picker_tests.rs`

**Interfaces:**
- Add `--device PATH|VID:PID|SERIAL` to `Cli`.
- Add `--profile-pack PATH` to `Cli`.
- `DeviceSelection` stores exact path bytes, optional serial, or VID/PID criteria.
- `DeviceSession::discover(catalog: &ProfileCatalog) -> Result<Vec<DeviceCandidate>>` performs read-only enumeration.
- `DeviceSession::open(candidate: &DeviceCandidate, catalog: &ProfileCatalog) -> Result<Controller>` opens exact path, constructs the matching `ProfileDriver`, and rejects non-selectable candidates before any write.
- `DevicePickerState` stores candidates, selected index, diagnostics, and retry timing.
- `DevicePickerState::new(candidates: Vec<DeviceCandidate>, catalog: &ProfileCatalog) -> Self` classifies and sorts candidates.
- `DevicePickerState::entries(&self) -> &[PickerEntry]` returns display entries with candidate, status, and diagnostic.
- `DeviceSelection::parse(value: &str) -> Result<Self>` accepts `PATH`, `VID:PID`, or `SERIAL`.
- `DeviceSelection::match_candidate(&self, candidate: &DeviceCandidate) -> SelectionMatch` compares exact path bytes before fallback identity fields.
- `DeviceSession::open_candidate(candidate: &DeviceCandidate, catalog: &ProfileCatalog) -> Result<Controller>` performs classification, exact-path open, driver construction, and controller construction.
- `run_app` and `run_headless_app` receive a constructed controller or picker/session state, not a raw Zen Go transport.

The picker test module defines `orion_candidate()`, `disabled_orion_candidate()`, `candidate_with_path(path: &[u8])`, `candidate_with_serial(serial: &str)`, `picker_for_candidates(supported: DeviceCandidate, disabled: DeviceCandidate) -> DevicePickerState`, `open_selected_candidate(candidate: DeviceCandidate) -> Result<Controller>`, `select_candidate(candidates: &[DeviceCandidate], selection: &DeviceSelection) -> Result<&DeviceCandidate>`, `switch_sessions_for_test() -> (TestSession, TestSession)`, and `open_mock_session() -> Result<TestSession>`. `TestSession` exposes `write_after_close()`, `write_count()`, and `driver_definition()` through test-only methods backed by `MockTransport`. `SelectionMatch` has `ExactPath`, `Serial`, and `Identity` variants.

- [ ] **Step 1: Write failing picker and CLI tests**

Add tests for:

```rust
#[test]
fn picker_sorts_supported_candidates_before_disabled_candidates() {
    let picker = picker_for_candidates(orion_candidate(), disabled_candidate());
    assert_eq!(picker.candidates()[0].classification().status, CandidateStatus::Supported);
}

#[test]
fn picker_rejects_disabled_orion_candidate_before_open() {
    let error = open_selected_candidate(disabled_orion_candidate())
        .expect_err("disabled candidate must not open");
    assert!(error.to_string().contains("disabled"));
}

#[test]
fn cli_selects_candidate_by_exact_path() {
    let candidate = candidate_with_path(b"hidraw-orion");
    let selection = DeviceSelection::parse("hidraw-orion").expect("path selection");
    assert_eq!(selection.match_candidate(&candidate), SelectionMatch::ExactPath);
}

#[test]
fn cli_selects_candidate_by_serial_only_when_identity_is_unique() {
    let candidates = vec![candidate_with_serial("ORION-1"), candidate_with_serial("ORION-1")];
    let error = select_candidate(&candidates, &DeviceSelection::Serial("ORION-1".into()))
        .expect_err("duplicate serial must be ambiguous");
    assert!(error.to_string().contains("ambiguous"));
}

#[test]
fn switching_closes_old_worker_before_opening_new_candidate() {
    let (old_session, new_session) = switch_sessions_for_test();
    old_session.write_after_close().expect_err("old session must be closed");
    new_session.write_count().assert_eq(0);
}

#[test]
fn mock_mode_constructs_zen_go_driver_without_hid_discovery() {
    let session = open_mock_session().expect("mock session");
    assert_eq!(session.driver_definition().pid, 0xa015);
}
```

Use `MockTransport` and pure `DeviceCandidate` records. Do not mock HID internals by opening a real path.

- [ ] **Step 2: Run picker tests and verify they fail**

Run:

```bash
cargo test -p zen-go-tui picker runtime_device_selection
```

Expected result: tests fail because startup opens fixed Zen Go VID/PID before discovery and there is no picker/session interface.

- [ ] **Step 3: Load catalog and discover before opening HID**

Parse `--profile-pack` before HID discovery. Start with `ProfileCatalog::builtin`, then merge the external pack. Use `enumerate_antelope_devices` and candidate classification. Show supported candidates first and retain disabled diagnostics.

For `--device`, match exact path bytes first, then unique serial, then unique VID/PID. Reject multiple matches with an ambiguity error containing candidate paths. Do not call `HidTransport::open` for a disabled, unsupported, or ambiguous candidate.

For mock mode, bypass discovery and construct `ZenGoDriver` with `MockTransport`.

- [ ] **Step 4: Construct transport and driver as one session**

Add `DeviceSession::open`. Validate candidate classification, call exact-path `HidTransport::open_path`, construct `ProfileDriver` from the selected runtime profile, and then construct `Controller`. If driver construction fails, drop the transport before returning the error.

Replace `ZEN_GO_VID`, `ZEN_GO_PID`, and the fixed `HidTransport::open` retry closure in normal startup. Keep compatibility constants only in tests or an explicitly named mock path.

- [ ] **Step 5: Add picker lifecycle and reconnect switching**

When no selectable candidate exists, render a wait/picker state and retry read-only discovery. On disconnect, stop the old `ThreadedTransport`, drop its controller, rediscover, and construct a new session. Never mutate a transport under an active controller.

Preserve exact path bytes across reconnect. If path changes, require validated serial plus unique VID/PID/interface identity. Report ambiguity instead of selecting the first match.

- [ ] **Step 6: Render picker diagnostics and selection**

Add picker layout, keyboard, and mouse actions. Show product, serial, path, profile readiness, and diagnostic reason. Disable selection for `Partial`, `Unverified`, `Disabled`, `Ambiguous`, and `Unsupported` candidates. Show the selected device name in the main titlebar.

- [ ] **Step 7: Run runtime and picker tests**

Run:

```bash
cargo fmt --all -- --check
cargo test -p zen-go-tui picker runtime_device_selection transport::
cargo test --workspace
```

Expected result: startup selects Zen Go or Orion by catalog, disabled candidates never open, and reconnect does not write through an old worker.

---

### Task 7: Finalize profile drift checks, documentation, and release verification

**Files:**
- Modify: `README.md`
- Modify: `docs/zen-go-tui.md`
- Create: `docs/device-support.md`
- Modify: `tools/generate_device_catalog.py`
- Modify: `tools/test_generate_device_catalog.py`
- Modify: existing CI configuration only if a workflow already exists

**Interfaces:**
- Documentation names Antelope-Ctl profile source, normalized pack generation, built-in fallback, optional `--profile-pack`, `--device` selection, readiness rules, exact-path safety, and hardware validation procedure.
- Generator drift checks validate both generated Rust and normalized JSON pack.
- Documentation lists five initial profiles and their support statuses.

- [ ] **Step 1: Write failing drift and support-matrix tests**

Add a Python test that runs the generator against a temporary copy of the five canonical profiles and compares both outputs. Change one profile byte and assert the check command exits nonzero. Add a Rust test that asserts the catalog support matrix:

```rust
assert_eq!(entry("Antelope Zen Go Synergy Core").readiness, Readiness::Supported);
assert_eq!(entry("Antelope Orion Studio III").readiness, Readiness::Supported);
assert_eq!(entry("Antelope Discrete 8 Pro Synergy Core").readiness, Readiness::Partial);
assert_eq!(entry("Antelope Discrete 4 Synergy Core").readiness, Readiness::Unverified);
assert_eq!(entry("Antelope Discrete 4 Pro Synergy Core").readiness, Readiness::Unverified);
```

- [ ] **Step 2: Run drift tests and verify expected failures**

Run:

```bash
python3 -m unittest discover -s tools -p 'test_*.py' -v
python3 tools/generate_device_catalog.py \
  --check /home/ryodeushii/repos/Antelope-Ctl/profiles \
  --generated src/device/generated.rs \
  --pack-generated src/device/generated_profiles.json
cargo test -p zen-go-tui device::
```

Expected result: the new pack check and support assertion fail until generator output and documentation policy are updated.

- [ ] **Step 3: Write support and workflow documentation**

Document:

- canonical raw profile location and `mic_models.json` exclusion;
- Rust and normalized JSON generation commands;
- provenance hashes and generator version;
- built-in catalog fallback when no external pack is supplied;
- `--profile-pack` and `--device` behavior;
- supported Zen Go and Orion dimensions and controls;
- disabled Discrete profile reasons;
- exact HID path and ambiguity safety rules;
- fixture versus physical hardware validation requirements;
- saved-state TOML profile separation.

Keep claims tied to current tests or captured evidence. Mark hardware checks as pending when no hardware run exists.

- [ ] **Step 4: Run complete verification**

Run each command separately and record exit status:

```bash
cargo fmt --all -- --check
cargo test --workspace
python3 -m unittest discover -s tools -p 'test_*.py' -v
python3 tools/generate_device_catalog.py \
  --check /home/ryodeushii/repos/Antelope-Ctl/profiles \
  --generated src/device/generated.rs \
  --pack-generated src/device/generated_profiles.json
git diff --check
```

Run primary LSP diagnostics for every edited Rust and Python file. Run `lens_diagnostics` with `mode=all` for the same files. Do not report hardware validation unless a separate physical-device run produced evidence.

- [ ] **Step 5: Review final scope**

Check that every changed file maps to one task. Search for fixed runtime assumptions:

```bash
rg -n "ZEN_GO_PID|HidTransport::open\(|\[u8; 320\]|\[.*; 2\]|\[.*; 3\]|Mix1|Mix2" src antelope-protocol zen-go-slint
```

Allowed matches are Zen Go adapter code, compatibility tests, raw packet fixtures, and documentation examples. Runtime startup, picker, controller, queue, dynamic state, and UI paths must use catalog or profile-driver data.

- [ ] **Step 6: Commit the completed implementation when requested**

Use one focused commit after all checks pass:

```bash
git add antelope-protocol src tools README.md docs
git commit -m "feat: add profile-driven Antelope device support"
```

## Plan self-review checklist

- Spec source and generated-artifact constraints are covered by Task 1 and Task 7.
- Owned profile loading and cross-crate ownership are explicit in Task 1.
- Generic `ProfileDriver`, typed operations, safe readback bounds, and no formula evaluation are explicit in Task 2.
- Dynamic inputs, outputs, mixers, routing, and raw events are explicit in Task 3.
- Capability-aware rendering, pagination, labels, and hit areas are explicit in Task 4.
- Orion dimensions, operations, readback, fixtures, and readiness enablement are explicit in Task 5.
- Discovery, exact-path opening, CLI selection, picker diagnostics, and worker replacement are explicit in Task 6.
- Drift checks, docs, formatter, tests, diagnostics, and hardware-validation limits are explicit in Task 7.
- No step relies on an unspecified formula evaluator, an implicit profile shape, or an unvalidated default.
