# Zen Go SC protocol implementation audit

Date: 2026-08-31  
Scope: compare current capture-derived implementation with `docs/zengosc_report_format_1.1.26`.

All protocol IDs, offsets, lengths, counts, and widths in this report use hexadecimal notation so they can be matched directly against code. `0x73` and `0x83` offsets are payload-relative unless stated otherwise.

## Verdict

Current implementation is a grounded capture dialect, not a direct implementation of the official report schema. Do not overlay official `0x73` offsets or rename current query decoders from the JSON alone.

Highest-risk findings:

1. Official cyclic `0x73` payload is `0x17a` bytes. Current snapshot payload is `0xe6` bytes inside a fixed `0x140`-byte HID frame. The two layouts cannot be the same byte layout.
2. Official `0x74` query meanings collide with live current decoders at `0x04`, `0x0b/0x03`, `0x17/0x00`, and `0x18/0x00`. This is a confirmed semantic collision under a shared-wire assumption, but not proof that current captures and official schema use the same protocol variant.
3. Official output state has independent `mute`, `dim_on`, `mono`, and `trim` fields. Current state compresses output mode to one enum while sending mute and dim as separate commands. Simultaneous output states cannot be represented correctly.
4. `docs/cpl.md` contains stale or ungrounded claims: `0x0f` channels, `EMU MIC 0x01-0x04`, dry Record routing, and Hi-Z `0x00-0x3c dB`. Current grounded implementation documents `0x10` strips, EMU MIC `0x01-0x02`, no proven DAW routing, and Hi-Z `0x00-0x2d dB`.

No implementation patch is included. Format/version boundary must be established before changing decoders or controls.

## Evidence boundary

### Official schema

`docs/zengosc_report_format_1.1.26` defines logical report records:

- cyclic reports `0x73` and `0x83`;
- set report `0x70`;
- get report `0x74`;
- nested record counts and widths.

### Current implementation

Relevant current paths:

- Frame parsing: `antelope-protocol/src/frame.rs:64-118`.
- Query encoding: `antelope-protocol/src/encoder.rs:142-149`.
- Query decoding and startup query list: `antelope-protocol/src/query.rs`.
- Query-reply application: `src/app/mod.rs:345-458`.
- Passive mixer application: `src/app/mod.rs:270-296`.
- Output control: `src/app/controller.rs:979-1017`.
- Optimistic mutation application: `src/app/controller.rs:747-937`.
- Raw map: `src/ui/raw_map.rs:205-557`.
- Existing grounded protocol notes: `docs/protocol/mixer-protocol.md`, `docs/protocol/preamp-protocol.md`.

Current implementation intentionally uses capture-derived wire families such as `0x13/0x47..0x52`, `0x13/0x66`, `0x14/0xa2`, and `0x16/0xd4`. Official logical command IDs must not be substituted for these without a translation layer.

## Findings

