# Zen Go SC capture-backed protocol audit

Date: 2026-08-31  
Scope: USBPcap captures in `antelope_pcap/`, compared with current capture-derived implementation.

This is separate from `docs/protocol/zengosc-implementation-audit.md`. It records only evidence derived from captures and source-level coordinate checks. All protocol IDs, offsets, lengths, counts, widths, and values use hexadecimal notation.

## Executive conclusions

- Current device traffic uses fixed `0x140`-byte HID frames. Host writes use endpoint `0x01`; device state uses endpoint `0x82`. Current report types are `0x70`, `0x73`, `0x74`, `0x75`, and `0x83`.
- USBPcap header length is not constant. Target interrupt packets use header length `0x1b`; payload starts at each packet's header length. A fixed `0x1c` skip corrupts the first protocol byte.
- Host command payload begins at HID-frame offset `0x10`. Query ID is at frame offset `0x08`; query sub-ID is at `0x0c`. This matches `antelope-protocol/src/encoder.rs`.
- Captures confirm current snapshot offsets for sample rate, clock, output volume, output mute/dim mode, and preamp cluster. Output mode updates can lag a command by several `0x73` snapshots.
- Preamp capture behavior is consistent with mode-scoped gain: changing mode updates the mode/flag byte while the settled gain value remains assigned to its mode. Current `PreampInputState` stores only one `gain_raw` per input, so current application state cannot preserve one gain assignment for each mode.
- Captures confirm current query-dialect shapes that differ from the official logical schema: `0x17/0x00` produces a `0x04`-byte meaningful body; `0x18/0x00` produces about `0x42` meaningful bytes; `0x04/*` replies extend to about `0x85` bytes. Fixed transport length `0x140` is not semantic body length.
- Capture evidence does not validate current passive-link byte patterns. Standalone link captures contain level-like values at current candidate heads and constant `0x60` tails, never any of current decoder's accepted vectors.
- Capture evidence does not include a combined output mute-plus-dim sequence. Isolated mode codes are proven; composition remains unproven.

No runtime code was changed for this audit.

## Evidence and coordinate system

### PCAPNG and USBPcap extraction

Each file was parsed as PCAPNG directly:

- Enhanced Packet Block type: `0x06`;
- EPB packet data begins at block offset `+0x1c`;
- captured and original lengths are at `+0x14` and `+0x18`;
- USBPcap packet header length is the little-endian `u16` at packet offset `0x00`;
- endpoint address is byte `0x15`;
- transfer type is byte `0x16`;
- USB data length is little-endian `u32` at `0x17..0x1a`;
- transfer payload begins at packet offset `header_len`.

Data-bearing target packets have USBPcap header length `0x1b`, `dlen=0x140`, and frame length `0x15b`. Some `tshark -T jsonraw` fields represent the USB header as `0x1c` bytes; direct packet length and the protocol boundary both use `0x1b` for these interrupt packets. Use the packet header length, never a hard-coded skip.

Completion URBs have `dlen=0x00` and were excluded from protocol counts.

### HID frame coordinates

For a `0x140`-byte HID frame:

- frame bytes `0x00..0x03`: little-endian report type;
- frame bytes `0x04..0x07`: declared transport/request length;
- frame byte `0x08`: query ID for `0x74`/`0x75` traffic;
- frame byte `0x0c`: query sub-ID;
- frame bytes `0x10..`: command payload or query-reply body.

Snapshot offsets in this document are relative to the `0x73` frame payload beginning at frame offset `0x10`, matching `parse_snapshot73`'s `payload` slice in `antelope-protocol/src/frame.rs`.

### Endpoint roles observed

| Endpoint | Data shape | Capture interpretation |
|---|---:|---|
| `0x01` | `0x140` | Host command/query writes |
| `0x82` | `0x140` | Device `0x73`, `0x75`, and `0x83` reports |
| `0x81` | `0x06` | High-rate asynchronous packets; not current snapshot grammar |
| `0x84` | `0x840` or `0x870` | Non-protocol bulk/isochronous traffic; excluded |
| `0x05` | bulk traffic | Enumeration/transport traffic; excluded |

## Capture inventory

The following captures were processed. Counts are data-bearing protocol packets after USBPcap header parsing; completion packets are not included.

