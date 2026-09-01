# Multi-device Antelope support specification

## Goal

Make `zen-go-tui` support Antelope hardware through validated profile data. Preserve Zen Go behavior. Add full Orion Studio III support. Permit future profile packs without rebuilding Rust when the profile uses supported, confirmed operations.

## Decision

Use a hybrid profile path:

```text
Antelope-Ctl canonical JSON
        │
        ▼
Python normalizer
        │
        ├── checked-in Rust catalog for built-in fallback and discovery
        └── versioned normalized JSON profile pack for optional runtime loading
        │
        ▼
validated typed profile
        │
        ▼
ProfileDriver
        │
        ▼
normalized actions/events
        │
        ▼
dynamic state and UI
```

Generated Rust is the built-in path. A normalized JSON pack is the extension path. Both paths produce the same typed runtime profile before a driver or HID handle is created.

Profile data is parsed once at startup. The command and decode paths use typed in-memory records. They never inspect JSON or evaluate formula text in the HID loop.

## Source of truth

Antelope-Ctl `profiles/*.json` is canonical. `mic_models.json` is not hardware and is excluded. The existing Python normalizer remains responsible for heterogeneous canonical JSON shapes.

The runtime loader accepts only normalized profile-pack JSON. It does not reimplement canonical-profile normalization in Rust. The generator command must be able to produce that pack from `/home/ryodeushii/repos/Antelope-Ctl/profiles`.

Ordinary Cargo builds must not require an Antelope-Ctl checkout. Built-in generated data and its provenance stay checked in.

## Profile artifacts

### Built-in catalog

`src/device/generated.rs` remains the built-in catalog. It contains typed identity, transport, capability, frame, decoder, parameter, constraint, hazard, readiness, and provenance records. Existing static catalog callers remain supported during migration.

The generator may retain descriptive raw profile text for diagnostics, but driver code must use typed records. The built-in catalog is the fallback when no external pack is configured.

### Normalized profile pack

The generator emits a versioned JSON document with this top-level shape:

```json
{
  "schema_version": 1,
  "generator_version": "1.4.0",
  "profiles": [
    {
      "id": "orion_studio_3",
      "identity": {},
      "transport": {},
      "address_spaces": [],
      "inputs": [],
      "outputs": [],
      "mixers": [],
      "frames": [],
      "decoders": [],
      "params": [],
      "constraints": [],
      "hazards": [],
      "startup_queries": [],
      "readback": {},
      "readiness": "supported",
      "support_reason": "",
      "provenance": {}
    }
  ]
}
```

The empty objects above indicate sections, not optional validation. The normalized schema must contain the fields needed by the existing static records plus:

- stable profile id;
- typed startup queries;
- readback magic, discriminator, offsets, and category bounds;
- typed frame operations for fixed offsets, indexed offsets, strides, masks, pair indexes, and allowed values;
- support reason and provenance.

Formula descriptions remain display metadata. The generator translates only an allowlisted formula into typed operations. Unknown formulas make the affected operation unavailable and cannot produce a normal command.

## Profile loading interface

`antelope-protocol/src/profile.rs` owns the typed runtime profile model used by both loaders and drivers. It provides owned counterparts for the borrowed generated records:

- `ProfilePack` owns one normalized pack;
- `RuntimeProfile` owns one normalized profile;
- `RuntimeEntry` owns one profile and its readiness decision;
- `ReadbackDefinition` owns safe category bounds and query layout;
- `ProfileLoadError` reports schema, JSON, duplicate-identity, provenance, and validation failures.

`src/device/profile.rs` owns catalog assembly. It converts the checked-in static catalog into `RuntimeEntry` values and merges optional packs.

Required functions:

```rust
pub const PROFILE_PACK_SCHEMA_VERSION: u16 = 1;

pub fn load_profile_pack(bytes: &[u8]) -> Result<ProfilePack, ProfileLoadError>;
pub fn load_profile_pack_file(path: &Path) -> Result<ProfilePack, ProfileLoadError>;

impl ProfileCatalog {
    pub fn builtin() -> Self;
    pub fn add_external(&mut self, pack: ProfilePack) -> Result<(), ProfileLoadError>;
    pub fn entries(&self) -> &[RuntimeEntry];
    pub fn find(&self, vid: u16, pid: u16) -> Option<&RuntimeEntry>;
}
```

`ProfileCatalog::builtin` converts the checked-in static catalog once. `add_external` rejects duplicate VID/PID identities by default. External data supplements built-ins and cannot shadow a known profile silently.

