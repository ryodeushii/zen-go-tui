# RAW Packet Semantic Map Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an exact, color-coded semantic map to the root Ratatui RAW view without changing protocol decoding or hiding unresolved bytes.

**Architecture:** Keep packet bytes in the existing offset-preserving dump. Add a UI-only descriptor model that maps report ranges to grounded fields, readbacks, observations, parser fields, unresolved bytes, and fixed padding. Render descriptors beside the dump at 140x40 and stack compact descriptors above the scrolling dump at 80x24. Keep map scope and scroll state in the application state so keyboard and mouse handlers do not depend on UI rendering modules.

**Tech Stack:** Rust 2021, Ratatui 0.30, crossterm 0.29, terminput, TestBackend, existing `cargo test` and Clippy checks.

**Spec:** `docs/superpowers/specs/2026-08-31-raw-packet-map-design.md`

## Global Constraints

- Preserve the exact 320-byte report dump.
- Use UI-only descriptors in `src/ui/raw_map.rs`.
- Use existing protocol constants and query shape guards.
- Do not change protocol parsing, encoding, transport, device commands, or Slint UI.
- Use exact report and payload offsets in every mapped entry.
- Use exact field labels such as `CH07 observed meter lane`, `CH01 assignment`, `HP1 output mode`, `preamp 2 phase bit`, and `Mix1 CH01/CH02 link correlation`.
- Treat mixed mixer bytes as correlation groups, not standalone controls.
- Use `USED`, `READBACK`, `OBSERVED`, `PARSER`, `UNMAPPED`, and `PADDING` coverage values.
- Use fixed `PADDING` only where source length is known.
- Keep baseline changes as a separate underline and background signal.
- Keep packet-tab behavior and Query75 history behavior unchanged.
- Do not weaken existing raw dump, query panel, mouse, or layout tests.
- Run formatter, targeted tests, workspace tests, Clippy, LSP diagnostics, and `lens_diagnostics` before completion.
- Verify the headed TUI through a unique tmux session and uniquely titled Kitty window when available.

## File Structure

- Create `src/ui/raw_map.rs` for coverage values, map scopes, ranges, descriptor construction, query guards, overlap detection, and complement derivation.
- Modify `src/app/types.rs` for `RawMapScope` and raw map, scope, and scroll intents.
- Modify `src/app/state.rs` for selected scope and map and dump scroll state.
- Modify `src/app/controller.rs` for scope selection, scope cycling, scroll handling, and reset calls.
- Modify `src/terminal.rs` for PageUp and PageDown normalization.
- Modify `src/ui/styles.rs` for coverage-aware hex and ASCII styles.
- Modify `src/ui/layouts.rs` for semantic subtab hit areas and responsive map and dump regions.
- Modify `src/ui/mouse.rs` for subtab clicks and raw dump wheel scrolling.
- Modify `src/ui/render/text.rs` for map text, coverage-aware dump rows, and selected Query75 bytes.
- Modify `src/ui/render/mod.rs` for semantic subtabs, legend, map and dump panes, and scroll offsets.
- Modify `src/ui/mod.rs` to declare `raw_map`.
- Add focused tests in `src/ui/raw_map.rs` and `src/ui/tests.rs`.
- Modify `docs/zen-go-tui.md` only in its RAW view section.
- Do not modify `antelope-protocol` or `zen-go-slint`.

---

### Task 1: Add raw scope and scroll state

**Files:**
- Modify: `src/app/types.rs`
- Modify: `src/app/state.rs`
- Modify: `src/app/controller.rs`
- Test: `src/app/types.rs` and `src/ui/tests.rs`

**Interfaces:**
- `RawMapScope::options_for(tab: RawPacketTab) -> &'static [RawMapScope]` returns the approved scope list for one packet tab.
- `RawMapScope::label(self) -> &'static str` returns the displayed scope label.
- `RawViewState::select_tab(&mut self, tab: RawPacketTab)` selects a packet, resets invalid scope to `All`, and resets both scroll values.
- `RawViewState::select_scope(&mut self, scope: RawMapScope)` accepts only a scope offered by the selected packet and resets both scroll values.
- `RawViewState::cycle_scope(&mut self, forward: bool)` cycles the selected packet scope and resets both scroll values.
- `RawViewState::scroll_raw_view(&mut self, increase: bool, page: bool)` changes dump and map scroll by one row or ten rows.
- `Intent::SelectRawMapScope(RawMapScope)` selects a semantic scope.
- `Intent::CycleRawMapScope { forward: bool }` cycles semantic scopes.
- `Intent::ScrollRawDump { increase: bool, page: bool }` scrolls map and dump content without importing layout code into `app`.

- [ ] **Step 1: Write failing scope and reset tests**

Add tests that define the state contract before implementation:

```rust
#[test]
fn raw_scope_options_match_packet_kind() {
    assert_eq!(
        RawMapScope::options_for(RawPacketTab::State73),
        &[
            RawMapScope::All,
            RawMapScope::Base,
            RawMapScope::Outputs,
            RawMapScope::Preamps,
            RawMapScope::Mixer,
            RawMapScope::Unmapped,
        ]
    );
    assert_eq!(
        RawMapScope::options_for(RawPacketTab::Query75),
        &[
            RawMapScope::All,
            RawMapScope::Metadata,
            RawMapScope::Mixer,
            RawMapScope::Status,
            RawMapScope::Unmapped,
        ]
    );
}

#[test]
fn selecting_packet_resets_unsupported_scope_and_scroll() {
    let mut state = AppState::default();
    state.raw_view.raw_map_scope = RawMapScope::Mixer;
    state.raw_view.raw_dump_scroll = 7;
    state.raw_view.raw_map_scroll = 4;

    state.raw_view.select_tab(RawPacketTab::DeviceNotification);

    assert_eq!(state.raw_view.raw_map_scope, RawMapScope::All);
    assert_eq!(state.raw_view.raw_dump_scroll, 0);
    assert_eq!(state.raw_view.raw_map_scroll, 0);
}
```

