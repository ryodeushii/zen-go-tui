# Orion meter evidence (provisional)

This is a compact evidence summary for the Orion Studio III profile. It separates
capture-verified meter lanes from user-approved provisional output hypotheses.
The meter path adds no hardware writes or automatic page selection.

## Current evidence status

| Status | Meter evidence |
|---|---|
| Capture confirmed | Mix 1 strips 1-32 map to full-report `0x73` @125-156 when same-report @121 equals 21. @157-188 is an exact mirror. ADAT Out 1-16 maps to @125-140. Mix 2 strips 20-32 map to @144-156 when @121 equals 22. Active UI Mix 1-4 IDs are `0..3` at @122. Surround channels 1/2 form `75/1f` plateaus at @32/@33, mirrored at @48/@49. |
| Bounded negative | The six provisional output bytes equal 96 in all 28,689 dated output `0x73` reports. HP1, HP2, and S/PDIF have constant full `0x73` and `75/1f` reports. Mix 3/4 change only known physical-input bytes @221-232 in `0x73`, while `75/1f` stays constant. |
| Provisional | The output candidates @157-160 and @177-178 conclusively overlap Mix 1 mirrors under observed selector 21. They are not unconditional output meters. The Surround pairs lack exclusive ownership. Reamp `0x73` @234 has one unresolved transient. |
| Withdrawn | Claims that all bits were scanned exhaustively, or that the evidence proves a selected-page-only architecture, have been withdrawn. |
| Analysis pending | Mix 2 strips 1-19, Mix 3/4, physical output ownership, and other mixer mappings remain unknown. |
| Implementation pending | Mix 1 meter decoding remains unimplemented. Active mix selection is a captured contract for task 54. No target-0 selector-21 write is authorized. |

The new evidence changes the capability label, not the current runtime or profile.

## Dedicated Mix 1 sweep

### Source and integrity

The added capture path is relative to the capture checkout root:

`antelope_pcap/orion 3/captures 2026-09-07/antelope-orion-mix1-ch1-32-oscillator1khz-4secpauses.pcapng`

- size: 34,126,460 bytes
- SHA-256: `3df29fd4a67c4b93ed38bbe96a5ef1ed828d1d7f59a2a93a32e42dc4a83d009d`
- packets: 155,126
- capture duration: 309.968865 seconds
- complete IN `0x73` reports: 38,746
- complete IN strict `0x75/0x1f` reports: 38,747
- complete OUT `0x70` reports: 64

A path-and-size comparison found this one addition to the historical 77-file Orion inventory.
That bounded accounting state has 78 Orion files and 1,864,082,368 bytes.
The comparison did not rehash the prior 77 files, so it does not prove a complete 78-file content verification.
The later [active mix selection analysis](orion-active-mix-selection.md) adds a separate bounded file increment.

All 38,746 `0x73` reports have @121 equal to 21. The capture contains no selector command or selector transition.
This proves a read-state condition only. It does not authorize a selector write or prove a selected-page-only design.

### Confirmed 32-strip map

Offsets are zero-based full-report offsets. Subtract 16 to get the profile payload offset.
For strip `s` in 1 through 32, the primary offset is `124+s`. The observed mirror is `156+s`.

| Strip | Primary | Mirror | Strip | Primary | Mirror |
|---:|---:|---:|---:|---:|---:|
| 1 | @125 | @157 | 17 | @141 | @173 |
| 2 | @126 | @158 | 18 | @142 | @174 |
| 3 | @127 | @159 | 19 | @143 | @175 |
| 4 | @128 | @160 | 20 | @144 | @176 |
| 5 | @129 | @161 | 21 | @145 | @177 |
| 6 | @130 | @162 | 22 | @146 | @178 |
| 7 | @131 | @163 | 23 | @147 | @179 |
| 8 | @132 | @164 | 24 | @148 | @180 |
| 9 | @133 | @165 | 25 | @149 | @181 |
| 10 | @134 | @166 | 26 | @150 | @182 |
| 11 | @135 | @167 | 27 | @151 | @183 |
| 12 | @136 | @168 | 28 | @152 | @184 |
| 13 | @137 | @169 | 29 | @153 | @185 |
| 14 | @138 | @170 | 30 | @154 | @186 |
| 15 | @139 | @171 | 31 | @155 | @187 |
| 16 | @140 | @172 | 32 | @156 | @188 |