| Capture | Host activity | Device activity | Evidence focus |
|---|---|---|---|
| `capture_01_enumeration.pcapng` | `0x03` queries, all `0x74` | `0x73`, `0x83`, `0x75` | Initial query/reply grammar |
| `capture_01_enumeration_diff_filter.pcapng` | Enumeration/control traffic | `0x73`, `0x83` | Endpoint and USBPcap boundary check |
| `capture_02_volume_down.pcapng` | `0x31` set frames, `0x2e` queries | `0x73`/`0x83` each `0x8f9`, `0x75` `0x2e` | Mixer writes and query body extents |
| `capture_03_volume_up.pcapng` | `0x2d` set frames | `0x73`/`0x83` each `0x848` | Mixer level command direction |
| `capture_04_mute_toggle.pcapng` | `0x04` set frames | `0x73` `0x5ee`, `0x83` `0x5ef` | Mixer mute state-byte transition |
| `capture_05_sample_rate_in_the_end_48_32_44.1_44.1default.pcapng` | `0x09` set frames | `0x73` `0x1700`, `0x83` `0x16ff` | Sample-rate mapping |
| `capture_06_clock_source_no_word_clock_exists.pcapng` | `0x04` set frames | `0x73`/`0x83` each `0xa1d` | Clock-source mapping |
| `capture_07_dsp.pcapng` | `0x1e` set frames, `0x04` queries | `0x73`/`0x83` each `0x3220`, `0x75` `0x04` | Preamp, DSP, link, and query families |
| `capture_08_mixer_volume_faders_on_2_mixes.pcapng` | `0x13d` set frames | `0x73`/`0x83` each `0xeb6` | Surface selection and mixer writes |
| `capture_09_idle_polling.pcapng` | none | `0x73`/`0x83` each `0xf2d` | Idle stream and dynamic-byte baseline |
| `capture_10_2_mixers_linked_comp12_fades_mute_unmute.pcapng` | `0x134` set frames | `0x73`/`0x83` stream | Linked mixer workflow |
| `capture_10_mixer_single_channel_one_change_at_a_time_then_unlink_comp12_and_go_individually_onc1andc2.pcapng` | `0x20b` report-`0x70` frames | `0x73` `0x42ae`, `0x83` `0x42ad` | Single-channel mixer and link writes |
| `capture_11_output_controls_once_change_at_a_time.pcapng` | `0x227` report-`0x70` frames | `0x73` `0x282b`, `0x83` `0x282a` | Output volume, mute, and dim |
| `channel links in mixes.pcapng` | `0x11` report-`0x70` frames | `0x73` `0x1d07`, `0x83` `0x1d06` | Standalone link-vector test |

### Recursive coverage

The root table above is only the root scope. The following table covers every PCAPNG under `antelope_pcap/`, including all subdirectories: `0x7f` files, `0x0a` directories, and `0x7251d3e8` captured bytes (`~1.79 GiB`). Counts are data-bearing protocol frames after USBPcap header parsing.

| Scope | Files | PCAP bytes | Host `0x70` | Host `0x74` / device `0x75` | Device `0x73` / `0x83` | Scenario evidence |
|---|---:|---:|---:|---:|---:|---|
| root (`.`) | `0x0e` | `0x10b39338` | `0x741` | `0x38` / `0x38` | `0x12a50` / `0x12a4b` | enumeration, output, mixer, sample/clock, DSP |
| `channel_assignments` | `0x14` | `0x14c91884` | `0xb7` | `—` / `—` | `0x10ca8` / `0x10cab` | assignment banks, source substitutions, surface propagation |
| `metering` | `0x17` | `0x7727b6c` | `0x27c` | `—` / `—` | `0x8b5f` / `0x8b66` | isolated CH01..CH16, mix/output, preamp meters |
| `mixer` | `0x17` | `0x171dabbc` | `0x2ac` | `—` / `—` | `0xc312` / `0xc315` | faders, pan, mute, solo, links, surfaces |
| `mixer_levels` | `0x06` | `0x5e45db8` | `0x42` | `0x114` / `0x114` | `0x28a3` / `0x28a2` | max/min and isolated level readback |
| `mixer_links` | `0x09` | `0x8f918e4` | `0x50` | `0x199` / `0x199` | `0x45fb` / `0x45f9` | isolated linked-pair bitmap scenarios |
| `mixer_pans` | `0x07` | `0x6e00c94` | `0x4d` | `0x142` / `0x142` | `0x3044` / `0x3045` | isolated Mix 1/Mix 2 pan readback |
| `mutes/no signal` | `0x07` | `0x66ca23c` | `—` | `—` / `—` | `0x1473` / `0x1473` | Mix 1/Mix 2 CH01/02 mute baselines |
| `mutes/with signal` | `0x07` | `0x67670dc` | `—` | `—` / `—` | `0x15db` / `0x15db` | Mix 1/Mix 2 CH01/02 mute with signal |
| `preamps` | `0x0b` | `0xba46abc` | `0x1c4` | `—` / `—` | `0x8b68` / `0x8b66` | A1/A2 mode, per-mode gain, phantom, phase |
| **recursive total** | **`0x7f`** | **`0x7251d3e8`** | **—** | **—** | **—** | **all captures processed** |

