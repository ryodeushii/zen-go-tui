# Zen Go Synergy Core USB Protocol Analysis

## Scope

This document is the canonical analysis of the Wireshark captures in `antelope_pcap/`.

It covers the device control plane only.
The audio-streaming endpoints are present in the captures but are not part of the vendor control protocol described here.

The main conclusion is straightforward:

- The Windows control panel is a thin client.
- It sends short HID interrupt writes on endpoint `0x01`.
- The device continuously publishes state on endpoint `0x82`.
- The host does not need a local shadow model if it parses the device snapshots correctly.

## Capture Catalog

| Capture | Coverage | Confirmed command families |
|---|---|---|
| `capture_01_enumeration.pcapng` | USB enumeration, startup metadata/state bootstrap | `0x74`, `0x75`, `0x73`, `0x83` |
| `capture_01_enumeration_diff_filter.pcapng` | Filtered view of the same startup exchange | `0x74`, `0x75`, `0x73`, `0x83` |
| `capture_02_volume_down.pcapng` | Actual traffic is a mixer-channel fade-down sequence, not master-output volume | `0x70/0x16`, `0x74`, `0x73`, `0x83`, `0x81` |
| `capture_03_volume_up.pcapng` | Matching mixer-channel fade-up sequence | `0x70/0x16`, `0x73`, `0x83`, `0x81` |
| `capture_04_mute_toggle.pcapng` | Mixer-channel mute/unmute toggle | `0x70/0x16`, `0x73`, `0x83`, `0x81` |
| `capture_05_sample_rate_in_the_end_48_32_44.1_44.1default.pcapng` | Full sample-rate sweep | `0x70/0x12`, `0x73`, `0x83`, `0x81` |
| `capture_06_clock_source_no_word_clock_exists.pcapng` | Clock-source selection among Internal/S/PDIF/USB | `0x70/0x12`, `0x73`, `0x83`, `0x81` |
| `capture_07_dsp.pcapng` | Preamp/DSP toggles and opaque advanced control writes | `0x70/0x13`, `0x70/0x14`, `0x70/0x16`, `0x70/0x1a`, `0x70/0x1c`, `0x70/0x23`, `0x74`, `0x73`, `0x83`, `0x81` |
| `capture_08_mixer_volume_faders_on_2_mixes.pcapng` | Mixer-send fades on both mixers and multiple channel pairs | `0x70/0x16`, `0x70/0x13` |
| `capture_09_idle_polling.pcapng` | Idle behavior and steady-state polling | `0x73`, `0x83`, `0x81` |
| `capture_10_2_mixers_linked_comp12_fades_mute_unmute.pcapng` | Linked COMP1/COMP2 behavior plus link-state toggles around that workflow | `0x70/0x16`, `0x70/0x13`, `0x70/0x14` |
| `capture_10_mixer_single_channel_one_change_at_a_time_then_unlink_comp12_and_go_individually_onc1andc2.pcapng` | Single-channel mixer actions, then unlinked COMP1/COMP2 | `0x70/0x16`, `0x70/0x13`, `0x70/0x14` |
| `capture_11_output_controls_once_change_at_a_time.pcapng` | Master output page: Monitor, HP1, HP2 volume plus the captured DIM/MUTE boolean process | `0x70/0x13`, `0x73`, `0x83`, `0x81` |

## Transport Summary

| Endpoint | Direction | Typical length | Role |
|---|---|---:|---|
| `0x01` | host -> device | 320 bytes | Vendor HID control writes |
| `0x82` | device -> host | 320 bytes | Main state snapshots, query replies, auxiliary state |
| `0x81` | device -> host | 6 bytes | High-rate async notification/heartbeat |
| `0x84` | device -> host | 2112 / 2160 bytes | Audio stream, ignored here |
| `0x05` | host -> device | 320 / 2112 bytes | Audio/path data, ignored here |

All vendor control traffic is carried as HID interrupt reports padded to 320 bytes on the wire.

## Frame Types

All multi-byte integers below are little-endian unless noted.

### Common header

| Offset | Size | Meaning |
|---|---:|---|
| `0x00` | 4 | Frame type |
| `0x04` | 4 | Meaningful frame length |
| `0x08` | 8 | Type-specific header area |
| `0x10` | ... | Payload for `0x70`, `0x73`, `0x83` |

### Observed frame types

| Type | Direction | Meaning |
|---|---|---|
| `0x70` | host -> device | State-changing command |
| `0x73` | device -> host | Main dynamic state snapshot |
| `0x74` | host -> device | Short query |
| `0x75` | device -> host | Query reply |
| `0x83` | device -> host | Auxiliary mostly-static state block |

`0x74` and `0x75` are the exception to the `0x10` payload rule.
Their meaningful data begins immediately at `0x08`.

## Startup and Discovery

At startup the host issues three short queries:

| Host frame | Body at `0x08` | Device reply | Meaning |
|---|---|---|---|
| `74 00 00 00 10 00 00 00` | `01 00 00 00 00 00 00 00` | `0x75` | Product metadata |
| `74 00 00 00 10 00 00 00` | `00 00 00 00 00 00 00 00` | `0x75` | Short capability/default block |
| `74 00 00 00 10 00 00 00` | `11 00 00 00 00 00 00 00` | `0x75` | Small capability/status value |

### Metadata reply

The `query = 0x01` reply contains plain ASCII strings.
Observed fields:

- Product name: `Zen Go Synergy Core`
- Serial-like identifier: `4502721001300`
- Firmware/software version string: `6.6`

## Stable-State Rule

The device usually emits multiple `0x73` frames after a host write.
The first post-write snapshot is not always the settled state.

For reliable decoding:

- Apply the host command.
- Keep consuming `0x73` frames.
- Treat the last `0x73` before the next host write as the stable state.

This matters for sample-rate changes in particular.

## Main Device State: `0x73`

`0x73` is the primary source of truth for the control panel.

### Confirmed global fields

Offsets below are relative to the `0x73` payload start at `0x10`.

