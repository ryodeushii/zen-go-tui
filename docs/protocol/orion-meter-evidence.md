# Orion meter evidence (provisional)

This is a compact evidence summary for the Orion Studio III profile. The
implemented change corrects canonical meter metadata and its generated runtime
mapping; it does not add hardware writes or selected-page behavior.

## Scope and limits

The earlier six-capture review covered these captures:

- `vumeter-test-ch1.pcapng`
- `audioplaying-audiostop-meter.pcapng`
- `vumeters-sinewave.pcapng`
- `preamp1-2-allouts mute.pcapng`
- `matrixtest-pre1-cmpplay1-2.pcapng`
- `mix1-masterfaderplay.pcapng`

Offsets in this note are **full 320-byte report offsets**. Profile
`payload_offset` values are payload-relative and add `0x10`. Free-running
`0x75` meter reports are selected by byte 1 == `0x1f`; byte 1 == `0x00`
responses are readback and excluded.

These captures establish signal activity and bounded correlation only. They do
not establish route-independent physical-input ownership, stereo/L/R mapping,
or a hardware-confirmed fixed lane owner. No new capture or hardware test is
implied.

## Approved provisional output assignment

The runtime stores this explicit user-approved packet-order hypothesis:

| Full-report `0x73` offset | Profile output id | Output | Lane |
|---:|---:|---|---:|
| 157 | 0 | Monitor A | 0 |
| 158 | 1 | Headphone 1 | 0 |
| 159 | 2 | Headphone 2 | 0 |
| 160 | 3 | Line Out | 0 |
| 177 | 4 | Reamp | 0 |
| 178 | 5 | Monitor B | 0 |

These are six one-lane candidates in packet order, not independently verified
physical-output meters. The feed, meter stage, and physical post-fader
ownership remain unknown. No L/R geometry is inferred. In particular,
@177/@178 were observed as a playback-coupled pair; assigning them separately
to Reamp and Monitor B is an explicit ordering assumption, not proof that they
represent distinct physical outputs.

The inverted scale is bounded to raw `0x00..0x60`: `0x60` (96) is retained as
silence/rest and values fall toward `0x00` as signal rises.

## Repeated regions

The later exact mirror table is the governing bounded result:

- `158 ↔ 222`
- `159 ↔ 223`
- `160 ↔ 224`

Those pairs were exact throughout the six captures. First-lane copies
`157 ↔ 169` and `157 ↔ 221` are **not universal** and must not be promoted to
additional lanes or a universal 12-byte mirror block.

## Playback-coupled observation

In the playback capture, `0x73` @177/@178 and meter-only `0x75` @34/@35
co-varied nearest in time within 5 ms (`r≈0.998`). The explicit output
assignment above does not strengthen that observation into ownership evidence.
The `0x75` pair remains excluded so the same activity is not counted twice.

The 2026-09-07 captures supersede the broad-aggregate and flag descriptions.
They show route-correlated pairs at @32/@48 and @33/@49, with unresolved ownership.

## Bounded 2026-09-07 evidence

### Source identity and offset convention

This review used the eight captures under the following absolute directory:

`/home/ryodeushii/repos/zen-go-tui/antelope_pcap/orion 3/captures 2026-09-07/`

The canonical profile was `modules/Antelope-Ctl/profiles/orion_studio_sc.json`
at nested commit `cd7adb61e73585dcbc6e2859204f960c3a32fbdb`. Its file SHA-256 was
`59953cc084f266ffb6b6aaed68a98b7b536d2ce3d1462f4e1b4c09836a8acde1`.

Offsets in this section are zero-based offsets in the full 320-byte HID report.
Subtract 16 to get a profile payload-relative offset where that representation applies.