- [ ] **Step 2: Run the focused tests and verify failure**

Run:

```bash
cargo test -p zen-go-tui raw_scope_options --lib
cargo test -p zen-go-tui selecting_packet_resets_unsupported_scope_and_scroll --lib
```

Expected result: compilation fails because `RawMapScope` and the new `RawViewState` fields and methods do not exist.

- [ ] **Step 3: Add `RawMapScope` and its packet-specific options**

Add the enum and static scope lists to `src/app/types.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawMapScope {
    All,
    Base,
    Outputs,
    Preamps,
    Mixer,
    Query,
    Metadata,
    Status,
    Parser,
    Unmapped,
}

impl RawMapScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Base => "BASE",
            Self::Outputs => "OUTPUTS",
            Self::Preamps => "PREAMPS",
            Self::Mixer => "MIXER",
            Self::Query => "QUERY",
            Self::Metadata => "METADATA",
            Self::Status => "STATUS",
            Self::Parser => "PARSER",
            Self::Unmapped => "UNMAPPED",
        }
    }

    pub fn options_for(tab: RawPacketTab) -> &'static [Self] {
        const STATE73: &[RawMapScope] = &[
            RawMapScope::All,
            RawMapScope::Base,
            RawMapScope::Outputs,
            RawMapScope::Preamps,
            RawMapScope::Mixer,
            RawMapScope::Unmapped,
        ];
        const QUERY74: &[RawMapScope] = &[
            RawMapScope::All,
            RawMapScope::Query,
            RawMapScope::Mixer,
            RawMapScope::Unmapped,
        ];
        const QUERY75: &[RawMapScope] = &[
            RawMapScope::All,
            RawMapScope::Metadata,
            RawMapScope::Mixer,
            RawMapScope::Status,
            RawMapScope::Unmapped,
        ];
        const PARSER_ONLY: &[RawMapScope] = &[
            RawMapScope::All,
            RawMapScope::Parser,
            RawMapScope::Unmapped,
        ];

        match tab {
            RawPacketTab::State73 => STATE73,
            RawPacketTab::Query74 => QUERY74,
            RawPacketTab::Query75 => QUERY75,
            RawPacketTab::Auxiliary | RawPacketTab::DeviceNotification => PARSER_ONLY,
        }
    }

    pub fn next_for(self, tab: RawPacketTab, forward: bool) -> Self {
        let scopes = Self::options_for(tab);
        let index = scopes.iter().position(|scope| *scope == self).unwrap_or(0);
        let next = if forward {
            (index + 1) % scopes.len()
        } else {
            index.checked_sub(1).unwrap_or(scopes.len() - 1)
        };
        scopes[next]
    }
}
```

Add the three intents beside the existing raw intents:

```rust
SelectRawMapScope(RawMapScope),
CycleRawMapScope { forward: bool },
ScrollRawDump { increase: bool, page: bool },
```

- [ ] **Step 4: Add state methods and controller dispatch**

Add `raw_map_scope`, `raw_dump_scroll`, and `raw_map_scroll` to `RawViewState`. Initialize them as `RawMapScope::All`, `0`, and `0`.

Implement the state methods with these rules:

```rust
impl RawViewState {
    pub fn reset_raw_view_scroll(&mut self) {
        self.raw_dump_scroll = 0;
        self.raw_map_scroll = 0;
    }

    pub fn select_tab(&mut self, tab: RawPacketTab) {
        self.selected_tab = tab;
        if !RawMapScope::options_for(tab).contains(&self.raw_map_scope) {
            self.raw_map_scope = RawMapScope::All;
        }
        self.reset_raw_view_scroll();
    }

    pub fn select_scope(&mut self, scope: RawMapScope) {
        if RawMapScope::options_for(self.selected_tab).contains(&scope) {
            self.raw_map_scope = scope;
            self.reset_raw_view_scroll();
        }
    }

    pub fn cycle_scope(&mut self, forward: bool) {
        self.raw_map_scope = self.raw_map_scope.next_for(self.selected_tab, forward);
        self.reset_raw_view_scroll();
    }

    pub fn scroll_raw_view(&mut self, increase: bool, page: bool) {
        let amount = if page { 10 } else { 1 };
        if increase {
            self.raw_dump_scroll = self.raw_dump_scroll.saturating_add(amount);
            self.raw_map_scroll = self.raw_map_scroll.saturating_add(amount);
        } else {
            self.raw_dump_scroll = self.raw_dump_scroll.saturating_sub(amount);
            self.raw_map_scroll = self.raw_map_scroll.saturating_sub(amount);
        }
    }
}
```

Update `handle_select_raw_packet_tab` to call `self.state.raw_view.select_tab(tab)`. Update the existing packet-cycle method to use the same helper. Add controller handlers for the three new intents. Update `handle_select_query_reply_entry` to reset raw map and dump scroll after clamping the selected index. Keep all frame decode and transport code unchanged.

- [ ] **Step 5: Run focused tests and commit**

Run:

```bash
cargo test -p zen-go-tui raw_scope_options --lib
cargo test -p zen-go-tui selecting_packet_resets_unsupported_scope_and_scroll --lib
cargo test -p zen-go-tui --lib
```

Expected result: PASS. Commit the state-only change:

```bash
git add src/app/types.rs src/app/state.rs src/app/controller.rs src/ui/tests.rs
git commit -m "feat: add RAW map navigation state"
```

### Task 2: Build exact packet descriptors

**Files:**
- Create: `src/ui/raw_map.rs`
- Modify: `src/ui/mod.rs`
- Test: `src/ui/raw_map.rs`