| Payload offset | Size | Meaning | Evidence |
|---|---:|---|---|
| `0x00` | 1 | Constant `0x08` in all observed captures | All `0x73` frames |
| `0x01` | 1 | Usually `0x00`; occasional transient `0x08` even while idle | `capture_09` |
| `0x02` | 1 | Sample-rate code | `capture_05` |
| `0x03` | 1 | Clock-source code | `capture_06` |
| `0x04..0x07` | 4 | Sample rate as big-endian integer | `capture_05`, `capture_06` |
| `0x08..0x0a` | 3 | Front-panel / preamp mode bytes | `capture_07` |
| `0x0c` | 1 | Monitor output volume | `capture_11` |
| `0x0d` | 1 | Monitor output mode | `capture_11` |
| `0x0e` | 1 | HP1 output volume | `capture_11` |
| `0x0f` | 1 | HP1 output mode | `capture_11` |
| `0x10` | 1 | HP2 output volume | `capture_11` |
| `0x11` | 1 | HP2 output mode | `capture_11` |
| `0x18..0x1b` | 4 | DSP / preamp mode cluster | `capture_07` |
| `0x6a` | 1 | Surface/context selector mirrored into the snapshot | `capture_10*`, `capture_11` |

### Sample-rate code map

| Code | Rate |
|---|---:|
| `0x00` | 32000 |
| `0x01` | 44100 |
| `0x02` | 48000 |
| `0x03` | 88200 |
| `0x04` | 96000 |
| `0x05` | 176400 |
| `0x06` | 192000 |

### Clock-source code map

| Code | Source |
|---|---|
| `0x00` | Internal |
| `0x01` | S/PDIF |
| `0x02` | USB |

### Output mode bytes in `0x73`

These bytes are relative to the `0x73` payload start at `0x10`.

| Payload offset | Output | Values observed |
|---|---|---|
| `0x0d` | Monitor | `0x00` normal, `0x01` mute, `0x02` dim |
| `0x0f` | HP1 | `0x00` normal, `0x01` mute, `0x02` dim |
| `0x11` | HP2 | `0x00` normal, `0x01` mute, `0x02` dim |

This mapping is confirmed from `capture_11_output_controls_once_change_at_a_time.pcapng` using the user-confirmed action order.

Together with the volume bytes below, this gives a compact repeated output tuple layout:

| Payload offset | Meaning |
|---|---|
| `0x0c..0x0d` | Monitor `{volume, mode}` |
| `0x0e..0x0f` | HP1 `{volume, mode}` |
| `0x10..0x11` | HP2 `{volume, mode}` |

### Output volume bytes in `0x73`

These bytes are also relative to the `0x73` payload start at `0x10`.

| Payload offset | Output |
|---|---|
| `0x0c` | Monitor volume |
| `0x0e` | HP1 volume |
| `0x10` | HP2 volume |

These bytes track the final `0x47 oo vv` command value directly.

### Additional output-related `0x73` bytes

The output mode bytes above are the primary state flags.
`capture_11` also shows stable secondary bytes tied to output-page state:

| Payload offset | Observed role | Notes |
|---|---|---|
| `0x6a` | Current surface selector | `0x0f` on the Monitor/HP1 surface, `0x0c` on the HP2 surface |
| `0x6e` / `0x8e` / `0xce` | Repeated row-head / attenuation-style bytes | Stable output/mixer changes hit these coarse row-heads; `0x6e` tracks only the active Monitor/HP1 vs HP2 surface group switch, while `0x8e` and `0x0ce` carry most post-switch output/mixer state changes |
| `0x6f` / `0x8f` / `0xcf` | Repeated row-local state bytes | `0x0cf` is the noisiest stable row-local byte across output, mixer, and DSP workflows; current captures support a dense local-status interpretation rather than one direct level enum |
| `0xda..0xe5` | Late output-state shadow cluster | Stable output, mixer, and DSP transitions perturb this region in smaller confirmed subclusters rather than as one always-dense block |

Observed recurring state values in those later tables:

- `0x60` behaves like the neutral/default late-row baseline
- `0x5a` behaves like the common active baseline once a row/surface is live
- `0x54` behaves like a stronger alternate-engaged tier below full extended DSP mode; current stable evidence links it to dim-style and stronger mixer-state transitions, but not to one unique user-facing concept such as mute, dim, or link by itself
- `0x51` is not a generic fourth row-head value: current stable evidence ties it specifically to the extended DSP/preamp mode entered by `51 01 01`
- `0x32` behaves like a default filler value in neighboring table entries

The cleanly proven part is:

- `MUTE` changes the primary per-output mode byte to `0x01`
- `DIM` changes the primary per-output mode byte to `0x02`
- `0x49` changes payload byte `0x6a`, which mirrors the current output surface (`0x0f` vs `0x0c`)
- the same surface-select writes also retune `0x6e` and `0x6f` together (`0x60 <-> 0x5a` in the observed output/mixer captures), making row `0x6e` the clearest surface-group latch for the current `0x6a` context rather than a row that carries the later HP2-local control deltas itself
- output volume / mute / dim all perturb row-local byte `0x0cf`; stronger evidence now shows this byte is not one compact level or mode code: the same output level can settle with different `0x0cf` values, and the same `0x0cf` value recurs across wide output-level ranges. The best-supported current model is a dense row-local status code that mixes selection/focus context with local mode/progression within the current row/page.
- output volume, some mute/unmute transitions, and some dim transitions also retune coarse row-heads `0x8e` and `0x0ce`, usually together with shadow bytes `0x0da..0x0dd` and `0x0e2`; this block behaves like a coarse shadow of the row-head state
- `0x0de..0x0df` act like a second finer shadow pair: they often move on local dim/mute/state changes even when `0x8e`, `0x0ce`, and `0x0da..0x0dd` plus `0x0e2` do not. `0x0de` is the more frequently independent byte in current captures and currently fits a local engaged-tier latch better than a commit/apply pulse; `0x0df` behaves more like a finer local substate/index byte.