| ID | Class | Confidence | Finding | Effect | Safe disposition |
|---|---|---:|---|---|---|
| F-01 | Framing / schema boundary | High | Official cyclic `0x73` length is `0x17a`; current snapshot payload length is `0xe6`. Official `0x73` needs `0x18a` bytes including current `0x10`-byte header, larger than current `0x140` HID frame. | Direct official-to-current offset mapping is impossible. | Add a protocol-variant/framing gate first. Keep current capture map unchanged until a frame from official path is identified. |
| F-02 | Query semantic collision | High conditional | Official `0x74/0x18/0x00` is `get_reverb_sends`; current decoder treats it as two-surface mixer readback. | If wire fields are shared, reverb data is mislabeled as mixer state. Current app can apply wrong state. | Require response-shape and variant checks before `0x18/0x00` mixer decoding. |
| F-03 | Query semantic collision | High conditional | Official `0x74/0x17/0x00` is `get_reverb_returns`; current decoder treats first `0x04` bytes as generic quad state. | If wire fields are shared, reverb return data is mislabeled. | Guard by variant and expected response shape. Do not broaden current quad decoder. |
| F-04 | Query semantic collision | High conditional | Official `0x74/0x04/0x00` is `get_mixer` with `0x21` records of width `0x09`; current `0x04/0x00..0x01` parser reads one surface of `0x10` records with width `0x02`. | Same query ID cannot safely mean both formats. | Keep capture parser under an explicit variant. Add official parser separately if official frames are observed. |
| F-05 | Query semantic collision | High conditional | Official `0x74/0x0b/0x03` is `get_mixer_links` with `0x40` one-byte records; current parser consumes `0x18` bytes as selector bitmap and expands them to `0x08` stereo links per surface. | Link state can be decoded with wrong cardinality and semantics. | Do not relabel current bitmap as official mixer links. Add shape/version discrimination. |
| F-06 | Query family mismatch | High conditional | Official `0x74/0x03/0x00` routing response is one record of width `0x81`; current `0x03` family expects a leading sub-ID and assignment tables for sub `0x05`/`0x06`. | Routing readback has incompatible body grammar. | Keep current assignment decoder capture-scoped; implement official routing separately after wire confirmation. |
| F-07 | Output state model | High | Official `volumes` records have independent `volume`, `mute`, `dim_on`, `mono`, and `trim`. Current UI/raw map represent each of three outputs as one level plus one `OutputMode`; controller sends mute and dim independently. | `mute + dim` cannot coexist in state. `mono` and `trim` are absent. UI can misreport device state after either toggle. | Replace enum with independent fields only after output record mapping is proven. Preserve unknown bits/fields during transition. |
| F-08 | Mixer readback application | High conditional | Capture decoder `QueryResponse::mixer_strip_readback` parses level, pan/state byte, mute, and solo candidates from `0x18/0x00`; `src/app/mod.rs` applies only `slot.soloed` for this reply. | In the current capture dialect, startup/query mixer level, pan, and mute readback remains stale or optimistic. | Apply fields only after current `0x18` response shape is proven distinct from official reverb-send shape. Add fixture tests for both variants. |
| F-09 | Passive mixer state | Medium | `decode_passive_mixer_state` computes a pan candidate, but `apply_passive_mixer_decode` drops it while applying meters, mute, and link. | UI state cannot reflect passive pan even when decoder reports one. | Keep pan marked observed/unresolved unless repeated captures prove byte semantics; do not expose as authoritative control yet. |
| F-10 | Preamp meter label | Medium | Current `0xcf` candidate is labeled as A2 meter from capture correlation and accepts nonzero values through `0x49`. Official cyclic schema has aggregate `peaks_preamp` at `0x15a..0x15b`, not a channel-specific `0xcf` field. | A late byte may be mislabeled as A2 meter, especially when late bytes contain mixed state. | Downgrade to raw candidate until multi-capture invariants distinguish it from mixed/AFX data. |
| F-11 | Snapshot field coverage | High | Official `0x73` names power, lock, presence, pull mode, frequency, trim, brightness, availability, six output records, preamp HPF/zero-cross, digital gains, bank source, and multiple peak groups. Current `parse_snapshot73` maps a smaller capture-derived set and leaves several ranges unresolved. | Official fields are missing from current state/raw-map labels. | Treat as intentionally incomplete until framing is reconciled. Add only fields backed by matching captures. |
| F-12 | Command coverage | High | Official set/get schema includes routing, mixer, links, feature mask, reverb returns/sends, DAW mode, output dim, output trim/mono, preamp HPF/zero-cross, and digital controls. Current command surface covers only grounded preamp, output, mixer, link, assignment, sample, and clock families. | Control-panel functionality remains incomplete relative to official schema. | Track as future scope, not as unsupported UI controls. Do not send official `0x70` payloads through current capture encoders. |
| F-13 | Wire-layer naming | High | Current encoders use capture opcodes: preamp `0x13/0x4f..0x52`, output `0x13/0x47..0x48` and `0x66`, mixer `0x16/0xd4`, link `0x14/0xa2`. Official logical set IDs are in report `0x70`. | A name-level comparison can falsely imply current command is official command. | Document current commands as capture dialect; introduce an explicit adapter if official wire is implemented. |
| F-14 | Documentation drift | High | `docs/cpl.md` claims `0x0f` mixer channels, `EMU MIC 0x01-0x04`, dry pre-mixer Record routing, and Hi-Z `0x00-0x3c dB`; current grounded docs/code support `0x10` strip slots, EMU MIC `0x01-0x02`, no proven DAW routing, and Hi-Z `0x00-0x2d dB`. | Users may rely on controls/ranges/routing behavior that implementation does not support. | Mark `docs/cpl.md` as non-authoritative or correct it against capture-grounded docs after user approval. |
| F-15 | Official `0x83` coverage | Medium | Official cyclic `0x83` is one `afx_meters.data` block spanning `0x00..0x12f`, total `0x130` bytes. This matches current query-body capacity after the `0x10` header, but current raw map leaves the block unresolved. | One official block-level label is missing; subfield semantics remain unknown. | Safe documentation improvement: label the whole `0x83` body as an AFX-meter candidate, without inventing sub-offsets. |