| Capture | SHA-256 | Last report time | Evidence used here |
|---|---|---:|---|
| `antelope-orion-adatout-ch1-16-oscillator1khz-4secpauses.pcapng` | `de0506c91dfd770ec0b6a50f381e007cb62c5f5564f36a69accb0744eeae76e2` | 143.539080 s | selected bank, ADAT Out 1-16 |
| `antelope-orion-baseline-allmute.pcapng` | `333ce0a8bb2cd146b82874c64033b3643f2a2307e0dd1d1eb18e3f7009552112` | 14.965394 s | resting values and family inventory |
| `antelope-orion-mix2-ch1-32-oscillator1khz-4secpauses.pcapng` | `0e7ba242fce5a09ab750a91bed1b175596d5b7dcd6c179ed8bb0edfd2a957ad2` | 366.444940 s | selector transition and Mix 2 channels 20-32 |
| `antelope-orion-mix3-ch1-32-oscillator1khz-4secpauses.pcapng` | `0f574f16446ffb826fe1e623f692d8da2348e4effacdfdf7020818b2c45d7f55` | 275.322483 s | unselected-bank negative control |
| `antelope-orion-mix4-ch1-32-oscillator1khz-4secpauses.pcapng` | `6926ef727b9facc59e30b4d393555f3d98f37ec28851f2c987f616487534a6b2` | 223.258452 s | unselected-bank negative control |
| `antelope-orion-preamps-sine-ch1-12-4secpauses.pcapng` | `e251cb51684730ff0ab12cf18f7fe958bf206cb4e89c184459b01b68ed0b480d` | 129.518578 s | physical preamps 1-12 |
| `antelope-orion-preamps-sine-ch1-4-clipping.pcapng` | `8eaf8bf8f26ff31580957f33f14fe522482977c068e54c914c5128b21b9667a5` | 34.684076 s | raw-0 saturation |
| `antelope-orion-surround-ch1-16-oscillator1khz-4secpauses.pcapng` | `fd8e93d550d2c3bcf8bbb0f1a979c1c2918c7463abe8c1e8bbf2329728ce9947` | 104.579540 s | `75/1f` route correlation |

### Inbound family boundary

The scan found 161,541 `0x73` reports and 161,541 `0x75/0x1f` reports.
It found no data-bearing inbound family other than those two families.
It found no `0x75/0x00` response and no `0x74` request.

This was an all-family inventory, not a Zen-family filter applied to Orion.
The result supports the current strict `0x75/0x1f` meter discriminator.
It gives no positive evidence for a runtime filter change.

### Physical preamp bank

The sequential sine capture maps physical preamps 1-12 to `0x73` @221-232.
The profile payload-relative range is `0xcd..0xd8`.
Each long activity window was isolated to the corresponding lane.

| Preamp | Full offset | Payload offset | Main activity time | Exemplar frame / time | Dominant raw |
|---:|---:|---:|---:|---:|---:|
| 1 | 221 | `0xcd` | 4.593100-9.404877 s | 2357 / 4.689147 s | 13 |
| 2 | 222 | `0xce` | 12.348870-18.116708 s | 6249 / 12.476935 s | 13 |
| 3 | 223 | `0xcf` | 21.124722-27.124605 s | 10621 / 21.220693 s | 12 |
| 4 | 224 | `0xd0` | 31.580454-38.140393 s | 15853 / 31.684501 s | 12 |
| 5 | 225 | `0xd1` | 42.540258-48.764108 s | 21297 / 42.572204 s | 12 |
| 6 | 226 | `0xd2` | 53.100091-58.267920 s | 26561 / 53.100091 s | 12 |
| 7 | 227 | `0xd3` | 63.587873-69.979728 s | 31821 / 63.619888 s | 12 |
| 8 | 228 | `0xd4` | 74.507635-81.163576 s | 37281 / 74.539658 s | 12 |
| 9 | 229 | `0xd5` | 85.435429-92.979393 s | 42745 / 85.467460 s | 12 |
| 10 | 230 | `0xd6` | 97.147201-102.419115 s | 48617 / 97.211212 s | 12 |
| 11 | 231 | `0xd7` | 107.011049-113.538881 s | 53633 / 107.243048 s | 12 |
| 12 | 232 | `0xd8` | 118.394819-125.186636 s | 59209 / 118.394819 s | 12 |

A direct spot check at frame 2357 found @221=13 and @222=96.
The selector byte @121 was 15, but the clipping capture used selector value 18.
Both captures used the same physical-preamp offsets.