### Repeated late-payload table structure

The late `0x73` payload is not random filler.
Current captures show a repeated row pattern with a `0x20` stride, but the active stable bytes are sparse rather than uniformly dense:

| Row base | Stable bytes with strongest evidence | Notes |
|---|---|---|
| `0x6e` | `0x6e`, `0x6f` | Surface-group row. `0x49` surface-select writes move both bytes together, but later HP2-local writes still land mainly on `0x8e` / `0x0ce`. Current evidence fits UI-surface partitioning (`Monitor/HP1` vs `HP2`) better than a per-physical-output or per-hardware-path row map. |
| `0x8e` | `0x8e`, sometimes `0x8f` | Cross-surface coarse row. Output volume/dim, mixer topology changes, and DSP mode changes often move `0x8e`. `0x8f` is mostly quiet except in the extended DSP enter/exit pair. |
| `0xae` | `0xae`, `0xaf`, `0xb0`, `0xb1`, `0xb3`, `0xb4` | Not active in ordinary output/mixer captures. The only strong stable evidence is the extended DSP enter/exit path in `capture_07`, so this row should currently be treated as DSP-page state for the richer preamp/front-end mode rather than a general fourth mixer/output row. |
| `0xce` | `0xce`, `0xcf` | Global late row. `0x0cf` is the most consistently moving late byte across output, mixer, topology, and DSP workflows, and is best treated as a dense row-local status code rather than a scalar field. |

This is the strongest currently supported structural interpretation of the late payload:

- the block is table-like rather than a flat bag of flags
- several row anchors are spaced exactly `0x20` bytes apart
- the stable bytes are not laid out as one uniformly active fixed-width row per anchor; in ordinary output/mixer traffic the strongest repeatable bytes are `base+0`, `base+1`, plus the shared shadow cluster
- `0x6f`, `0x8f`, `0xaf`, and especially `0xcf` behave like row-local substate bytes rather than direct volume fields; `0x0cf` is best treated as a dense mixed local-status byte, not a single scalar or clean enum
- `0x6e`, `0x8e`, and `0xce` flip among a small coarse-state set where `0x60` is neutral/default, `0x5a` is the common active baseline, and `0x54` is a stronger alternate-engaged tier
- `0x51` should currently be treated as an extended-DSP/global-row state rather than a general-purpose row-head enum: on `51 01 01`, rows `0x8e` and `0x0ce` move `0x54 -> 0x51`, while the DSP-only `0x0ae` row instead moves `0x60 -> 0x5a`
- the shared shadow cluster is itself split into smaller confirmed groups:
  `0x0da..0x0dd` plus `0x0e2` track coarse row-head changes at `0x8e` / `0x0ce`, while `0x0de..0x0df` behave like a secondary local shadow pair that can move independently on dim/mute/state transitions
- the DSP-only expansion around `0x0ae` is real, but current evidence supports it only for the extended `510101` / `a2000000` enter-exit pair and nearby advanced DSP writes; it behaves like a page-specific DSP/preamp editor-page substate block that becomes meaningful only after the richer front-end mode is entered

### Other `0x73` regions

The rest of the payload clearly contains the mutable mixer/output state tables.
Those fields move when controls are changed, and they are the reason the host does not need its own shadow state.

Confirmed mutable regions include the following `0x73` payload offsets:

- front global/state bytes: `0x0c..0x11`, `0x18..0x1b`
- surface/context selector: `0x6a`
- repeated table rows: `0x6e..0x71`, `0x8e..0x91`, `0xae..0xb4`, `0xce..0xd3`
- late shadow cluster: `0xda..0xe5`

The exact field-by-field layout of those tables is not fully decoded yet, but the stable state changes are device-originated and consistent.

## Auxiliary Device State: `0x83`

`0x83` is another 320-byte device-to-host block.

Observed behavior:

- It is emitted continuously alongside `0x73`, typically in a strict `0x83`/`0x73` alternation on endpoint `0x82`.
- During pure idle (`capture_09`) it is completely constant while `0x73` still jitters in a few late-table bytes.
- In the output and mixer captures (`capture_10`, `capture_10_2`, `capture_11`), there is no stable `0x83` delta at all after host writes once transient inter-frame jitter is ignored.
- Stable `0x83` changes also appear during the sample-rate sweep (`capture_05`), but not during the clock-source sweep (`capture_06`).
- Across the captures that do show stable `0x83` movement (`capture_05`, `capture_07`), the deltas stay concentrated in the first `14` payload bytes, with the strongest repeatable activity at payload offsets `0x00`, `0x02`, `0x04`, `0x05`, `0x06`, `0x08`, `0x09`, and `0x0a`.
- The repeating front pattern in those moving captures is strongly pair-structured: bytes `(0,1)`, `(2,3)`, and `(4,5)` often behave like three little-endian 16-bit selector/code fields, while bytes `0x06..0x0a` look like a smaller auxiliary sub-block rather than mixer/output state.

Practical interpretation:

- `0x73` is the dynamic state block you must parse first.
- `0x83` is a secondary block containing auxiliary context or lookup-style state, not the canonical live-control snapshot.
- Current evidence points to the front of `0x83` carrying compact auxiliary selectors or mode/capability context tied to DSP/front-end workflows and sample-rate family changes, rather than mixer/output levels.

## Async Notification: `0x81`

Endpoint `0x81` returns 6-byte reports at high rate, even while idle.

Observed properties:

- Present in all non-trivial captures
- Continues during idle polling with a median inter-arrival time of about `3 ms` in `capture_09`, but arrives in bursts rather than at a perfectly uniform cadence
- Byte `0x05` is usually `0x00`, but not universally: rare bursts of `0xff` are present in `capture_11`, and `0x01` / `0xff` also appear in a few DSP-capture packets
- Bytes `0x00..0x04` change constantly and do not line up with a simple per-control state mirror
- Byte `0x00` is almost always `0x00`, with rare one-packet pulses to `0x01` or `0x02` during idle that do not coincide with any stable `0x73`/`0x83` state change
- Notifications become denser around many host-write windows in the busy mixer/output captures, but most packets still occur outside any narrow write-adjacent window; this makes `0x81` useful as event-adjacent timing noise, not a clean commit marker
- Bytes `0x01..0x04` are not a simple monotonic counter in either little-endian or big-endian interpretation, and they also do not split cleanly into two monotonic 16-bit counters
- Does not by itself expose the full control-panel state

