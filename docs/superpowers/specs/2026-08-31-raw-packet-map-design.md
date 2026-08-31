# RAW packet semantic map

**Date:** 2026-08-31  
**Status:** Approved design, written spec review pending  
**Scope:** Root Ratatui TUI only

## Context

The RAW view shows five packet types as unannotated hexadecimal dumps. The dump preserves bytes, but it does not show which bytes the application decodes, reads through query responses, observes, or leaves unresolved.

The protocol already defines a safe set of fields. It also documents mixer bytes that carry correlated state rather than one independent control per byte. The RAW view must expose both facts without adding protocol claims.

The TUI must show exact field labels. A label such as `CH01 passive mute correlation` is useful. A label such as `mixer` is not sufficient.

## Goals

1. Keep the exact 320-byte report dump visible.
2. Show report offsets and payload-relative offsets for every mapped entry.
3. Label exact controls, lanes, query fields, and correlation groups.
4. Distinguish typed state, query readback, observed evidence, parser fields, unresolved bytes, and padding.
5. Add packet-specific semantic subtabs.
6. Show unresolved bytes without deleting or reordering them.
7. Make the map useful at 140x40 and readable at 80x24.
8. Verify colors, borders, labels, and keyboard navigation in a headed Kitty window backed by tmux.

## Non-goals

- Do not add protocol decoders.
- Do not change frame parsing, encoding, transport, or device commands.
- Do not claim full passive mixer level, pan, solo, source, or output-meter decoding.
- Do not claim that mixed late-row bytes are standalone mute, pan, or link fields.
- Do not change the Slint RAW page.
- Do not remove bytes from the raw dump.
- Do not infer variable frame padding for query or auxiliary packets. Current app state does not retain their original lengths.

## Coverage model

Add a UI-only presentation module, `src/ui/raw_map.rs`. It defines descriptors for the current packet type and selected query shape. It uses existing protocol constants and existing query shape guards. It does not change `antelope-protocol`.

Each `RawMapEntry` contains:

- one or more report-offset ranges
- the matching payload-relative range when a payload exists
- a domain such as `BASE`, `OUTPUT`, `PREAMP`, `MIXER`, `QUERY`, or `PARSER`
- an exact field label
- a coverage value
- a short note when the range is observed, correlated, overlapping, or unresolved

Use half-open ranges for contiguous offsets. Preserve non-contiguous correlated ranges in one entry. Display each entry once in the map pane, even when entries overlap.

Use these coverage values:

| Coverage | Meaning | Color |
| --- | --- | --- |
| `USED` | Typed bytes populate current application state. | Green |
| `READBACK` | A grounded query decoder applies bytes to controls. | Blue |
| `OBSERVED` | Passive evidence has a narrow, documented interpretation. | Amber |
| `PARSER` | The parser uses the frame envelope or query identifiers. | Cyan |
| `UNMAPPED` | No decoder claim exists for the byte or group. | Red, dim |
| `PADDING` | Padding is known from a fixed frame length. | Dark gray |

Coverage color is the primary byte color. Domain appears in the entry label and uses the existing accent family. Baseline changes keep the existing underline and background marker. These signals must compose instead of replacing each other.

When several entries cover one byte, use this precedence for the dump:

1. selected scope
2. `USED`
3. `READBACK`
4. `OBSERVED`
5. `PARSER`
6. `UNMAPPED`
7. `PADDING`

Add `OVERLAP` to the map entry note when a range shares bytes with another entry. Do not render one byte more than once in the dump.

Derive complement ranges as `UNMAPPED` entries. Keep the report offset in every derived entry. Use `PADDING` only where the source length is known.

## Exact 0x73 map

All payload offsets below add `SNAPSHOT_PAYLOAD_OFFSET` (`0x10`) to get report offsets.

