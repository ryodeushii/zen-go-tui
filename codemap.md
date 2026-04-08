# Repository Atlas: antelope-analysis-gpt54

## Project Responsibility
Hosts a Rust-based terminal control panel and protocol-research workspace for the Antelope Zen Go Synergy Core. The repository combines a production TUI application with captured packet corpora and protocol notes used to decode the device's HID control surface.

## System Entry Points
- `Cargo.toml`: crate manifest defining the `zen-go-tui` library and binary targets plus runtime dependencies.
- `src/main.rs`: binary entrypoint, terminal session lifecycle, reconnect loop, and event-to-command routing.
- `src/lib.rs`: library surface exporting the application, protocol, transport, and UI modules.
- `src/codemap.md`: detailed implementation map for the Rust application layer.

## Repository Directory Map
| Directory | Responsibility Summary | Detailed Map |
|-----------|------------------------|--------------|
| `src/` | Implements the runtime shell, controller/state layer, HID protocol codec, transport abstraction, and Ratatui views for the Zen Go control panel. | [View Map](src/codemap.md) |

## Root Assets
- `docs/protocol/`: reverse-engineering notes, capture plans, and open questions that document discovered protocol behavior.
- `antelope_pcap/`: packet capture corpus used to derive mixer, preamp, metering, and output semantics.
- `docs/zen-go-tui.md`, `docs/cpl.md`, and `docs/wireshark-capture-plan.md`: top-level product and capture-analysis notes that complement the executable Rust implementation.

## Repository Flow
1. The binary defined by `Cargo.toml` starts in `src/main.rs` and opens either the HID-backed transport or the mock transport.
2. `src/app.rs` sends the captured startup query sweep, polls HID frames, and keeps the TUI state synchronized with decoded device snapshots.
3. `src/protocol.rs` converts between raw HID frames and typed device/domain models used by the controller and UI.
4. `src/ui.rs` renders the decoded model and maps user interactions back into controller commands.
5. Packet captures in `antelope_pcap/` and research notes in `docs/` support ongoing refinement of the codec and the TUI's device-state interpretation.

## Integration Points
- Build/runtime toolchain: Rust 2021 via Cargo.
- Hardware boundary: Antelope Zen Go HID device accessed through `hidapi`.
- Documentation boundary: protocol discoveries are corroborated against `docs/protocol/` and `antelope_pcap/` artifacts outside the executable crate.