Treat it as async notification, sequencing, or heartbeat data.
It is useful for event timing, but not sufficient as a standalone state source.

## Confirmed Host Command Families

### `0x70 / length 0x12`: global clock/sample commands

Payload starts at `0x10`.

#### Sample rate

Format:

```text
70 00 00 00 12 00 00 00 00 00 00 00 00 00 00 00 03 rr
```

`rr` uses the sample-rate code table above.

Examples:

- `03 00` -> 32 kHz
- `03 01` -> 44.1 kHz
- `03 02` -> 48 kHz
- `03 03` -> 88.2 kHz
- `03 04` -> 96 kHz
- `03 05` -> 176.4 kHz
- `03 06` -> 192 kHz

#### Clock source

Format:

```text
70 00 00 00 12 00 00 00 00 00 00 00 00 00 00 00 04 cc
```

`cc` uses the clock-source code table above.

Examples:

- `04 00` -> Internal
- `04 01` -> S/PDIF
- `04 02` -> USB

### `0x70 / length 0x13`: output-page commands

#### Surface select

Format:

```text
70 00 00 00 13 00 00 00 00 00 00 00 00 00 00 00 49 00 ss
```

Observed selectors:

| Selector | Meaning |
|---|---|
| `0x0f` | Monitor/HP1 surface |
| `0x0c` | HP2 surface |

#### Master output volume

Format:

```text
70 00 00 00 13 00 00 00 00 00 00 00 00 00 00 00 47 oo vv
```

| Field | Meaning |
|---|---|
| `oo` | Output index |
| `vv` | Volume step |

Observed output indexes:

| Index | Output |
|---|---|
| `0x00` | Monitor |
| `0x01` | HP1 |
| `0x02` | HP2 |

Observed volume range:

- `0x00 .. 0x60`
- Ramps are sent as many consecutive single-step writes

#### Output mute

Format:

```text
70 00 00 00 13 00 00 00 00 00 00 00 00 00 00 00 48 oo bb
```

Observed values:

- `48 00 01`, `48 00 00`
- `48 01 01`, `48 01 00`
- `48 02 01`, `48 02 00`

Mapped from the confirmed action order in `capture_11_output_controls_once_change_at_a_time.pcapng`:

- volume adjustments
- mute toggle
- dim toggle

So this family is the output `MUTE` command.
It updates the corresponding `0x73` output mode byte from `0x00` to `0x01` and back.

#### Output dim

Format:

```text
70 00 00 00 13 00 00 00 00 00 00 00 00 00 00 00 66 oo bb
```

Observed values:

- `66 00 01`, `66 00 00`
- `66 01 01`, `66 01 00`
- `66 02 01`, `66 02 00`

Mapped from the confirmed action order in `capture_11_output_controls_once_change_at_a_time.pcapng`.

This family is the output `DIM` command.
It definitely changes the stable `0x73` state.
It updates the corresponding `0x73` output mode byte from `0x00` to `0x02` and back.
Unlike mute, it also changes additional attenuation-related state bytes in later `0x73` regions.

### `0x70 / length 0x16`: mixer send / per-channel state

Format:

```text
70 00 00 00 16 00 00 00 00 00 00 00 00 00 00 00 d4 04 mm cc ll pp
```

| Field | Meaning |
|---|---|
| `mm` | Mixer index |
| `cc` | Channel index |
| `ll` | Level step |
| `pp` | Pan/state byte |

#### Mixer index

| `mm` | Meaning |
|---|---|
| `0x00` | MIX 1 / Monitor-HP1 mixer |
| `0x01` | MIX 2 / HP2 mixer |

#### Channel index

Observed indexes are consistent with 1-based mixer-strip numbering.
Confirmed values include `0x01`, `0x03`, `0x04`, `0x05`, `0x06`, `0x09`, `0x0a`, `0x0d`.

#### Level step

Observed range for fades:

- `0x00 .. 0x58`

#### Pan/state byte

The last byte is not a simple boolean.
It combines a base pan position with flag bits.

Confirmed base values:

| Base value | Meaning |
|---|---|
| `0x02` | Left side of a linked stereo pair |
| `0x20` | Center / mono default |
| `0x3e` | Right side of a linked stereo pair |

Confirmed flag bits:

| Bit | Meaning |
|---|---|
| `0x40` | Mute flag |
| `0x80` | Additional channel flag seen in DSP capture, semantic unresolved |

Examples:

- `d4 04 00 03 2c 02` -> MIX1, channel 3, level `0x2c`, left-panned, unmuted
- `d4 04 00 03 00 42` -> same channel, muted (`0x02 + 0x40`)
- `d4 04 00 04 2c 3e` -> MIX1, channel 4, level `0x2c`, right-panned, unmuted
- `d4 04 00 04 00 7e` -> same channel, muted (`0x3e + 0x40`)
- `d4 04 01 01 00 60` -> MIX2, channel 1, centered and muted (`0x20 + 0x40`)
- `d4 04 01 01 00 e0` -> same channel with the additional `0x80` flag asserted

#### Linked-channel behavior

Linked stereo pairs are not represented by one combined command.
The host sends one write per side.

Observed pattern in `capture_10_2_mixers_linked_comp12_fades_mute_unmute.pcapng`:

- Left member: `... 03 <level> 02`
- Right member: `... 04 <level> 3e`
- Muted pair: `... 03 00 42` and `... 04 00 7e`

After unlinking in `capture_10_...onc1andc2.pcapng`, the paired writes stop and channels are controlled independently.

### `0x70 / length 0x14`: short channel/selection commands