| Report range | Payload range | Label | Coverage | Note |
| --- | --- | --- | --- | --- |
| `0x00..0x10` | n/a | frame envelope and header | `PARSER` | Parser-known 0x73 frame area. |
| `0x10..0x11` | `0x00..0x01` | status flags 0-1 | `USED` | Typed snapshot status bytes. |
| `0x12` | `0x02` | sample-rate code | `USED` | Decoded through `SampleRate`. |
| `0x13` | `0x03` | clock source | `USED` | Decoded through `ClockSource`. |
| `0x14..0x18` | `0x04..0x08` | sample-rate Hz | `USED` | Big-endian rate value. |
| `0x18..0x1c` | `0x08..0x0c` | front-panel cluster | `UNMAPPED` | Preserved. Individual controls are not decoded. |
| `0x1c` | `0x0c` | Monitor output level | `USED` | Output attenuation value. |
| `0x1d` | `0x0d` | Monitor output mode | `USED` | Normal, dim, or mute. |
| `0x1e` | `0x0e` | HP1 output level | `USED` | Output attenuation value. |
| `0x1f` | `0x0f` | HP1 output mode | `USED` | Normal, dim, or mute. |
| `0x20` | `0x10` | HP2 output level | `USED` | Output attenuation value. |
| `0x21` | `0x11` | HP2 output mode | `USED` | Normal, dim, or mute. |
| `0x28` | `0x18` | preamp 1 gain | `USED` | Typed preamp cluster. |
| `0x29` | `0x19` | preamp 2 gain | `USED` | Typed preamp cluster. |
| `0x2a` | `0x1a` | preamp 1 mode, phantom bit, phase bit | `USED` | Low nibble is mode. Bits `0x10` and `0x40` are phantom and phase. |
| `0x2b` | `0x1b` | preamp 2 mode, phantom bit, phase bit | `USED` | Low nibble is mode. Bits `0x10` and `0x40` are phantom and phase. |
| `0x7a` | `0x6a` | active mixer surface selector | `USED` | Mix1 or Mix2 selection. |
| `0x7e` | `0x6e` | unknown control byte | `UNMAPPED` | Documented unknown byte. |
| `0x9e + n` | `0x8e + n` | CH{n+1:02} observed meter lane | `OBSERVED` | `n` ranges from 0 through 15. The app applies this shared observation to both mixer surfaces. |
| `0xde` | `0xce` | observed preamp 1 meter lane | `OBSERVED` | Narrow observed meter range. |
| `0xdf` | `0xcf` | observed preamp 2 meter lane | `OBSERVED` | Overlaps Mix2 state correlation bytes. |
| `0xea..0xf0` | `0xda..0xe0` | late mixer correlation lanes | `OBSERVED` | Preserve correlated Mix1 and Mix2 lanes. Do not claim one control per byte. |
| `0xf0..0xf6` | `0xe0..0xe6` | shared late shadow bytes | `UNMAPPED` | No per-control claim. |
| `0xf6..0x140` | n/a | fixed snapshot padding | `PADDING` | Snapshot payload ends at report `0xf6`. |

Add these logical correlation entries. Their report ranges are non-contiguous where shown.

| Report ranges | Payload ranges | Label | Coverage | Note |
| --- | --- | --- | --- | --- |
| `0x9f`, `0xdf`, `0xea..0xed` | `0x8f`, `0xcf`, `0xda..0xdd` | active mixer CH01 mute correlation | `OBSERVED` | Narrow passive decode. Not a standalone byte field. |
| `0x9f`, `0xdf`, `0xea`, `0xed`, `0xee`, `0xef` | `0x8f`, `0xcf`, `0xda`, `0xdd`, `0xde`, `0xdf` | active mixer CH01 pan correlation | `OBSERVED` | Narrow passive decode. Unresolved outside documented codebook. |
| `0x9f`, `0xdf`, `0xea..0xef` | `0x8f`, `0xcf`, `0xda..0xdf` | active mixer CH01/CH02 link correlation | `OBSERVED` | Applied only to active-surface CH01/CH02. |
| `0xea/0xeb` | `0xda/0xdb` | Mix1 late lane A/B | `OBSERVED` | Correlated lane pair. |
| `0xec/0xed` | `0xdc/0xdd` | Mix1 late lane A/B mirror | `OBSERVED` | Mirror of the Mix1 pair. |
| `0xee/0xef` | `0xde/0xdf` | Mix2 late lane A/B | `OBSERVED` | Correlated lane pair. |
| `0xf0..0xf5` | `0xe0..0xe5` | shared late shadow bytes | `UNMAPPED` | No per-control claim. |

The map must show the lane number for every shared meter byte. It must show both the byte-level lane entry and any overlapping correlation entry in the map list.

## Query maps