Each of the 32 route-on reports contains one actual `(12,0)` tuple at the named strip.
Each paired route-mute report contains 32 `(11,0)` tuples. The capture has exactly 32 ordered pairs.

Every target lane is raw 0 throughout its core window. The 20,026 core samples contain no non-96 value in the other 31 primary lanes.
Every primary/mirror pair matches in all 38,746 reports. A decoder must publish one 32-strip bank, not 64 channels.

Raw 96 is rest or silence. Raw 0 is the stable captured 1 kHz level.
Raw 18 appears during the strip-16 attack. A coherent raw-1 transient also occurs after the strip-1 mute.
The evidence supports an inverted raw `0..96` domain. It does not support binary decoding or a dB calibration.

The new mirror result bounds the old output hypothesis. Under selector 21, @157-160 are Mix 1 strips 1-4 mirrors.
Under the same condition, @177-178 are strips 21-22 mirrors. The capture proves no unconditional physical output ownership.

Full-report @205 has 133 non-96 reports in 31 short runs after mutes 2-32.
It remains 96 in all 21,628 route-active samples. This correlation does not establish meter ownership.

### Fixture and analysis provenance

Artifact set `e901b159-1ee1-4f99-87f9-0d37f21a5eb8` contains the authoritative report and machine evidence.

| Fixture | Frame / time | Full 320-byte payload SHA-256 | Key assertion |
|---|---|---|---|
| Mix 1 strip 1 | 2077 / 4.124239 s | `4483d3dd5b779d989376ed9df3e67a5941d680a030dfe15553c13210f8bb3cab` | @121=21, @125=@157=0, and other primary lanes=96 |
| Mix 1 strip 16 | 70133 / 140.117006 s | `63bfbd78708125ba4bc91876287ceb7ab8fd22a32e757117c336dd7cc76b02c8` | @121=21, @140=@172=0, and other primary lanes=96 |
| Mix 1 strip 32 | 150641 / 301.005063 s | `7ede5cd7e7b47c9985f58fae90330d3f28aa34719d0abde5770e13c67e825609` | @121=21, @156=@188=0, and other primary lanes=96 |
| Strict-family negative | 2079 / 4.128204 s | `a853678692ef5ada2b78da878101500804409ec5ffd6cca7c369ac702c62046b` | `75/1f` template `(50,50)`, identical across this family |
| Selector-21 baseline | 1947 / 3.868266 s | `a095bca82b627178ea9e8aaa4116d7d355079bb1b2817adaa811a1f0f47b4d1d` | all @125-188 values are 96 |

| Artifact | SHA-256 |
|---|---|
| `new-mix1-meter-capture-analysis.md` | `7c7c633c4f52a4f1079a5b86261d12ca4d2bf9f1bea583987292948a24b2e0bd` |
| `new-mix1-partial-inventory.json` | `8e651424e8fcb9a6910466136c4b09eee8ccdb9bcfadf65c507ad23d54118079` |
| `orion-77-to-78-comparison.json` | `5806ecd4ededca07402fcdc75323d4c5b6694a1fd17946fb2b425d9700500195` |
| `new-mix1-window-stats.json` | `3773d2c94a9efb86ef71b5c7dc8056f03cf97dde06293bf246cbc0d4337fbad9` |
| `analyze_new_mix1.py` | `19194a1a2c0dea7727cadcdc2765091fa4914a0cd8c7567439346f1586cb17d4` |

The detailed JSON retains exact distributions, bit counts, route windows, timing, transients, controls, and full fixture bytes.
The scan included only the new capture and targeted ADAT and Mix 2 positive controls. It was not a 78-file rescan.

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
or a hardware-confirmed fixed lane owner. No additional hardware test is implied.

## Historical Todo 41 corpus update

### Inventory and provenance

The Todo 41 inventory contained **99 files** and **15 dated PCAPNGs**,
compared with the earlier 92-file snapshot and eight dated PCAPNGs. The seven
additions exercise HP1, HP2, Line Out, Monitor A, Monitor B, Reamp, and S/PDIF.
The later [historical complete family inventory](capture-family-inventory.md) counts 77 Orion PCAPNGs.
This retracts any absolute statement that output-capture coverage was
exhausted. All output captures in this 99-file snapshot were inventoried.
An independent challenge replaced their broad negative conclusions with the narrower exact findings below.