**Interfaces:**
- `Coverage` has `Used`, `Readback`, `Observed`, `Parser`, `Unmapped`, and `Padding` variants.
- `RawDomain` has `Base`, `Output`, `Preamp`, `Mixer`, `Query`, `Status`, `Parser`, and `Unknown` variants.
- `RawMapRange { report: Range<usize>, payload: Option<Range<usize>> }` stores half-open ranges.
- `RawMapEntry { ranges, domain, scope, label, coverage, note }` stores one logical field, including non-contiguous ranges.
- `RawByteClassification { coverage, selected, overlap }` describes one rendered byte.
- `RawPacketMap { entries, report_len }` exposes `entries()`, `entries_for_scope(scope)`, and `classify(report_offset, scope)`.
- `build_raw_packet_map(tab: RawPacketTab, bytes: &[u8]) -> RawPacketMap` builds descriptors and complements for one selected raw packet.
- `fn entry<'a>(map: &'a RawPacketMap, label: &str) -> &'a RawMapEntry` finds one test entry by exact label and panics when it is absent.
- `fn query_bytes(query_id: u8, sub_id: u8, body: &[u8]) -> [u8; 320]` builds a test 0x75 report with identifiers at report offsets `0x08` and `0x0c`, then copies body bytes at `0x10`.
- `fn add_entry(entries: &mut Vec<RawMapEntry>, domain: RawDomain, scope: Option<RawMapScope>, coverage: Coverage, label: impl Into<String>, note: impl Into<String>, ranges: Vec<RawMapRange>)` appends one logical descriptor.
- `fn payload_ranges(ranges: &[Range<usize>]) -> Vec<RawMapRange>` converts payload-relative ranges to report ranges by adding `SNAPSHOT_PAYLOAD_OFFSET`.

- [ ] **Step 1: Write failing descriptor and query-guard tests**

Create `src/ui/raw_map.rs` with the model declarations and tests first. Use helpers that search by exact label and inspect `report` and `payload` ranges.

```rust
#[test]
fn snapshot_maps_exact_base_output_and_preamp_offsets() {
    let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);

    assert_eq!(entry(&map, "clock source").ranges[0].report, 0x13..0x14);
    assert_eq!(entry(&map, "clock source").ranges[0].payload, Some(0x03..0x04));
    assert_eq!(entry(&map, "HP1 output mode").ranges[0].report, 0x1f..0x20);
    assert_eq!(entry(&map, "preamp 2 gain").ranges[0].report, 0x29..0x2a);
    assert_eq!(entry(&map, "preamp 2 mode, phantom bit, phase bit").ranges[0].payload, Some(0x1b..0x1c));
}

#[test]
fn snapshot_maps_every_meter_lane_to_exact_channel() {
    let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);

    for channel in 1..=16 {
        let label = format!("CH{channel:02} observed meter lane");
        let item = entry(&map, &label);
        assert_eq!(item.coverage, Coverage::Observed);
        assert_eq!(item.ranges[0].report, (0x9e + channel - 1)..(0x9f + channel - 1));
        assert_eq!(item.ranges[0].payload, Some((0x8e + channel - 1)..(0x8f + channel - 1)));
    }
}

#[test]
fn mixer_correlation_keeps_non_contiguous_ranges_and_warning_note() {
    let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
    let item = entry(&map, "active mixer CH01/CH02 link correlation");

    assert_eq!(item.ranges.len(), 3);
    assert!(item.note.contains("not a standalone byte field"));
    assert_eq!(item.ranges[0].report, 0x9f..0xa0);
    assert_eq!(item.ranges[1].report, 0xdf..0xe0);
    assert_eq!(item.ranges[2].report, 0xea..0xf0);
}

#[test]
fn invalid_query_shape_does_not_create_readback_labels() {
    let bytes = query_bytes(0x03, 0x05, &[0x06]);
    let map = build_raw_packet_map(RawPacketTab::Query75, &bytes);

    assert!(map.entries().iter().all(|item| item.coverage != Coverage::Readback));
    assert!(map.entries().iter().any(|item| item.label == "unresolved query body"));
}
```

Add similar tests for snapshot padding, notification padding, unknown 0x83 payload, recognized 0x03 assignment pairs, 0x0b/0x03 selector bytes, 0x04 startup state pairs, 0x18 dual-surface pairs, metadata, indexed entries, and quad state.

- [ ] **Step 2: Run the focused map tests and verify failure**

Run:

```bash
cargo test -p zen-go-tui raw_map --lib
```

Expected result: compilation fails because the map module and descriptor model do not exist.

- [ ] **Step 3: Add descriptor model and coverage precedence**

Declare the model in `src/ui/raw_map.rs`:

```rust
use std::ops::Range;

use antelope_protocol::{
    control_panel_startup_queries, QueryResponse, SNAPSHOT_PAYLOAD_OFFSET,
    SNAPSHOT_PAYLOAD_SIZE,
};
use crate::app::{RawMapScope, RawPacketTab};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Coverage {
    Used,
    Readback,
    Observed,
    Parser,
    Unmapped,
    Padding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawDomain {
    Base,
    Output,
    Preamp,
    Mixer,
    Query,
    Status,
    Parser,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawMapRange {
    pub(crate) report: Range<usize>,
    pub(crate) payload: Option<Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawMapEntry {
    pub(crate) ranges: Vec<RawMapRange>,
    pub(crate) domain: RawDomain,
    pub(crate) scope: Option<RawMapScope>,
    pub(crate) label: String,
    pub(crate) coverage: Coverage,
    pub(crate) note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawByteClassification {
    pub(crate) coverage: Coverage,
    pub(crate) selected: bool,
    pub(crate) overlap: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawPacketMap {
    entries: Vec<RawMapEntry>,
    report_len: usize,
}

impl RawPacketMap {
    pub(crate) fn entries(&self) -> &[RawMapEntry] {
        &self.entries
    }

    pub(crate) fn classify(
        &self,
        report_offset: usize,
        scope: RawMapScope,
    ) -> RawByteClassification {
        let all = self
            .entries
            .iter()
            .filter(|entry| entry_contains(entry, report_offset))
            .collect::<Vec<_>>();
        let selected = all
            .iter()
            .copied()
            .filter(|entry| scope_matches(entry, scope))
            .max_by_key(|entry| entry.coverage.rank());
        let chosen = selected
            .or_else(|| all.iter().copied().max_by_key(|entry| entry.coverage.rank()));

        RawByteClassification {
            coverage: chosen.map_or(Coverage::Unmapped, |entry| entry.coverage),
            selected: selected.is_some(),
            overlap: all.len() > 1,
        }
    }
}

impl Coverage {
    pub(crate) const fn rank(self) -> u8 {
        match self {
            Self::Used => 6,
            Self::Readback => 5,
            Self::Observed => 4,
            Self::Parser => 3,
            Self::Unmapped => 2,
            Self::Padding => 1,
        }
    }
}
```

