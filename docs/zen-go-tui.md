# Zen Go Synergy Core TUI

## What this app is

`zen-go-tui` is a Rust terminal UI for the Zen Go Synergy Core control plane.
It uses `ratatui` for the interface and a HID transport abstraction for device I/O.

The implementation follows the reverse-engineered USB HID protocol documented in:

- `docs/protocol/pcap-analysis.md`
- `docs/protocol/open-questions.md`
- `cpl.md`

## Transport choice

The concrete HID implementation uses the `hidapi` crate.
That choice is pragmatic for Linux HID access in Rust, but the app is structured around the `Transport` trait so HID access can be swapped without touching protocol or UI code.

## Run

### Real device

```bash
cargo run
```

### Mock mode

```bash
cargo run -- --mock
```

The app uses the hardcoded Zen Go Synergy Core HID identifiers for real-device mode:

- VID: `0x23e5`
- PID: `0xa015`

## Keyboard controls

- `Tab` — cycle focus between status, outputs, mixer, preamp
- `Left` / `Right` — select output or mixer channel
- `+` / `-` — adjust selected output volume or mixer fader
- `m` — toggle output mute or mixer mute
- `d` — toggle output dim
- `s` — cycle sample rate
- `c` — cycle clock source
- `1` — switch to Monitor / HP1 surface
- `2` — switch to HP2 surface
- `?` — show quick help
- `q` — quit

## Confirmed protocol support exposed in the app

- startup `0x74` queries and `0x75` metadata parsing
- `0x73` snapshot parsing for:
  - sample-rate code and numeric sample rate
  - clock source
  - output volume for Monitor / HP1 / HP2
  - output mode for Monitor / HP1 / HP2 (`normal`, `mute`, `dim`)
  - current surface selector (`Monitor/HP1` vs `HP2`)
  - front DSP cluster bytes (`0x18..0x1b`) shown read-only
- confirmed host writes for:
  - sample rate (`0x70 / 0x12`)
  - clock source (`0x70 / 0x12`)
  - output volume (`0x47`)
  - output mute (`0x48`)
  - output dim (`0x66`)
  - mixer level (`0xd4 04 mm cc ll pp`)
  - mixer mute (`0x40` bit in `pp`)
  - surface select (`0x49 00 ss`)

Output volume notes:

- the device uses inverse raw steps for output volume
- raw `0x00` = `0 dB`
- raw `0x60` = `-96 dB` / effectively minimum
- the TUI now displays output volume in dB while still showing the raw step for reference

Mixer level notes:

- mixer strip levels follow the same inverse scale as outputs in the current app
- raw `0x00` = `0 dB`
- raw `0x60` = `-96 dB` / effectively minimum
- the TUI now displays known mixer-strip levels in dB rather than raw hex

## Experimental / intentionally limited areas

- **lock indicator**: not safely decoded from current captures, so the UI marks it as experimental/unknown
- **mixer state decode from `0x73`**: current captures confirm command families and mixer-related late-table movement, but the candidate late bytes also churn before the first host write and during pure idle. That means a safe passive per-strip startup decode is still unresolved, so the TUI intentionally shows only last confirmed command round-trip mixer state rather than inventing startup strip values. Tracked mixer overlays are kept per surface (`MIX 1` vs `MIX 2`) so writes on one surface do not pollute the other.
- **preamp / DSP editing**: current protocol understanding is only partial; the TUI exposes the front DSP byte cluster read-only and labels it experimental
- **link / unlink controls**: the `0x70 / 0x14` family is documented, but selector semantics are not resolved enough to expose as a normal interactive control yet

## Verification expectations without hardware

Unit tests use `MockTransport` and do not require a real device.
Hardware validation still needs a connected Zen Go device.