Use report offsets for query frames. Query identifiers are parser-known at report `0x08` and `0x0c`. Query bodies start at report `0x10`.

Show these descriptors only when the query ID, sub-ID, and body shape pass the existing guards:

- Query `0x03` assignment banks:
  - sub-ID `0x05`, body bytes `1..9`, report `0x11..0x19`, maps two-byte pairs to `CH01 assignment` through `CH04 assignment`
  - sub-IDs `0x06..0x09`, body bytes `9..33`, report `0x19..0x31`, maps two-byte pairs to `CH05 assignment` through `CH16 assignment`
  - include bank and pair offsets in each label or note
- Query `0x0b/0x03` selector bitmap:
  - body bytes `0..24`, report `0x10..0x28`
  - map Mix1 pair selectors 1/2 through 15/16 and Mix2 pair selectors 1/2 through 15/16
  - mark bitmap bytes without a set selector as `READBACK`, because the decoder reads the complete bitmap
- Query `0x04/0x00` and `0x04/0x01` startup pan state:
  - body bytes `2 + index*2` and `3 + index*2`, report `0x12..0x32`
  - map each pair to `Mix1 CH01 level`, `Mix1 CH01 pan/mute/solo`, through `CH16`, or the corresponding Mix2 label
  - state byte label must name all decoded components: pan, mute, and solo
- Query `0x18/0x00` full strip readback:
  - body bytes `0..64`, report `0x10..0x50`
  - map 16 level/state pairs to Mix1 CH01 through CH16, then 16 pairs to Mix2 CH01 through CH16
  - label each pair as `level` and `pan/mute/solo state`
- Query `0x01` metadata:
  - map the NUL-separated product name, hardware version, and serial fields when the existing metadata shape succeeds
  - show exact body ranges found by the shape parser
- Query `0x15/0x00` indexed table:
  - map each decoded two-byte indexed entry with its index
- Query `0x17/0x00` quad state:
  - map each decoded quad-state byte group with its field label

Query ID and sub-ID entries use `PARSER` coverage. Grounded body entries use `READBACK`. Recognized but unsupported body bytes remain `UNMAPPED`.

For 0x74 request frames, show the parser-known query ID and sub-ID. Show request body entries only when the existing startup-query request shape identifies them. Unknown requests keep their body as `UNMAPPED`.

Unknown query IDs, short bodies, and failed shape checks must show the parser-known identifiers and an unresolved body entry. Do not show a recognized semantic label for a failed guard.

## 0x83 and 0x81 maps

For 0x83, show the parser-known frame envelope as `PARSER`. Keep its payload `UNMAPPED`, because the current code preserves it but does not decode it.

For 0x81, show the six-byte notification as `PARSER`. Mark report bytes `0x06..0x140` as `PADDING`, because the parser accepts exactly six bytes before it pads the app copy to 320 bytes.

Do not mark the 0x83 tail as padding. The app does not retain the original 0x83 length.

## Subtabs and state

Add `RawMapScope` to `RawViewState` and add intents for selecting and cycling scopes. Keep one selected scope in state. When a packet tab does not offer the current scope, reset scope to `ALL`.

Expose these dynamic scopes:

- 0x73: `ALL`, `BASE`, `OUTPUTS`, `PREAMPS`, `MIXER`, `UNMAPPED`
- 0x74: `ALL`, `QUERY`, `MIXER`, `UNMAPPED`
- 0x75: `ALL`, `METADATA`, `MIXER`, `STATUS`, `UNMAPPED`
- 0x83: `ALL`, `PARSER`, `UNMAPPED`
- 0x81: `ALL`, `PARSER`, `UNMAPPED`

Keep packet-tab behavior unchanged. Keep Left and Right for packet tabs. For Query75, keep Left and Right for reply history as they work today. Bind `[` and `]` to previous and next semantic scope. Add mouse hit areas for subtabs.

Track raw dump scroll separately from semantic scope. Reset dump scroll when the packet tab, scope, or selected Query75 entry changes. When content exceeds the viewport, bind PageUp, PageDown, and the existing raw-view wheel path to dump scrolling. Show the scroll position in the footer. Off-screen rows must scroll rather than clip.

`ALL` brightens every mapped range and shows unresolved ranges in a subdued style. A semantic scope brightens matching entries and dims other mapped bytes. `UNMAPPED` brightens unresolved ranges and dims mapped ranges, but keeps all offsets and rows.

