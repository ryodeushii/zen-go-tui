# Zen Go profile and dynamic runtime repair design

## Goal

Restore Zen Go mixer readback and preamp meter state on the multi-device branch. Make preamp, output, and mixer controls use profile-defined topology and ranges. Keep unverified protocol fields out of normal writes.

## Context

The feature branch moved Zen Go through a normalized driver and dynamic state model, but two legacy data paths stop at the adapter boundary.

- Zen Go still sends the legacy 47 startup queries. `ZenGoDriver::decode` emits `QueryReply` with `patch: None`. q04 mixer level readback and q18 state readback therefore never reach application state.
- The 0x73 parser stores candidate preamp meter bytes in `mixer_decode`, while `PreampState::from_cluster` leaves `PreampInputState::observed_meter` empty. The normalized snapshot conversion reads only the latter.
- The profile identifies a separate 0x83 meter report, but it does not contain a verified byte-to-channel map. The frame parser treats 0x83 as auxiliary data. The repair must not guess this map.
- The application creates profile-sized vectors, but Zen Go render and mouse paths still use fixed two-input, three-output, two-surface, and sixteen-strip layouts.
- Zen Go fader values use attenuation semantics. Raw 0 means unity and raw 90 means -90 dB. Existing inverse slider math and decrement-on-increase behavior match this domain. Missing readback and raw dynamic labels create the reported reverse-level symptom. No wire inversion is planned.

## Decisions

### Use the submodule profile as source

Base a local Antelope-Ctl profile branch on submodule main `27bb70abd28746da94c0588dfeed903789a13b66`. Edit `modules/Antelope-Ctl/profiles/zen_go_sc.json`. Commit that profile change in the submodule, then update this repository's gitlink and generated artifacts.

The unmodified pinned and latest Zen Go profiles are byte-identical. The parent generator and runtime changes are required in addition to the profile edit.

### Keep a Zen Go adapter, but make it profile-backed

Do not force Zen Go through `ProfileDriver`. Its readback categories and 0x83 meter map are not validated. Keep proven legacy command encoding in `ZenGoDriver`, but give the adapter the validated `RuntimeProfile` and use profile data for topology, safe startup queries, fader semantics, and typed readback layouts.

The adapter produces normalized events. It does not expose fixed legacy arrays to the application UI.

### Represent safe readback pairs explicitly

Category counts cannot represent Zen Go's sparse, capture-scoped query set without allowing unsafe indexes. Extend the normalized readback model with an explicit ordered `safe_queries` list. Keep `category_counts` empty unless a contiguous bound is proven.

The Zen Go source profile will contain the existing 47 known-safe startup pairs as explicit entries. The generator will copy them without expanding unknown categories. Query encoding will accept only an explicit pair or a proven category/count pair.

Add typed, capture-scoped readback layouts for confirmed q04/0 and q04/1 mixer state. Keep q18/0 as an observed legacy layout only for fields supported by existing evidence. Unknown safe replies remain available as raw `QueryReply` events without a typed patch.

### Treat known and unknown meters differently

Add profile data for the observed 0x73 candidate preamp meter offsets 0xce and 0xcf with their capture caveat. Use these offsets to preserve the existing working candidate-meter behavior.

Do not add a typed 0x83 channel map. When a profile lacks a verified map, keep 0x83 as auxiliary raw data. A later capture-backed profile edit can add a typed meter layout without changing the application contract.

Zen Go AFX context does not change this rule. AFX areas above mixer channels 1 through 4 can host Synergy Core effects. The device supports up to four mono AFX paths or two stereo AFX paths, with up to eight processors per path. Antelope-Ctl also mentions an EmuMic feature, but this repository has no capture-backed EmuMic evidence yet. Treat current 0x83 data as a possible aggregate AFX/DSP meter block, not as physical A1/A2 meters. AFX and EmuMic controls remain outside this repair.

### Make profile topology the application source of truth

