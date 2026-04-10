# antelope-protocol/src/

## Responsibility

Protocol codec layer for the Antelope Audio Zen Go Synergy Core audio interface over USB HID. Provides bidirectional translation between raw 320-byte HID reports and strongly-typed Rust structs/enums covering: sample rate, clock source, preamp configuration (gain, mode, phantom, phase), output volume/mute/dim, mixer strip state (level, pan, mute, solo, assignment, stereo link), device metadata, and startup capability queries.

## Module Structure

| Module | Lines | Role |
|--------|-------|------|
| `lib.rs` | 38 | Crate root; re-exports all public symbols from 5 private modules |
| `types.rs` | 703 | Core protocol types, enums, error types, snapshot struct, meter conversions |
| `frame.rs` | 667 | Incoming HID report parser; dispatches by frame type ID to typed variants |
| `encoder.rs` | 485 | Outgoing HID report builder; encodes `Command` variants into 320-byte frames |
| `mixer.rs` | 860 | Mixer surface/strip/assignment/link types; passive snapshot decode logic |
| `query.rs` | 1095 | Query request/response types; startup query sequence; response decoders |

## Design Patterns

**Typed protocol envelope.** All raw byte interpretation is encapsulated behind `from_code`/`code()` method pairs on enums (`SampleRate`, `ClockSource`, `OutputMode`, `Surface`, `PreampMode`). Unknown codes map to `Unknown(u8)` variants rather than failing, enabling forward compatibility.

**Dual-frame representation.** `Frame` owns raw bytes alongside decoded data for debugging/replay; `DeviceSnapshot` is the owned, raw-free counterpart suitable for long-term storage. Conversion via `From<Frame>` and `into_snapshot_and_raw()`.

**Partial-state decoding.** Passive snapshot decoding (`MixerPassiveDecode`, `MixerPassiveStripState`) uses `Option<T>` fields because not all state is observable from every snapshot frame. Active query decoding (`QueriedMixerStripState`, `StartupMixerStripState`) produces fully-resolved fields because the device responds with complete data.

**Bank-based write routing.** Mixer strip assignments target different HID banks depending on strip number: strips 1–4 use bank `0x05` (early AFX-adjacent), strips 5–16 use banks `0x03`/`0x06`–`0x09` (ordinary). `MixerStrip::assignment_write_banks()` encodes this routing logic.

**Composite state encoding.** `PanState` packs pan position (lower 6 bits), mute (bit 6), and solo (bit 7) into a single protocol byte. `state_code()`/`from_state_code()` handle packing/unpacking.

**Correlated byte group decoding.** Passive mixer decode (`decode_passive_mixer_state`) cross-references bytes from multiple payload regions (`0x8f`, `0xcf`, `0xda`–`0xdf`, `0xde`/`0xdf`) to determine mute, pan, and link state — no single byte is authoritative.

## Data & Control Flow

### Incoming direction (device → host)

```
raw HID report (320 bytes)
  → Frame::parse() / Frame::parse_owned()          [frame.rs]
    ├─ 6 bytes → Frame::Notification
    ├─ type 0x73 → Frame::Snapshot
    │   └─ parse_snapshot73() → DeviceStateSnapshot
    │       ├─ SampleRate::from_code(payload[0x02])
    │       ├─ ClockSource::from_code(payload[0x03])
    │       ├─ OutputState::new() × 3 (Monitor, HP1, HP2)
    │       ├─ PreampState::from_cluster(payload[0x18..0x1c])
    │       ├─ Surface::from_code(payload[0x6a])
    │       └─ decode_passive_mixer_state(payload)  [mixer.rs]
    │           ├─ decode_strip_meter() × 16 channels
    │           ├─ decode_preamp_meter() × 2
    │           ├─ decode_mute_from_group()
    │           ├─ decode_pan_from_group()
    │           └─ decode_link_state()
    ├─ type 0x75 → Frame::QueryReply
    │   └─ QueryResponse { query_id, sub_id, body }
    │       → decoded via QueryResponse methods    [query.rs]
    │           ├─ metadata()           (query 0x01)
    │           ├─ assignment_readback() (query 0x03)
    │           ├─ startup_pan_state_readback() (query 0x04)
    │           ├─ selector_bitmap()    (query 0x0b/0x03)
    │           ├─ mixer_strip_readback() (query 0x18)
    │           └─ ...
    └─ type 0x83 → Frame::Auxiliary (passthrough)
```