## Layout

Keep packet tabs above semantic subtabs.

At 140x40, use a two-column content area:

- 0x73, 0x74, 0x83, and 0x81 show a field-map pane beside the offset-preserving dump.
- 0x75 keeps recent reply history in the left column. Put the field-map pane and selected dump in the remaining area.
- Show exact report and payload offsets in the map pane. Truncate long notes, not labels or offsets.

At 80x24, collapse the map pane to compact labeled lines above or below the dump. Keep dump offsets, byte order, and baseline markers unchanged. Scroll dump rows when the viewport cannot show all 20 rows. Test that the layout does not horizontally clip the selected subtabs, field labels, offsets, or `UNMAPPED` label.

Add a legend to the content header or footer:

```text
USED green | READBACK blue | OBSERVED amber | PARSER cyan | UNMAPPED red | PADDING gray
```

Use borders and titles to distinguish the map pane, byte dump, and Query75 reply history. Use existing terminal style adaptation.

## Data flow

1. Select the current raw packet and baseline from `RawViewState`.
2. Build descriptors from the packet type and, for query packets, the selected raw query shape.
3. Derive `UNMAPPED` and known `PADDING` complements.
4. Filter map entries by `RawMapScope` for the map pane.
5. Render each raw byte once with coverage, scope, overlap, and baseline style composition.
6. Keep waiting text when no packet has arrived.
7. Keep Query75 map and dump tied to the selected history entry, not only `latest_raw_75`.

Do not change how `AppState::observe_frame` decodes or stores packets. Do not change query readback behavior.

## Testing

Add focused tests for these requirements:

1. 0x73 descriptors use the correct report and payload offsets.
2. Every shared meter lane has its exact channel label.
3. Preamp mode labels include mode, phantom, and phase bits.
4. Output entries distinguish level from mode for Monitor, HP1, and HP2.
5. Correlation entries preserve non-contiguous ranges and state that they are not standalone fields.
6. Overlapping entries produce one dump byte and an `OVERLAP` map note.
7. Snapshot tail becomes `PADDING` at report `0xf6`.
8. Notification tail becomes `PADDING` after report `0x05`.
9. Query and auxiliary unknown tails remain `UNMAPPED`.
10. Query descriptors appear only for valid IDs, sub-IDs, and body lengths.
11. Assignment, selector, pan/state, and full strip readback labels name exact channels.
12. Scope cycling follows each packet type list and resets to `ALL` when needed.
13. Subtab mouse hit areas return the expected scope intent.
14. Query75 map and dump use the selected history entry.
15. 140x40 and 80x24 layouts retain labels, offsets, and borders.
16. Narrow layouts scroll dump rows instead of clipping them.
17. Coverage styles and baseline styles compose.

Run the existing UI and protocol test suites. Do not weaken existing raw dump tests.

## Verification

Run formatter on touched Rust files. Run targeted tests, LSP diagnostics, and `lens_diagnostics` with `mode=all` for edited source files.

Use a unique dedicated tmux session at 140x40. Attach it through a uniquely titled Kitty window when Kitty and a graphical display are available. Capture text before and after these transitions:

1. Open RAW view.
2. Cycle semantic scopes with `[` and `]`.
3. Select `UNMAPPED`.
4. Switch packet tabs.
5. Select a Query75 reply.
6. Resize to 80x24 and back to 140x40.

Capture a targeted screenshot of the unique Kitty window. Use it to verify coverage colors, pane borders, scope hierarchy, labels, spacing, and clipping. Do not infer visual results from text capture.

If visual targeting cannot identify exactly one Kitty window, complete tmux checks and report visual verification as `PARTIAL`. Preserve a failed tmux session for inspection.

## Acceptance criteria

The change is ready when:

- exact field-level labels appear for every grounded mapping
- unresolved and padding regions remain visible and clearly marked
- mixed mixer state bytes appear as correlation groups, not false standalone controls
- semantic scopes work by keyboard and mouse
- Query75 selection keeps map and dump synchronized
- 140x40 and 80x24 layouts expose labels, offsets, and borders without horizontal clipping
- narrow dump rows scroll instead of clipping
- focused tests, formatter, diagnostics, and headed TUI verification pass
- root checkout remains unchanged outside the isolated worktree
