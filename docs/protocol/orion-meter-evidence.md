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

## Retained candidate mapping

Retain full-report `0x73` offsets **157..160** as one provisional mono lane per
current Mix 1..4 label. DSP activity was observed at each lane, but fixed
lane-to-mix ownership is low confidence. Do not infer a physical-preamp or
stereo mapping from these labels.

The inverted scale is approximately raw `0x60` (96) at rest/silence, falling
toward `0x00` as signal rises.

## Repeated regions

The later exact mirror table is the governing bounded result:

- `158 ↔ 222`
- `159 ↔ 223`
- `160 ↔ 224`

Those pairs were exact throughout the six captures. First-lane copies
`157 ↔ 169` and `157 ↔ 221` are **not universal** and must not be promoted to
additional lanes or a universal 12-byte mirror block.

## Separate unresolved observation

In the playback capture, `0x73` @177/@178 and meter-only `0x75` @34/@35
co-varied nearest in time within 5 ms (`r≈0.998`). The owner remains
unresolved; this does not establish Mix 1 L/R, stereo, or physical ownership.

`0x75` @32 remains a broad aggregate/monitor observation and @33 is a flag.
Do not describe @32 as the only live byte.