This update was consolidated at root
`7a436608239ca61d474707cea6101f09b70006b4` and nested Antelope-Ctl
`eb950a3f9582662571dab750775530e5adf3f47e`. At that revision, the canonical Orion
profile SHA-256 was
`9e76a4e4b77279961741c8c1ebf7ed01b3f3aff3d9325b3d9b9ed53966d6193f`.
At the current nested revision `d5f9bb9`, its SHA-256 is
`d6eeccd5c7ddf0cb13037f598c11faa30369dde50d11277775d75dd51d63e5b5`.
The managed analysis is artifact set `b67fef34-0986-4688-b9cd-3dd44e8cf143`,
file `orion-updated-capture-analysis.md`, SHA-256
`76c432aabd172bc7025141bb10dd4a496378f3153fd993bff32b5eb0e7809697`.
Its inventory, state-context, and full-window inputs are respectively:

- `orion-current-inventory.json`, SHA-256 `83201ec3dc9495009440f7bcab5d36d09c0a48c347315b8d7873729bb73d76ae`;
- `dated-state-context.json`, SHA-256 `1dcfa19d01a261cd0cfea6b1f7c1fce86193b7a48ef2e4bd63e835a38ae4939c`;
- `dated-full-scan.json`, SHA-256 `c3aad0769efa16fc8256b30d17ef2cb05c8f31cbc5c9499284f92e3b34b1e5d6`.

The route windows below use `on frame/time -> mute frame/time`. They come from
the full 320-byte OUT reports, not from filenames.

| New capture | SHA-256 | Complete route windows |
|---|---|---|
| `antelope-orion-hp1-1-2-oscillator1khz-4secpauses.pcapng` | `baca5f3d9f6005cd73f8dcd4359dd958913a02c1462f82fa5c05b68ce9f4d3e8` | ch1 1429/2.829457 -> 3079/6.126455; ch2 4061/8.084999 -> 5885/11.732428 |
| `antelope-orion-hp2-1-2-oscillator1khz-4secpauses.pcapng` | `5a8295c8ce0e4e4a66edeb15a8b3350863aa75a885c708394209c2dd23d667fb` | ch1 2037/4.049911 -> 5819/11.608810; ch2 6523/13.013124 -> 9021/18.004798 |
| `antelope-orion-lineout-1-16-oscillator1khz-4secpauses.pcapng` | `7bd6001f0411a22c49bf74ffb86fa9c12fb6c640bddfca528984d6e5a8c5d4d8` | ch1 1699/3.373887 -> 3917/7.806729; ch2 4397/8.762186 -> 6599/13.159807; ch3 7365/14.687756 -> 10465/20.886097; ch4 10987/21.924246 -> 13711/27.367801; ch5 14271/28.483780 -> 16937/33.812807; ch6 17995/35.922943 -> 22429/44.786728; ch7 22937/45.800819 -> 26113/52.147213; ch8 26811/53.540463 -> 30265/60.444927; ch9 31053/62.017288 -> 35013/69.929865; ch10 35757/71.416710 -> 38615/77.128252; ch11 39181/78.254027 -> 41591/83.070567; ch12 42009/83.902842 -> 44445/88.772904; ch13 45103/90.084728 -> 47341/94.553828; ch14 48373/96.614879 -> 51431/102.727858; ch15 52111/104.080875 -> 54325/108.508127; ch16 54955/109.761416 -> 56993/113.835872 |
| `antelope-orion-monitora-1-2-oscillator1khz-4secpauses.pcapng` | `bf53cb17a14e7453b22c96039dc5963e6b8a04c382a907214688f77f5250a8e7` | ch1 1901/3.775574 -> 4133/8.235754; ch2 5223/10.412089 -> 7889/15.739989 |
| `antelope-orion-monitorb-1-2-oscillator1khz-4secpauses.pcapng` | `8007e4d4ab4a9a11986820a0b22acc02a52fbb2602faad681d8cd5a16b1c7e5e` | ch1 2015/4.002805 -> 4939/9.846135; ch2 5525/11.014136 -> 9631/19.224749 |
| `antelope-orion-reamp-1-16-oscillator1khz-4secpauses.pcapng` | `1d15d0e747c77f077521b1fcace198cd29ac1dfa1002b904e7c391da45895f9e` | ch1 2053/4.084083 -> 4509/8.988928; ch2 5285/10.538832 -> 7903/15.771708 |
| `antelope-orion-spdif-1-2-oscillator1khz-4secpauses.pcapng` | `d75ae8c05de5884cd5544d5afcdab8f14ee209f4c516f67c85560095c114b4fe` | ch1 1793/3.557094 -> 5043/10.053071; ch2 6107/12.178771 -> 9229/18.417756 |

