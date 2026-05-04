# Repository Atlas: antelope-analysis-gpt54

## Project Responsibility
Hosts a Rust-based terminal control panel and protocol-research workspace for the Antelope Zen Go Synergy Core audio interface. The repository combines a production TUI application (`zen-go-tui`) with a standalone protocol codec library (`antelope-protocol`), packet capture corpora, and reverse-engineering documentation.

## System Entry Points
- `Cargo.toml`: Workspace manifest defining `antelope-protocol` library and `zen-go-tui` library+binary targets.
- `src/main.rs`: Binary entrypoint — CLI parsing (`clap`), terminal lifecycle, reconnect loop, input/event routing, and `MouseAction` → `Command` translation.
- `src/lib.rs`: Library surface exporting `app`, `profile`, `terminal`, `transport`, and `ui` modules.
- `antelope-protocol/src/lib.rs`: Standalone protocol codec crate — frame parsing, command encoding, typed state models, startup query sequences.
- `antelope-protocol/src/codemap.md`: Protocol codec implementation map.
- `src/codemap.md`: Application layer implementation map.
- `src/ui/codemap.md`: TUI rendering and input pipeline map.

## Repository Directory Map
| Directory | Responsibility Summary | Detailed Map |
|-----------|------------------------|--------------|
| `antelope-protocol/src/` | Protocol definitions and HID codec: frame parsing, command encoding, typed device state models (sample rate, clock source, preamp, mixer, output), startup query sequences. Zero external dependencies beyond `thiserror`. | [View Map](antelope-protocol/src/codemap.md) |
| `src/` | Application runtime: CLI startup, device transport abstraction, controller/state layer with optimistic confirmation, profile persistence (TOML), terminal input normalization, and Ratatui rendering pipeline. | [View Map](src/codemap.md) |
| `src/ui/` | TUI presentation layer: layout computation, mouse hit-testing, `MouseAction` command enum, widget rendering (mixer strips, output cards, preamp bars, popups, raw packet viewer), styling utilities with terminal profile adaptation. | [View Map](src/ui/codemap.md) |

## Root Assets
- `docs/protocol/`: Reverse-engineering notes, capture plans, and open questions documenting discovered protocol behavior.
- `antelope_pcap/`: Packet capture corpus used to derive mixer, preamp, metering, and output semantics.
- `docs/zen-go-tui.md`, `docs/cpl.md`: Top-level product and protocol reference notes complementing the executable Rust implementation.
- `PERFORMANCE_TUNING.md`: Performance optimization findings and tuning parameters.
- `bench.js`: Benchmark wrapper for `cargo test --bench` output formatting.
- `udev_rules/`: Linux udev rules for Zen Go HID device access.

## Repository Flow
1. **Binary startup** (`src/main.rs`): Parses CLI flags (`--mock`, `--headless`, `profile save/load`), opens `HidTransport` or `MockTransport`, initializes terminal (alternate screen, raw mode, mouse capture), spawns background input reader thread.
2. **Bootstrap** (`src/app.rs::Controller::bootstrap()`): Sends `control_panel_startup_queries()` through transport, logs query requests, waits for device responses.
3. **Main loop** (`app_loop`): Collects input events from channel, dispatches keyboard/mouse to `handle_key_press`/`handle_mouse_event`, polls device for HID frames (`Controller::poll_device`), parses frames via `Frame::parse_owned`, reconciles state through `AppState::observe_frame` with optimistic confirmation of pending writes.
4. **Rendering** (`src/ui/render.rs::draw`): Renders `AppState` into Ratatui frame — titlebar (device status, metadata), mixer page (preamp bars, 16-channel mixer strips with meters/pan/mute/solo/link, output cards), popups (profiles, assignment picker, selector, routing, hotkeys), or raw packet viewer.
5. **Protocol boundary** (`antelope-protocol`): Encodes outgoing HID frames (`encode_command`, `encode_query`, `encode_mixer_assignment_frames_with_table`), parses incoming HID reports (`Frame::parse_owned` → `DeviceSnapshot`), provides typed domain models (`SampleRate`, `ClockSource`, `MixerChannelState`, `PreampInputState`, etc.).
6. **Profile persistence** (`src/profile.rs`): Captures device state into TOML-serializable `DeviceProfile`, validates channel completeness and stereo pair consistency, persists to `$XDG_CONFIG_HOME/zen-go-tui/profiles/`.

## Integration Points
- **Build/runtime**: Rust 2021 workspace with `resolver = "2"`. Dependencies: `clap` (CLI), `crossterm` + `ratatui` (terminal I/O/rendering), `hidapi` (hardware access), `anyhow`/`thiserror` (errors), `serde`/`toml` (profiles), `terminput`/`termprofile`/`tui-slider` (input/styling/slider widgets).
- **Hardware boundary**: Antelope Zen Go HID device (`VID=0x23e5`, `PID=0xa015`) accessed through `hidapi` with platform-specific features (`linux-static-hidraw` / `windows-native`).
- **Testing seam**: `MockTransport` provides hardware-free transport double; `antelope-protocol` crate has zero runtime dependencies beyond `thiserror`, enabling isolated codec testing.
- **Documentation boundary**: Protocol discoveries are corroborated against `docs/protocol/` and `antelope_pcap/` artifacts outside the executable crate.