`AppState::from_profile` allocates all input spaces, outputs, mixer surfaces, masters, strips, and peak slots from `RuntimeProfile`. Dynamic snapshots and patches update existing entries by numeric address. They do not replace profile topology or introduce fixed fallback dimensions.

### Use dynamic UI paths for every driver

Remove Zen Go-only branches from preamp, output, mixer layout, rendering, and mouse hit testing. Use profile-sized vectors, profile capabilities, profile names, and profile ranges for all drivers. Keep legacy protocol conversion inside `ZenGoDriver` only.

Dynamic mixer widgets will render strip meters when a meter exists. Fader labels will use the profile's fader domain rather than printing an unexplained raw value. For Zen Go, the label will show attenuation in dB while the slider remains visually louder at the top.

## Data flow

```text
Antelope-Ctl zen_go_sc.json
        │
        ▼
Python normalizer
        │
        ├── generated.rs
        └── generated_profiles.json
                │
                ▼
        RuntimeProfile
                │
                ▼
        ZenGoDriver(profile)
          ┌─────┼──────────────┐
          │     │              │
        0x73   0x75           0x83
       state  readback     raw auxiliary
          │     │              │
          └─────┴──────────────┘
                │
                ▼
       normalized DeviceEvent
                │
                ▼
       profile-shaped AppState
                │
                ▼
       dynamic layouts, widgets, and controls
```

For 0x73, `state_from_snapshot` allocates the profile topology, overlays legacy values by bounded address, and assigns candidate preamp meters from `mixer_decode.observed_preamp1_meter` and `observed_preamp2_meter`.

For q04/0 and q04/1, the typed layout reads sixteen records. Each record uses level at body offset `2 + channel * 2` and state at `3 + channel * 2`. Level remains raw attenuation. The adapter emits a mixer patch for the profile surface selected by the query index.

For q18/0, the observed two-byte records remain bounded to 32 records and two 16-strip surfaces. The adapter emits only fields supported by the profile layout and capture evidence. The raw body remains attached to the event.

For 0x83, the adapter emits `DeviceEvent::Meter` only when the profile supplies a verified channel map. The current Zen Go profile does not, so the event remains auxiliary and the application does not display guessed values.

`AppState` merges mixer patches by `(surface, strip)`. A patch cannot change strip count or surface count. It preserves existing meter values when a readback patch omits meters. Input meter patches merge by `InputAddress`.

## Profile changes

Update `modules/Antelope-Ctl/profiles/zen_go_sc.json` with:

- explicit ordered `safe_queries` containing the current 47 startup pairs;
- q04/0 and q04/1 mixer-state layout metadata, including body size, record count, stride, level offset, state offset, and surface mapping;
- q18/0 observed mixer-record metadata with capture-scoped status;
- 0x73 candidate preamp meter offsets 0xce and 0xcf with a mixed-signal caveat;
- mixer fader domain `0..90`, attenuation direction, unity value `0`, and minimum write interval `250 ms`;
- profile-defined input, output, and mixer topology fields used by the dynamic UI.

Do not add Orion category counts, Zen Go 0x83 channel offsets, unverified physical output names, or untested Zen Go routing readback semantics.

The generator will emit these fields into typed Rust and normalized JSON. It will reject duplicate safe pairs, out-of-range offsets, malformed layouts, and startup pairs not present in the safe list.

## Module boundaries

- `modules/Antelope-Ctl/profiles/zen_go_sc.json` owns Zen Go evidence, safe query pairs, layout declarations, and topology.
- `tools/generate_device_catalog.py` validates and renders those declarations.
- `antelope-protocol/src/profile.rs` owns typed readback allowlists, layouts, fader-domain metadata, and candidate meter metadata.
- `antelope-protocol/src/profile_codec.rs` encodes only validated safe queries and profile-defined control ranges.
- `antelope-protocol/src/zen_go.rs` owns legacy Zen Go byte encoding and conversion from legacy frames to normalized state and patches.
- `antelope-protocol/src/frame.rs` continues to preserve unknown 0x83 reports as raw auxiliary frames unless the selected driver has a typed meter layout.
- `src/app/mod.rs` and `src/app/state.rs` own profile-shaped state and address-based patch merging.
- `src/ui/layouts.rs`, `src/ui/render/mod.rs`, `src/ui/mouse.rs`, and `src/ui/widgets/mixer.rs` own dynamic geometry, controls, labels, and meters. These modules must not branch on `RuntimeDriverKind::ZenGo` to choose collection sizes.