### Reassessment boundary

The operator confirms that the earlier dated captures are dedicated metering captures.
They use a true no-signal baseline and a 1 kHz oscillator per strip with about four-second pauses.
This context applies to Mixes 2-4 and similarly named ADAT, preamp, Reamp, S/PDIF, Surround, and output captures.
That eight-capture review omitted Mix 1. The later dedicated Mix 1 capture now supplies positive evidence.

All seven dated output captures keep full-report @121 at 18. Each route-on report installs source `(0x0c, 0)`.
The paired report restores mute `(0x0b, 0)`. No file contains `SET_GLOBAL(0x0a)`.
That absence does not negate the confirmed oscillator signal because generator setup can predate the capture.

The negative conclusions from artifact set `a4f9ffcc-8fe0-4f30-b67a-a3d026d3881e` were challenged.
`fresh_window_scan.py` lines 61-73 require an 80 percent whole-byte or multibyte modal gate.
The script attaches bit rates only to the five highest-ranked whole bytes.
It also pools `75/1f` templates instead of analyzing each template separately.
No selected post window overlaps the next route-on, but the method did not prove this in advance.
A route write is also not a sample-accurate signal marker.
Do not cite this scan as an exhaustive bit or encoding exclusion.

The independent raw-record check establishes narrower exact facts:

- HP1 has 1,656 `0x73` and 1,655 `75/1f` reports. Each family is constant across the full capture.
- HP2 has 2,459 `0x73` and 2,460 `75/1f` reports. Each family is constant across the full capture.
- S/PDIF has 2,697 reports in each family. Each family is constant across the full capture.
- The 27,805 pre-selector Mix 2 state reports change only within physical-input @223-232.
- Mix 2 strict `75/1f` is constant across all 45,805 reports in that capture.
- Mix 3 and Mix 4 change only @221-232 in `0x73`. The strict `75/1f` reports are constant.
- The profile independently identifies @221-232 as physical-input meters.
- The six provisional output bytes equal 96 in all 28,689 dated output `0x73` reports.
- This output check covers 172,134 exact byte observations across seven captures.

These facts disprove the six candidates as unconditional meters for the dated output sweeps.
They do not prove that output meters do not exist on another page, template, or transport.
The findings also do not prove a selected-page-only architecture.
The confirmed ADAT and selector-gated Mix 2 mappings remain valid positive evidence.
Other dated mappings stay unknown, not uncaptured.

The Reamp capture contains unresolved changes at full-report @205 and @234.
At @234, four `0x73` reports equal 90 from 10.812139 to 10.836057 seconds.
The event starts 273.307 ms after the Reamp 2 route-on.
Strict `75/1f` @32, @33, @48, and @49 equal 0 in five reports after Reamp 1 mute.
That event starts 267.150 ms after mute and lasts 32 ms.
Neither event establishes Reamp ownership.

Surround channel 1 forms a real plateau at `75/1f` @32 and mirrors at @48.
Channel 2 uses @33 and mirrors at @49.
The channel 1 release reaches 96 about 116 ms after mute.
Exclusive Surround ownership remains unproved.

### Challenged analysis provenance

Artifact set `a4f9ffcc-8fe0-4f30-b67a-a3d026d3881e` remains a traceable analysis record.
Artifact set `c200b5ac-255f-4c10-bbc8-bc78a0036bf4` contains the independent challenge.

| Artifact | SHA-256 | Current disposition |
|---|---|---|
| `mixer-output-capture-reassessment.md` | `3faeabe7f8baa639e5f0fa0453656d8717cd24ab0001b6332d7b775ccfbd7029` | Positive mappings retained. Negative completeness claims withdrawn. |
| `fresh_window_scan.py` | `4d92fa1fbb9854910205da0fc4421dfc47177776f4a76e6551756f85a40c3460` | Method record with the whole-byte gate and top-five bit limitation. |
| `fresh-window-scan.json` | `f4e5905357b78e4a24899e0b372724778b4ead65de34332f27469442d50ed143` | Candidate output from the challenged method. |
| `route-inventory.json` | `6d5e88e1e9d952b0481ae36b7a7dcfb76f901b37c134e9432ff9fc4cb375fdb4` | Route and family inventory, not meter-encoding proof. |
| `meter-negative-evidence-challenge.md` | `47b702fcc40ecdf52b9f75deb119c7638721472c40a84a44e77941467ecfd4bd` | Accepted narrow exact-constancy check and method audit. |