Keep `RawPacketMap::report_len` equal to `bytes.len()`. Use report offsets for all rendering. Convert payload ranges with `SNAPSHOT_PAYLOAD_OFFSET` instead of duplicating the payload offset literal.

- [ ] **Step 4: Add the exact 0x73 descriptors**

Build the snapshot map with these entries:

| Report range | Payload range | Label | Coverage |
| --- | --- | --- | --- |
| `0x00..0x10` | none | `frame envelope and header` | `PARSER` |
| `0x10..0x11` | `0x00..0x01` | `status flags 0-1` | `USED` |
| `0x12..0x13` | `0x02..0x03` | `sample-rate code` | `USED` |
| `0x13..0x14` | `0x03..0x04` | `clock source` | `USED` |
| `0x14..0x18` | `0x04..0x08` | `sample-rate Hz` | `USED` |
| `0x18..0x1c` | `0x08..0x0c` | `front-panel cluster` | `UNMAPPED` |
| `0x1c..0x1d` | `0x0c..0x0d` | `Monitor output level` | `USED` |
| `0x1d..0x1e` | `0x0d..0x0e` | `Monitor output mode` | `USED` |
| `0x1e..0x1f` | `0x0e..0x0f` | `HP1 output level` | `USED` |
| `0x1f..0x20` | `0x0f..0x10` | `HP1 output mode` | `USED` |
| `0x20..0x21` | `0x10..0x11` | `HP2 output level` | `USED` |
| `0x21..0x22` | `0x11..0x12` | `HP2 output mode` | `USED` |
| `0x28..0x29` | `0x18..0x19` | `preamp 1 gain` | `USED` |
| `0x29..0x2a` | `0x19..0x1a` | `preamp 2 gain` | `USED` |
| `0x2a..0x2b` | `0x1a..0x1b` | `preamp 1 mode, phantom bit, phase bit` | `USED` |
| `0x2b..0x2c` | `0x1b..0x1c` | `preamp 2 mode, phantom bit, phase bit` | `USED` |
| `0x7a..0x7b` | `0x6a..0x6b` | `active mixer surface selector` | `USED` |
| `0x7e..0x7f` | `0x6e..0x6f` | `unknown control byte` | `UNMAPPED` |
| `0x9e+n..0x9f+n` | `0x8e+n..0x8f+n` | `CH{n+1:02} observed meter lane` | `OBSERVED` |
| `0xde..0xdf` | `0xce..0xcf` | `observed preamp 1 meter lane` | `OBSERVED` |
| `0xdf..0xe0` | `0xcf..0xd0` | `observed preamp 2 meter lane` | `OBSERVED` |
| `0xea..0xf0` | `0xda..0xe0` | `late mixer correlation lanes` | `OBSERVED` |
| `0xf0..0xf6` | `0xe0..0xe6` | `shared late shadow bytes` | `UNMAPPED` |
| `0xf6..0x140` | none | `fixed snapshot padding` | `PADDING` |

Use `n` from `0` through `15`. Add these exact non-contiguous entries:

```rust
add_entry(
    &mut entries,
    RawDomain::Mixer,
    Some(RawMapScope::Mixer),
    Coverage::Observed,
    "active mixer CH01 mute correlation",
    "Narrow passive decode. Not a standalone byte field.",
    payload_ranges(&[0x8f..0x90, 0xcf..0xd0, 0xda..0xdd]),
);
add_entry(
    &mut entries,
    RawDomain::Mixer,
    Some(RawMapScope::Mixer),
    Coverage::Observed,
    "active mixer CH01 pan correlation",
    "Narrow passive decode. Unresolved outside documented codebook.",
    payload_ranges(&[0x8f..0x90, 0xcf..0xd0, 0xda..0xdb, 0xdd..0xe0]),
);
add_entry(
    &mut entries,
    RawDomain::Mixer,
    Some(RawMapScope::Mixer),
    Coverage::Observed,
    "active mixer CH01/CH02 link correlation",
    "Applied only to active-surface CH01/CH02. Not a standalone byte field.",
    payload_ranges(&[0x8f..0x90, 0xcf..0xd0, 0xda..0xe0]),
);
```

Also add `Mix1 late lane A/B`, `Mix1 late lane A/B mirror`, `Mix2 late lane A/B`, and the unresolved `shared late shadow bytes` entries with the ranges in the approved spec. Add `OVERLAP` to notes for entries that share bytes with another logical entry. Keep the shared meter labels exact for all 16 channels.

- [ ] **Step 5: Add guarded 0x74 and 0x75 descriptors**

Construct a `QueryResponse` only after checking `bytes.len() >= 0x10`:

```rust
fn query_response(bytes: &[u8]) -> Option<QueryResponse> {
    (bytes.len() >= 0x10).then(|| QueryResponse {
        query_id: bytes[0x08],
        sub_id: bytes[0x0c],
        body: bytes[0x10..].to_vec(),
    })
}
```

Use the existing methods as shape guards. Add `PARSER` entries for report `0x00..0x10`, query ID at `0x08`, and sub-ID at `0x0c`.

For 0x74 requests, compare the query ID and sub-ID with `control_panel_startup_queries()`. Mark report `0x10..0x140` as known request padding only for a matching pair. Mark it `UNMAPPED` for an unknown pair. Do not add request body fields because `encode_query` writes no body fields.