The loader rejects an unknown schema version, malformed required fields, duplicate profile ids, duplicate VID/PID identities, invalid report geometry, out-of-range offsets, unsafe readback indexes, uncompiled formulas, and commands that reference unconfirmed fields.

## Readiness and support

Every canonical hardware profile appears in catalog data. Profile existence does not imply control support.

A profile is selectable only when all of these checks pass:

- transport geometry is confirmed and matches the selected HID interface;
- required frame fields and typed operations are present;
- command parameters have confirmed values, ranges, and targets;
- readback queries have explicit safe category bounds;
- no selected command crosses an unconfirmed hazard;
- `ProfileDriver` can resolve the requested capability set;
- golden tests cover each supported protocol shape.

Unknown profiles may become selectable through the generic driver when they pass these checks. A profile with incomplete or unconfirmed data remains visible with a reason and cannot open a controlling driver.

Initial policy remains:

- Zen Go `0x23e5:0xa015`: selectable through the existing adapter;
- Orion Studio III `0x23e5:0xa221`: selectable after generic profile-driver tests pass;
- Discrete 8 Pro `0x23e5:0xa2b5`: visible but disabled because it has no readable state report;
- Discrete 4 `0x23e5:0xa2be`: visible but disabled because transport and frame data are incomplete;
- Discrete 4 Pro `0x23e5:0xa2bf`: visible but disabled because transport and frame data are incomplete.

The readiness table is a safety policy. The generator must not mark Orion selectable only because its raw JSON contains many fields. The profile validator and driver capability tests must also pass.

## Protocol seam

Application code uses one driver interface:

```rust
pub trait DeviceDriver: Send {
    fn definition(&self) -> &DriverDefinition;
    fn startup_requests(&self) -> &[QueryRequest];
    fn encode(&self, action: Action) -> Result<CommandBatch, DriverError>;
    fn decode(&self, bytes: &[u8]) -> Result<Option<DeviceEvent>, DriverError>;
}
```

`DriverDefinition` owns its `id` and `name` strings so runtime profiles do not need `'static` data. `ZenGoDriver` and `ProfileDriver` both implement this interface.

`antelope-protocol/src/profile_driver.rs` provides `ProfileDriver`. It is the default adapter for profiles whose frames fit the normalized operation set. It owns a validated `RuntimeProfile`, precomputes parameter and frame lookup tables, and returns errors for unsupported or unsafe actions.

The normalized action model uses numeric profile addresses rather than Zen Go enums or fixed arrays:

- `InputAddress { space, index }`;
- `OutputAddress { id }`;
- `MixerAddress { mixer, strip }`;
- `RoutingPair { bank, index }`;
- typed input, output, mixer, link, routing, global, and query actions.

A mixer state has one optional master slot and a dynamic vector of input strips. This represents Orion's readback slot 0 master plus 32 input strips without treating the master as an out-of-range input strip.

The normalized event model uses owned vectors and stable numeric addresses:

- `DynamicDeviceState.inputs` contains physical, ADAT, S/PDIF, and other profile-defined input spaces;
- `DynamicDeviceState.outputs` contains profile-defined output buses;
- `DynamicDeviceState.mixers` contains profile-defined surfaces and strips;
- `DynamicDeviceState.routing` contains profile-defined destination groups and source pairs;
- raw report bytes remain available on every event.

Readback responses carry both their original body and an optional typed state patch. The patch lets application state apply mixer and routing records without parsing profile bytes. Define these protocol types:

```rust
pub enum DynamicStatePatch {
    Inputs(Vec<DynamicInputState>),
    Outputs(Vec<DynamicOutputState>),
    Mixer(DynamicMixerSurface),
    Routing(DynamicRoutingGroup),
    Globals(Vec<DynamicGlobalState>),
}

pub enum DeviceEvent {
    Snapshot { state: DynamicDeviceState, raw: Vec<u8> },
    QueryReply {
        query_id: u8,
        sub_id: u8,
        body: Vec<u8>,
        patch: Option<DynamicStatePatch>,
        raw: Vec<u8>,
    },
    Auxiliary { bytes: Vec<u8>, raw: Vec<u8> },
    Notification { bytes: Vec<u8>, raw: Vec<u8> },
}
```

`ProfileDriver::decode` creates a patch for confirmed state, meter, mixer, and routing readback records. It leaves `patch` as `None` for an unknown or unconfirmed category while retaining the body and raw bytes.

