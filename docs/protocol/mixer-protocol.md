# Zen Go Mixer Protocol

This document collects the mixer-side protocol findings that are grounded well enough to guide parser and UI work.

It is intentionally narrower than `docs/protocol/pcap-analysis.md`.
This file focuses on the mixer model currently supported by captures and the existing TUI: surfaces, level/mute writes, pan writes, link family, assignment writes, metering boundaries, and the current boundaries around passive `0x73` decoding.

## Scope

Covered here:

- mixer surfaces `MIX 1` and `MIX 2`
- confirmed host writes for mixer level and mixer mute
- confirmed host writes for mixer pan
- confirmed link/unlink command family at the current evidence level
- confirmed strip-assignment write family for ordinary strips
- grounded metering boundaries
- what durable mixer state does and does not show up safely in `0x73`

Not covered here:

- AFX / DSP behavior on strips `1..4`
- full passive per-strip startup reconstruction from `0x73`
- complete strip-index map for all assignment table entries
- DAW I/O routing
- digital-outs page details

## Working Mixer Model

Current grounded model:

- there are `2` mixer surfaces:
  - `MIX 1` for `Monitor / HP1`
  - `MIX 2` for `HP2`
- the `0x73` surface/context byte is payload offset `0x6a`
  - `0x0f` = `Monitor / HP1`
  - `0x0c` = `HP2`
- current mixer capture work assumes `16` strip slots per surface
- adjacent stereo-pair links are exposed on odd-numbered strips only:
  - `1-2`, `3-4`, `5-6`, `7-8`, `9-10`, `11-12`, `13-14`, `15-16`

Important current behavior boundary from captures and user confirmation:

- strip source assignment is shared across surfaces
- link state is stored independently per surface
- strip level is stored independently per surface

## Confirmed Host Writes

The current app implements two confirmed direct mixer write families via `0x70 / length 0x16`.

### Level

Payload body:

```text
d4 04 <mixer> <channel> <level> <pan>
```

Implemented in `src/protocol.rs` as `Command::SetMixerLevel`.

Grounded details:

- `<mixer>`: `0x00` = `MIX 1`, `0x01` = `MIX 2`
- `<channel>` is the strip/channel selector byte used by the host
- `<level>` follows the same inverse scale used by output volume in the current app
- `<pan>` carries pan state as part of the same write

Current TUI interpretation for level display:

- raw `0x00` = `0 dB`
- raw `0x60` = `-96 dB`

### Mute

Payload body:

```text
d4 04 <mixer> <channel> 00 <pan-with-mute>
```

Implemented in `src/protocol.rs` as `Command::SetMixerMute`.

Grounded details:

- mute is carried by setting bit `0x40` in the final pan-state byte
- the same command family is used for ordinary level and mute workflows

### Pan Encoding

The dedicated pan captures confirm that pan is encoded in the final byte of the same
`d4 04 <mixer> <channel> <level> <pan>` host family used for level writes.

Grounded host-side model:

- tested range: `0x02 .. 0x3e`
- center: `0x20`
- the tested mono strip (`channel = 0x01`) and playback-pair members (`channel = 0x03`, `0x04`) use the same raw pan byte encoding
- pan-only sweeps keep `<level> = 0x00` while varying only the final pan byte

Grounded anchor values used by the current app:

| Pan state | Byte |
|---|---|
| Left | `0x02` |
| Center | `0x20` |
| Right | `0x3e` |

Mono-strip vs playback-pair behavior that is now safe to state:

- mono-strip pan is one scalar position over the full `0x02 .. 0x3e` range
- playback-pair members are not using a separate command family; each member is still just one strip selector with the same scalar pan byte
- the captures show default playback-pair members starting at opposite pan extremes (`left member` near `0x02`, `right member` near `0x3e`) and then being moved individually through the same raw range

Important boundary:

- the existing `PanState` enum in `src/protocol.rs` is enough for hard-left / center / hard-right writes only
- it is not a full representation of the grounded capture range, so it should be treated as a partial app-facing convenience model rather than a complete pan decoder

### Stable `0x73` Effects for Pan

What is currently safe:

- pan state reaches the late mixer `0x73` region, not the front global bytes
- the clearest confirmed pan-related stable movement is in row-local late bytes:
  - `0x8f`
  - `0xcf`
  - sometimes shadow bytes `0xdb` and `0xdd`
- `0x83` stays stable in the tested pan captures

Important boundary:

- current captures do **not** yet justify a passive per-strip pan-field decoder from one `0x73` frame
- many anchor transitions, especially on the playback-pair capture, are obscured by the same late-row churn already seen in other mixer workflows

### Solo

The dedicated solo captures confirm that solo does **not** use a separate host family.
It rides on the same `d4 04 <mixer> <channel> <level> <pan/state>` write used for level,
mute, and pan.

Grounded host-side model:

- solo is carried by setting bit `0x80` in the final pan/state byte
- centered unsolo is `0x20`
- centered solo is `0xa0`
- the tested clear operations are ordinary per-strip writes back to the unsolo state; no dedicated global clear-solos family appears in the current captures

Current boundary:

- the write-side encoding is grounded
- passive solo state is **not** grounded enough yet for a trustworthy `0x73` decoder from one snapshot

## Link / Unlink Family

The currently grounded link family is `0x70 / length 0x14` with `a2` payloads.

Observed host forms:

```text
a2 03 <selector> <0|1>
a2 04 <bank> <0|1>
```

Current evidence supports:

- `a203` as the selector-bearing durable link write
- `a204<bank><x>` as a companion/helper write rather than the durable state carrier
- companion bank `0x00` for the tested `MIX 1` pair `1-2`
- companion bank `0x01` for the tested `MIX 2` pair `1-2`
- selector `0x00` for the tested `MIX 1` pair `1-2`
- selector `0x03` for the tested `MIX 1` pair `7-8`
- selector `0x01` for the corresponding `MIX 2` link target observed after switching surface in `capture_mixer_18_surface_independence_link.pcapng`
- no companion `a204` write was needed in the tested `MIX 1` pair `7-8` capture

Current boundary:

- the command family is real and persistent
- `a204` alone shows no stable `0x73` or `0x83` delta in the tested captures, which makes it look like a helper/master latch rather than the durable state carrier
- exact semantics of `a203` vs `a204` are still not fully resolved
- link state is known to be surface-local

### Stable Link State in `0x73`

What is safe to claim:

- link/unlink state is durable and device-originated because stable `0x73` changes remain after the write window
- the tested link workflows still do not produce stable `0x83` changes
- the link-related stable state remains in the same late mixer cluster already used by level/mute/pan workflows
- surface-local behavior is confirmed: assignment is shared, but link state is not shared between `MIX 1` and `MIX 2`

The current app therefore keeps link/unlink documented but intentionally does not expose it as a normal interactive control yet.

## Strip Assignment Writes

The mixer assignment captures introduce a distinct host write family for strip-source assignment.

Payload form:

```text
70 00 00 00 53 00 00 00 00 00 00 00 00 00 00 00 d3 41 bb ...
```

Grounded details:

- `d3 41` is the logical assignment-family marker
- `bb` behaves like a bank/subwrite selector, not the source id itself
- for the ordinary strip-11 sweeps, the changing source tuple sits in zero-based table entry `10`
- that changing entry is carried at payload offsets `0x17..0x18` relative to the `d3 41` payload body

Confirmed ordinary-strip enum values from the assignment captures:

| Entry bytes | Source |
|---|---|
| `00 00` | `Preamp 1` |
| `00 01` | `Preamp 2` |
| `01 00` | `Computer Play 1` |
| `01 01` | `Computer Play 2` |
| `01 07` | `Computer Play 8` |
| `02 00` | `SPDIF In 1` |
| `02 01` | `SPDIF In 2` |
| `08 00` | `Mute` |
| `09 00` | `Oscillator 1` |
| `09 01` | `Oscillator 2` |
| `0a 00` | `Emu Mic 1` |
| `0a 01` | `Emu Mic 2` |

Strong candidate interpolation:

| Entry bytes | Likely source |
|---|---|
| `01 02 .. 01 06` | `Computer Play 3 .. 7` |

Important boundary:

- early-strip entries include values such as `03 00`, `03 01`, `03 02`, `03 03`
- those are likely tied to the special strips `1..4` / AFX-capable area
- they should not be merged into the ordinary-strip enum yet

## What `0x73` Safely Tells Us Today

The device remains authoritative, and `0x73` is still the canonical state snapshot.

What is safely grounded for mixer work:

- the surface/context byte at `0x6a`
- durable assignment effects do reach `0x73`
- durable link/unlink and mute/level workflows also reach `0x73`
- the meaningful mixer activity remains concentrated in the late payload region rather than the front global field area
- for the currently grounded strip `1` / pair `1-2` workflows, the active-surface late-row cluster is decodable enough for passive state reconstruction in app code