The clipping capture drove @221-224 predominantly to raw 0.
For example, frame 3049 at 6.072861 seconds had @221=0.
No independent report byte changed as a clip indicator across that capture.
Raw 0 means top-of-scale saturation in this evidence, not a separate clip bit.
The captures do not provide a calibrated transfer curve for all raw values.

### Selector-dependent 32-lane bank

`0x73` @125-156 is a selected 32-lane bank, not a static ownership table.
The state selector is full-report @121.

| Selected observation | Positive mapping | Selector evidence | Representative frame / time |
|---|---|---|---|
| ADAT Out sweep | channels 1-16 to @125-140 | @121 remained 18. No selector write was captured. | ch1 @125=0, frame 2093 / 4.158456 s |
| Mix 2 sweep | channels 20-32 to @144-156 | `SET_PARAM(0x49, channel=0, value=0x16)` changed @121 from 21 to 22. | command 111299 / 222.418533 s; state 111315 / 222.448416 s; @144=0 at 111359 / 222.536465 s |
| Mix 3 and Mix 4 sweeps | none | @121 remained 22, so these were unselected-bank controls. | no ownership mapping |

Mix 2 channels 1-19 were exercised while @121 remained 21.
They did not appear in @125-143 under that state.
Only Mix 2 channels 20-32 are positively observed after selector value 22.
Mix 1 and all Mix 3 and Mix 4 channel mappings remain unresolved.

### `0x75/0x1f` route-correlated pairs

The Surround In sweep changed only two route-correlated pairs.
Channel 1 changed @32 and @48 together.
Channel 2 changed @33 and @49 together.

| Routed channel | Route command frame / time | Meter frame / time | Active values |
|---:|---:|---:|---|
| 1 | 1991 / 3.954135 s | 2023 / 4.018022 s | @32=0 and @48=0; @33/@49 remained 96 |
| 2 | 5467 / 10.898723 s | 5511 / 10.985827 s | @33=0 and @49=0; @32/@48 remained 96 |

The values returned from 0 to 96 after each route mute.
Channels 3-16 produced no `0x75/0x1f` change in this configuration.
These pairs correlate with routing, but fixed Surround ownership is not established.
They can be downstream copies or an enabled stereo subset.

### Output mapping boundary

These captures did not isolate the existing physical-output candidates.
They provide no current output-mapping delta for @157-160 or @177-178.
The approved packet-order hypothesis and its uncertainty remain unchanged.
This review does not reopen UI output labels or the user-confirmed output-metering decision.

### Current implementation boundary

The canonical profile now identifies physical preamps 1-12 at full-report
@221-232 and the generator emits exactly one finite state-report
`physical_meter` operation: base 221, stride 1, width 1, maximum index 11.
The runtime keeps unknown samples as `None`; an observed raw 96 remains a real
rest/silence sample. Raw 0 is top-of-scale saturation, not a separate or sticky
clip flag. The existing historical dB curve is not upgraded into a new
calibration claim.

The canonical `meter_report.notes` no longer calls @32 a broad aggregate or
@33 a flag. The controlled Surround routing evidence supersedes both
descriptions but does not establish fixed Surround channel ownership.

The existing provisional output hypotheses remain separate from the physical
input bank. The device manual does not specify HID meter byte ownership, so no
manual conflict was found.

### Implemented mapping and remaining capture questions

The bounded 12-channel state-report `physical_meter` layout uses full-report
base 221, payload base `0xcd`, stride 1, count 12, and inverted raw range 0-96.
The selector-dependent @125-156 bank and `0x75/0x1f` pairs remain excluded from
static mappings.

Before a selected-page implementation, capture these items:

1. Record each Meters-tab row name with its `0x49` write and the next @121 value.
2. Select Mix 2 before sweeping channels 1-32 to test @125-143.
3. Select Mix 1, Mix 3, and Mix 4 before one isolated sweep each.
4. Record the Surround format and selected page before a channels 1-16 sweep.
5. Isolate physical outputs if the existing provisional output hypothesis needs stronger ownership evidence.

## Settings annotation audit