## Failure and safety rules

- Reject a query pair not in `safe_queries` or inside a proven category bound.
- Reject readback bodies shorter than the declared layout.
- Reject mixer patches whose surface or strip address is outside the selected profile.
- Preserve raw bytes when a frame or category is unknown.
- Do not convert an unresolved 0x83 report into physical input meters.
- Do not write a control whose profile status is unconfirmed.
- Do not use a profile count as evidence for an unverified query category.
- Keep fader conversion in one profile-aware helper. Do not invert values in both UI and driver layers.

## Testing

### Generator and profile tests

- Load the edited canonical profile and assert safe query pairs remain explicit and ordered.
- Assert q04 layouts, candidate meter offsets, fader domain, topology, and provenance appear in both generated artifacts.
- Reject duplicate safe pairs, an out-of-range layout offset, and a startup pair absent from the safe list.
- Run generator drift checks against both `generated.rs` and `generated_profiles.json`.

### Protocol tests

- Assert `ZenGoDriver` startup requests equal the profile's 47 requests and encode category/index at bytes 8 and 12.
- Decode q04/0 and q04/1 fixtures and assert raw attenuation values, pan, mute, and solo reach the correct profile mixer surfaces.
- Assert q04 values `0x00`, `0x12`, `0x1e`, and `0x5a` remain 0, -18, -30, and -90 dB respectively.
- Assert q18/0 remains bounded to 32 two-byte records and preserves raw response bytes.
- Decode a 0x73 fixture with candidate bytes at 0xce and 0xcf and assert normalized input meters contain those values.
- Assert a 0x83 fixture remains auxiliary with the current profile and never creates a false typed meter event.
- Assert a profile fixture with a verified meter layout produces `DeviceEvent::Meter` and input-addressed values.

### Application and UI tests

- Apply a q04 mixer patch and assert fader, pan, mute, and solo update without changing profile topology or deleting an existing meter.
- Construct profiles with counts different from Zen Go and assert preamp cards, output cards, mixer surfaces, and strips match profile vectors.
- Assert Zen Go uses dynamic input, output, mixer, and mouse paths rather than legacy fixed-card paths.
- Assert dynamic fader click and wheel behavior maps top to the profile's unity value and louder adjustment toward lower attenuation.
- Assert dynamic mixer strips render available meter values and profile-based dB labels.
- Assert clicks outside profile-sized vectors create no action.

Hardware validation remains separate. Fixture tests cannot claim 0x83 channel mapping or physical output/routing correctness.

## Implementation order

1. Initialize the submodule at latest main, edit and commit the Zen Go profile, and update generator schema and tests.
2. Regenerate checked-in Rust and JSON artifacts.
3. Make `ZenGoDriver` profile-backed and bridge q04/q18 plus known 0x73 candidate meters.
4. Make normalized application state merge profile-addressed patches.
5. Remove fixed Zen Go UI branches and complete dynamic mixer meter/control rendering.
6. Run generator checks, Rust formatting, workspace tests, diagnostics, and diff checks.
7. Prepare separate submodule and parent-repository commits for review and later PR creation.

## Non-goals

- Do not claim a 0x83 physical-channel map without a capture-backed invariant.
- Do not copy Orion readback category counts into Zen Go.
- Do not rewrite proven Zen Go command bytes.
- Do not enable unverified Zen Go output or routing controls.
- Do not change saved-state TOML schema.
