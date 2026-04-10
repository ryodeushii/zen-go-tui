# src/ui/

## Responsibility

Terminal UI rendering, layout computation, mouse event routing, and styling for the Antelope Zen Go Synergy Core TUI application. Translates `AppState` into ratatui frames, converts mouse coordinates into semantic `MouseAction` commands, and provides consistent visual theming across all UI surfaces.

## Design Patterns

### Four-Submodule Separation of Concerns

| Module | Role | Side Effects |
|--------|------|-------------|
| `layouts.rs` | Pure Rect computation, ratio-to-value conversions, viewport math | None — all pure functions |
| `mouse.rs` | Hit-testing, coordinate-to-action translation, slider ratio sampling | None — returns `Option<MouseAction>` |
| `render.rs` | Frame drawing, widget rendering, text composition | Writes to `ratatui::Frame` / `Buffer` |
| `styles.rs` | Block builders, chip styling, color theming, syntax highlighting | None — returns `Style`/`Block`/`Span` builders |

### Command Pattern (`MouseAction` enum)

`mod.rs` defines `MouseAction` — a ~40-variant enum encoding every mouse-driven interaction as a pure data value. Variants fall into categories:

- **View toggles**: `ToggleRawView`, `ToggleHotkeysPopup`
- **Popup lifecycle**: `OpenProfilesPopup`, `CloseProfilesPopup`, `OpenRoutingPopup`, `CloseRoutingPopup`, `CloseSelectorPopup`, `CloseAssignmentPicker`
- **Profile CRUD**: `SelectProfile`, `LoadSelectedProfile`, `StartSaveProfile`, `StartRenameProfile`, `DeleteSelectedProfile`
- **Output controls**: `SelectOutput`, `AdjustOutputLevel`, `SetOutputLevel`, `ToggleOutputDim`, `ToggleOutputMute`
- **Mixer strip controls**: `SelectMixerChannel`, `AdjustMixerLevel`, `SetMixerLevel`, `AdjustMixerPan`, `SetMixerPan`, `ToggleMixerMute`, `ToggleMixerSolo`, `ToggleMixerLink`, `OpenAssignmentPicker`, `PickAssignment`
- **Preamp controls**: `SelectPreampInput`, `AdjustPreampGain`, `SetPreampGain`, `OpenPreampModeSelector`, `CyclePreampMode`, `PickPreampMode`, `TogglePreampPhase`, `TogglePreampPhantom`
- **Selectors**: `OpenSampleRateSelector`, `OpenClockSourceSelector`, `PickSampleRate`, `PickClockSource`
- **Navigation**: `SelectPage`, `SelectSurface`, `PageMixerStripsLeft`, `PageMixerStripsRight`, `SelectRawPacketTab`, `SelectQueryReplyEntry`, `ScrollQueryReplyList`

Mouse events never mutate state directly — they produce `MouseAction` values consumed by the app's event loop.

### Priority-Dispatch Mouse Routing

`mouse_action()` in `mouse.rs` implements a priority cascade: popup states are checked first (hotkeys > profile editor > profiles popup > selector popup > assignment picker > routing popup), then falls through to mixer page regions. This ensures popups capture all clicks and dismiss on outside-click. Separate entry points exist for slider drag (`slider_mouse_action`) and scroll wheel (`slider_wheel_action`) to avoid button-hit conflicts.

### Ratio-Based Slider Abstraction

All sliders operate on normalized `[0.0, 1.0]` ratios computed from pixel coordinates (`slider_ratio_for_horizontal_point`, `slider_ratio_for_vertical_point`). Conversion functions map ratios to domain values:

- `output_step_from_ratio` → `[0, 96]` step range
- `mixer_level_from_ratio` → `[0, 90]` level range
- `pan_from_ratio` → `PanState::MIN..=MAX`
- `preamp_gain_from_ratio` → mode-dependent (Mic: 0–65, Line: -6–20, HiZ: 0–45)
- `level_db_ratio` → dB to ratio for display
- `meter_db_ratio_option` → meter dB to ratio for bar coloring

### Widget Rendering Pipeline

`draw(frame, state)` is the sole entry point (`render.rs:22`). Dispatch flow:

```
draw()
├── raw_view_open? → draw_raw_page() + draw_hotkeys_popup()
└── mixer_page path:
    ├── draw_titlebar()
    │   ├── device_panel_layout → render_device_header() + render_device_metadata()
    │   └── render_inspector_summary()
    └── draw_mixer_page()
        ├── draw_preamp_bar() → render_preamp_visual_widget() × 2
        ├── draw_mixer_main()
        │   ├── surface tab chips + PROFILES/ROUTING buttons
        │   ├── render_mixer_strip_widget() × viewport_capacity
        │   └── experimental_mix_meter() → render_mix_meter_widget()
        └── draw_output_panel() → render_output_card_widget() × 3
```

Popups overlay via conditional draws: `draw_routing_popup()`, `draw_profiles_popup()`, `draw_assignment_picker()`, `draw_selector_popup()`, `draw_hotkeys_popup()`.

## Data & Control Flow

### Layout Computation (layouts.rs)

All functions accept `Rect` and optionally `&AppState`, returning `[Rect; N]` or `Vec<Rect>`. Key layout hierarchies:

```
root_chunks (vertical: 3 / Min(17))
├── titlebar_layout (horizontal: Min(24) / Length(21))
│   ├── device_panel_layout (horizontal: Min(24) / dynamic metadata)
│   └── inspector panel
└── mixer_page_layout (vertical: Min(14) / Length(8))
    ├── mixer_main_layout (vertical: Length(5) / Min(12))
    │   ├── preamp_bar_layout (horizontal: 50% / 50%)
    │   └── mixer_layout (vertical: Length(3) / Min(9))
    │       ├── mixer header (tabs + buttons)
    │       └── mixer_strip_panel_layout (optional mix meter row)
    └── output_panel (3 cards at ~33% each)
```