The source annotation is `settings-general-explained.md`, SHA-256
`3c3ff8cb0398d95e3ec2798b3904bf5b40b39fafbd3e34b2edb4c8e6219a2e98`.
The audit compared its full contents with the canonical profile at `cd7adb6`.
The annotation describes operator intent, not protocol truth by itself.

| Annotated captures | Current profile clarification | Audit result |
|---|---|---|
| `spdif-gain-link`; `ADAT-link1-2-7-8`; `adat-ch1-2-3-12-link12` | S/PDIF and ADAT gain use signed `-6..12` dB values. Readback uses `0x73` @91-92 and @75-90. S/PDIF link uses space 1. ADAT and physical links both use ambiguous space 0. | The action order supports provenance. Existing frame bytes provide the encoding. No new encoding is claimed. |
| `ch-link-gain-ph-inv-test`; `ch-link-on-off` | Physical gain ranges depend on mode. Mode is enum 0-3. Phantom and phase are booleans. Link has no HID readback and Launcher mirrors partner writes. | The annotation identifies the controls. It does not prove firmware-side propagation. No profile change is needed. |
| `hp1-ctl`; `hp2-ctl`; `mon-a-ctl`; `mon-b-ctl` | Bus level is raw 0-96. Dim, mute, and mono are booleans. State uses three-byte bus slots at @28-45. | The annotation supplies click order and labels. Current profile ranges and readback are more precise. |
| `settings-linevol-mute-reampvol-toggle` | Line is bus 3 and Reamp is bus 4. Level is raw 0-96. Line mute is explicit. Reamp mute was not emitted in this capture. | Explicit `0x47` and `0x48` writes support the current profile. Do not infer an observed Reamp mute write. |
| `settings-trim-mona-monb-line-panlaw` | Trim is enum 0-6 for 20 through 14 dBu. Readback is packed at @24-25. Pan law is separate param `0x24`, from a later capture, with no known readback. | The old file contains only `0x4b` trim writes. Its filename and annotation do not provide pan-law byte evidence. |
| `settings-scrbrght-surroundEQ` | Brightness is 0-100 with readback @26, proven by a later native-macOS capture. Surround pre/post is bit 7 at `0xab` frame @18, with category `0x1b` readback, proven later. | The old file has two `0xab/0xeb` writes and no brightness write. Brightness intent in this file is capture-intent-only. |
| `settigs-thunderb-lat-dccp` | DC coupling is later confirmed as global boolean param `0x26`, without `0x73` readback. Latency modes have no vendor-HID encoding in current evidence. | This old file has no 320-byte OUT payload. Both actions are capture-intent-only here. |
| `settings-osc1-2-fq-lvl` | Later captures show global packed param `0x0a`. Frequency polarity and whether level is shared remain partly unresolved. No `0x73` readback exists. | This old file has no 320-byte OUT payload. It cannot support the later encoding. |
| `talkback-gain-int-ch1-2-12` | Source enum is 0-12. Gain is per-source raw 0-96 and reads at @74 for the selected source. | This file proves INT, preamps 1, 2, and 12 behavior. The annotation author's assumption alone does not generalize all 12 preamps. |
| `talkback-select`; `talkback-bttn` | Source selection uses global param `0x27`. Destinations use targeted param `0x5d`. Momentary press uses global param `0x1f`. | The annotation conflicts with the bytes. `talkback-select` contains source writes only. `talkback-bttn` contains destination and momentary-button writes. |
| `AntelopeINIT`; `matrixtest-pre1-cmpplay1-2` | Startup/readback and routing have separate, stronger evidence in the profile. | The annotation records startup intent and admits the route destination is forgotten. It adds no mapping. |
| `vumeters-sinewave`; `vumeter-lvls-clip--10-20-30-60-inf`; `vumeter-test-ch1` | The inverse meter direction remains useful. The new captures establish the 12-lane @221-232 bank and raw-0 saturation. | Blank or uncertain annotations do not identify ownership. Retain them only as action context. |

The remaining annotation entries add action names, not new protocol fields.
No profile enum, range, readback, or encoding changes result from this bounded audit.

### Targeted old-frame checks

Full offsets below refer to each 320-byte HID report.
The command param is @16, target or global value starts at @17, and targeted values use @18.

