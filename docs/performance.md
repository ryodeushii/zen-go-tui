# Performance Verification

## Purpose

This document tracks the current performance-oriented verification flow for `zen-go-tui`.
It focuses on CPU-sensitive paths that were tuned without changing confirmed protocol behavior.

The repository now sets the default Cargo target to the current Linux host triple in `.cargo/config.toml`:

- `x86_64-unknown-linux-gnu`

That means the standard commands below can be run without repeating `--target x86_64-unknown-linux-gnu`.

## Verification steps

### 1. Native test suite

Run the library and binary tests with the repo default target:

```bash
cargo test --lib --bins
```

Expected result:

- all native tests pass
- ignored benchmark tests stay ignored unless explicitly requested
- plain `cargo test --lib --bins` now uses the repo-local Linux default target and should not require `--target x86_64-unknown-linux-gnu`

### 2. Poll-path benchmark

Measure the device poll loop with the synthetic backlog benchmark:

```bash
cargo test --release perf_poll_device_snapshot_backlog -- --ignored --nocapture
```

This exercises:

- frame parsing
- pending write confirmation
- snapshot observation
- dirty-state calculation

### 3. Full-frame UI benchmark

Measure complete UI rendering with the synthetic draw benchmark:

```bash
cargo test --release perf_draw_full_frame -- --ignored --nocapture
```

This exercises:

- full layout construction
- widget rendering
- frame submission through `ratatui::Terminal`

### 4. Regression checks for the tuned behavior

Run the targeted regressions that guard the recent CPU work:

```bash
cargo test poll_device_does_not_mark_identical_snapshot_dirty_when_view_is_unchanged -- --nocapture
cargo test snapshot_frame_parse_owned_preserves_raw_bytes -- --nocapture
```

These checks verify:

- identical snapshots do not force redraws when the visible state is unchanged
- owned frame parsing preserves raw packet bytes used by protocol inspection paths

## Benchmark report

### Synthetic benchmark results

| Benchmark | Baseline | Best tuning run | Latest validation run |
| --- | ---: | ---: | ---: |
| `perf_poll_device_snapshot_backlog` | `670 ns/frame` | `495 ns/frame` | `398 ns/frame` |
| `perf_draw_full_frame` | `169220 ns/frame` | `150337 ns/frame` | `162727 ns/frame` |

### Approximate improvement from baseline

- poll path: about `41%` lower synthetic cost in the latest validation run
- full-frame draw: about `4%` lower synthetic cost in the latest validation run
- full-frame draw best observed run during tuning: about `11%` lower than baseline

## Real-device verification

### Method

For live-device idle checks, prefer the built release binary instead of `cargo run` so the measurement excludes Cargo wrapper noise.

Build once:

```bash
cargo build --release
```

Then measure the built binary while the real device is connected:

- headless mode: `target/x86_64-unknown-linux-gnu/release/zen-go-tui --headless`
- interactive TUI mode: `target/x86_64-unknown-linux-gnu/release/zen-go-tui`

The retained measurements below were taken from steady-state idle samples after startup settled.

### Live-device report

| Mode | Pre-adaptive live baseline | Retained state |
| --- | ---: | ---: |
| Headless idle CPU | about `0.56%` avg | about `0.47%` avg |
| TUI idle CPU | about `0.81%` avg | about `0.53%` avg |

### Live-device tuning notes

- the retained improvement came from adaptive idle poll backoff, not from stretching the idle redraw cadence
- a later attempt to reduce TUI idle redraw frequency did not hold up in longer real-device measurements and was reverted
- the current retained scheduler uses:
  - active poll timeout: `16ms`
  - TUI idle poll timeout: `50ms`
  - headless idle poll timeout: `250ms`
  - idle backoff threshold: `1s`

## What changed during tuning

### Poll path

- `Controller::poll_device()` now reports dirty state only when visible state actually changes or a pending mutation confirmation updates local state
- the poll loop now uses owned frame parsing to avoid an extra raw-byte clone on every parsed frame
- snapshot application no longer clones the parsed snapshot just to both apply and store it

### Render path

- several hot layout helpers were changed from `Vec<Rect>` allocations to fixed-size arrays
- that reduces repeated heap work on full redraws while preserving the existing layout structure

## Safety constraints used during tuning

- no protocol write encoding was intentionally changed
- no device-facing behavior was widened beyond redraw and allocation optimization
- raw inspector behavior was preserved with dedicated regression coverage
- unit tests continue to use mocked dependencies only

## Notes

- benchmark numbers above are synthetic and intended for relative comparison between revisions, not as an absolute promise for every terminal or device state
- the draw benchmark shows normal run-to-run variance, so both best-observed and latest-validation numbers are recorded here
- real-device idle CPU checks are still useful for future PDCA loops, especially around redraw cadence and live HID traffic density