For valid 0x75 responses, add these entries:

- Query `0x03`, sub-ID `0x05`, only when `assignment_readback()` returns `Some`: body `1..9`, report `0x11..0x19`, two bytes per label from `CH01 assignment` through `CH04 assignment`.
- Query `0x03`, sub-ID `0x06..=0x09`, only when `assignment_readback()` returns `Some`: body `9..33`, report `0x19..0x31`, two bytes per label from `CH05 assignment` through `CH16 assignment`. Include bank and pair offsets in the note.
- Query `0x0b/0x03`, only when `selector_bitmap()` returns `Some`: body `0..24`, report `0x10..0x28`, one label per selector byte for Mix1 and Mix2 channel pairs. Label unselected bytes as readback bytes because the decoder reads the complete bitmap.
- Query `0x04/0x00` and `0x04/0x01`, only when `startup_pan_state_readback()` returns `Some`: body level byte `2 + index * 2` and state byte `3 + index * 2`, report `0x12..0x32`. Label each pair `Mix1 CH01 level` and `Mix1 CH01 pan/mute/solo state`, or the corresponding Mix2 labels.
- Query `0x18/0x00`, only when `mixer_strip_readback()` returns `Some`: body `0..64`, report `0x10..0x50`, with 16 Mix1 level/state pairs followed by 16 Mix2 pairs. Label each pair with the exact channel and component.
- Query `0x01`, only when `metadata()` returns `Some`: find the exact three non-empty NUL-separated field ranges and label them `product name`, `serial`, and `hardware version` in the existing decoder order.
- Query `0x15/0x00`, only when `startup_indexed_code_table()` returns `Some`: map each two-byte pair as `indexed entry 00` through `indexed entry 31`.
- Query `0x17/0x00`, only when `startup_quad_state()` returns `Some`: map the four bytes as `quad state byte 0` through `quad state byte 3`.
- Query kind `StatusValue` gets a `Status` entry for the status or capability body. Keep its body `UNMAPPED` because no field decoder exists for it.

Unknown query IDs, short bodies, and failed shape checks get parser-known identifiers plus one `unresolved query body` entry. Do not create semantic labels for failed guards.

- [ ] **Step 6: Add 0x83, 0x81, and complement derivation**

Build these packet maps:

- 0x83: report `0x00..0x10` as `PARSER`. Mark report `0x10..0x140` as `UNMAPPED` with payload `0x00..0x130`. Do not mark it as padding.
- 0x81: report `0x00..0x06` as `PARSER`. Mark report `0x06..0x140` as `PADDING`. The parser accepts exactly six notification bytes before app padding.
- 0x73: mark report `0xf6..0x140` as fixed `PADDING`. Derive all uncovered ranges inside report length as `UNMAPPED` with payload ranges where report offsets fall inside the payload.
- 0x74 and 0x75: derive all uncovered bytes as `UNMAPPED`, except recognized 0x74 request padding.

Merge adjacent uncovered offsets into one derived entry. Preserve report offsets in every derived label. Run overlap annotation after static and derived entries are present.

- [ ] **Step 7: Run descriptor tests and commit**

Run:

```bash
cargo test -p zen-go-tui raw_map --lib
cargo test -p zen-go-tui --lib
```

Expected result: PASS. Commit the map model:

```bash
git add src/ui/raw_map.rs src/ui/mod.rs
git commit -m "feat: describe RAW packet coverage"
```

### Task 3: Render coverage colors and field map text

**Files:**
- Modify: `src/ui/styles.rs`
- Modify: `src/ui/render/text.rs`
- Test: `src/ui/tests.rs`

**Interfaces:**
- `style_for_raw_hex_byte(byte: u8, first_in_row: bool, changed: bool, classification: RawByteClassification) -> Style` composes coverage, scope, and baseline signals.
- `style_for_raw_ascii_byte(byte: u8, changed: bool, classification: RawByteClassification) -> Style` uses the same coverage signal for ASCII output.
- `render_full_packet_dump(bytes: &[u8], baseline: Option<&[u8]>, map: &RawPacketMap, scope: RawMapScope) -> Text<'static>` renders every report byte once.
- `render_dump_line(offset: usize, chunk: &[u8], baseline: Option<&[u8]>, map: &RawPacketMap, scope: RawMapScope) -> Line<'static>` styles one 16-byte row.
- `render_raw_map_text(map: &RawPacketMap, scope: RawMapScope, compact: bool) -> Text<'static>` lists exact ranges, labels, coverage, and notes.
- `selected_query_reply_bytes<'a>(fallback: &'a [u8], state: &'a AppState) -> &'a [u8]` returns the same raw bytes for map and dump rendering.

- [ ] **Step 1: Write failing style and map text tests**

Add tests that inspect both text and Ratatui styles:

```rust
#[test]
fn semantic_dump_uses_coverage_color_and_composes_baseline_marker() {
    let bytes = [0x42; 320];
    let map = raw_map::build_raw_packet_map(RawPacketTab::State73, &bytes);
    let mut baseline = bytes;
    baseline[0x13] = 0x00;

    let dump = render::render_full_packet_dump(
        &bytes,
        Some(&baseline),
        &map,
        RawMapScope::Base,
    );
    let clock_span = &dump.lines[1].spans[1 + (0x13 - 0x10)];

    assert_eq!(clock_span.style.fg, Some(Color::Green));
    assert!(clock_span.style.add_modifier.contains(Modifier::UNDERLINED));
    assert_eq!(clock_span.style.bg, Some(Color::DarkGray));
}

#[test]
fn map_text_contains_exact_offsets_labels_and_overlap_note() {
    let map = raw_map::build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
    let text = render::render_raw_map_text(&map, RawMapScope::Mixer, false).to_string();

    assert!(text.contains("report 0x9f"));
    assert!(text.contains("payload 0x8f"));
    assert!(text.contains("active mixer CH01/CH02 link correlation"));
    assert!(text.contains("OVERLAP"));
}
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
cargo test -p zen-go-tui semantic_dump_uses_coverage_color_and_composes_baseline_marker --lib
cargo test -p zen-go-tui map_text_contains_exact_offsets_labels_and_overlap_note --lib
```