| Capture | SHA-256 | Direct command inventory and first frame/time |
|---|---|---|
| `settings-scrbrght-surroundEQ.pcapng` | `1eb1f446fc9c60e9c835d8f6179ec1f63876eee04579cf8c37a4a869c07333df` | two `opcode 0xab`, param `0xeb`; first frame 20872 / 17.891487 s; no brightness command |
| `settigs-thunderb-lat-dccp.pcapng` | `ab451d85eea3de4eecfec74c7e51f79c945b293ac537da6bdeb5ac6b8f260a81` | no 320-byte OUT payload |
| `settings-osc1-2-fq-lvl.pcapng` | `692ddb426c89e9046ee01011afe401c85d3f330ea877d022bc6a8a367a5bbe3c` | no 320-byte OUT payload |
| `settings-linevol-mute-reampvol-toggle.pcapng` | `fa46f2f7d99a83d3b95452e5a01ec0ce9975c6627153df216d5df3f6ac051ad7` | 52 `0x13/0x47` writes, first frame 13754 / 3.750569 s; two `0x13/0x48` writes |
| `settings-trim-mona-monb-line-panlaw.pcapng` | `95bf1dd4e60ece079ede2a3b56650b61b246b7e6ea468bd6642f40b70f631ec1` | 20 `0x13/0x4b` writes, first frame 14408 / 5.013470 s; no other command param |
| `talkback-gain-int-ch1-2-12.pcapng` | `c43d7e3b75e8b9640c28ab9f9352af17089805b194e9c6d3a067e6aee70b2221` | 106 `0x12/0x20` writes and three `0x12/0x27` writes; first gain frame 10087 / 2.240865 s |
| `talkback-select.pcapng` | `681fb10d274a1a5d17889fbb5c75675680f2aa3f86ded6a953caa8cbe1b39823` | 16 `0x12/0x27` writes with 13 distinct values; first frame 13570 / 3.309618 s |
| `talkback-bttn.pcapng` | `0d053832fa2ffbf79cb012007fde587ec4924154a836d63498dbe6795e439e5d` | eight `0x12/0x1f` and eight `0x13/0x5d` writes; first destination frame 13273 / 2.947413 s |

### Reproduction

The detailed report, analyzer, and scan output remain outside the repository.
They share the artifact directory assigned to this review.

- detailed report: `orion-meter-capture-analysis.md`, SHA-256 `992f334a0498bf06b1955e5562168b53d88a9404ea04d1d455be70ac559266f4`
- analyzer: `orion_meter_analyze.py`, SHA-256 `a38bf5babea15b04540b2212ddc7e7f99d4c0542d0119eb6235891b7be6f091a`
- scan: `orion_meter_scan.json`, SHA-256 `4372630f086442b9aca4a8fac7143a0482af10199dbae721793b10c15658b922`

Run the full bounded scan with TShark 4.7.3:

```bash
OUT='/home/ryodeushii/.pi/agent/sessions/--home-ryodeushii-repos-zen-go-tui--/subagent-artifacts/outputs/056f36f0-8a36-4790-a4e8-11f680c94f69'
D='/home/ryodeushii/repos/zen-go-tui/antelope_pcap/orion 3/captures 2026-09-07'
python3 "$OUT/orion_meter_analyze.py" "$D"/*.pcapng \
  -o "$OUT/orion_meter_scan.json"
```

Inventory all inbound data-bearing families from the scan:

```bash
jq '[.captures[].families[]
  | select(.direction == "IN" and .payload_len == 320)
  | {family, count}]
  | group_by(.family)
  | map({family: .[0].family, count: (map(.count) | add)})' \
  "$OUT/orion_meter_scan.json"
```

Extract any cited frame directly from a capture:

```bash
tshark -r "$D/antelope-orion-mix2-ch1-32-oscillator1khz-4secpauses.pcapng" \
  -Y 'frame.number==111299 || frame.number==111315 || frame.number==111359' \
  -T fields -E 'separator=/t' \
  -e frame.number -e frame.time_relative -e usb.src -e usb.dst \
  -e usb.endpoint_address -e usb.data_len -e usbhid.data -e usb.capdata
```