## Confirmed frame and report grammar

### Enumeration

`capture_01_enumeration.pcapng` contains exactly `0x03` host query frames. Query pairs are:

- `0x01/0x00`;
- `0x00/0x00`;
- `0x11/0x00`.

Each has one device `0x75` reply. Each reply is transported in a `0x140`-byte frame with report type `0x75` and declared length `0x140`. The declaration describes the fixed transport frame, not a compact semantic body.

### Host writes and device state

Across action captures:

- host endpoint `0x01` carries report `0x70` command frames;
- host endpoint `0x01` carries report `0x74` query frames;
- device endpoint `0x82` carries report `0x73` snapshots and report `0x83` auxiliary frames at almost one-for-one cadence;
- device endpoint `0x82` carries report `0x75` query replies;
- endpoint `0x81` carries frequent `0x06`-byte packets unrelated to the fixed snapshot body.

Current `host_frame` and `encode_query` coordinate with these captures: `host_frame` writes report `0x70` and command payload at `0x10`; `encode_query` writes report `0x74`, query ID at `0x08`, and sub-ID at `0x0c`.

## Confirmed control mappings

Pairing method: retain the last report `0x73` snapshot, queue host writes after that snapshot, and compare each queued write with following report `0x73` snapshots. Isolated controls were checked over a horizon of up to `0x20` snapshots; output mode commands were also checked over an `0x08`-snapshot horizon because their state update is delayed.

### Sample rate and clock source

`capture_05_sample_rate_in_the_end_48_32_44.1_44.1default.pcapng` contains host command payloads:

```text
03 01
03 02
03 03
03 04
03 05
03 06
03 02
03 00
03 01
```

The parameter is reflected in snapshot payload offset `0x02`. This confirms current `OFFSET_SAMPLE_RATE_CODE = 0x02`.

`capture_06_clock_source_no_word_clock_exists.pcapng` contains:

```text
04 00
04 01
04 02
04 00
```

The parameter is reflected in snapshot payload offset `0x03`. This confirms current `OFFSET_CLOCK_SOURCE = 0x03`.

### Output volume

`capture_11_output_controls_once_change_at_a_time.pcapng` contains output volume command family `0x47`. Grouping by target gives stable snapshot mappings:

| Command target byte | Snapshot volume offset | Paired events |
|---:|---:|---:|
| `0x00` | `0x0c` | `0xc0` |
| `0x01` | `0x0e` | `0xa8` |
| `0x02` | `0x10` | `0xb1` |

The command value sweeps are reflected in the corresponding snapshot byte. This confirms current three-output volume mapping:

- output `0x00`: volume `0x0c`;
- output `0x01`: volume `0x0e`;
- output `0x02`: volume `0x10`.

### Output mute and dim

The same capture contains isolated on/off commands for all three targets. Snapshot mode offsets and observed mode codes are:

| Target | Mute command | Mute snapshot transition | Dim command | Dim snapshot transition |
|---:|---|---|---|---|
| `0x00` | `48 00 01` / `48 00 00` | `0x0d: 0x00 -> 0x01 -> 0x00` | `66 00 01` / `66 00 00` | `0x0d: 0x00 -> 0x02 -> 0x00` |
| `0x01` | `48 01 01` / `48 01 00` | `0x0f: 0x00 -> 0x01 -> 0x00` | `66 01 01` / `66 01 00` | `0x0f: 0x00 -> 0x02 -> 0x00` |
| `0x02` | `48 02 01` / `48 02 00` | `0x11: 0x00 -> 0x01 -> 0x00` | `66 02 01` / `66 02 00` | `0x11: 0x00 -> 0x02 -> 0x00` |

The first following `0x73` often has no mode-byte change. The change appears within the following `0x08` snapshots. Any controller state machine must allow delayed confirmation.

These captures prove isolated mode codes, not composition. No capture enables mute and dim together, so this evidence cannot decide whether a device mode byte is a mutually exclusive enum or a composed bitfield. Do not infer composition from these isolated transitions.

### Preamp cluster and mode-scoped gain

`capture_07_dsp.pcapng` correlates commands with snapshot payload cluster `0x18..0x1b`:

| Function | Command payload | Snapshot field |
|---|---|---|
| input `0x00` gain | `50 00 <value>` | `0x18` |
| input `0x01` gain | `50 01 <value>` | `0x19` |
| input `0x00` mode | `4f 00 <mode>` | `0x1a` |
| input `0x01` mode | `4f 01 <mode>` | `0x1b` |
| input `0x00` phantom | `51 00 <enabled>` | bit `0x10` in `0x1a` |
| input `0x01` phantom | `51 01 <enabled>` | bit `0x10` in `0x1b` |
| input `0x00` phase | `52 00 <enabled>` | bit `0x40` in `0x1a` |
| input `0x01` phase | `52 01 <enabled>` | bit `0x40` in `0x1b` |

