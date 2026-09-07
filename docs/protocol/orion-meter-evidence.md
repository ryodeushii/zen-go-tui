# Orion meter evidence (provisional)

This is a compact evidence summary for the Orion Studio III profile. It does
not change the WebUI parser, upstream CLI, or hardware behavior.

## Scope and limits

The bounded review covered these captures:

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

`0x75` @32 remains a broad aggregate/monitor observation and @33 is a flag.
Do not describe @32 as the only live byte.