### Surround-export boundary and follow-ups

`new/srrnd-20-21.frames.txt` is a legitimate full-report export, SHA-256
`3a2ad5b8710beda965bc1fe5ff07673371cd961897d4bc957baaa88d51deac8b`.
Its lines 1 and 35, at 7.248712 s and 13.730188 s, are 320-byte 2.1 reports
with report SHA-256
`f0e167bf8cba68b76873b92337f741fb64c1f02ad1bea2d861784830d31e571e`;
they are not synthetic. The separate gap is a paired category-`0x1b` 2.1
readback/full-write contract. `new/macos-srrnd-tab.frames.txt` contains 32-byte
heads, but that limitation must not be generalized to the other `new/`
exports.

| Todo | Evidence-bound next step |
|---:|---|
| 35-36 | Keep unknown meter maps pending independent analysis. Do not classify them as missing captures or confirmed non-exposure. |
| 37 | The existing 2.0 global Surround UI can resume after this documentation commit; the separate 2.1 paired-readback gap remains. |
| 38 | Orion AuraVerb can proceed independently. The Zen wire contract remains unverified. |
| 39 | Mixer evidence blocker; this is not a navigation/UI todo. |
| 40 | Output-ownership blocker; this is not a navigation/UI todo. |
| 53 | Inspect existing mixer-fader controls, backend support, and captures for missing per-strip gain control. Do not confuse it with physical preamp gain. |

## Approved provisional output assignment

The runtime stores this explicit user-approved packet-order hypothesis.
The new selector-21 capture proves that each listed byte overlaps a Mix 1 mirror in that context:

| Full-report `0x73` offset | Profile output id | Output | Lane |
|---:|---:|---|---:|
| 157 | 0 | Monitor A | 0 |
| 158 | 1 | Headphone 1 | 0 |
| 159 | 2 | Headphone 2 | 0 |
| 160 | 3 | Line Out | 0 |
| 177 | 4 | Reamp | 0 |
| 178 | 5 | Monitor B | 0 |

These are six one-lane candidates in packet order, not independently verified
physical-output meters. They cannot be unconditional output lanes because they
mirror Mix 1 strips under selector 21. The feed, meter stage, and physical
post-fader ownership remain unknown. No L/R geometry is inferred. In particular,
@177/@178 were observed as a playback-coupled pair. Their separate Reamp and
Monitor B labels are ordering assumptions, not distinct physical-output proof.

The user reports that the current Monitor A and Headphone 1 meters display Mix 1 strips 1 and 2.
This observation matches the @157/@158 mirror overlap. It does not establish physical-output ownership.

The inverted scale is bounded to raw `0x00..0x60`: `0x60` (96) is retained as
silence/rest and values fall toward `0x00` as signal rises.

## Historical repeated regions

An earlier six-capture review found these exact pairs in that corpus:

- `158 ↔ 222`
- `159 ↔ 223`
- `160 ↔ 224`

First-lane copies `157 ↔ 169` and `157 ↔ 221` were not universal in that corpus.
Do not promote them to additional lanes or a universal 12-byte mirror block.
The dedicated Mix 1 result governs @125-188 only when the same report has @121 equal to 21.

## Playback-coupled observation

In the playback capture, `0x73` @177/@178 and meter-only `0x75` @34/@35
co-varied nearest in time within 5 ms (`r≈0.998`). The explicit output
assignment above does not strengthen that observation into ownership evidence.
The `0x75` pair remains excluded so the same activity is not counted twice.

The 2026-09-07 captures supersede the broad-aggregate and flag descriptions.
They show route-correlated pairs at @32/@48 and @33/@49, with unresolved ownership.

## Historical eight-capture evidence

### Source identity and offset convention

This historical review used eight captures under this capture-root-relative directory:

`antelope_pcap/orion 3/captures 2026-09-07/`

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
| `antelope-orion-mix3-ch1-32-oscillator1khz-4secpauses.pcapng` | `0f574f16446ffb826fe1e623f692d8da2348e4effacdfdf7020818b2c45d7f55` | 275.322483 s | dedicated Mix 3 sweep, mapping analysis pending |
| `antelope-orion-mix4-ch1-32-oscillator1khz-4secpauses.pcapng` | `6926ef727b9facc59e30b4d393555f3d98f37ec28851f2c987f616487534a6b2` | 223.258452 s | dedicated Mix 4 sweep, mapping analysis pending |
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

