# Split app.rs into Focused Modules

## Discovery

### Original Request
- "let's go with p2 1" — referring to Priority 2 task #1: "Split app.rs into focused modules"

### Current State
- `src/app.rs` is 5161 lines, single file
- No `src/app/` directory exists yet
- 8 files import from `crate::app::*`: `settings.rs`, `ui/mod.rs`, `ui/render.rs`, `ui/tests.rs`, `ui/mouse.rs`, `ui/widgets/mixer.rs`, `profile.rs`, `ui/layouts.rs`
- `lib.rs` declares `pub mod app;` and re-exports `QUERY_REPLY_VISIBLE_COUNT`

### Proposed Module Structure
```
src/app/
  mod.rs          — re-exports, AppState struct + Default impls, QUERY_REPLY_VISIBLE_COUNT
  types.rs        — Intent enum + impl, FocusArea, RawPacketTab, MainPage, PendingMutation
  state.rs        — DeviceStatus, ConnectionState, DeviceState, MixerState, OutputData, PreampData, UiState, PopupState, RawViewState, MeterPeak, AppSettings, RefreshRate, PeakHoldDuration
  controller.rs   — Controller struct + all impl methods (new, bootstrap, send, apply_intent, etc.)
  profile_editor.rs — ProfileEditorMode, ProfileEditorState, StructuralSnapshot impl
  picker.rs       — AssignmentPickerState, SelectorPopupKind, SelectorPopupState, QueryReplyLogEntry
```

### Line Mapping (current app.rs → target modules)

| Lines | Content | Target Module |
|-------|---------|---------------|
| 1-16 | Imports | Each module gets its own imports |
| 18-43 | DeviceStatus + Default | state.rs |
| 45-50 | ConnectionState | state.rs |
| 52-383 | Intent enum + impl | types.rs |
| 385-391 | FocusArea | types.rs |
| 393-400 | RawPacketTab | types.rs |
| 402-405 | MainPage | types.rs |
| 407-410 | AssignmentPickerState | picker.rs |
| 412-417 | SelectorPopupKind | picker.rs |
| 419-422 | SelectorPopupState | picker.rs |
| 424-428 | QueryReplyLogEntry | picker.rs |
| 430-434 | ProfileEditorMode | profile_editor.rs |
| 436-453 | ProfileEditorState | profile_editor.rs |
| 455-470 | StructuralSnapshot impl | profile_editor.rs |
| 472-508 | RefreshRate + impl | state.rs |
| 510-546 | PeakHoldDuration + impl | state.rs |
| 548-586 | AppSettings + Default + impl | state.rs |
| 588-594 | DeviceState | state.rs |
| 596-604 | MixerState | state.rs |
| 606-611 | OutputData | state.rs |
| 613-619 | PreampData | state.rs |
| 621-629 | UiState | state.rs |
| 631-644 | PopupState | state.rs |
| 646-665 | RawViewState | state.rs |
| 667-676 | AppState | mod.rs |
| 678-749 | Default impls (MixerState, OutputData, UiState, RawViewState) | state.rs |
| 751-762 | MeterPeak + impl | state.rs |
| 764-1299 | impl AppState (prune_expired_peaks, apply_snapshot, observe_frame, etc.) | mod.rs |
| 1301-1394 | PendingMutation enum | types.rs |
| 1396-1403 | Controller struct | controller.rs |
| 1405-5161 | impl Controller (all 46 methods) | controller.rs |

---

