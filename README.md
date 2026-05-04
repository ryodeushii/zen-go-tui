# zen-go-tui

Terminal UI for the Antelope Zen Go Synergy Core audio interface.

<p align="center">
  <img src="assets/mixer-strips-9-16.png" alt="Mixer view" width="600">
</p>

<details>
<summary>More screenshots</summary>

**Options**

![Options](assets/options.png)

**Preamp / Source Type**

![Preamp](assets/preamp-source-type.png)

**Profiles**

![Profiles](assets/profiles.png)

**Routing**

![Routing](assets/routing.png)

</details>

## What it is

`zen-go-tui` is a Rust-based terminal application that lets you control your Zen Go Synergy Core from the command line. It provides a real-time TUI for:

- **Outputs** — Monitor, HP1, HP2 volume, mute, dim
- **Mixer** — 16-channel stereo mixer with two surfaces (MIX 1 / MIX 2), per-strip level, mute, pan, link, and source assignment
- **Preamp** — A1/A2 gain, input mode (Mic / Line / Hi-Z), phantom power, phase invert
- **Meters** — per-strip and input peak meters on a shared scale
- **Profiles** — save and load device state to TOML profiles
- **Raw view** — live HID packet inspection with diff highlighting

All protocol communication is reverse-engineered from USB HID captures. See [docs/](docs/) for the full protocol reference.

## Requirements

- **Rust 1.70+** (edition 2021)
- **Linux** — primary supported platform
- **Antelope Zen Go Synergy Core** connected via USB

### Windows

A Windows build target exists (`x86_64-pc-windows-gnu`) but is **untested on real hardware**. If you try it, please report your experience in [Issues](https://github.com/ryodeushii/zen-go-tui/issues).

## Install

### From this repository

```bash
cargo install --git https://github.com/ryodeushii/zen-go-tui.git --bin zen-go-tui
```

This places the `zen-go-tui` binary in Cargo's bin directory.

## Build from source

```bash
git clone https://github.com/ryodeushii/zen-go-tui.git
cd zen-go-tui
cargo build --release
```

Output: `target/release/zen-go-tui`

### Windows cross-build (from Linux)

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --target x86_64-pc-windows-gnu --release
```

Requires a MinGW-w64 cross-toolchain.

## Run

### Device permissions (Linux)

To access the Zen Go without `sudo`, install the bundled udev rule:

```bash
sudo cp udev_rules/99-antelope.rules /etc/udev/rules.d/99-antelope.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Then unplug and reconnect the device.

### Real device

```bash
zen-go-tui
# or from source:
cargo run
```

### Mock mode (no device)

```bash
zen-go-tui --mock
# or:
cargo run -- --mock
```

## Keyboard controls

### Global

| Key | Action |
|-----|--------|
| `Tab` | Cycle focus between panels |
| `←` / `→` | Select output or mixer channel |
| `↑` / `↓` | Adjust level / gain / fader |
| `m` | Toggle mute |
| `d` | Toggle dim |
| `q` | Quit |
| `?` | Show quick help |
| `p` | Open profiles panel |
| `r` | Toggle routing panel |
| `O` | Toggle options panel |
| `R` | Refresh device state |
| `Ctrl+D` | Toggle raw view |
| `Ctrl+O` | Toggle options panel |
| `Ctrl+C` | Quit |

### Mixer (focus on mixer strips)

| Key | Action |
|-----|--------|
| `[` / `]` | Adjust mixer pan |
| `a` | Open source assignment picker |
| `o` | Toggle solo |
| `l` | Toggle link on visible pairs |

### Preamp (focus on preamp)

| Key | Action |
|-----|--------|
| `3` | Cycle input mode (Mic / Line / Hi-Z) |

### Global shortcuts

| Key | Action |
|-----|--------|
| `s` | Cycle sample rate (when clock source is internal) |
| `c` | Cycle clock source |
| `1` | Switch to Monitor / HP1 surface |
| `2` | Switch to HP2 surface |

### Raw view

| Key | Action |
|-----|--------|
| `b` | Capture baseline |
| `x` | Clear baseline |
| `←` / `→` | Cycle packets or scroll query replies |

### Profiles popup

| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate profiles |
| `Enter` | Load selected profile |
| `s` | Save profile |
| `r` | Rename profile |
| `d` | Delete profile |
| `Esc` | Close |

### Options popup

| Key | Action |
|-----|--------|
| `1` / `2` / `3` | Set refresh rate (15 / 30 / 60 fps) |
| `↑` / `↓` | Cycle peak threshold |
| `p` | Toggle peak meters |
| `h` / `H` | Cycle peak hold duration forward / back |
| `a` | Toggle auto-save |
| `Esc` | Close |

On the raw packet view page, `b` captures a baseline and `x` clears it.

## Protocol documentation

The full reverse-engineered protocol reference lives in [docs/](docs/):

- [Control Panel reference](docs/cpl.md)
- [Mixer protocol](docs/protocol/mixer-protocol.md)
- [Preamp protocol](docs/protocol/preamp-protocol.md)
- [Packet capture analysis](docs/protocol/pcap-analysis.md)
- [Performance tuning](PERFORMANCE_TUNING.md)

## Disclaimer

This project is **not affiliated with Antelope Audio**. All protocol details were derived independently from USB HID packet captures. Use at your own risk.

## License

MIT