Observed payloads include:

- `a2 03 01 01`
- `a2 03 01 00`
- `a2 04 01 01`
- `a2 04 01 00`
- `a2 03 11 00`
- `a2 03 11 01`

This family appears in mixer and DSP-oriented captures and is strongly associated with link/unlink state rather than level data.

What is confirmed from `capture_10`, `capture_10_2`, `capture_07`, and the capture notes:

- `a2030101` appears immediately before the linked `COMP 1/2` fade and mute/unmute sequence in `capture_10_2`.
- `a2040100` followed by `a2030100` appears immediately before the unlinked `COMP 1` / `COMP 2` individual control sequence in `capture_10`.
- `a2040101` followed by `a2030101` appears before relinking on the first mixer surface in `capture_10`.
- After switching surfaces with `49000c`, `a2031100` and later `a2040101 + a2031101` appear around the corresponding `COMP` workflow on the other mixer surface.
- In `capture_07`, `a2040101 + a2031101` is followed by a run of `a2031201` through `a2031701` selector-like writes while the DSP/preamp front-byte cluster at `0x18..0x1b` stays unchanged.

Current best decomposition:

- byte `0` = family marker `0xa2`
- byte `1` = subfamily (`0x03` and `0x04` both observed)
- byte `2` = selector / target id (`0x01`, `0x11`, and in DSP capture `0x12..0x17`)
- byte `3` = state byte; `0x01` means linked / asserted, `0x00` means unlinked / cleared in the confirmed `COMP1/2` workflows

Additional narrowing from command adjacency:

- `a2 04 01 xx` appears only as a companion write immediately adjacent to `a2 03 ss xx`
- in the current captures, `a204` is seen only with selector `0x01`; there is no observed `a20411xx` or `a20412..17xx`
- confirmed paired forms are `a2040100 + a2030100`, `a2040101 + a2030101`, and `a2040101 + a2031101`
- `a2030101` can also appear by itself at the start of an already-linked workflow (`capture_10_2`)
- when `a204010x` is compared in isolation against the last stable `0x73` before the next host write, it shows no stable `0x73` or `0x83` delta by itself in the current captures
- the stable topology/selection delta appears on the following `a203...` write, usually at `0x73` payload byte `0x0cf` and, for some relink cases, also at `0x8e`, `0xce`, `0xda..0xdf`, and `0xe2`
- this makes `a204` look like a helper/master latch for the same topology family rather than an unrelated target selector

Selector-level interpretation that is supported by current evidence:

- selector `0x01` is the first mixer-surface `COMP1/2` link target used in `capture_10` and `capture_10_2`
- selector `0x11` is the corresponding target on the other mixer surface after `49000c` switches to the `HP2` / `MIX2` context
- selectors `0x12..0x17` are not link toggles in the current evidence; in `capture_07` they behave like DSP/preamp local selectors because they do not change the packed DSP mode bytes at `0x18..0x1b` or `0x83`, and mostly only retune row-local late byte `0x0cf`

Still unresolved:

- the exact difference between the `a203` and `a204` subfamilies, although `a204` is only seen with selector `0x01`
- the precise UI labels behind selector values `0x11` and DSP-side `0x12..0x17`

### `0x70 / length 0x53`: mixer strip assignment table writes

The assignment captures (`capture_mixer_03_assignment_core.pcapng`,
`capture_mixer_04_assignment_extended.pcapng`) introduce a distinct host write family:

```text
70 00 00 00 53 00 00 00 00 00 00 00 00 00 00 00 d3 41 bb ...
```

What is confirmed from the current evidence:

- the logical assignment family marker is `d3 41`
- the third payload byte (`bb`) behaves like a bank/subwrite selector rather than the source id itself
- strip `11` assignment sweeps emit four near-identical writes with `bb = 0x06`, `0x07`, `0x08`, `0x09`
- `capture_mixer_18_surface_independence_assignment.pcapng` also contains a `d3 41` write, but with `bb = 0x05` and an early-strip payload shape; that aligns with the current decision to keep strips `1..4` / AFX-adjacent assignment encoding out of scope for this pass

For the ordinary-strip sweeps on strip `11`, the only changing table entry is the zero-based entry `10` inside the `d3 41` payload body.
That entry sits at payload offsets `0x17..0x18` relative to the start of the `d3 41` payload (after bytes `d3 41 bb`).
This is the strongest current evidence that the assignment target is encoded as a table-entry position, with strip `11` mapped to entry `10`.

Observed strip-11 entry values from the action-ordered sweeps:

| Entry bytes | Mapping status | Source | Evidence |
|---|---|---|---|
| `00 00` | Confirmed | `Preamp 1` | `capture_mixer_03`, host frames `18716..18728` |
| `00 01` | Confirmed | `Preamp 2` | `capture_mixer_03`, host frames `22308..22320` |
| `01 00` | Confirmed | `Computer Play 1` | `capture_mixer_03`, host frames `25832..25844` |
| `01 01` | Confirmed | `Computer Play 2` | `capture_mixer_03`, host frames `29298..29310` |
| `01 07` | Confirmed | `Computer Play 8` | `capture_mixer_04`, host frames `14635..14648` |
| `02 00` | Confirmed | `SPDIF In 1` | `capture_mixer_04`, host frames `27456..27468` |
| `02 01` | Confirmed | `SPDIF In 2` | `capture_mixer_04`, host frames `31214..31226` |
| `08 00` | Confirmed | `Mute` | `capture_mixer_03`, host frames `32574..32586`; `capture_mixer_04`, host frames `42258..42270` |
| `09 00` | Confirmed | `Oscillator 1` | `capture_mixer_04`, host frames `35062..35074` |
| `09 01` | Confirmed | `Oscillator 2` | `capture_mixer_04`, host frames `38842..38854` |
| `0a 00` | Confirmed | `Emu Mic 1` | `capture_mixer_04`, host frames `19228..19242` |
| `0a 01` | Confirmed | `Emu Mic 2` | `capture_mixer_04`, host frames `23962..23974` |