“High conditional” means evidence proves the two definitions conflict if they share a wire grammar; current evidence does not yet prove that official JSON and current capture dialect are the same version/transport.

## Hex-normalized official cyclic layout

### `0x73` payload

| Payload range | Official field | Shape |
|---|---|---|
| `0x00..0x02` | `current_preset` | `0x03` bytes |
| `0x03` | `power_on` | `ubyte` |
| `0x04` | `reserved` | `ubyte` |
| `0x05..0x06` | `usb_mode` | `0x02` bytes |
| `0x07` | `device_updated` | `ubyte` |
| `0x08` | `oven` | `ubyte` |
| `0x09` | `locked` | `ubyte` |
| `0x0a` | `locked_atomic` | `ubyte` |
| `0x0b` | `comp_present` | `ubyte` |
| `0x0c` | `madi_present` | `ubyte` |
| `0x0d` | `adat_present` | `ubyte` |
| `0x0e` | `spdif_present` | `ubyte` |
| `0x0f` | `reserved0` | `ubyte` |
| `0x10..0x13` | `base_index` | `0x04` bytes |
| `0x14..0x15` | `us_pull_mode` | `0x02` bytes |
| `0x16..0x17` | `eu_pull_mode` | `0x02` bytes |
| `0x18` | `sync_source` | `ubyte` |
| `0x19..0x1a` | `freq_left` | `0x02` bytes |
| `0x1b..0x1c` | `freq_right` | `0x02` bytes |
| `0x1d..0x1e` | `level` | `0x02` bytes |
| `0x1f` | `mute_left` | `ubyte` |
| `0x20` | `mute_right` | `ubyte` |
| `0x21` | `sync_freq_hi` | `ubyte` |
| `0x22` | `sync_freq_mid` | `ubyte` |
| `0x23` | `sync_freq_low` | `ubyte` |
| `0x24..0x26` | `adc_trim` | `0x03` bytes |
| `0x27` | `usb_available_ch_ix` | `ubyte` |
| `0x28..0x2b` | `reserved01` | `0x04` bytes |
| `0x2c` | `reserved02` | `ubyte` |
| `0x2d` | `lock_sync_src` | `ubyte` |
| `0x2e..0x30` | `monitor_trim` | `0x03` bytes |
| `0x31..0x33` | `line_out_trim` | `0x03` bytes |
| `0x34` | `brightness` | `ubyte` |
| `0x35` | `adat_avail_channels` | `ubyte` |
| `0x36..0x95` | `volumes` | `0x06` records × `0x10` bytes |
| `0x96..0x97` | `preamp_gains` | `0x02` bytes |
| `0x98..0xa7` | `preamps` | `0x02` records × `0x08` bytes |
| `0xa8..0xaf` | `adat_gains` | `0x08` bytes |
| `0xb0..0xb1` | `spdif_gains` | `0x02` bytes |
| `0xb2..0xf5` | `reserved` | `0x44` bytes |
| `0xf6..0xf9` | `pm_bank_src` | `0x04` bytes |
| `0xfa..0x119` | `peaks_meters` | `0x20` bytes |
| `0x11a..0x139` | `peaks_mixer` | `0x20` bytes |
| `0x13a..0x159` | `reserved` | `0x20` bytes |
| `0x15a..0x15b` | `peaks_preamp` | `0x02` bytes |
| `0x15c..0x15d` | `peaks_spdif` | `0x02` bytes |
| `0x15e..0x165` | `peaks_usb_play` | `0x08` bytes |
| `0x166..0x167` | `peaks_monitor` | `0x02` bytes |
| `0x168..0x169` | `peaks_hp1` | `0x02` bytes |
| `0x16a..0x16b` | `peaks_hp2` | `0x02` bytes |
| `0x16c..0x16d` | `peaks_spdif_out` | `0x02` bytes |
| `0x16e..0x175` | `peaks_usb_rec` | `0x08` bytes |
| `0x176..0x179` | `peaks_reverb` | in `0x176..0x177`, out `0x178..0x179` |
| — | total | `0x17a` bytes |