Expected result: compilation fails because coverage-aware rendering APIs do not exist.

- [ ] **Step 3: Add coverage-aware style composition**

Import `Coverage` and `RawByteClassification` into `src/ui/styles.rs`. Use this palette:

```rust
pub(crate) fn raw_coverage_color(coverage: Coverage, selected: bool) -> Color {
    match coverage {
        Coverage::Used => Color::Green,
        Coverage::Readback => Color::Blue,
        Coverage::Observed => Color::Yellow,
        Coverage::Parser => Color::Cyan,
        Coverage::Unmapped => {
            if selected { Color::LightRed } else { Color::Red }
        }
        Coverage::Padding => Color::DarkGray,
    }
}
```

Build the style in this order:

1. Set the foreground from coverage.
2. Add `DIM` to unselected mapped bytes and all padding bytes.
3. Add `BOLD` to selected bytes and the first byte in each row.
4. Add dark-gray background and underline when the baseline differs.
5. Pass the result through `terminal::adapt_style`.

Do not replace the baseline marker when applying coverage. Keep the existing generic `style_for_hex_byte` and `style_for_ascii_byte` behavior for callers that still use them.

- [ ] **Step 4: Refactor dump rows and add map text**

Change the dump helpers to accept `RawPacketMap` and `RawMapScope`. For every byte, call `map.classify(offset + index, scope)` once. Render the hex span and ASCII span with the same classification. Keep offset formatting, 16-byte grouping, and byte order unchanged.

Format every map entry with exact report and payload ranges. Use two lines for wide entries when needed:

```text
report 0x9f,0xdf,0xea..0xf0 / payload 0x8f,0xcf,0xda..0xe0
OBSERVED MIXER active mixer CH01/CH02 link correlation OVERLAP
```

In compact mode, keep both offset forms and the exact label. Truncate only the note. Render `UNMAPPED` and `PADDING` entries rather than omitting them. Keep map entries ordered by their first report offset, then coverage rank.

Use `selected_query_reply_bytes` in both Query75 map and dump paths. Preserve the existing recent request and recent reply log text. Query75 must not use `latest_raw_75` when a selected history entry exists.

- [ ] **Step 5: Run render tests and commit**

Run:

```bash
cargo test -p zen-go-tui semantic_dump_uses_coverage_color_and_composes_baseline_marker --lib
cargo test -p zen-go-tui map_text_contains_exact_offsets_labels_and_overlap_note --lib
cargo test -p zen-go-tui hex_dump_renders_offset_and_ascii --lib
cargo test -p zen-go-tui zero_bytes_are_dimmed_and_offsets_are_bold --lib
cargo test -p zen-go-tui --lib
```

Expected result: PASS. Commit the render change:

```bash
git add src/ui/styles.rs src/ui/render/text.rs src/ui/tests.rs
git commit -m "feat: colorize RAW packet fields"
```

### Task 4: Add responsive panes and keyboard and mouse navigation

**Files:**
- Modify: `src/ui/layouts.rs`
- Modify: `src/ui/mouse.rs`
- Modify: `src/ui/render/mod.rs`
- Modify: `src/runtime.rs`
- Modify: `src/terminal.rs`
- Modify: `src/app/controller.rs`
- Test: `src/ui/tests.rs` and `src/terminal.rs`

**Interfaces:**
- `raw_scope_hit_areas(area: Rect, tab: RawPacketTab) -> Vec<Rect>` returns clickable scope chip rectangles.
- `raw_content_layout(area: Rect, query_replies: bool) -> RawContentLayout` returns wide or narrow map, dump, and optional history regions.
- `RawContentLayout` has wide and narrow variants for ordinary packets and Query75. `map(&self) -> Rect`, `dump(&self) -> Rect`, and `history(&self) -> Option<Rect>` return the corresponding regions. `compact_map(&self) -> bool` identifies narrow layouts.
- `raw_mouse_action` returns `Intent::SelectRawMapScope` for scope chips.
- `slider_wheel_action` returns `Intent::ScrollRawDump` over raw map or dump regions and preserves Query75 history wheel behavior.
- `raw_dump_wheel_action(area: Rect, state: &AppState, point: (u16, u16), increase: bool) -> Option<Intent>` returns `ScrollRawDump { increase, page: false }` only when the point is inside the current map or dump region.
- `AppKeyCode` adds `PageUp` and `PageDown`.

- [ ] **Step 1: Write failing layout and mouse tests**

Add tests for both target sizes and subtab hit areas:

```rust
#[test]
fn raw_layout_has_packet_and_scope_rows() {
    let rows = layouts::raw_page_layout(Rect::new(0, 0, 140, 40));
    assert_eq!(rows.len(), 5);
    assert!(rows[1].height >= 3);
    assert!(rows[2].height >= 3);
    assert!(rows[3].width >= 140);
}

#[test]
fn raw_scope_mouse_action_selects_unmapped_scope() {
    let area = Rect::new(0, 0, 140, 40);
    let rows = layouts::raw_page_layout(area);
    let scopes = layouts::raw_scope_hit_areas(rows[2], RawPacketTab::State73);
    let point = (scopes[5].x + 1, scopes[5].y);
    let mut state = AppState::default();
    state.popup.raw_view_open = true;
    state.raw_view.selected_tab = RawPacketTab::State73;

    assert_eq!(
        mouse_action(area, &state, point.0, point.1),
        Some(Intent::SelectRawMapScope(RawMapScope::Unmapped))
    );
}

#[test]
fn narrow_raw_content_reserves_dump_after_compact_map() {
    let content = layouts::raw_content_layout(Rect::new(0, 0, 80, 13), false);
    assert!(content.map().height > 0);
    assert!(content.dump().height > 0);
}
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
cargo test -p zen-go-tui raw_layout_has_packet_and_scope_rows --lib
cargo test -p zen-go-tui raw_scope_mouse_action_selects_unmapped_scope --lib
```