The old `DeviceSnapshot` conversion remains private compatibility code until dynamic application state no longer needs it. It is not part of `ProfileDriver`. `ProfileDriver` depends only on `RuntimeProfile` and normalized protocol types, not on `src/app` or saved-state profiles.

## Generic frame operations

The generic codec supports only typed operations emitted by the generator. It must validate before writing:

- report length and numbered-report rules;
- frame magic and opcode;
- parameter id and target space;
- channel, bus, mixer, strip, category, and destination bounds;
- enum membership and numeric ranges;
- bit masks and shifts;
- minimum write interval;
- readback category counts.

The codec does not interpret arbitrary formula strings, execute profile code, or guess missing offsets.

## Orion mapping

Orion uses the generic profile driver. No hand-written Orion adapter is required unless a confirmed frame shape cannot be represented by the typed operation set.

The Orion profile and tests must cover:

- 12 physical inputs;
- 16 ADAT inputs;
- 2 S/PDIF inputs;
- 6 output buses;
- four mixer surfaces;
- one master plus 32 input strips per mixer surface;
- profile-defined routing destinations and source pairs;
- physical input mode, gain, phantom, and phase;
- output level, mute, dim, and confirmed bus controls;
- mixer fader, pan, send, mute, solo, and link;
- global settings and confirmed AuraVerb controls;
- 0x74/0x10 category-index startup requests;
- 0x75 readback responses, including discriminator and safe category bounds;
- 0x73 state reports and meter reports.

The profile's readback category counts are the only valid query bounds. Categories without confirmed counts are not queried automatically.

## HID discovery and runtime selection

Discovery filters `HidApi::device_list()` by Antelope VID `0x23e5`, retains exact path bytes, classifies against `ProfileCatalog`, and sorts selectable candidates first. It never opens an unsupported, disabled, ambiguous, or unverified candidate.

Runtime accepts an optional normalized profile pack path. Startup loads and validates profiles before opening HID. It then constructs the selected `ProfileDriver` and `Controller` together.

The picker and headless selection use exact path, serial, or validated VID/PID criteria. A device switch closes the old worker before constructing a new transport and driver. Reconnect prefers exact path and reports identity ambiguity instead of guessing.

No profile path defaults to `~/repos/Antelope-Ctl`. The application remains runnable with checked-in built-ins only.

## Dynamic state and UI

Application state uses vectors keyed by profile addresses. Saved TOML files in `src/profile.rs` remain user presets and do not define hardware protocol geometry.

UI code renders profile names, counts, and capabilities. It must show all six Orion output buses, all 12 physical inputs, all 16 ADAT inputs, all 2 S/PDIF inputs, all four mixer surfaces, all 32 input strips per surface, and routing groups without out-of-bounds indexing.

Large collections use the existing viewport and pagination patterns. Unsupported actions show a stable error message and do not enqueue a command.

## Failure and safety behavior

Handle malformed packs, unknown schema versions, missing profile files, invalid generated data, duplicate identities, missing devices, permissions, unsupported profiles, ambiguous interfaces, multiple identical units, unplug/replug, path changes, short writes, wrong report lengths, unknown frames, missing readback bounds, and unconfirmed fields.

Discovery is read-only. Profile validation happens before HID opening. Driver errors propagate to the controller. Unsupported or ambiguous candidates cannot cause writes.

## Migration

1. Keep the checked-in static catalog and Zen Go adapter working.
2. Add owned normalized profile loading without changing saved-state TOML.
3. Generalize normalized actions/events and migrate controller and queue callers.
4. Add `ProfileDriver` and make Zen Go pass through the same normalized interface.
5. Replace fixed application collections with dynamic collections.
6. Enable Orion through generic profile-driver golden tests.
7. Move CLI startup to catalog discovery and picker selection.
8. Remove private Zen Go-only compatibility conversions after all UI paths use dynamic state.

Each step keeps existing Zen Go tests green.

## Verification

Run these checks before enabling a profile:

- Python generator unit tests and normalized-pack schema tests;
- generated catalog drift check;
- profile loader tests for valid, malformed, duplicate, unsafe, and unknown-schema packs;
- driver contract tests for identity, startup queries, actions, events, bounds, and unsupported fields;
- Zen Go byte-preservation tests;
- Orion golden encoding and decoding tests using captured or profile-derived fixtures;
- dynamic state dimension and no-out-of-bounds tests;
- picker and runtime switching tests;
- `cargo fmt --all -- --check`;
- primary LSP diagnostics;
- `cargo test --workspace`;
- `git diff --check`.

Hardware validation remains separate. Do not claim hardware validation from fixture or simulator tests.