Each `volumes` record is `0x10` bytes:

- `volume`: `0x08` bytes;
- `mute`: `0x01` byte;
- `dim_on`: `0x01` byte;
- `mono`: `0x01` byte;
- `trim`: `0x05` bytes.

Record `0x00` therefore spans `0x36..0x45`: volume `0x36..0x3d`, mute `0x3e`, dim `0x3f`, mono `0x40`, trim `0x41..0x45`.

Each `preamps` record is `0x08` bytes. Record `0x00` spans `0x98..0x9f`: type `0x98..0x9b`, phantom `0x9c`, HPF `0x9d`, phase invert `0x9e`, zero-cross `0x9f`.

### `0x83` payload

| Payload range | Official field | Shape |
|---|---|---|
| `0x00..0x12f` | `afx_meters.data` | `0x130` bytes |
| — | total | `0x130` bytes |

## Hex-normalized official request crosswalk

### Set report `0x70`

All listed set commands use `ext2=0x00`, `ext3=0x00`.

| Logical command | Payload ID |
|---|---:|
| power | `0x01` |
| sample rate | `0x03` |
| sync source | `0x04` |
| volume | `0x07` |
| mute | `0x08` |
| brightness | `0x0e` |
| preamp type | `0x0f` |
| preamp gain | `0x10` |
| preamp phantom | `0x11` |
| preamp phase invert | `0x12` |
| routing | `0x13` |
| mixer | `0x14` |
| stereo link | `0x22` |
| dim | `0x26` |
| reverb return | `0x28` |
| reverb send | `0x29` |
| DAW mode | `0x2a` |

Official `set_mixer` carries level `0x01`, pan `0x06`, mute `0x01`, and solo `0x01` in its logical payload. This is not equivalent to current `0x16/0xd4` state-byte writes without an adapter.

### Get report `0x74`

| Logical query | `ext2` | `ext3` |
|---|---:|---:|
| routing | `0x03` | `0x00` |
| mixer | `0x04` | `0x00` |
| mixer links | `0x0b` | `0x03` |
| feature mask | `0x11` | `0x01` |
| reverb returns | `0x17` | `0x00` |
| reverb sends | `0x18` | `0x00` |
| assignment status | `0x11` | `0x00` |
| assignment request | `0x11` | `0x02` |

Official response shapes:

| Query | Response shape | Total body |
|---|---|---:|
| routing | count `0x01` × width `0x81` (`bank_idx 0x01`, `bank_configs 0x80`) | `0x81` |
| mixer | count `0x21` × width `0x09` (`level 0x01`, `pan 0x06`, `mute 0x01`, `solo 0x01`) | `0x129` |
| mixer links | count `0x40` × width `0x01` (`linked`) | `0x40` |
| feature mask | raw mask | `0x122` |
| reverb returns | count `0x04` × width `0x08` (`level 0x07`, `mute 0x01`) | `0x20` |
| reverb sends | count `0x21` × width `0x09` (`level 0x01`, `pan 0x06`, `mute 0x01`, `solo 0x01`) | `0x129` |

## Current-vs-official query collision detail

Current `encode_query` emits report `0x74`, writes frame length `0x10` at bytes `0x04..0x07`, query ID at byte `0x08`, and sub-ID at byte `0x0c`. `parse_owned` then exposes every query reply as `0x130` body bytes from raw offset `0x10`, regardless of original declared response length.

Current live decoders:

| Current query | Current interpretation | Minimum body | Official interpretation |
|---|---|---:|---|
| `0x18/0x00` | two `0x10`-strip mixer readback surfaces; `0x02` bytes per strip | `0x40` | reverb sends: `0x21` × `0x09` = `0x129` |
| `0x17/0x00` | generic four-byte startup state | `0x04` | reverb returns: `0x04` × `0x08` = `0x20` |
| `0x04/0x00..0x01` | one `0x10`-strip surface; `0x02` bytes per strip | `0x22` | mixer: `0x21` × `0x09` = `0x129` |
| `0x0b/0x03` | `0x18`-byte selector bitmap | `0x18` | mixer links: `0x40` one-byte records |
| `0x03/*` | sub-ID-prefixed assignment tables | variant-specific | routing: one width-`0x81` record for `0x03/0x00` |

Because current checks use minimum lengths rather than exact variant signatures, a longer official body could be silently accepted by a capture decoder. That is unsafe until report variant is known.

## Current snapshot offset comparison

Current capture-derived `parse_snapshot73` uses these payload-relative locations:

- status `0x00..0x01`;
- sample rate `0x02`;
- clock `0x03`;
- sample frequency `0x04..0x07`;
- output bytes `0x0c..0x11`;
- preamp cluster `0x18..0x1b`;
- surface `0x6a`;
- shared mixer meter lanes `0x8e..0x9d`;
- observed preamp meter candidates `0xce..0xcf`;
- late shadow/correlation range `0xda..0xe5`.

Official `0x73` places output records at `0x36..0x95`, preamp records at `0x98..0xa7`, and peak groups at `0xfa..0x179`. These are not alternate labels for current offsets; they belong to a larger layout.

## What is missing, what is mislabeled, and what is unsafe

### Missing but safe to track as future work

- Official power, lock, presence, pull-mode, trim, brightness, digital-gain, DAW, reverb, AFX, and peak-group state.
- Official six output record fields, including mono and trim.
- Official preamp HPF and zero-cross fields.
- Official routing bank records and feature mask.
- Official mixer/reverb record arrays and independent six-byte pan values.

These should remain unimplemented until official frame length and variant identity are demonstrated.

### Mislabeled or at risk of mislabeling

- Current `0x18/0x00`, `0x17/0x00`, `0x04/0x00`, `0x0b/0x03`, and `0x03/0x00` names if they are presented as official `0x74` queries.
- Current output “mode” if presented as official output state. Official has independent flags.
- Current `0xcf` A2 meter label. Capture correlation is not enough to connect it to official `peaks_preamp`.
- `docs/cpl.md` claims listed above.

### Miscontrolled or state-incomplete

- The bounded Zen Go runtime intentionally keeps one `OutputMode` for the capture-proven isolated states. Unknown mode bytes remain unavailable; direct Mute->Dim and Dim->Mute writes are rejected, and a pending mode change cannot be followed by another mode request until a snapshot confirms the intermediate Normal state. Composition remains unverified, so this safeguard is not a rule for other profile-driven devices.
- Current capture-dialect mixer readback applies only solo from `0x18/0x00`; level, pan, and mute remain stale or optimistic.
- Passive pan is decoded but discarded. This is safer than applying an unproven mapping, but UI must not imply authoritative pan readback.

## Recommended implementation order

1. Capture one complete frame for each official report family, including actual transport length and bytes `0x04..0x07`; determine whether official `0x73` is a different HID report size, a reassembled report, or a different protocol version.
2. Introduce an explicit protocol variant/descriptor boundary. Do not make current decoders accept official shapes through minimum-length checks.
3. Add fixture tests for exact body lengths and query identity before adding field application.
4. Implement official `0x74` parsers separately from capture-derived `query.rs` parsers.
5. Split output state into independent volume, mute, dim, mono, and trim fields only after output record mapping is confirmed.
6. Label current raw-map regions as capture-derived/observed/unresolved; reserve official names for verified official frames.
7. Correct or quarantine `docs/cpl.md` claims against `docs/protocol/mixer-protocol.md` and `docs/protocol/preamp-protocol.md`.

## Audit limitations

- Official JSON defines a schema, not proof that current device firmware emits that schema over current `0x140` HID reports.
- Capture-derived offsets remain valid as observations for current captures; this report does not discard them.
- No device-side capture, transport trace, or protocol-variant discriminator was added during this audit.