Expected result: compilation fails because the new layout functions, scope hit areas, and scope mouse intent do not exist.

- [ ] **Step 3: Add responsive layout helpers**

Change `raw_page_layout` to return five rows:

```rust
pub(crate) fn raw_page_layout(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area)
}
```

Add scope hit areas with the labels from `RawMapScope::options_for(tab)`. Add `RawContentLayout` and split at `area.width >= 120`.

At wide sizes, use these regions:

- Ordinary packets: map at 42 percent and dump at 58 percent.
- Query75: history at 25 percent, then map at 42 percent and dump at 58 percent inside the remaining width.

At narrow sizes, stack compact map above dump. Use a map height no greater than six rows and give the dump the remaining height. For Query75, stack history, compact map, and dump. Keep every dump offset and all map labels in their own widget bounds.

- [ ] **Step 4: Wire renderer panes and scope subtabs**

Update `draw_raw_page` to render:

1. Packet tabs.
2. Dynamic scope tabs from `RawMapScope::options_for(selected_tab)`.
3. A coverage legend with exact text:

```text
USED green | READBACK blue | OBSERVED amber | PARSER cyan | UNMAPPED red | PADDING gray
```

4. A bordered field-map pane and bordered byte-dump pane.
5. Query75 history in the left pane at wide sizes.
6. A footer with `[` and `]` scope help, PageUp and PageDown help, and map and dump scroll positions.

Build one `RawPacketMap` from the bytes actually rendered. For Query75, get bytes from `selected_query_reply_bytes` before building both map and dump. Apply `state.raw_view.raw_dump_scroll` to the dump Paragraph and `state.raw_view.raw_map_scroll` to the map Paragraph. Clamp each offset to its own content height before rendering. Keep waiting text when no packet exists.

Use `section_block` or `panel_block` borders for the map, dump, and Query75 history. Use `terminal::adapt_style` through existing style helpers.

- [ ] **Step 5: Add scope clicks and raw wheel scrolling**

Extend `raw_mouse_action` with scope hit testing before Query75 history testing. Return the matching `SelectRawMapScope` intent.

Keep existing Query75 history wheel behavior. Route wheel events over map and dump regions to the new scroll intent:

```rust
if state.popup.raw_view_open {
    if state.raw_view.selected_tab == RawPacketTab::Query75 {
        if let Some(intent) = query_reply_wheel_action(area, state, point, increase) {
            return Some(intent);
        }
    }
    return raw_dump_wheel_action(area, state, point, increase);
}
```

Do not route wheel events over the Query75 history pane to dump scrolling.

- [ ] **Step 6: Add PageUp, PageDown, bracket keys, and runtime guards**

Add variants and normalization in `src/terminal.rs`:

```rust
pub enum AppKeyCode {
    Char(char),
    Tab,
    BackTab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Enter,
    Esc,
    Unknown,
}
```

Map `terminput::KeyCode::PageUp` and `PageDown` in `normalize_key`.

In `handle_key_press`, add raw-view guards before the existing mixer `[` and `]` bindings:

```rust
AppKeyCode::Char('[') if controller.state.popup.raw_view_open => {
    controller.apply_intent(Intent::CycleRawMapScope { forward: false }, area)?;
    Ok(())
}
AppKeyCode::Char(']') if controller.state.popup.raw_view_open => {
    controller.apply_intent(Intent::CycleRawMapScope { forward: true }, area)?;
    Ok(())
}
AppKeyCode::PageUp if controller.state.popup.raw_view_open => {
    controller.apply_intent(
        Intent::ScrollRawDump { increase: false, page: true },
        area,
    )?;
    Ok(())
}
AppKeyCode::PageDown if controller.state.popup.raw_view_open => {
    controller.apply_intent(
        Intent::ScrollRawDump { increase: true, page: true },
        area,
    )?;
    Ok(())
}
```

Keep existing raw Left and Right handling. Left and Right continue to cycle packet tabs except Query75, where they continue to move through reply history.

- [ ] **Step 7: Add selection synchronization and responsive render tests**

Use two Query75 entries with different first bytes. Select each entry and render the raw page. Assert that the map label and dump bytes change together. Add tests for 140x40 and 80x24 that assert map and dump rectangles have positive dimensions, packet and scope labels fit their hit areas, and the dump scroll offset changes after a PageDown intent.

Add terminal normalization tests with `terminput::KeyCode::PageUp` and `PageDown`. Add state tests for bracket scope cycling and scroll reset after scope and selected-reply changes.

- [ ] **Step 8: Run UI and terminal tests and commit**

Run:

```bash
cargo test -p zen-go-tui raw_layout_has_packet_and_scope_rows --lib
cargo test -p zen-go-tui raw_scope_mouse_action_selects_unmapped_scope --lib
cargo test -p zen-go-tui --lib
```

Expected result: PASS. Commit the interaction change:

```bash
git add src/ui/layouts.rs src/ui/mouse.rs src/ui/render/mod.rs src/runtime.rs src/terminal.rs src/app/controller.rs src/ui/tests.rs
git commit -m "feat: navigate RAW semantic scopes"
```

### Task 5: Update documentation and run repository checks

**Files:**
- Modify: `docs/zen-go-tui.md`
- No protocol or Slint files

- [ ] **Step 1: Update the focused RAW view documentation**

Add the following behavior to the existing RAW view section:

- Packet tabs remain `0x74`, `0x73`, `0x83`, `0x75`, and `0x81`.
- Semantic subtabs show packet-specific scopes.
- `[` and `]` move between semantic scopes.
- PageUp, PageDown, and the raw-view wheel scroll the map and dump.
- Query75 map and dump follow the selected reply.
- The legend defines `USED`, `READBACK`, `OBSERVED`, `PARSER`, `UNMAPPED`, and `PADDING`.
- `UNMAPPED` keeps offsets visible and highlights bytes without a grounded decoder.
- Mixed mixer bytes use correlation-group labels.