### Outgoing direction (host → device)

```
Command enum variant                                  [encoder.rs]
  → encode_command()
    ├─ SetSampleRate     → host_frame(0x12, [0x03, rate.code()])
    ├─ SetClockSource    → host_frame(0x12, [0x04, source.code()])
    ├─ SelectSurface     → host_frame(0x13, [0x49, 0x00, surface.code()])
    ├─ SetPreampMode     → host_frame(0x13, [0x4f, input, mode.code()])
    ├─ SetPreampGain     → host_frame(0x13, [0x50, input, raw])
    ├─ SetPreampPhantom  → host_frame(0x13, [0x51, input, enabled])
    ├─ SetPreampPhase    → host_frame(0x13, [0x52, input, enabled])
    ├─ SetOutputVolume   → host_frame(0x13, [0x47, target.index(), step])
    ├─ SetOutputMute     → host_frame(0x13, [0x48, target.index(), enabled])
    ├─ SetOutputDim      → host_frame(0x13, [0x66, target.index(), enabled])
    ├─ SetMixerLevel     → host_frame(0x16, [0xd4, 0x04, mixer, ch, level, pan.state_code()])
    ├─ SetMixerMute      → host_frame(0x16, [0xd4, 0x04, mixer, ch, 0x00, pan.state_code()])
    ├─ SetMixerSolo      → host_frame(0x16, [0xd4, 0x04, mixer, ch, 0x00, pan.state_code()])
    ├─ SetMixerPan       → host_frame(0x16, [0xd4, 0x04, mixer, ch, 0x00, pan.state_code()])
    ├─ SetMixerAssignment→ encode_mixer_assignment_frames() → assignment_frame(bank) × N
    └─ SetLinkState      → host_frame(0x14, [0xa2, 0x03, selector, enabled])

QueryRequest → encode_query() → host_frame(0x74, [query_id, sub_id])

companion link → encode_link_companion(bank, enabled) → host_frame(0x14, [0xa2, 0x04, bank, enabled])
```

### Startup sequence

`control_panel_startup_queries()` returns a static slice of 47 `QueryRequest`s sent during device initialization. Responses arrive as `Frame::QueryReply` and are decoded via `QueryResponse` methods keyed by `query_id`/`sub_id`.

## Integration Points

**HID transport layer (zen-go-tui).** This crate produces/consumes `Vec<u8>` frames of exactly `HID_REPORT_SIZE` (320 bytes). The TUI application handles the actual `hidapi` read/write syscalls and passes raw bytes to `Frame::parse()` and `encode_command()`.

**State management.** `DeviceStateSnapshot` and `DeviceSnapshot` serve as immutable point-in-time captures. The TUI accumulates these into mutable application state (e.g., `DeviceMetadata`, `MixerChannelState` arrays).

**UI rendering helpers.** `display_db()`, `gain_ratio()`, `meter_ratio()`, `meter_display_db()`, `ratio()`, `attenuation_steps()` convert raw protocol bytes into UI-ready values (dB labels, 0.0–1.0 sliders, percentage offsets).

**No external dependencies beyond `thiserror`.** All protocol decoding is hand-written byte manipulation. `ProtocolError` derives `thiserror::Error` for ergonomic error propagation.

## Key Abstraction Boundaries

| Boundary | Inside crate | Outside crate |
|----------|-------------|---------------|
| Byte ownership | `Frame` preserves `raw: Vec<u8>`; `DeviceSnapshot` drops it | TUI stores `DeviceSnapshot` for state, keeps raw only for diagnostics |
| Partial vs full state | `MixerPassiveStripState` (snapshot-derived, `Option` fields) | `QueriedMixerStripState` (query-derived, concrete fields) |
| Frame type dispatch | `Frame::parse()` routes by 4-byte LE type ID | TUI matches on `Frame` variants to route to appropriate handlers |
| Mixer bank routing | `MixerStrip::assignment_write_banks()` returns bank IDs | TUI iterates returned banks and sends each encoded frame |
| Stereo link addressing | `MixerLinkTarget::from_selector()` / `from_channel()` | TUI uses these to map UI click → protocol selector byte |