Strong candidate interpolation from the unchanged ordinary-strip entries that appear in the same `d3 41` tables:

| Entry bytes | Status | Likely source | Why |
|---|---|---|---|
| `01 02` .. `01 06` | Candidate | `Computer Play 3` .. `Computer Play 7` | Present as stable ordinary-strip table entries in the same assignment writes, and `01 00`, `01 01`, `01 07` are action-confirmed as `Computer Play 1`, `2`, `8` |

Important boundary:

- the early-strip entries visible in the same tables include `03 00`, `03 01`, `03 02`, `03 03`
- those values likely belong to the special strips `1..4` / AFX-capable area and should **not** be merged into the ordinary-strip enum yet

#### Stable device state after assignment writes

The repo's existing stable-state rule continues to hold here: use the **last** `0x73` before the next host write as the settled device-side result.

What the assignment captures show:

- stable assignment changes are reflected in `0x73`, not in `0x83`
- there is **no** stable `0x83` delta in any confirmed assignment step from captures `03`, `04`, or `18`
- the assignment-related `0x73` deltas remain in the late payload region rather than the front global bytes

Most repeatable changing `0x73` offsets across the confirmed assignment transitions:

- `0x98`
- `0xcf`
- `0xda`, `0xdc`, `0xde`
- sometimes `0xdf`
- oscillator transitions additionally move `0x6e`, `0x8e`, `0xce`, `0xdb`, `0xdd`, `0xe2`

Examples:

- `capture_mixer_03`, `00 00 -> 00 01` (`Preamp 1 -> Preamp 2`) changes the stable `0x73` tuple at `0x98`, `0xda`, `0xdc`, `0xde`
- `capture_mixer_03`, `01 00 -> 01 01` (`Computer Play 1 -> Computer Play 2`) does **not** produce an additional stable `0x73` delta in the current idle capture
- `capture_mixer_04`, `09 00 -> 09 01` (`Oscillator 1 -> Oscillator 2`) changes `0xdb`, `0xdd`, `0xdf`

This is enough to say that durable assignment state does reach `0x73`, but not enough to claim a complete passive exact-source decoder from `0x73` alone yet.
The host-side `d3 41` table is the cleanest reusable representation currently available for parser/model work.

### `0x70 / length 0x16`: mixer pan writes

The dedicated pan captures (`capture_mixer_07_pan_mono_strip(ch1,preamp1).pcapng`,
`capture_mixer_08_pan_oaur_Strip(ch3,ch4 - cp1,cp2).pcapng`) confirm that pan uses the same
host family as ordinary mixer level writes:

```text
d4 04 <mixer> <channel> <level> <pan>
```

What is confirmed from the host side:

- the tested pan sweeps keep `<mixer> = 0x00` (`MIX 1`)
- the tested mono strip uses `<channel> = 0x01`
- the tested playback-pair members use `<channel> = 0x03` and `<channel> = 0x04`
- the pan sweep varies the final byte while keeping `<level> = 0x00`
- the observed pan byte domain is the contiguous range `0x02 .. 0x3e`
- `0x20` is the grounded center value
- the currently implemented app constants are confirmed by capture:
  - `0x02` = hard-left edge
  - `0x20` = center
  - `0x3e` = hard-right edge

The mono-strip capture shows a full sweep across that range:

- center -> left reaches `0x20 -> 0x02`
- left -> center climbs back to `0x03 .. 0x20`
- center -> right continues through `0x21 .. 0x3e`
- right -> center returns through `0x3d .. 0x20`

The playback-pair capture grounds the same encoding on two different selectors:

- left pair member (`channel = 0x03`) sweeps from `0x02` up to `0x3e`
- right pair member (`channel = 0x04`) also sweeps from `0x3e` down to `0x02` and back
- there is no separate playback-pair-only pan family in the current evidence

This is the strongest current encoding model:

- pan is one scalar byte shared by mono strips and playback-pair members
- the UI range is asymmetric in raw form because it includes both endpoints plus center (`0x02 .. 0x3e`, center `0x20`)
- current `PanState` in code is therefore only a partial enum of grounded anchor values, not a full pan model

#### Stable device-side state after pan writes

The device-side evidence is weaker than the host-side write mapping.

What is confirmed:

- pan writes do not produce stable `0x83` changes in the tested captures
- stable pan-related `0x73` effects, when visible, remain in the late mixer rows rather than the front global bytes
- the clearest mono-strip leftward edge transition (`0x20 -> 0x02`) changes late row-local bytes at payload offsets:
  - `0x8f` (`0x4e -> 0x4c`)
  - `0xcf` (`0x4e -> 0x4c`)
  - `0xdb` and `0xdd` also retune in the same step
- later mono-pan anchor transitions and the tested playback-pair anchor transitions often show no clean one-frame stable delta because the late rows continue their usual churn

So the grounded claim is narrow:

- durable pan state reaches the same late `0x73` mixer cluster already used by other mixer workflows
- current captures are still **not** strong enough to assign one exact passive `0x73` byte as “the pan field” for each strip

### Metering in mixer captures

The metering captures (`capture_mixer_15_meter_single_strip_playback(ch1).pcapng`,
`capture_mixer_16_meter_same_signal_different_strip(ch11).pcapng`,
`capture_mixer_17_preamp_panel_and_strip(ch2).pcapng`,
`capture_mixer_17_preamp_panel_only.pcapng`) narrow the meter boundary substantially.

What is confirmed:

- metering is device-originated traffic; the test captures contain no meter-driving host family
- `0x83` remains fully stable in all four tested metering captures
- the moving device-side state is carried by `0x73` plus the existing 6-byte async packets on endpoint `0x81`
- the visible, repeated meter-correlated movement in `0x73` is again concentrated in late payload rows rather than front global bytes

Stable observations by capture family:

- playback on strip `1` (`capture_15`) is dominated by movement at `0xce` / `0xcf`, especially `0xcf`
- moving the same playback signal to strip `11` (`capture_16`) shifts the stable baseline between row slots:
  - `0x8e: 0x00 -> 0x60`
  - `0x98: 0x60 -> 0x00`
  - `0xce: 0x46 -> 0x4b`
  - `0xcf: 0x3d -> 0x3e`
  - shadow bytes `0xdb`, `0xdd`, `0xdf`, and row-local `0xe2` move with that reassignment
- this is strong evidence that ordinary mixer metering follows the **strip slot / row position**, not the abstract source class alone

Preamp-related captures add a second distinction:

- `capture_17_preamp_panel_and_strip(ch2)` changes both the ordinary late mixer row (`0xcf` and neighbors) **and** a broader band around `0xae..0xb1`
- `capture_17_preamp_panel_only.pcapng` changes the broader preamp-related band without turning on the strip-local repeated bytes `0xda..0xdf`, `0xe2`, `0xe3`
- the cleanest current interpretation is that preamp-panel metering and mixer-strip metering are distinct views that can coexist when the same preamp source is also assigned to a mixer strip

What remains unresolved:

- exact per-byte meter scaling
- whether the 6-byte `0x81` packets contain a meter-side clock, sequence, or compact meter stream
- exact separation of visible master-meter movement from strip meters

Current implementation consequence:

- the capture set is sufficient to document metering boundaries
- it is **not** yet sufficient for a trustworthy parser that claims exact strip or master meter fields

### Candidate mixer-state region inside `0x73`

The captures support a narrower candidate region for mixer-derived state than the earlier broad byte-range note.

Most useful offsets observed across `capture_10` and `capture_10_2`:

| Payload offset | Observation |
|---|---|
| `0x6a` | Surface/context byte: `0x0f` on the Monitor/HP1 surface, `0x0c` on the HP2 surface |
| `0x6e..0x71`, `0x8e..0x91`, `0xce..0xd3` | Repeated row-local state bytes that move across linked/unlinked, mute, and fade workflows |
| `0xcf` | Most sensitive row-local byte for mixer fades/mute/selection |
| `0xde`, `0xdf`, `0xe2`, `0xe3` | Late mixer/output shadow bytes that react to mute/state transitions |

What is confirmed:

- linked versus unlinked workflows change stable `0x73` state
- mute/unmute changes stable `0x73` state
- the stable changes cluster in the late payload rather than the front global field area
- mixer fades and mute/unmute repeatedly change payload byte `0xcf`
- stronger mixer state changes also retune coarse row-head bytes `0x8e` and `0xce` together with shadow bytes `0xda..0xdd`, `0xde..0xdf`, and `0xe2`
- row `0x6e` participates mainly when the current surface/context is the Monitor/HP1 side; it is much less active once the workflow is on the `0x6a = 0x0c` HP2 surface
- linked/unlinked transitions also change the same late cluster, confirming that mixer topology is published by the device
- the startup / pre-command `0x73` stream is not a single stable strip table: before the first host write, `capture_08`, `capture_10`, and `capture_10_2` already walk through many distinct late-row states while the front output bytes and `0x6a` surface byte remain unchanged
- `capture_09_idle_polling.pcapng` shows the same kind of late-row churn during pure idle, so the changing tuple around `0x6e`, `0x8e`, `0xce`, and `0xda..0xe5` is not by itself a trustworthy one-shot encoding of saved mixer strip levels/mutes
- current evidence therefore supports these late rows as dynamic mixer-page state that includes focus/selection/scan-like behavior in addition to durable mixer settings; a passive decoder cannot safely treat one startup `0x73` frame as a full per-strip strip snapshot

What is not yet safe to claim:

- a one-offset-per-fader-level map for each mixer channel
- a complete per-channel table layout
- a safe passive startup decode of mixer strip level/mute from the currently available `0x73` captures alone

### DSP / preamp-specific families observed in `capture_07_dsp.pcapng`

Confirmed families and current best mapping:

| Payload | Observed behavior |
|---|---|
| `4f 00 00`, `4f 00 01`, `4f 00 02` | 3-state selector reflected in payload `0x18` (`0x2f`, `0xfa`, `0x00`) and the low nibble of `0x1a` (`0x10`, `0x11`, `0x12`); selector `0x00` and `0x02` share the same `0x19=0x34` / `0x1b=0x00`, while selector `0x01` keeps the same high-level mode but uses distinct `0x18=0xfa`; late rows at `0x8e`, `0x0ce`, and `0x0da` track the change |
| `51 00 00`, `51 00 01` | 1-bit toggle reflected directly in payload byte `0x1a` (`0x00` <-> `0x10`); effectively toggles bit `0x10` |
| `52 00 00`, `52 00 01` | second 1-bit toggle reflected directly in payload byte `0x1a` (`0x10` <-> `0x50`); effectively toggles bit `0x40` on top of the `4f`/`51` state |
| `50 01 2f` | Companion write for the extended `51 01 01` workflow; no isolated stable `0x73` delta seen by itself |
| `51 01 01` | Extended DSP/preamp mode entry; keeps `0x18=0x2f` but changes `0x19:34->2f`, `0x1b:00->10`, rewrites rows `0x8e`, `0xae`, `0xce`, and produces the clearest stable `0x83` front-block change; this is the strongest evidence that the `0x0ae` band belongs to a richer DSP/preamp page rather than to ordinary output or mixer state |
| `d5 08 05 00 02 00 05 00 02 00` | Real host write, but no isolated stable `0x73` delta proven from current captures |
| `d5 0a 06 00 02 00 00 03 00 00 00 01` | Opaque advanced config write; does not touch `0x18..0x1b`, but changes late bytes `0x0b3`, `0x0b4`, `0x0cf`, `0x0de`, `0x0df` and front `0x83` bytes `0x00`, `0x02` |
| `d7 11 ...` | Opaque advanced config or preset/state write; observed variants mainly perturb front `0x83` bytes, with either no stable `0x73` delta or only a small late-state delta (`0x0cf`, sometimes `0x0de..0x0df`) |
| zero payload / all-zero short writes | Commit/nudge-like writes around the `d5`/`d7` sequence; only small late-state perturbations proven |