Use active voice and keep the update in the existing keyboard and RAW view sections.

- [ ] **Step 2: Format and run focused checks**

Run:

```bash
cargo fmt --all
cargo test -p zen-go-tui --lib
cargo test --workspace
cargo clippy --workspace --tests -- -D warnings
```

Run `lsp_diagnostics` with `serverScope: "primary"` for every edited Rust file. Then run `lens_diagnostics` with `mode: "all"` and paths limited to edited source files. Fix every blocking error before continuing.

- [ ] **Step 3: Check the final diff and repository isolation**

Run:

```bash
git diff --check

git status --short --branch
git diff --stat 955ff74..HEAD
git diff -- docs/zen-go-tui.md src/app/types.rs src/app/state.rs src/app/controller.rs src/terminal.rs src/ui/raw_map.rs src/ui/styles.rs src/ui/layouts.rs src/ui/mouse.rs src/ui/render/text.rs src/ui/render/mod.rs src/ui/mod.rs src/ui/tests.rs
```

Confirm that only the feature worktree contains feature changes. Confirm that the root checkout and the existing `zen-go-gui-release` worktree remain unchanged.

- [ ] **Step 4: Start dedicated headed TUI verification**

Use the exact TUI command from the worktree:

```bash
cargo run -- --mock
```

First check capabilities:

```bash
command -v tmux
tmux -V
command -v kitty || true
printenv XDG_SESSION_TYPE
printenv XDG_CURRENT_DESKTOP
printenv DISPLAY
printenv WAYLAND_DISPLAY
```

Create a unique session at 140x40 without reusing an existing session:

```bash
session="oc-tui-raw-map-$(date +%s)"
workdir="/home/ryodeushii/repos/zen-go-tui/.worktrees/raw-packet-map"
tmux has-session -t "$session" 2>/dev/null && exit 1
tmux new-session -d -s "$session" -x 140 -y 40 -c "$workdir"
tmux send-keys -l -t "$session" -- 'cargo run -- --mock'
tmux send-keys -t "$session" Enter
```

Capture the pane before each input and after each state transition. Open RAW with the documented raw shortcut. Capture these transitions:

1. Open RAW view.
2. Press `]` and `[`. Confirm scope title and highlighted byte classes change.
3. Select `UNMAPPED`. Confirm mapped bytes dim and unresolved entries brighten.
4. Switch packet tabs. Confirm scope list changes and invalid scope resets to `ALL`.
5. Select a Query75 reply. Confirm map labels and dump bytes use the selected entry.
6. Resize the dedicated tmux window to 80x24. Confirm compact map and scrollable dump.
7. Resize back to 140x40. Confirm two-pane layout and borders return.

Use literal control keys and record the visible text after each transition:

```bash
tmux capture-pane -p -t "$session"
tmux send-keys -t "$session" ']'
tmux capture-pane -p -t "$session"
tmux send-keys -t "$session" '['
tmux capture-pane -p -t "$session"
tmux resize-window -t "$session" -x 80 -y 24
tmux capture-pane -p -t "$session"
tmux resize-window -t "$session" -x 140 -y 40
tmux capture-pane -p -t "$session"
```

Use a mouse click or a documented terminal mouse sequence to select a Query75 history row. Capture the pane before and after the click. Do not confirm destructive actions.

- [ ] **Step 5: Attach Kitty and capture targeted visual evidence**

When Kitty and a graphical display are available, attach the same tmux session in a uniquely titled window:

```bash
window_title="zen-go-raw-map-$(date +%s)"
kitty --detach --title "$window_title" tmux attach-session -t "$session"
```

Inspect the active graphical session and installed window and screenshot tools. Resolve exactly one window by the unique title before capturing. Store the screenshot under `/tmp/pi-tui/` with a unique filename. Capture only the Kitty window. Read the image and verify:

- coverage colors match the legend
- map, dump, and history borders are visible
- exact labels and offsets are readable
- selected scope has clear hierarchy
- changed bytes retain their baseline marker
- 80x24 has no horizontal clipping
- 140x40 restores the two-pane hierarchy

If no installed backend can identify exactly one Kitty window, complete tmux text checks and report visual verification as `PARTIAL`. Do not capture the full desktop.

- [ ] **Step 6: Clean up successful verification and report result**

On success, close only the dedicated session:

```bash
tmux kill-session -t "$session"
```

Keep a failed session alive for inspection. Report the exact session name, dimensions, inputs, text checks, visual checks, evidence path, cleanup state, and final `PASS`, `FAIL`, or `PARTIAL` result.

## Final Acceptance Checklist

- [ ] Exact field labels appear for every grounded mapping.
- [ ] Report and payload offsets appear for every map entry.
- [ ] Shared meter lanes identify CH01 through CH16.
- [ ] Outputs distinguish level and mode for Monitor, HP1, and HP2.
- [ ] Preamp entries identify gain, mode, phantom, and phase bits.
- [ ] Mixed mixer state bytes appear as correlation groups.
- [ ] Unknown and fixed-padding regions stay visible.
- [ ] Query descriptors require valid IDs, sub-IDs, and body shapes.
- [ ] 0x83 payload stays unresolved.
- [ ] 0x81 tail is fixed padding.
- [ ] Semantic scopes work with keyboard and mouse.
- [ ] Query75 map and dump follow selected history entries.
- [ ] 140x40 uses bordered map and dump panes.
- [ ] 80x24 keeps labels and offsets visible without horizontal clipping.
- [ ] Narrow dump rows scroll instead of clipping.
- [ ] Existing packet-tab and Query75 history behavior remains intact.
- [ ] Focused tests, workspace tests, formatter, Clippy, LSP, and lens diagnostics pass.
- [ ] Root checkout and unrelated worktrees remain unchanged.