### Selector-gated observed bank

The positive mapping in `0x73` @125-156 depends on the captured selector context.
This does not define a complete selected-page architecture.
The state selector is full-report @121.

| Selected observation | Positive mapping | Selector evidence | Representative frame / time |
|---|---|---|---|
| ADAT Out sweep | channels 1-16 to @125-140 | @121 remained 18. No selector write was captured. | ch1 @125=0, frame 2093 / 4.158456 s |
| Mix 1 sweep | strips 1-32 to @125-156, mirrored at @157-188 | @121 remained 21. No selector write was captured. | ch1 @125=@157=0, frame 2077 / 4.124239 s |
| Mix 2 sweep | strips 20-32 to @144-156 | `SET_PARAM(0x49, channel=0, value=0x16)` changed @121 from 21 to 22. | command 111299 / 222.418533 s; state 111315 / 222.448416 s; @144=0 at 111359 / 222.536465 s |
| Mix 3 and Mix 4 sweeps | no mapping | @121 remained 22. `0x73` changes only at physical-input @221-232, and strict `75/1f` is constant. | no confirmed ownership mapping |

Mix 2 channels 1-19 were exercised while @121 remained 21.
Strip 20 stayed at 96 under 21, then reached 0 after the captured transition to 22.
Only Mix 2 channels 20-32 are positively mapped under selector value 22.
The other Mix 2 lanes and all Mix 3/4 mappings remain unknown.

The [active mix selection contract](orion-active-mix-selection.md) confirms a separate selector domain.
`SET_PARAM(0x49, target=1, value=0..3)` selects active UI Mix 1 through Mix 4.
Complete `0x73` reports return that active UI mix ID at full-report @122.

Full-report @121 stayed 18 throughout all four active UI selections. Every mixer lane at @125..188 also stayed 96.
This no-signal capture does not prove that @122 selects the physical meter bank.

The Mix 2 capture still provides the only target-0 selector write. It changed @121 from 21 to 22.
The Mix 1 capture still proves only an @121 read gate of 21. No target-0 selector-21 write is proven.

Do not map active UI IDs 2 and 3 to @121 values 23 and 24. Do not infer Mix 3 or Mix 4 meter offsets from them.

### `0x75/0x1f` route-correlated pairs

The Surround In sweep changed only two route-correlated pairs.
Channel 1 changed @32 and @48 together.
Channel 2 changed @33 and @49 together.

| Routed channel | Route command frame / time | Meter frame / time | Active values |
|---:|---:|---:|---|
| 1 | 1991 / 3.954135 s | 2023 / 4.018022 s | @32=0 and @48=0; @33/@49 remained 96 |
| 2 | 5467 / 10.898723 s | 5511 / 10.985827 s | @33=0 and @49=0; @32/@48 remained 96 |

The channel 1 value returned to 96 about 116 ms after route mute.
Channel 2 has the same bounded plateau and mirrored shape.
Channels 3-16 produced no `0x75/0x1f` plateau in this configuration.
These pairs correlate with routing, but fixed Surround ownership is not established.
They can be downstream copies or an enabled stereo subset.

### Output mapping boundary

The dated captures isolate output exercises by named channel according to the operator's capture log.
All six candidate bytes equal 96 in 28,689 state reports across the seven output captures.
This gives 172,134 exact observations and disproves unconditional output ownership in these sweeps.
HP1, HP2, and S/PDIF also have constant full reports in both cyclic families.
The packet-order hypothesis remains provisional for other contexts.
No fixed ownership, L/R geometry, page, or alternate transport follows from these facts.

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

### Implemented mapping and remaining questions

The bounded 12-channel state-report `physical_meter` layout uses full-report
base 221, payload base `0xcd`, stride 1, count 12, and inverted raw range 0-96.
The selected bank now has one bounded runtime mapping: Mix 2 strips 20-32 use
full-report @144-156 (payload @128-140) only when the same complete `0x73`
report reads selector @121 (payload @105) == 22. The profile names the real
normalized Mix 2 surface (`mix_index` 1) and the 1-based strip ids 20-32. Each
lane remains unknown until a matching complete report arrives; a selector
mismatch, an out-of-range sample, or a recognized truncated `0x73` invalidates
only these 13 strip readings. Physical-input and provisional output meters are
not cleared by that selector gate. Decoding is read-only and emits no page
selection write. The existing dynamic mixer-strip meter UI is reused, with
`None` remaining visibly unknown rather than false zero/silence.