Most useful currently-observed mixer-related `0x73` offsets:

| Offset / region | Current interpretation boundary |
|---|---|
| `0x6a` | current surface/context selector |
| `0x6e..0x71`, `0x8e..0x91`, `0xce..0xd3` | repeated late mixer rows that react to mixer workflows |
| `0xcf` | dense row-local status byte; highly sensitive to fades, mutes, selections, and other local mixer state |
| `0xda..0xe5` | late shadow cluster tied to coarse/local mixer state |

Assignment-specific stable `0x73` movement currently clusters around:

- `0x98`
- `0xcf`
- `0xda`, `0xdc`, `0xde`
- sometimes `0xdf`
- oscillator assignment changes can additionally move `0x6e`, `0x8e`, `0xce`, `0xdb`, `0xdd`, `0xe2`

## Grounded Passive Decode Implemented In Code

The current code now passively reconstructs a **narrow grounded subset** of mixer state from a single `0x73` snapshot.

Implemented passive subset:

- an observed late-cluster meter value surfaced on `A2` in the TUI, because the current captures do not justify claiming it as decoded strip state yet
- `MIX 1` / `MIX 2` strip `1` mute/unmute state, from the active-surface late-row cluster
- `MIX 1` / `MIX 2` strip `1` pan, but only at the currently grounded center/near-center anchors
- link state for the active `1-2` pair on the active surface when the late-row cluster matches the dedicated link/unlink captures strongly enough

Grounded evidence pattern used by the passive decoder:

- active surface still comes from payload `0x6a`
- `MIX 1` active strip-1 workflows are best reflected by `0x6f`, `0x8f`, `0xcf`, `0xda..0xdf`
- `MIX 2` active strip-1 workflows hold the active surface row at `0x6e/0x6f = 0x60`, while the same stable state still appears in `0x8f`, `0xcf`, `0xda..0xdf`
- level captures in `capture_mixer_19_surface_independence_level(...)` show a repeatable tier set around `0x43`, `0x47`, `0x49` that is safe to collapse to coarse passive steps for the tested strip
- mute captures in `capture_mixer_11_mute_unlinked(...)` still ground the narrow strip-1 passive decoder (`0x51` muted vs `0x4c/0x4e` unmuted in the main strip cluster)
- the newer dedicated pair-state captures under `antelope_pcap/mutes/` rule out `0xe0/0xe1` as mute bytes: they stay fixed at `0x60` in every tested file
- those same captures instead narrow the surface-local pair-state region further:
  - active `MIX 1` (`0x6a = 0x0f`): pair-local state shows up in `0xda..0xdd`
  - active `MIX 2` (`0x6a = 0x0c`): pair-local state shows up in `0xde..0xdf`
- with signal present, those pair-local bytes separate the four tested `CH1/CH2` mute combinations strongly enough to show they carry pair state, but the values are mixed with meter/activity and are therefore not yet safe as a durable mute-only decoder:
  - `MIX 1`: `both mute = 60`, `ch1 mute/ch2 unmute = 01`, `ch1 unmute/ch2 mute = 00/06`, `both unmute = 0a/05` across `0xda..0xdd`
  - `MIX 2`: `both mute = 60`, `ch1 mute/ch2 unmute = 01`, `ch1 unmute/ch2 mute = 00/06`, `both unmute = 0a/05` across `0xde..0xdf`
- the signal-present XOR view is consistent across surfaces and supports a test-only experimental model:
  - `MIX 1` duplicates the two-lane code across `0xda/0xdc` and `0xdb/0xdd`
  - `MIX 2` carries the same two-lane code once in `0xde/0xdf`
  - repeated lane XORs: `0x0b/0x04` (`both_unmute ^ ch1_mute`), `0x0a/0x03` (`both_unmute ^ ch2_mute`), `0x01/0x07` (`ch1_mute ^ ch2_mute`)
- link captures in `capture_mixer_05_link_pair_1_2_only.pcapng` and `capture_mixer_18_surface_independence_link.pcapng` keep the linked state in the `0x5a/0x4c/0x51` family and unlink in the `0x4e/0x51` family

Important boundary of the implementation:

- this is **not** a full per-strip passive mixer decoder
- it currently updates only the grounded strip/pair subset above
- the newer `antelope_pcap/mutes/` captures show promising surface-local pair-state bytes, but they are still not clean mute-only fields and should not yet be promoted into a broader passive mute decoder
- the moving late-cluster meter bytes are still not grounded enough to call a strip meter or master meter parser; the current TUI only exposes the observed value on `A2` as a temporary operator aid
- passive pan remains intentionally narrow; the captures do not yet justify a continuous one-frame pan-value decoder over the full `0x02 .. 0x3e` command range
- links are still only grounded for the tested `1-2` pair target on each surface; broader selector coverage remains deferred

## Metering Boundary

The current meter captures are sufficient to document boundaries, but not enough for parser implementation.

What is grounded:

- meter traffic is device-originated; the tested captures contain no dedicated meter host-write family
- `0x83` remains stable in the tested meter captures, including the two new `capture_mixer_20_*` surface-isolated files
- meter-correlated movement is visible in `0x73` late rows and in the 6-byte async packets on endpoint `0x81`
- ordinary playback metering follows strip slot / row placement rather than source identity alone
- preamp-panel metering is distinct from mixer-strip metering and can coexist with it when a preamp source is also assigned to a strip
- `0x81` behaves like an activity side channel rather than a second canonical snapshot: packet rate and byte diversity rise with some passive meter setups, but the byte patterns do not stay stable enough to map directly to strip/master meter values
- the new `capture_mixer_20_mix1_master_and_chan2.pcapng` and `capture_mixer_20_mix2_master_and_chan2.pcapng` files narrow the passive meter window further, but only to late-row clusters:
  - `MIX 1` file: `0x6e`, `0x8e`, `0xce`, `0xcf`, `0xe2`
  - `MIX 2` file: `0x8e`, `0xce`, `0xcf`, `0xda..0xdd`, `0xe2`
- the newer `antelope_pcap/mutes/with signal/` files reinforce that `0xda..0xdd` (`MIX 1`) and `0xde..0xdf` (`MIX 2`) are pair-local mixed state/activity bytes rather than static mute-only fields, while `0xe0/0xe1` remain pinned at `0x60`

What is not grounded enough yet:

- exact meter-value scaling
- exact master-meter field separation
- a trustworthy parser for strip meters or master meters, whether from `0x73` alone or from `0x73 + 0x81 + 0x83`

Important current boundary:

- because strip assignment is shared across `MIX 1` and `MIX 2`, the same source can legitimately drive both visible master meters
- the new `capture_mixer_20_*` files confirm that source selection alone was the wrong isolation strategy, but they still do not justify claiming that any one of those moving late-row bytes is the visible master meter rather than a mixed master/strip/surface meter state cluster

## What Is Not Yet Safe To Claim

Still not safely grounded from the current capture set:

- a passive exact-source decoder from one startup `0x73` frame
- a general one-offset-per-strip level decoder from `0x73` alone
- a complete strip table layout for all mixer channels
- an exact passive per-strip pan decoder from one `0x73` frame, even though the host-side pan byte range is now grounded
- a trustworthy passive solo-state decoder from one `0x73` frame
- exact metering packet/offset mapping

The late `0x73` rows churn before the first host write and also during pure idle, so one passive startup frame should not yet be treated as a full saved-strip snapshot.

## Implementation Boundary in the Current TUI

The current app intentionally uses a split model:

- authoritative device state for the parts of `0x73` that are safely decoded
- command-round-trip overlays for mixer strip values that are not yet passively reconstructable from startup snapshots alone

That is why the mixer UI can show confirmed level/mute values after writes without pretending that the same values were passively decoded from an arbitrary idle snapshot.

## Recommended Next Decode Work

Best next protocol-doc targets once the corresponding captures are analyzed:

1. full mixer pan-value mapping
2. passive solo-state mapping in `0x73`, if solo needs to be shown from snapshots rather than command overlays
3. surface-isolated master-meter capture if meter parsing becomes an implementation goal
4. dedicated ordinary-strip vs early-strip assignment index map

## Code References

- `src/protocol.rs`: `MixerSurface`, `PanState`, `Command::SetMixerLevel`, `Command::SetMixerMute`, `Command::SetLinkState`
- `src/app.rs`: mixer pending-write overlay and per-surface state handling
- `src/ui.rs`: mixer-surface tabs and strip rendering

## Related Analysis

- canonical capture analysis: `docs/protocol/pcap-analysis.md`
- preamp/front-end protocol: `docs/protocol/preamp-protocol.md`
- capture planning and filenames: `docs/protocol/mixer-capture-plan.md`
- remaining unresolved protocol gaps: `docs/protocol/open-questions.md`
- control-panel feature map: `cpl.md`
