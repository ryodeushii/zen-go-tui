# Zen Go Synergy Core TUI

## What this app is

`zen-go-tui` is a Rust terminal UI for the Zen Go Synergy Core control plane.
It uses `ratatui` for the interface and a HID transport abstraction for device I/O.

The implementation follows the reverse-engineered USB HID protocol documented in:

- `docs/protocol/mixer-protocol.md`
- `docs/protocol/preamp-protocol.md`
- `cpl.md`

## Transport choice

The concrete HID implementation uses the `hidapi` crate.
That choice is pragmatic for Linux HID access in Rust, but the app is structured around the `Transport` trait so HID access can be swapped without touching protocol or UI code.

## Install

### Install from GitHub with Cargo

Install the current-host binary directly from GitHub:

```bash
cargo install --git https://github.com/ryodeushii/antelope-analysis-gpt54.git --bin zen-go-tui
```

That installs the executable as `zen-go-tui` in Cargo's bin directory.

## Build

The repository ships a local Cargo config that defaults builds and tests to the current Linux host target:

- `x86_64-unknown-linux-gnu`

### Linux release binary

```bash
cargo build --release
```

Output:

- `target/release/zen-go-tui`

### Windows release binary

The documented Windows build path is the GNU target:

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --target x86_64-pc-windows-gnu --release
```

Output:

- `target/x86_64-pc-windows-gnu/release/zen-go-tui.exe`

If you build the Windows binary from Linux, make sure a MinGW-w64 cross linker/toolchain is installed so Cargo can invoke the `x86_64-w64-mingw32-*` tools.

## Run

### Linux device permissions

To use the TUI with the real Zen Go device on Linux without `sudo`, install the bundled udev rule first:

```bash
sudo cp udev_rules/99-antelope.rules /etc/udev/rules.d/99-antelope.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Then unplug and reconnect the interface so the new permissions are applied.

The bundled rule grants access to the Zen Go USB and `hidraw` nodes for:

- VID: `0x23e5`
- PID: `0xa015`

### Real device

```bash
cargo run
```

If the rule is installed correctly, the TUI should open the device without `sudo`.

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
- `Up` / `Down` — adjust the focused output level, mixer fader, or preamp gain
- `m` — toggle output mute or mixer mute
- `d` — toggle output dim
- `[` / `]` — adjust mixer pan left/right for the selected strip
- `a` — cycle grounded mixer assignments on the selected strip
- `l` — toggle currently grounded mixer link selectors only
- `3` — cycle preamp mode for the selected preamp input
- `p` — toggle preamp phase for the selected preamp input
- `s` — cycle sample rate
- `c` — cycle clock source
- `Left` / `Right` on preamp page — select `A1` or `A2`
- `m` on preamp page — toggle phantom for the selected preamp input
- `1` — switch to Monitor / HP1 surface
- `2` — switch to HP2 surface
- `?` — show quick help
- `q` — quit

## Raw frame view

The TUI now includes a raw-data page for live protocol inspection.

- keep pressing `Tab` until the raw page is focused
- the page shows the latest live `0x73`, `0x83`, `0x75`, and `0x81` packets
- each pane renders a full hex + ASCII dump of the current packet state
- press `b` on the raw page to capture a baseline
- press `x` to clear the baseline
- bytes that changed relative to the baseline are highlighted, which is useful when isolating mixer-strip-related changes in late `0x73`

This is especially useful while continuing reverse-engineering of late `0x73` mixer state and auxiliary traffic.

## Confirmed protocol support exposed in the app

- startup `0x74` queries and `0x75` metadata parsing
- grounded startup `0x75` query classification and summary surfacing for:
  - `query 0x01` = product metadata
  - `query 0x00` = short capability/default block (surfaced conservatively as raw-byte summary only)
  - `query 0x11` = small capability/status value (surfaced conservatively as raw-byte summary only)
- grounded startup `0x75` mixer readback for:
  - `query 0x03 / sub 0x05` = strip-assignment readback for strips `1..4`
  - `query 0x03 / sub 0x06..0x09` = ordinary-strip assignment readback for strips `5..16`
- grounded startup `0x75` selector-family diagnostics for:
  - `query 0x0b / sub 0x03` = asserted selector bitmap
  - `query 0x04 / sub 0x00..0x03` = selector pair banks shown conservatively in Raw view
- grounded startup visible-link seeding from `0x75 0b/03` for:
  - selector bits `0..7` = `MIX 1` pairs `1-2`, `3-4`, `5-6`, `7-8`, `9-10`, `11-12`, `13-14`, `15-16`
  - selector bits `16..23` = `MIX 2` pairs `1-2`, `3-4`, `5-6`, `7-8`, `9-10`, `11-12`, `13-14`, `15-16`