Viewport scrolling: `mixer_strip_visible_bounds()` computes `(start, end)` from scroll position and `mixer_strip_viewport_capacity_for_inner()`, which divides available width by `MIXER_STRIP_CARD_WIDTH + MIXER_STRIP_GAP`.

Popup positioning: All popups use centered placement via `area.x + area.width.saturating_sub(width) / 2` pattern with min/max clamping.

### Mouse Event Resolution (mouse.rs)

Three public entry points, each recomputing layout from root `Rect`:

1. **`mouse_action(area, state, x, y)`** — click/tap dispatch. Checks popup states in priority order, then hits device header, raw view tabs, output cards, mixer tabs, mixer panel buttons, mixer strip cards, and preamp cards.

2. **`slider_mouse_action(area, state, x, y)`** — drag on slider tracks. Guards against all popup states. Checks output sliders → mixer sliders → preamp sliders. Computes ratio from point, converts to domain value via `*_from_ratio` functions.

3. **`slider_wheel_action(area, state, x, y, increase)`** — scroll wheel. Same guards as above plus special handling for `RawPacketTab::Query75` (scrolls reply list). Checks output wheel hitboxes → mixer wheel hitboxes → preamp wheel hitboxes.

Hit-testing uses `contains_point(Rect, (u16, u16))` — simple AABB check. Slider hitboxes are expanded by `wheel_hitbox()` to minimum 5-cell width for reliable wheel targeting.

Mixer strip hit-testing iterates visible slots, computing per-card rects via `mixer_strip_card_area()`, then testing sub-regions: pan slider → level slider → assignment source chip → L/S/M buttons.

### Experimental Mix Meter

Both `mouse.rs:848` and `render.rs:1299` contain identical `experimental_mix_meter()` functions (duplicate). Reads raw `0x73` packet bytes at payload offsets `0x6a` (surface discriminator), `0xda`/`0xdb` (MIX 1 L/R), `0xde`/`0xdf` (MIX 2 L/R). Surface `0x0f` = MIX 1, `0x0c` = MIX 2. Used to conditionally render a 2-row meter bar below mixer strips.

## Integration Points

### Upstream Dependencies

- **`crate::app::AppState`** — single source of truth. All layout and render functions consume `&AppState`. Mouse functions return `MouseAction` for the app event loop to apply.
- **`antelope_protocol`** — protocol types: `MixerAssignment`, `PanState`, `PreampMode`, `PreampInputState`, `SampleRate`, `ClockSource`, `Surface`, `OutputState`, `OutputMode`, `MixerChannelState`, `MixerSurface`. Also `meter_db_ratio`, `meter_ratio`, `meter_display_db` for meter conversion.
- **`crate::terminal`** — `adapt_color()` and `adapt_style()` for terminal profile compatibility (light/dark mode adaptation). Applied universally in `styles.rs` and widget rendering.
- **`ratatui`** — core TUI framework: `Rect`, `Layout`, `Constraint`, `Frame`, `Buffer`, `Paragraph`, `List`, `ListItem`, `ListState`, `Block`, `Borders`, `Clear`, `Widget`, `Wrap`, `Line`, `Span`, `Text`, `Style`, `Color`, `Modifier`, `Alignment`.
- **`tui_slider`** — `Slider`, `SliderOrientation`, `SliderState` for horizontal level bars.

### Downstream Consumers

- **`crate::app`** — consumes `MouseAction` variants from mouse event handlers. Calls `draw(frame, state)` each frame tick. Uses `profile_editor_cursor()` for cursor positioning during profile rename/save.
- **`crate::terminal`** — receives styled widgets from render functions.

### Re-exported Public API (`mod.rs`)

```rust
pub use mouse::mixer_strip_panel_contains;  // Region containment check
pub use mouse::mixer_strip_viewport_capacity;  // Visible strip count
pub use mouse::mouse_action;                // Click → MouseAction
pub use mouse::slider_mouse_action;         // Drag → MouseAction
pub use mouse::slider_wheel_action;         // Scroll → MouseAction
pub use render::draw;                       // Frame entry point
pub use render::profile_editor_cursor;      // Cursor position for text input
```

### Key Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `MIXER_STRIP_CARD_WIDTH` | 18 | Fixed width per mixer strip card |
| `MIXER_STRIP_GAP` | 1 | Horizontal gap between strip cards |
| `MIXER_STRIP_DB_MARKERS` | `[0, 5, 10, 15, 20, 30, 40, 60]` | dB scale labels on vertical combo strip |
| `SIGNAL_LABEL_WIDTH` | 12 | Reserved width for slider labels (e.g. "GAIN -12 dB") |
| `MAX_SIGNAL_ROW_WIDTH` | 40 | Maximum width for signal meter rows |
| `CONNECTION_STALE_AFTER` | 2s | Timeout before connection badge turns red |
| `MIX_METER_YELLOW_START_RATIO` | 0.8 | Meter bar color transition threshold |
| `MIX_METER_RED_START_RATIO` | 0.95 | Meter bar clip threshold |
| `MIX_METER_CHANNEL_LABEL_WIDTH` | 2 | Width for "L"/"R" label in mix meter |
| `MIX_METER_DB_WIDTH` | 7 | Width for dB value in mix meter |
| `ADJUST_DOWN_BUTTON_LABEL` | "↓" | Decrement button symbol |
| `ADJUST_UP_BUTTON_LABEL` | "↑" | Increment button symbol |