## Non-Goals (What we're NOT building)
- No behavioral changes — this is pure refactoring
- No new tests needed (existing tests must continue to pass)
- No API changes — all public types remain accessible via `crate::app::*`
- No splitting of Controller itself (that's a separate future task)

---

## Tasks

### 1. Create module directory and mod.rs

**Depends on**: none

**Files:**
- Create: `src/app/mod.rs`
- Modify: `src/lib.rs` (change `pub mod app;` → `pub mod app;` stays same, but mod.rs becomes directory module)

**What to do**:
1. Create `src/app/` directory
2. Move `src/app.rs` → `src/app/mod.rs` temporarily (we'll split it next)
3. Verify build still works:
   - Run: `cargo build`
   - Expected: 0 errors, 0 warnings
4. Commit:
   ```bash
   git add src/app/mod.rs src/lib.rs
   git commit -m "refactor: convert app.rs to app/ module directory"
   ```

**Verify**:
- [ ] `cargo build` → 0 errors
- [ ] `cargo test` → all pass

### 2. Extract types.rs — Intent, enums, PendingMutation

**Depends on**: 1

**Files:**
- Create: `src/app/types.rs`
- Modify: `src/app/mod.rs` — remove lines 52-383 (Intent), 385-405 (FocusArea/RawPacketTab/MainPage), 1301-1394 (PendingMutation)

**What to do**:
1. Create `src/app/types.rs` with:
   - All imports needed for Intent, FocusArea, RawPacketTab, MainPage, PendingMutation
   - `Intent` enum (all variants) + `impl Intent` block (pending_mutation method)
   - `FocusArea` enum
   - `RawPacketTab` enum
   - `MainPage` enum
   - `PendingMutation` enum
2. In `mod.rs`, add `mod types;` and `pub use types::*;`
3. Remove the moved content from `mod.rs`
4. Verify:
   - Run: `cargo build`
   - Expected: 0 errors
   - Run: `cargo test`
   - Expected: all pass
5. Commit:
   ```bash
   git add src/app/types.rs src/app/mod.rs
   git commit -m "refactor: extract Intent, enums, PendingMutation into types.rs"
   ```

**Verify**:
- [ ] `cargo build` → 0 errors
- [ ] `cargo test` → all pass

### 3. Extract state.rs — All state structs

**Depends on**: 2

**Files:**
- Create: `src/app/state.rs`
- Modify: `src/app/mod.rs` — remove state struct definitions and Default impls

**What to do**:
1. Create `src/app/state.rs` with:
   - All imports needed for state structs
   - `DeviceStatus` + `impl Default`
   - `ConnectionState`
   - `RefreshRate` + `impl` (all, label, fps, loop_sleep_ms)
   - `PeakHoldDuration` + `impl` (all, label, duration)
   - `AppSettings` + `impl Default` + `impl` (peak_threshold_db)
   - `DeviceState`, `MixerState`, `OutputData`, `PreampData`, `UiState`, `PopupState`, `RawViewState`
   - `impl Default` for MixerState, OutputData, UiState, RawViewState
   - `MeterPeak` + `impl` (is_active)
2. In `mod.rs`, add `mod state;` and `pub use state::*;`
3. Remove the moved content from `mod.rs`
4. Verify:
   - Run: `cargo build`
   - Expected: 0 errors
   - Run: `cargo test`
   - Expected: all pass
5. Commit:
   ```bash
   git add src/app/state.rs src/app/mod.rs
   git commit -m "refactor: extract state structs into state.rs"
   ```

**Verify**:
- [ ] `cargo build` → 0 errors
- [ ] `cargo test` → all pass

### 4. Extract picker.rs — UI picker/popup state types

**Depends on**: 2

**Files:**
- Create: `src/app/picker.rs`
- Modify: `src/app/mod.rs` — remove picker types

**What to do**:
1. Create `src/app/picker.rs` with:
   - `AssignmentPickerState`
   - `SelectorPopupKind`
   - `SelectorPopupState`
   - `QueryReplyLogEntry`
2. In `mod.rs`, add `mod picker;` and `pub use picker::*;`
3. Remove the moved content from `mod.rs`
4. Verify:
   - Run: `cargo build`
   - Expected: 0 errors
5. Commit:
   ```bash
   git add src/app/picker.rs src/app/mod.rs
   git commit -m "refactor: extract picker/popup state types into picker.rs"
   ```

**Verify**:
- [ ] `cargo build` → 0 errors

### 5. Extract profile_editor.rs — Profile editor types

**Depends on**: 2

**Files:**
- Create: `src/app/profile_editor.rs`
- Modify: `src/app/mod.rs` — remove profile editor types

**What to do**:
1. Create `src/app/profile_editor.rs` with:
   - `ProfileEditorMode` enum
   - `ProfileEditorState` struct
   - `impl StructuralSnapshot for ProfileEditorState`
2. In `mod.rs`, add `mod profile_editor;` and `pub use profile_editor::*;`
3. Remove the moved content from `mod.rs`
4. Verify:
   - Run: `cargo build`
   - Expected: 0 errors
5. Commit:
   ```bash
   git add src/app/profile_editor.rs src/app/mod.rs
   git commit -m "refactor: extract profile editor types into profile_editor.rs"
   ```

**Verify**:
- [ ] `cargo build` → 0 errors

### 6. Extract controller.rs — Controller struct and all impl methods

**Depends on**: 2, 3, 4, 5

**Files:**
- Create: `src/app/controller.rs`
- Modify: `src/app/mod.rs` — remove Controller struct and impl block

**What to do**:
1. Create `src/app/controller.rs` with:
   - All imports needed for Controller (Transport, CommandQueue, DeviceProfile, antelope_protocol types, etc.)
   - `Controller` struct definition
   - `impl Controller` block (all 46 methods: new, bootstrap, transport_available, refresh_queried_state, apply_profile, send, flush_commands, send_mixer_level_change, send_mixer_mute_change, send_mixer_solo_change, send_mixer_link_change, apply_intent, poll_device, confirm_pending_write, etc.)
2. In `mod.rs`, add `mod controller;` and `pub use controller::Controller;`
3. Remove the moved content from `mod.rs`
4. Verify:
   - Run: `cargo build`
   - Expected: 0 errors
   - Run: `cargo test`
   - Expected: all pass
5. Commit:
   ```bash
   git add src/app/controller.rs src/app/mod.rs
   git commit -m "refactor: extract Controller into controller.rs"
   ```

**Verify**:
- [ ] `cargo build` → 0 errors
- [ ] `cargo test` → all pass

### 7. Finalize mod.rs — Clean re-exports and AppState impl

**Depends on**: 6

**Files:**
- Modify: `src/app/mod.rs`

**What to do**:
1. `mod.rs` should contain only:
   - Module declarations (`mod types;`, `mod state;`, `mod controller;`, `mod picker;`, `mod profile_editor;`)
   - Re-exports (`pub use types::*;`, `pub use state::*;`, `pub use controller::Controller;`, `pub use picker::*;`, `pub use profile_editor::*;`)
   - `AppState` struct definition
   - `impl AppState` block (all methods: prune_expired_peaks, startup_query_summary, selected_query_reply_entry, active_mixer_surface, active_mixer_channels, clamp_mixer_strip_scroll, ensure_selected_mixer_channel_visible, scroll_mixer_strip_viewport, page_mixer_strip_viewport, snapshot_structurally_differs, apply_snapshot, observe_frame, mark_disconnected, cycle_focus, toggle_raw_view, toggle_hotkeys_popup, toggle_options_popup, selected_profile_name, clamp_profile_selection, cycle_raw_packet, cycle_query_reply_entry, capture_raw_baseline, clear_raw_baseline, observe_query_request)
   - `QUERY_REPLY_VISIBLE_COUNT` constant
   - Helper function `startup_query_slot`
2. Verify:
   - Run: `cargo build`
   - Expected: 0 errors, 0 warnings
   - Run: `cargo test`
   - Expected: all pass (same count as before)
3. Commit:
   ```bash
   git add src/app/mod.rs
   git commit -m "refactor: finalize mod.rs with clean re-exports"
   ```

**Verify**:
- [ ] `cargo build` → 0 errors, 0 warnings
- [ ] `cargo test` → all pass (same count as baseline)
- [ ] No external import paths broken (check all 8 importing files still compile)

### 8. Update task memory and kanban

**Depends on**: 7

**Files:**
- Modify: memory note for this task
- Modify: kanban board

**What to do**:
1. Update the task note `Split app.rs into focused modules` → status: done
2. Add to kanban Done column: "Split app.rs (5161 lines) into app/ module directory with 5 focused modules (types.rs, state.rs, controller.rs, picker.rs, profile_editor.rs) + mod.rs"
3. Update Architectural Improvement Plan — mark #2 as done (if not already)

**Verify**:
- [ ] Task note status = done
- [ ] Kanban updated