Settled examples:

- `51 00 00` / `51 00 01` changes `0x1a` between `0x10` and `0x00`; gain bytes remain `0x2f/0x34`.
- `52 00 01` / `52 00 00` changes `0x1a` between `0x10` and `0x50`; `0x50` is the observed `0x10` phantom bit plus `0x40` phase bit.
- `4f 00 02`, `4f 00 01`, and `4f 00 00` settle at `0x1a=0x12`, `0x11`, and `0x10`. Transient changes in `0x18` return to `0x2f`.
- `50 01 2f` settles at cluster `0x2f/0x2f/0x10/0x10`; input `0x01` gain is at `0x19`.

A mode write therefore does not establish one global persistent gain value for an input in this capture. Model gain as input/mode-scoped state; do not discard or overwrite a mode's stored gain when `0x4f` changes mode. This capture does not enumerate every mode's stored value, but settled behavior supports separate gain assignments per preamp mode.

### Preamp gain state-model gap

`antelope-protocol/src/types.rs::PreampInputState` currently contains one `gain_raw` field alongside the active `mode`. `PreampState::from_cluster` maps one gain byte and one mode byte per input from cluster `0x18..0x1b`. That representation can display the active mode, but it cannot retain the other mode-specific gain assignments. A mode change must select an existing gain slot, not replace one universal input gain. Treat this as an implementation/state-model gap, separate from the confirmed wire offsets.

### Mixer and surface command families

Capture writes establish these command families:

- surface selection: `49 00 <surface>`;
- mixer strip writes: `d4 04 ...`;
- link writes: `a2 03 ...` and companion `a2 04 ...`.

`capture_02_volume_down.pcapng` and `capture_03_volume_up.pcapng` contain repeated mixer signatures beginning `d4 04 01 03`, with level parameters moving through values such as `0x58`, `0x56`, `0x3e`, `0x20`, and `0x02`. `capture_08_mixer_volume_faders_on_2_mixes.pcapng` contains `0x13a` mixer writes and `0x03` surface writes. The linked and single-channel captures contain mixer groups for both mixer selectors, including `d4 04 00 ...` and `d4 04 01 ...`.

`capture_04_mute_toggle.pcapng` contains repeated mixer signatures:

```text
d4 04 01 03 00 02 00 ...
d4 04 01 03 00 42 00 ...
```

These are capture-confirmed mixer state writes. Passive `0x73` deltas for mixer actions are dominated by meter and late-state changes; the captures do not isolate a single authoritative passive fader offset. Current query `0x18/0x00` readback remains the relevant capture-dialect candidate for mixer strip state, subject to the query-shape findings below.

### Link writes

Link captures contain:

- `a2 03 <selector> <enabled>` main writes;
- `a2 04 <bank> <enabled>` companion writes;
- enabled values `0x00` and `0x01`.

`capture_10_mixer_single_channel_one_change_at_a_time_then_unlink_comp12_and_go_individually_onc1andc2.pcapng` includes selector groups `a20301`, `a20311`, `a20315`, `a20400`, and `a20401`, plus mixer writes. `channel links in mixes.pcapng` contains only link-family writes and state reports, making it the cleanest link-specific test.

