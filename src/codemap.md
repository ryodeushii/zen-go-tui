# src/

## Responsibility
Implements the complete Rust application for the Zen Go terminal control panel: CLI startup, device transport, Antelope protocol encoding/decoding, application state reconciliation, and Ratatui rendering.

File responsibilities:
- `main.rs`: runtime entrypoint, terminal lifecycle, keyboard/mouse dispatch, reconnect handling.
- `app.rs`: controller layer and canonical `AppState`; translates UI intent into device writes and reduces decoded frames back into UI state.
- `protocol.rs`: protocol model and codec layer for HID snapshots, query replies, notifications, and outbound host commands.
- `transport.rs`: `Transport` abstraction plus HID-backed and mock implementations.
- `ui.rs`: presentational layer and mouse hit-testing derived from `AppState`.
- `lib.rs`: module export surface.

## Design Patterns
- Layered architecture: `main` orchestrates runtime concerns, `app` owns control/state transitions, `protocol` isolates byte-level semantics, `transport` owns I/O, and `ui` renders state.
- Dependency inversion via the `Transport` trait, allowing `Controller` to operate against either `HidTransport` or `MockTransport` without hardware-specific branching.
- Controller pattern: `Controller` is the single mutation boundary for device writes, startup queries, polling, and post-write reconciliation.
- Typed protocol model: enums such as `SampleRate`, `ClockSource`, `Surface`, `MixerSurface`, `MixerAssignment`, and `Command` prevent raw protocol bytes from leaking into higher layers.
- Optimistic confirmation pattern: outbound commands create a `PendingMutation`, and the next `0x73` snapshot confirms and normalizes the state reflected in `AppState`.
- State projection for reverse engineering: `AppState` stores decoded device state alongside raw `0x73`/`0x74`/`0x75`/`0x81`/`0x83` payloads, baselines, and query history to support protocol discovery from the TUI itself.

## Data & Control Flow
1. `main()` parses `--mock`, opens `HidTransport::open(...)` or `MockTransport::default()`, and calls `run_app()`.
2. `run_app()` initializes the alternate-screen terminal, constructs `Controller::new(transport)`, and triggers `Controller::bootstrap()`.
3. `Controller::bootstrap()` calls `refresh_queried_state()`, which iterates `protocol::control_panel_startup_queries()`, serializes each request with `encode_query()`, logs it into `AppState`, and writes it through `Transport::write()`.
4. The main loop in `app_loop()` continuously calls `Controller::poll_device()`, then `ui::draw(frame, &controller.state)`, then processes keyboard or mouse input.
5. `Controller::poll_device()` drains up to `MAX_FRAMES_PER_POLL` HID packets, parses each one through `Frame::parse()`, converts it into `DeviceSnapshot`, runs `confirm_pending_write()` for `0x73` snapshots, and forwards the raw plus decoded frame into `AppState::observe_frame()`.
6. `AppState::observe_frame()` updates connection status, device metadata, startup-query summaries, output state, preamp state, mixer strip state, raw packet logs, and the active mixer surface. `apply_snapshot()` also performs passive mixer decode so metering/mute/link observations can be projected from snapshot payloads.
7. User actions originate in `main.rs` keyboard handlers or `ui.rs` mouse hit-testing and are translated into `Command` values or local UI-only state transitions.
8. `Controller::send()` serializes normal commands with `encode_command()`. Mixer assignment writes use `encode_mixer_assignment_frames_with_table()` to send banked `0x70` frames, and link writes may emit both `encode_link_companion()` and the primary `SetLinkState` frame.
9. After writes, `pending_from_command()` records the expected mutation so the next parsed snapshot can reconcile levels, mute/solo/link flags, output modes, or DSP cluster-backed preamp values into the authoritative UI model.

## Integration Points
- Consumed by: the crate binary target `zen-go-tui` from `src/main.rs` and the library target exported through `src/lib.rs`.
- Depends on: `clap` for CLI parsing, `crossterm` and `ratatui` for terminal I/O and rendering, `hidapi` for hardware access, `anyhow`/`thiserror` for error handling.
- External boundary: Antelope Zen Go HID reports. `transport.rs` opens the physical device, while `protocol.rs` defines the framed command/query language and snapshot parsing.
- Internal dependencies:
  - `main.rs` depends on `app`, `protocol`, `transport`, and `ui`.
  - `app.rs` depends on `protocol` for domain commands and frame decoding and on `transport` for I/O.
  - `ui.rs` depends on `app::AppState` and protocol-domain enums for presentation labels and action routing.
- Testing seam: `MockTransport` provides a hardware-free transport double so controller and codec tests can validate behavior without using the real HID device.
