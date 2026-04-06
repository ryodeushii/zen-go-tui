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
- `3` — cycle preamp mode for the selected preamp input
- `p` — toggle preamp phase for the selected preamp input
- `s` — cycle sample rate
- `c` — cycle clock source
- `Left` / `Right` on preamp page — select `A1` or `A2`
- `+` / `-` on preamp page — adjust selected preamp gain
- `m` on preamp page — toggle phantom for the selected preamp input
- `1` — switch to Monitor / HP1 surface
- `2` — switch to HP2 surface
- `?` — show quick help
- `q` — quit

## Raw frame view

The TUI now includes a raw-data page for live protocol inspection.

- keep pressing `Tab` until the raw page is focused
- the page shows only the latest live `0x73` packet and the latest live `0x83` packet
- both panes render a full hex + ASCII dump of the current packet state
- press `b` on the raw page to capture a baseline
- press `x` to clear the baseline
- bytes that changed relative to the baseline are highlighted, which is useful when isolating mixer-strip-related changes in late `0x73`

This is especially useful while continuing reverse-engineering of late `0x73` mixer state and auxiliary traffic.

## Confirmed protocol support exposed in the app

- startup `0x74` queries and `0x75` metadata parsing
- `0x73` snapshot parsing for:
  - sample-rate code and numeric sample rate
  - clock source
  - output volume for Monitor / HP1 / HP2
  - output mode for Monitor / HP1 / HP2 (`normal`, `mute`, `dim`)
  - current surface selector (`Monitor/HP1` vs `HP2`)
  - preamp front-cluster decode from `0x18..0x1b`:
    - `0x18` = preamp 1 gain raw
    - `0x19` = preamp 2 gain raw
    - `0x1a` = preamp 1 mode raw
    - `0x1b` = preamp 2 mode raw
    - low nibble `0x00/0x01/0x02` = `Mic` / `Line` / `Hi-Z`
    - bit `0x10` on a mic-mode byte = phantom on
- confirmed host writes for:
  - sample rate (`0x70 / 0x12`)
  - clock source (`0x70 / 0x12`)
  - preamp mode (`0x4f <input> <mode>`)
  - preamp gain (`0x50 <input> <raw>`)
  - preamp phantom (`0x51 <input> <0|1>`)
  - preamp phase (`0x52 <input> <0|1>`)
  - output volume (`0x47`)
  - output mute (`0x48`)
  - output dim (`0x66`)
  - mixer level (`0xd4 04 mm cc ll pp`)
  - mixer mute (`0x40` bit in `pp`)
  - surface select (`0x49 00 ss`)

Preamp gain notes:

- preamp 1 line mode uses raw `0xfa..0x14` for `-6..+20 dB`
- preamp 1 Hi-Z mode uses raw `0x00..0x2d` for `0..45 dB`
- preamp 1 mic mode uses raw `0x00..0x41` for `0..65 dB`
- preamp 2 follows the same gain rules per mode

Preamp UI notes:

- the preamp pane now shows separate visual gain widgets for `A1` and `A2`
- gain color tracks mode: `Mic`, `Line`, `Hi-Z`
- phantom is shown as a stronger `48V` indicator on mic inputs
- phase is shown as `norm` vs `inv`

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
- **preamp / DSP editing**: preamp gains, modes, phantom, and phase are now exposed from the newly isolated captures; richer DSP/preamp page behavior around `51 01 01`, `a2`, `d5`, and `d7` remains intentionally out of scope
- **link / unlink controls**: the `0x70 / 0x14` family is documented, but selector semantics are not resolved enough to expose as a normal interactive control yet

## Verification expectations without hardware

Unit tests use `MockTransport` and do not require a real device.
Hardware validation still needs a connected Zen Go device.
