# Zen Go Preamp Protocol

This document collects the parts of the Zen Go preamp/front-end protocol that are grounded well enough to back the current TUI implementation.

It is intentionally narrower than `docs/protocol/pcap-analysis.md`.
This file focuses on the implemented preamp path: gain, source/type mode, phantom, phase, observed input metering, and the `0x73` decode that reflects those states.

## Scope

Covered here:

- preamp gain for inputs `1` and `2`
- input mode selection: `Mic`, `Line`, `Hi-Z`
- phantom-power toggle
- phase-invert toggle
- the authoritative `0x73` bytes that reflect those states
- narrow observed preamp metering used by the TUI
- the host write commands currently used by the TUI

Not covered here:

- richer DSP/editor-page behavior around `51 01 01`, `a2`, `d5`, `d7`
- AFX or mixer-strip DSP editing
- UI-only labels that are still not grounded by captures

## Authoritative Device State

The current implementation treats the `0x73` snapshot as authoritative device state.

For the preamp path, the grounded front-cluster is:

| `0x73` payload offset | Meaning |
|---|---|
| `0x18` | Input 1 gain raw |
| `0x19` | Input 2 gain raw |
| `0x1a` | Input 1 mode raw |
| `0x1b` | Input 2 mode raw |

This is implemented in `src/protocol.rs` via `Snapshot73::dsp_cluster` and `PreampState::from_cluster()`.

## Mode Raw Encoding

The low nibble selects the base signal type:

| Low nibble | Mode |
|---|---|
| `0x00` | `Mic` |
| `0x01` | `Line` |
| `0x02` | `Hi-Z` |

Additional confirmed bits used by the current TUI model:

| Bit | Meaning | Boundary |
|---|---|---|
| `0x10` | phantom enabled on mic inputs | confirmed for the implemented `51 00 xx` path |
| `0x40` | phase invert enabled | confirmed for the implemented `52 00 xx` path |

Important implementation boundary:

- phantom is only treated as meaningful on `Mic` inputs
- the same byte also participates in richer DSP/front-end workflows outside the narrow implemented path, so higher-level editor semantics should not be inferred from `0x1a` / `0x1b` alone

## Host Write Commands

The current TUI uses `0x70 / length 0x13` host writes for the implemented preamp controls.

| Control | Payload body | Meaning |
|---|---|---|
| Mode | `4f <input> <mode>` | select `Mic` / `Line` / `Hi-Z` |
| Gain | `50 <input> <raw>` | set gain raw byte |
| Phantom | `51 <input> <0|1>` | toggle phantom bit path |
| Phase | `52 <input> <0|1>` | toggle phase bit path |

Where:

- `<input>` is `0` for input 1 and `1` for input 2
- `<mode>` is `0x00`, `0x01`, `0x02` for `Mic`, `Line`, `Hi-Z`

This encoding is implemented in `src/protocol.rs` in `encode_command()`.

## Gain Interpretation

The TUI currently uses the following grounded gain interpretation from the capture set and the implemented parser.

### Mic

- raw range used by the app: `0x00..0x41`
- displayed range: `0..65 dB`

### Line

- raw byte is treated as signed `i8`
- displayed range is clamped to `-6..+20 dB`
- examples in code use values like `0xfa` for `-6 dB` and `0x14` for `+20 dB`

### Hi-Z

- raw range used by the app: `0x00..0x2d`
- displayed range: `0..45 dB`

These rules are implemented in `PreampInputState::gain_db_label()` and `gain_ratio()` in `src/protocol.rs`.

## Observed Metering

The current TUI also surfaces a narrow observed input meter for each preamp directly from the passive `0x73` snapshot.

| `0x73` payload offset | Meaning |
|---|---|
| `0xce` | observed `A1` meter raw |
| `0xcf` | observed `A2` meter raw |

Implementation boundary:

- these lanes are treated as narrow observed meter indicators for app use, not as a full parser-ready preamp metering model
- the app only accepts the lower plausible meter range on these lanes (`0x01..0x49`) so late-row status values such as `0x4b`, `0x4c`, `0x4e`, `0x51`, `0x54`, `0x5a`, and `0x60` are not promoted into fake meters
- this observed preamp metering is distinct from the shared strip-meter lane at `0x8e..0x9d` and can coexist with strip metering when the same input is also assigned to a mixer strip

This is implemented in `src/protocol.rs` via `decode_passive_mixer_state()` and `decode_preamp_meter()`, then surfaced in app/UI state through `src/app.rs` and `src/ui.rs`.

## Stable-State Model

The device remains authoritative.

The implemented workflow is:

1. Send one preamp command.
2. Wait for the next `0x73` snapshot.
3. Use the updated `{0x18,0x19,0x1a,0x1b}` cluster as the real device state.

The app also keeps a pending-write overlay so the UI can track the requested mutation while waiting for the next snapshot, but the protocol documentation should still treat the snapshot as the source of truth.

## Confirmed vs Deferred

Confirmed enough for the current app:

- input 1 / input 2 gain raw bytes
- `Mic` / `Line` / `Hi-Z` mode decode
- phantom toggle bit for the implemented path
- phase toggle bit for the implemented path
- observed `A1` / `A2` preamp meter lanes at `0xce` / `0xcf`
- `4f`, `50`, `51 00`, `52 00` command families

Deferred / intentionally not claimed here:

- the user-facing meaning of the richer `51 01 01` mode
- the full meaning of the DSP-only `0x0ae` row family
- hidden editor/page selectors carried by later `a2` traffic
- any claim that the entire DSP/front-end editor can be reconstructed from `0x18..0x1b` alone

## Code References

- `src/protocol.rs`: `PreampMode`, `PreampInputState`, `PreampState`, `Snapshot73`, `decode_passive_mixer_state()`, `decode_preamp_meter()`, `encode_command()`
- `src/app.rs`: pending mutation handling for gain, mode, phantom, phase, and observed-meter preservation
- `src/ui.rs`: preamp gauges and observed meter rendering

## Related Analysis

- canonical capture analysis: `docs/protocol/pcap-analysis.md`
- remaining unresolved protocol gaps: `docs/protocol/open-questions.md`
- control-panel feature map: `cpl.md`