This is verified **partial** Mix 2 runtime support, not a complete mixer-meter map.
Mix 1 strips 1-32 are capture-confirmed but not implemented at root revision `dbf1314`.
Mix 2 strips 1-19, every Mix 3/4 strip, and full-report @205 remain unmapped.
The `0x75/0x1f` family remains excluded from mixer-strip decoding and its
readback discriminator behavior is unchanged.

Regression fixtures are exact 320-byte `usbhid.data` extractions from capture
`antelope-orion-mix2-ch1-32-oscillator1khz-4secpauses.pcapng` (SHA-256
`0e7ba242fce5a09ab750a91bed1b175596d5b7dcd6c179ed8bb0edfd2a957ad2`):

- frame 111359, strip 20 fixture: `43ef942e447524305bcdd2e782002b24219acb8218bbef52408e92d88422a28b`
- frame 114099, strip 21 fixture: `0523dbd3ab0f2785541bfb9e3064ba054f0054ac232c5e44cbd99cab1c4a450d`
- frame 178351, strip 32 fixture: `384b23ae6ee9085705b149ab7ca2517af37e485dfe7c0facce9c7c0b79afcf32`

Before full mixer-meter support, complete these evidence and implementation reviews:

1. Review the capture-confirmed Mix 1 read-only mapping before implementation.
2. Implement the separate active UI selector contract only through task 54.
3. Keep @122 separate from the physical meter-context field at @121.
4. Compare `0x73` @205 across meter contexts without assigning ownership.
5. Event-index strict `75/1f` @32, @33, @48, and @49 by the @8/@20 template.
6. Keep Mix 2 strips 1-19 and all Mix 3/4 strips unknown pending positive evidence.

Treat nonmapped lanes as unknown unless reviewed evidence proves a stronger result.
Do not request replacement captures only because the challenged scan found no candidate.
Existing control fan-out proves addressing only.

### Todo 53 per-strip gain boundary

The user reports that per-strip gain control is missing. Todo 53 must inspect the existing fader UI, backend path, and captures.
This pending mixer-strip control is not physical preamp gain. No per-strip gain implementation follows from the meter evidence.

## Settings annotation audit

The source annotation is `settings-general-explained.md`, SHA-256
`3c3ff8cb0398d95e3ec2798b3904bf5b40b39fafbd3e34b2edb4c8e6219a2e98`.
A fresh full reread produced that same hash, exactly matching the hash already
recorded in this document; the file is unchanged relative to this documented
evidence. The managed inventory's lack of an older JSON-side annotation hash
does not make the prior hash unavailable. The audit compared its full contents
with the canonical profile at `cd7adb6`. The annotation describes operator
intent, not protocol truth by itself.

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
They use artifact set `056f36f0-8a36-4790-a4e8-11f680c94f69` in the agent session output store.

- detailed report: `orion-meter-capture-analysis.md`, SHA-256 `992f334a0498bf06b1955e5562168b53d88a9404ea04d1d455be70ac559266f4`
- analyzer: `orion_meter_analyze.py`, SHA-256 `a38bf5babea15b04540b2212ddc7e7f99d4c0542d0119eb6235891b7be6f091a`
- scan: `orion_meter_scan.json`, SHA-256 `4372630f086442b9aca4a8fac7143a0482af10199dbae721793b10c15658b922`

Run the full bounded scan with TShark 4.7.3:

```bash
OUT="$AGENT_ARTIFACT_ROOT/outputs/056f36f0-8a36-4790-a4e8-11f680c94f69"
D="$CAPTURE_CHECKOUT/antelope_pcap/orion 3/captures 2026-09-07"
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

### Surround first-slice meter boundary

The first functional Surround slice adds no meter ownership. The `0x75/0x1f`
pairs at @32/@48 and @33/@49 remain sustained route-correlated observations
for channels 1 and 2 respectively; the Reamp post-mute all-pairs transient above prevents an
exclusive Surround-owner interpretation. The runtime selector-gated `0x73` bank
remains scoped to its existing Mix 2 mapping. The evidence also confirms Mix 1,
but root revision `dbf1314` does not implement it. No Surround output, mix, or
16-channel meter map is inferred from those bytes.