The command family is proven. Passive link byte mapping is not; see [Link vector comparison](#link-vector-comparison).

## Recursive scenario findings

### Assignment tables

All `channel_assignments` files were parsed as host writes plus device snapshots. They establish assignment-write coverage but mostly do not contain query replies.

- Host assignment frames are report `0x70` with `d3 41 <bank>` at frame offset `0x10`; table entries begin at frame offset `0x13`.
- Bank `0x05` carries `0x04` early-channel entries. Banks `0x06..0x09` carry `0x10` entries. The current encoder's hard-coded early entries `(0x03, entry_index)` for banks `0x03`, `0x06..0x09` match observed table structure.
- Index-map scenarios change assignment indices `0x04`, `0x08`, and `0x0c`; early-channel scenarios change `0x00..0x03`; ordinary and source-substitution scenarios change `0x00..0x0f`. Observed pair transitions include `0001>0800`, `0100>0800`, `0200>0800`, `0300>0800`, `0900>0102`, `0900>0106`, and `0900>0800`.
- `control panel open.pcapng` is the useful assignment readback capture: it contains q03 replies for sub `0x05..0x07` and decoded assignment pairs. Most assignment write scenarios, including the file named `assignment_write_readback_ordinary`, contain `q75=0x00`; they cannot validate readback.

Capture conclusion: current assignment table write geometry is supported. `assignment_readback` coverage is limited to control-panel-open traffic. Do not generalize readback support from filenames that contain “readback”.

### Meter lanes and preamp meter candidates

The `metering` directory supplies isolated signal controls rather than only aggregate deltas.

- `baseline - everything mute.pcapng` contains `0x37a` report-`0x73` snapshots and no mixer-lane values below the `0x60` no-signal baseline.
- `ch1.pcapng` through `ch16.pcapng` each activate exactly one lane. `chN` maps to lane index `N-1`, confirming body offsets `0x8e..0x9d` for CH01..CH16. Other lanes have no `<0x60` hits in these isolated files.
- `mix1-ch1+output.pcapng` and `mix-ch1+output.pcapng` activate lane `0x00`; `preamp1+ch1.pcapng` activates lane `0x00` with minimum `0x12`; `preamp2+ch2.pcapng` activates lane `0x01` with minimum `0x13`.
- `preamp 1.pcapng` holds `0xce=0x15` across snapshots with no meaningful `0xcf`; `preamp 2.pcapng` holds `0xcf=0x0c` across most snapshots with no meaningful `0xce`. The all-mute baseline has `0xce=0x51` and `0xcf=0x4b`, which the current `<=0x49` meter filter rejects.

Capture conclusion: lane index mapping is confirmed for all `0x10` mixer strips. `0xce` and `0xcf` are separate A1/A2 meter candidates with known baseline sentinels and mixed-signal caveats. They are not official cyclic `0x73` offsets.

### Mixer command and readback scenarios

The `mixer` directory contains `0x17` write-only scenario files. The `mixer_levels` and `mixer_pans` directories contain the query/readback evidence.

- `d4 04` writes carry mixer/surface, channel, attenuation level, and pan/mute/solo state. Observed command state transitions include unlinked mute `0x20>0x60`, linked mute `0x02>0x42` and `0x3e>0x7e`, and solo `0x20>0xa0`. Pan command values span `0x02..0x3e`.
- Unlinked fader scenarios use channel `0x01` levels through `0x5a`; linked CH03/04 fader scenarios use common level ranges with state codes `0x02`/`0x3e`. Surface-independence scenarios use distinct `d404 00 01` and `d404 01 01` sequences.
- `mixer_levels` contains q04/`0x00` and q04/`0x01` replies with `0x10` two-byte lanes. q04/`0x00` varies Mix 1 scenarios; q04/`0x01` varies Mix 2. CH01/02 map to indices `0x00/0x01`; CH03/04 map to `0x02/0x03`. Scenario levels are captured as attenuation: max `0x00`, `-18` scenario `0x12`, `-30` scenario `0x1e`, and min `-90` scenario `0x5a`.
- `mixer_pans` q04 replies independently vary the expected Mix 1 or Mix 2 lanes. Center is `0x20`; endpoint pan codes are `0x02` and `0x3e`; `0x5e` demonstrates a pan-like value with the mute flag preserved. The current q04 two-byte parser and pan-code mapping match these positions.
- q18/`0x00` vectors are effectively identical across level and pan scenario files, with mostly `0x60` levels and a fixed state pattern. They are not primary evidence for these scenario controls.

Capture conclusion: current `d4 04` command fields, q04 per-surface lanes, attenuation scale, and pan/state codes are supported. Current q18 and q04 meanings remain capture-dialect meanings, not official logical names.

### Link bitmap scenarios

The `mixer_links` directory contains `0x09` isolated linked-pair scenarios. Each file has an initial q0b/`0x03` bitmap, idempotent `a2 03 <selector> <enabled>` setup writes, and the same bitmap again. The writes are already reflected in each scenario baseline, so a naive command-to-next-query diff reports no transition.

Scenario-specific bitmap flips establish direct selector-to-index mapping:

| Surface/pair | Command selector | Bitmap index |
|---|---:|---:|
| Mix 1 CH01/02 | `0x00` | `0x00` |
| Mix 1 CH11/12 | `0x05` | `0x05` |
| Mix 1 CH13/14 | `0x06` | `0x06` |
| Mix 1 CH15/16 | `0x07` | `0x07` |
| Mix 2 CH01/02 | `0x10` | `0x10` |
| Mix 2 CH11/12 | `0x15` | `0x15` |
| Mix 2 CH13/14 | `0x16` | `0x16` |
| Mix 2 CH15/16 | `0x17` | `0x17` |

The q0b/`0x03` response is a full `0x18`-byte selector bitmap. This validates current bitmap shape and `startup_link_readback_from_bitmap` indexing. It does not validate passive late-byte link decoding: the standalone vectors at `0x8f`, `0xcf`, and `0xda..0xdf` remain outside current accepted patterns.

### Mute-state vectors

The `mutes/no signal` and `mutes/with signal` directories contain only filtered device snapshots, not host writes. Their filenames encode controlled Mix 1/Mix 2 CH01/02 mute states.

With signal, modal late vectors are:

| Surface | Both unmute | CH01 mute | CH02 mute | Both mute |
|---|---|---|---|---|
| Mix 1 `0xda..0xdd` | `0a/05/0a/05` | `01/01/01/01` | `00/06/00/06` | `60/60/60/60` |
| Mix 2 `0xde..0xdf` | `0a/05` | `01/01` | `00/06` | `60/60` |

No-signal baselines are different: Mix 1 `0xda..0xdd` is `5a/5a/5a/5a` for both-unmute and CH01 mute, but `60/60/60/60` for CH02 mute and both mute; Mix 2 `0xde..0xdf` is `5a/5a`, `01/01`, or `60/60` for the corresponding scenarios.

These stable scenario differences are stronger evidence than generic delta counts, but they are not yet a boolean field definition. Current `decode_mute_from_group` returns `None` for these captures because none match its accepted `0x51`/active patterns. Treat current passive mute mapping as unvalidated, not as disproven.

### Preamp mode and stored-gain scenarios

The `preamps` directory contains `0x0b` files covering A1/A2 Mic, Line, Hi-Z, phantom, phase, and idle states. Mode-switch captures directly show separate stored gains:

- A1 starts `0x0a/0x0a/0x00/0x00`; mode `0x01` settles `0x14/0x0a/0x01/0x00`; mode `0x02` settles `0x2d/0x0a/0x02/0x00`; Mic mode returns `0x0a/0x0a/0x00/0x00`.
- A2 starts `0x0a/0x0a/0x00/0x00`; mode `0x01` settles A2 gain `0xfa` with mode `0x01`; mode `0x02` settles A2 gain `0x2d` with mode `0x02`; Mic mode returns A2 gain `0x0a` with mode `0x00`.
- Phantom toggles bit `0x10`; phase toggles bit `0x40`. Mic gain sweeps cover `0x00..0x41`; Hi-Z covers `0x00..0x2d`; signed Line byte values include `0xfa`/`0xfb`.

Capture conclusion: each input has a distinct stored gain for each preamp mode. Current `PreampInputState.gain_raw` stores only one gain per input, and `PreampState::from_cluster` maps only one gain plus one mode from `0x18..0x1b`; current state cannot retain alternate mode gains. A future state model must use per-input/per-mode gain storage and must not overwrite inactive mode gains on `0x4f` changes.

## Query-reply evidence

`capture_02_volume_down.pcapng` contains `0x2e` host `0x74` requests and `0x2e` device `0x75` replies. Query ID/sub-ID pairs match one-for-one. `capture_07_dsp.pcapng` adds query families `0x15/0x00`, `0x0c/0x00`, `0x07/0x05`, and `0x07/0x06`.

All replies use fixed `0x140` transport frames. The following are observed nonzero body extents relative to frame offset `0x10`; a nonzero extent is a lower bound when trailing fields can be zero:

| Query | Observed reply body evidence | Current capture interpretation |
|---|---:|---|
| `0x0b/0x03` | nonzero bytes through `0x03` in `0x04` replies | selector bitmap body; zero-valued selectors hide semantic width |
| `0x17/0x00` | nonzero through `0x03` | four-byte startup state |
| `0x18/0x00` | nonzero through `0x41` | about `0x42` bytes of mixer structure |
| `0x04/0x00..0x03` | nonzero through `0x84` | larger startup selector/pan structures |
| `0x15/0x00` | nonzero through `0xb4` | indexed code table / DSP readback |
| `0x03/0x06..0x09` | nonzero through `0x3f` | assignment-table bodies |
| `0x0c/0x00` | nonzero through `0xb2` in `capture_07` | DSP/query-specific body |
| `0x07/0x05`, `0x07/0x06` | no nonzero body bytes in observed replies | command/query acknowledgement shape unresolved |

`capture_01_enumeration.pcapng` adds `0x75` replies for `0x01/0x00`, `0x00/0x00`, and `0x11/0x00`.

### Capture dialect versus official logical names

Capture evidence establishes that current device traffic does not expose official logical response lengths directly:

- captured `0x17/0x00` is a four-byte body, not the official reverb-return record array shape;
- captured `0x18/0x00` is about `0x42` bytes, not the official reverb-send record array shape;
- captured `0x04/*` bodies extend to about `0x85` bytes, not the official mixer array shape;
- every reply is padded into a fixed `0x140` transport frame.

Therefore current query decoders must remain capture-dialect decoders. Do not rename `0x17/0x00` or `0x18/0x00` to official reverb queries without a transport/version discriminator. Do not use fixed frame declaration `0x140` as semantic body length.

## Official schema cross-check

The official `docs/zengosc_report_format_1.1.26` file is a logical report schema. Capture evidence identifies a current fixed-frame dialect. Shared report/query numbers are not sufficient to prove shared wire layout.

### Cyclic report boundary

- Official cyclic `0x73` payload length is `0x17a`. Current `0x73` frames are `0x140` bytes, with a `0x10`-byte frame header and a current snapshot payload of `0xe6` bytes. An official `0x73` payload plus the current header would require `0x18a` bytes, so official offsets cannot be overlaid on current snapshot offsets.
- Official cyclic `0x83` is one `afx_meters.data` block spanning `0x00..0x12f`, total `0x130` bytes. That size matches the current post-header capacity, but captures only prove an unresolved `0x83` block; they do not prove official subfields.
- Current capture-confirmed offsets remain: sample `0x02`, clock `0x03`, outputs `0x0c..0x11`, preamp cluster `0x18..0x1b`, and mixer lanes `0x8e..0x9d`. Official `0x73` places output records at `0x36..0x95`, preamps at `0x98..0xa7`, and peak groups at `0xfa..0x179`. These are different layouts.

### Get-report shape comparison

| Query | Official logical response | Capture evidence | Disposition |
|---|---|---|---|
| `0x74/0x04/0x00` | `0x21` records × `0x09` bytes = `0x129` (`level`, `pan[0x06]`, `mute`, `solo`) | q04/`0x00` and q04/`0x01` expose `0x10` two-byte level/state lanes; replies can contain nonzero bytes through `0x84` | Current q04 parser is capture-scoped; do not call it official mixer decoding |
| `0x74/0x0b/0x03` | `0x40` one-byte `linked` records | Full `0x18`-byte selector bitmap; scenario flips map selectors directly to bitmap indices | Capture bitmap path is confirmed; official link shape is not this body |
| `0x74/0x17/0x00` | `0x04` records × `0x08` bytes = `0x20` reverb returns | Four meaningful body bytes | Capture q17 is not official reverb-return decoding |
| `0x74/0x18/0x00` | `0x21` records × `0x09` bytes = `0x129` reverb sends | About `0x42` meaningful bytes, effectively static across level/pan scenarios | Capture q18 is not official reverb-send decoding |
| `0x74/0x03/0x00` | one routing record of width `0x81` | Assignment captures mostly have no `0x75`; control-panel-open q03 replies are sub-ID-prefixed assignment tables | Keep assignment readback separate from official routing |

Other official get shapes, including feature mask `0x122` bytes and digital/reverb records, were not observed as matching bodies in these captures.

### Set-report comparison

Official logical set IDs include volume `0x07`, mute `0x08`, brightness `0x0e`, preamp type/gain/phantom/phase `0x0f..0x12`, routing `0x13`, mixer `0x14`, stereo link `0x22`, dim `0x26`, reverb `0x28..0x29`, and DAW mode `0x2a` under report `0x70`. Captures instead show current wire payload families:

- output volume `0x47`, mute `0x48`, dim `0x66`;
- preamp mode/gain/phantom/phase `0x4f..0x52`;
- mixer state `d4 04`;
- assignments `d3 41`;
- links `a2 03`/`a2 04`;
- surface `49 00`.

The report type is shared, but payload IDs and body grammars differ. Current encoders must remain capture-dialect encoders until a translation layer is demonstrated.

### Output and preamp implications

Official `0x73` volume records at `0x36..0x95` contain independent volume, mute, dim, mono, and trim fields. Capture scenarios prove only isolated current mode bytes: volume offsets `0x0c/0x0e/0x10`, mode offsets `0x0d/0x0f/0x11`, mute code `0x01`, and dim code `0x02`. No capture enables mute and dim together, so composition remains unresolved.

Official preamp records at `0x98..0xa7` contain type, phantom, HPF, phase, and zero-cross fields. Capture scenarios instead prove current cluster `0x18..0x1b`, including mode-scoped gains, phantom bit `0x10`, and phase bit `0x40`. The capture result is implementation evidence, not an official-offset mapping.

## Snapshot delta findings

### Stable low offsets

The following offsets are repeatedly confirmed by isolated controls:

| Snapshot offset | Evidence | Confidence |
|---:|---|---:|
| `0x02` | sample-rate command `0x03 <code>` | confirmed |
| `0x03` | clock-source command `0x04 <code>` | confirmed |
| `0x0c` | output target `0x00` volume | confirmed |
| `0x0d` | output target `0x00` mute/dim mode | confirmed |
| `0x0e` | output target `0x01` volume | confirmed |
| `0x0f` | output target `0x01` mute/dim mode | confirmed |
| `0x10` | output target `0x02` volume | confirmed |
| `0x11` | output target `0x02` mute/dim mode | confirmed |
| `0x18`/`0x19` | preamp input `0x00`/`0x01` gain | confirmed |
| `0x1a`/`0x1b` | preamp input `0x00`/`0x01` mode/flags | confirmed |

### Dynamic and unresolved offsets

Across volume, mixer, preamp, idle, and link captures, paired deltas frequently occur at:

- `0x6e`;
- `0x8e`;
- `0x9e`/`0x9f`;
- `0xb3`/`0xb4`;
- `0xce`/`0xcf`;
- `0xda..0xe2`;
- `0xf2`/`0xf3`;
- `0xea..0xed` in some mixer events.

These bytes change with meters, delayed state, or multiple simultaneous device updates. The captures do not justify assigning all such changes to one control.

The `0xcf` candidate appears in unrelated sample-rate, clock, mixer, mute, preamp, and idle streams. It remains an observed late-byte candidate, not a proven A2 meter field.

## Link vector comparison

Current `antelope-protocol/src/mixer.rs::decode_link_state` reads:

```text
0x8f, 0xcf, 0xda, 0xdb, 0xdc, 0xdd, 0xde, 0xdf
```

It accepts only:

- all eight bytes `0x49` => linked;
- heads `0x51/0x51` and all tails `0x4e` => linked;
- heads `0x4e/0x4e` and tails each `0x4c` or `0x4e` => unlinked.

The standalone `channel links in mixes.pcapng` capture contains `0x1d07` report-`0x73` snapshots. At those candidate offsets:

- `0x8f` and `0xcf` are always equal and level-like, commonly `0x45`, `0x46`, `0x47`, `0x48`, or `0x49`;
- `0xda..0xdf` are constant `0x60` throughout the capture;
- vectors such as `47/47/60/60/60/60/60/60` and `49/49/60/60/60/60/60/60` occur;
- none match any accepted linked or unlinked vector.

Conclusion: current passive-link mapping is not validated by the clean link capture. This does not disprove the mapping for another state/layout; it means decoder output should remain unresolved when no accepted vector occurs. The command writes themselves are proven.

## Implementation implications

### Confirmed safe knowledge

- Keep current snapshot offsets `0x02`, `0x03`, `0x0c..0x11`, and `0x18..0x1b`.
- Keep output target mapping `0x00 -> 0x0c`, `0x01 -> 0x0e`, `0x02 -> 0x10`.
- Keep output isolated mode codes `0x01 = mute` and `0x02 = dim` as capture-dialect observations.
- Store preamp gain per input and per mode; preserve each mode's gain while changing mode. Current wire gain bytes remain `0x18`/`0x19` for the active mode.
- Use delayed snapshot confirmation; first following `0x73` is not sufficient for output mode changes.
- Parse USBPcap payload at packet `header_len`.

### Must remain gated or unresolved

- Do not overlay official `0x73` offsets onto the current `0xe6` snapshot layout.
- Do not treat fixed frame declaration `0x140` as body length.
- Do not map current query IDs directly to official logical names.
- Do not label `0xcf` as A2 meter without a stronger invariant.
- Do not apply passive pan/link meanings to late bytes without capture corroboration.
- Do not treat one `PreampInputState.gain_raw` as complete mode-state storage; retain separate gain assignments per mode.
- Do not claim mute-plus-dim composition until a capture enables both in sequence and records the resulting mode byte.

### Recommended next capture

One targeted capture would resolve the remaining output-state question:

1. start from normal output state;
2. enable mute on one output;
3. without disabling mute, enable dim on the same output;
4. record at least `0x20` settled `0x73` snapshots;
5. disable dim, then disable mute;
6. repeat for one additional output target.

Record the target mode byte (`0x0d`, `0x0f`, or `0x11`) and command-to-snapshot delay. This distinguishes enum replacement from independent bit composition.

## Limitations and reproducibility

- Pairing uses host-write order and subsequent `0x73` snapshots; it does not model every USB scheduling boundary.
- An observed nonzero extent is not an exact body length when trailing fields can be zero.
- Meter and late-state bytes change during otherwise isolated actions; byte deltas alone are insufficient semantic proof.
- Endpoint `0x84` traffic is intentionally excluded because its `0x840`/`0x870` data is not protocol-frame-shaped.
- All raw packet bytes were processed in sandboxed parsers. This document records derived offsets, counts, transitions, and signatures rather than dumping payloads.
- Existing unrelated working-tree changes were left untouched.