Additional confirmed DSP-state behavior:

- `d404010100e0` and `d40401010060` toggle a channel state that changes late payload bytes `0xcf`, `0xde`, and `0xdf`
- `a2000000` reverses the extended state changes introduced by `510101`
- payload byte `0x1a` is not a single enum: current captures support a packed interpretation where `4f` selects the low state (`0x10`/`0x11`/`0x12`), `51` toggles bit `0x10`, and `52` toggles bit `0x40`
- the cleanest packed interpretation of the front DSP cluster is now `{0x18,0x19,0x1a,0x1b}` rather than `0x1a` alone:
  `4f` primarily selects the base source/type state, `51 00 xx` gates bit `0x10` in `0x1a`, `52 00 xx` gates bit `0x40` in `0x1a`, and `51 01 01` switches into an extended mode that additionally rewrites `0x19` and `0x1b`
- `51 01 01` matches the control-panel concept of entering a richer preamp/front-end mode better than the simpler `4f`/`51`/`52` toggles, because it is the only short write here that changes both the front `0x73` mode cluster and the front `0x83` auxiliary block in one step
- stable `0x83` changes in `capture_07` are clustered at payload `0x00..0x0d` and accompany the more advanced DSP/preamp mode changes, especially `510101`, `d50a...`, and `d711...`
- the extended DSP enter/exit pair is also the strongest evidence that row `0x0ae` is real but DSP-specific: `510101` changes `0x0ae`, `0x0af`, `0x0b0`, `0x0b1`, `0x0b3`, `0x0b4`, and `0x0e3` together with front-byte changes `0x19:34->2f` and `0x1b:00->10`, while `a2000000` reverses the same cluster; ordinary output/mixer captures leave that band untouched
- the same extended enter/exit pair narrows the enum semantics further: `510101` drives global late rows `0x8e`, `0x0b3`, `0x0b4`, `0x0ce`, and shadow bytes `0x0da..0x0dd`, `0x0e2` from `0x54` to `0x51`, while the DSP-only row head at `0x0ae` moves from `0x60` to `0x5a`; this is strong evidence that `0x51` means extended DSP mode rather than a generic attenuation step
- after that extended mode is entered, late `a2` selectors such as `a2031201` through `a2031701` mostly leave the `0x0ae` band unchanged and instead retune `0x0cf`, which fits the interpretation that the `0x0ae` band describes the entered DSP page/submode itself while later `a2` traffic moves selection/focus within that page
- late `a2` selectors in `capture_07` do not behave like phantom/phase/source toggles; after the extended mode is entered they mostly only change row-local byte `0x0cf`, which is more consistent with local DSP/preamp item selection or focus state than with a durable hardware setting
- the `d5`, `d7`, and late `a2` commands mostly perturb the same late-state cluster rather than the front global bytes

These commands are real and persistent, but the existing captures are still not sufficient to map every family to a specific control-panel label without over-claiming.

## State Reconstruction Guidance

If this protocol is reimplemented, the safest model is:

1. On startup, issue the `0x74` queries and parse the `0x75` replies.
2. Subscribe to `0x82` and treat `0x73` as the canonical device state.
3. Decode `0x73` global fields immediately:
   sample-rate code, clock-source code, and big-endian sample-rate integer.
4. Send `0x70` commands when the user changes a control.
5. Wait for the next stable `0x73` before updating the local UI model.
6. Treat `0x81` as timing/notification support, not the source of truth.

## Coverage Summary by Feature

| Feature | Coverage status | Evidence |
|---|---|---|
| Startup metadata | Confirmed | `capture_01*`, `0x74/0x75` |
| Sample rate | Confirmed | `capture_05`, `0x70 03 rr`, `0x73` payload bytes `0x02..0x07` |
| Clock source | Confirmed | `capture_06`, `0x70 04 cc`, `0x73` payload byte `0x03` |
| Master output volume | Confirmed | `capture_11`, `0x47 oo vv` |
| Mixer fader level | Confirmed | `capture_02`, `capture_03`, `capture_08`, `capture_10`, `capture_10_2` |
| Mixer mute | Confirmed | `capture_04`, `capture_10`, `capture_10_2`, `pp + 0x40` |
| Linked stereo-pair propagation | Confirmed | `capture_10_2` |
| Unlinked independent channel control | Confirmed | `capture_10` |
| Link/unlink command family (`0x70/0x14`) | Confirmed at family level | `capture_10`, `capture_10_2` plus user-confirmed action order |
| Mixer strip assignment writes (`0x70/0x53`, `d3 41`) | Confirmed for ordinary strips | `capture_mixer_03`, `capture_mixer_04`, `capture_mixer_18` |
| Ordinary-strip assignment enum map | Confirmed for `Preamp 1..2`, `Computer Play 1..2,8`, `Emu Mic 1..2`, `SPDIF In 1..2`, `Mute`, `Oscillator 1..2`; candidate for `Computer Play 3..7` | `capture_mixer_03`, `capture_mixer_04` |
| Output mute (`0x48`) | Confirmed | `capture_11` plus user-confirmed action order |
| Output dim (`0x66`) | Confirmed | `capture_11` plus user-confirmed action order |
| Output volume bytes in `0x73` | Confirmed | `capture_11` correlation of `0x47` to payload offsets `0x0c`, `0x0e`, `0x10` |
| `0x83` as canonical live control state | Not supported by captures | no stable output/mixer-state deltas; only DSP/front-block auxiliary movement in `capture_07` |
| `0x81` as canonical state | Not supported by captures | event-adjacent but not a stable state mirror |
| DSP/preamp advanced features | Partially decoded | `capture_07` |
| Idle polling model | Confirmed | `capture_09` |

## Key Practical Conclusion

The device is authoritative.

The protocol is not built around the host remembering previous state.
The host writes compact commands, then learns the resulting configuration from the device's own `0x73` snapshots and `0x75` query replies.