- grounded startup `0x75 04` pan/mute readback for:
  - `query 0x04 / sub 0x00` = visible `MIX 1` startup pan/state bytes
  - `query 0x04 / sub 0x01` = visible `MIX 2` startup pan/state bytes
- grounded startup `0x75 04` level readback for:
  - `query 0x04 / sub 0x00` = visible `MIX 1` startup level bytes at even offsets
  - `query 0x04 / sub 0x01` = visible `MIX 2` startup level bytes at even offsets
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
- mixer strip assignment (`0x70 / 0x53`, `d3 41`, early + ordinary write families documented)
- mixer pan (`0xd4 04 mm cc 00 pp` with scalar raw pan byte support)
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

Mixer protocol notes:

- the current dedicated mixer protocol summary lives in `docs/protocol/mixer-protocol.md`
- strip source assignment is now documented as a separate `0x70 / 0x53` table-write family with early and ordinary strip variants
- assignment is shared across mixer surfaces, while link state and level are treated as surface-local
- the app now tracks `16` strips per surface with shared assignment plus per-surface level, mute, pan, and link overlay state
- mixer pan host encoding is now modeled as a scalar raw value over the grounded `0x02 .. 0x3e` range rather than only anchor states
- mixer assignment cycling now uses the grounded per-strip write map across `CH1..16`
- the assignment write path now covers early `bb = 0x05`, ordinary `CH5..8` `bb = 0x03/06/07/08/09`, and ordinary `CH9..16` `bb = 0x06/07/08/09`
- assignment writes now serialize the full current shared assignment table so untouched strips survive refresh/readback correctly
- the status pane now shows the grounded non-metadata startup `0x75` replies as conservative summaries instead of ignoring them
- mixer assignments now seed from grounded `0x75` readback instead of waiting for local write overlays
- startup visible link state for `CH1..16` now seeds from grounded `0x75 0b/03`
- startup pan and mute state now seed from grounded `0x75 04/00` for `MIX 1` and `0x75 04/01` for `MIX 2`
- startup levels now seed from the even-byte lanes of grounded `0x75 04/00` for `MIX 1` and `0x75 04/01` for `MIX 2`
- surface switching still triggers a refresh sweep so the newly selected surface can be inspected in Raw view while the remaining `0x75` mixer-state families are being decoded
- Raw `0x75` history now summarizes grounded selector-family replies instead of showing them only as opaque hex previews

## Known limitations and out-of-scope areas

- **lock indicator**: not safely decoded from current captures, so the UI marks it as unknown
- **mixer state decode from `0x73`**: full passive per-strip startup decode is still unresolved. The app now keeps assigned fader level separate from the narrow passive meter-related subset so live activity does not overwrite the stored level display.
- **preamp / DSP editing**: preamp gains, modes, phantom, and phase are now exposed from the newly isolated captures; richer DSP/preamp page behavior around `51 01 01`, `a2`, `d5`, and `d7` remains intentionally out of scope
- **link / unlink controls**: the `0x70 / 0x14` family is documented, but selector semantics are not resolved enough to expose as a normal interactive control yet
- **link / unlink controls**: the TUI now exposes link toggles across all visible adjacent pairs using the grounded selector pattern currently validated in captures; the directly observed selectors are fully grounded, and the remaining pairs align with the same now-promoted map
- **startup `0x75` blocks**: the app now reads back the grounded assignment subset from `0x03`, but it still intentionally does **not** decode the inner meaning of the `0x00` capability/default block or `0x11` status/capability value beyond conservative byte summaries, and it does not trust `0x18/00` for startup level/pan/mute yet
- **`0x04/*` and `0x0b/03`**: `0x0b/03` is now grounded for startup visible link state on `CH1..16`, and `0x04/00` plus `0x04/01` are grounded for startup level/pan/mute state
- **metering decode**: per-channel strip metering is now passively decoded from the shared `0x73` strip-lane window `0x8e..0x9d` and stays separate from stored level. The mixer view now shows the active mix raw meter lanes directly, and the preamp panel surfaces narrow observed input meters for `A1` from `0xce` and `A2` from `0xcf`. The UI presents all meters on a shared `-60..0 dB` scale, and values below that floor stay hidden instead of being shown as exactly `-60 dB`. Separate output-panel metering is still intentionally unresolved

## Verification expectations without hardware

Unit tests use `MockTransport` and do not require a real device.
Hardware validation still needs a connected Zen Go device.
