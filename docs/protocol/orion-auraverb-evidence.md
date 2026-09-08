# Orion Studio SC AuraVerb capture evidence

This note records the finite evidence used by the Rust AuraVerb backend. It does not generalize the wire format to Zen Go or to Mixes 2–4.

## Sources and report inventory

The OUT evidence is from these raw macOS captures:

- `/home/ryodeushii/repos/zen-go-tui/antelope_pcap/orion 3/macos-captures/macos-auraverb-on-off.pcapng`, SHA-256 `f610bfa4277b9648b41db57d2bc2b174606be31b4894ef391fa7dff74944d928`;
- `/home/ryodeushii/repos/zen-go-tui/antelope_pcap/orion 3/macos-captures/macos-auraverb-ctl-color-predelay-earlyrefgaij-laterefdelay-richness-reverbtime-roomsize-reverblevel.pcapng`, SHA-256 `7c2a0985cd4f9e1d0f1f67fecbcb6fc7ca79fb88d7438032e314da011ed334c7`.

`tshark` extraction found two and 1,836 complete 320-byte AuraVerb OUT reports, respectively. Every report has `0x70` at report offset 0, opcode `0x1d` at offset 4, parameter ID `0xda` at offset 16, subcommand `0x0b` at offset 17, and captured Mix-1 target `0` at offset 18. Parameter offsets are Room Size 19, Color 20, PreDelay 21, Early Reflection Gain 23, Late Reflection Delay 24, Richness 25, Reverb Time 26, and Reverb Level 27. Offset 22 is always 100. Offset 28 is enable. Offsets 29–319 are zero in all 1,838 matching reports.

Across the 1,836-report sweep, only offsets 19, 20, 21, 23, 24, 25, 26, and 27 vary. Every one reaches both 0 and 100. The mechanically extracted fixture pairs below were selected by grouping reports after removing the named byte, then choosing a group containing both endpoints. Therefore each pair differs at exactly the listed byte while every peer field and the reserved tail is byte-identical.

| Field | Offset | 0 frame / time / report SHA-256 | 100 frame / time / report SHA-256 |
|---|---:|---|---|
| Room Size | 19 | 87346 / 154.895661 s / `5b2c63bcdb6205ec1dc4f1d14c00e866cd24c7d16ee0ce186f9f8732efeff07b` | 89622 / 159.380078 s / `25b08c9f1249eb3d57c872829547df2517b6ef3da7629006a782c4ceddbab8ec` |
| Color | 20 | 4685 / 5.568133 s / `f393fdf0aa9612351c8115560064fc7f1f714d6ad17687877a5c744f4658652` | 2418 / 3.024066 s / `58ea3358e58bad167c08b7b9d2a39373911ed34108de1da605fb9f6b2467f7b7` |
| PreDelay | 21 | 4685 / 5.568133 s / `f393fdf0aa9612351c8115560064fc7f1f714d6ad17687877a5c744f4658652` | 20521 / 33.364718 s / `5d84d14c1472083dfd50b4ac81cf95c86f4012586bb56c0b940cdae6261ac893` |
| Early Reflection Gain | 23 | 31226 / 52.285126 s / `e39d220d4695cd94568692717b61f9bc2473652f41194282c0383069b8ebfe2f` | 33102 / 55.905205 s / `e841aaecc02d4572cf7e5ec995ccfaffb07e4941e1b2d2cb5a310f93028a843e` |
| Late Reflection Delay | 24 | 42934 / 72.793567 s / `29da64dc8026cfa0e0a13e517ca56facf7cf2dcb54659e88c38f84101cb84742` | 44854 / 76.501675 s / `364d367efcbfeb4c889c7dd5da364fa698afaa100ab079678afed7ba34215263` |
| Richness | 25 | 60812 / 104.282243 s / `e9334dd6cc5a1723386c6aa84390c525eebf7300fff8c669aeaf1be442e1ae1c` | 62824 / 108.334329 s / `bc717e85c27347b7220f00ae338840a299c4a42939b4d5fc54777add653acb93` |
| Reverb Time | 26 | 73816 / 128.170756 s / `9db7afb07c34f622a31611d2cb4239328cdb500af4d7d321cf160f3cad8836ff` | 76116 / 132.970855 s / `382a9ef45d9cd73b80f290f2e4500ee29b87a6afb0d2e230a8746316b8411c03` |
| Reverb Level | 27 | 102009 / 179.845232 s / `ca3b8176c3195b48093c67e95984c6706660fa4c054555e5d300202f4de630c6` | 104054 / 183.525361 s / `93753294e7b9598a08330f56df444011a330b34d209aaead4fb364953ed6dee3` |

The independent enable pair is frame 5824 at 3.449054 s, SHA-256 `58ea3358e58bad167c08b7b9d2a39373911ed34108de1da605fb9f6b2467f7b7`, and frame 8628 at 8.242198 s, SHA-256 `39c09da6c389f17e6b44633747a37bde7888e615c095c5dc6eba6ab0230d1903`. Only offset 28 changes, from 1 to 0.

All reports above are checked in verbatim as full-frame `.hex` fixtures under `antelope-protocol/tests/fixtures/orion/auraverb/`.

## Independent IN readback alignment

AuraVerb readback alignment comes from actual endpoint `0x82` HID IN reports, not from transforming OUT frames. Four independent init captures each contain two byte-identical 320-byte category-`0x0a`, index-0 reports with SHA-256 `53c50af3011aa165b52ce10ffce4e4b9b3cce863a86f7561fab913b25283a6dd`:

| Capture SHA-256 | Frames / relative times | Checked-in IN fixture |
|---|---|---|
| `cf11354ec1c0c2a43189f1902155aefad149f5e730465e1cd7ef8c8188873960` | 39467 / 46.913429 s; 42569 / 51.385561 s | `readback_mix1_poweroff_on2.hex` |
| `c6bac59df0340222421b8e4c19192e8da03ec015cf3a5358ba84f20608d437c0` | 16126 / 26.464686 s; 19078 / 30.896841 s | `readback_mix1_poweroff_on3_saved.hex` |
| `195fea4caefea43d3de5141b9457c50eaf72fe093899e47f8d46c645c7e40d80` | 13114 / 22.905237 s; 18236 / 27.417300 s | `readback_mix1_poweron.hex` |
| `d4544f599accb669c1272c681199d6575dd938b0d98f0ea627d6b48f3cda4d20` | 12978 / 22.530826 s; 15818 / 27.035536 s | `readback_mix1_poweron_previous.hex` |

Each fixture is an independently extracted endpoint-`0x82` IN report; none is synthesized from an OUT command. The exact report header is `75 00 00 00 40 01 00 00 0a 00 00 00 00 00 00 00`: category is at offset 8 and index at offset 12. The meaningful body begins at report offset 16:

- body byte 0 is a fixed `00` header;
- body bytes 1–11 are the complete Mix-1 block;
- body bytes 12–22 and 23–33 are complete peer-mix blocks;
- body bytes 34–42 are the truncated nine-byte fourth block;
- report offsets 59–319 are zero padding in all eight reports.

The complete block order is Room Size, Color, PreDelay, fixed wet=100, Early Reflection Gain, Late Reflection Delay, Richness, Reverb Time, Reverb Level, enabled (0 or 1), terminator `0xff`. The Rust authority path requires the exact report header, body header, complete first block, first-block wet/terminator constants, parameter ranges, enabled domain, and fixed zero tail. Peer-mix bytes are retained in the event body but are deliberately ignored for Mix-1 write authority; their values and the truncated fourth block cannot invalidate an otherwise valid first block.

## Scope boundary and recovery

Only Orion Studio SC identity `0x23e5:0xa221`, operation `0xda`, target 0/Mix 1, eight 0–100 parameters, and enable are writable. Every write is a complete frame reconstructed from authoritative readback or the latest complete pending expected state. Presets and Mixes 2–4 are not writable.

Category `0x0a` replies carry no transaction ID. A pending write accepts only an exact complete-state match; delayed mismatches do not replace authoritative or pending state. Timeout, disconnect, or failed I/O makes the session stale and fail-closed. Authority is recovered only by constructing a fresh controller/device session and receiving a valid actual readback.

The Zen Go canonical profile contains no AuraVerb command contract, and no AuraVerb/reverb-named capture exists under `/home/ryodeushii/repos/zen-go-tui/antelope_pcap/zen go sc/`. Its manual documents product availability, but that does not establish Orion wire compatibility, so Zen remains absent from this backend capability.

## TUI boundary

The capability-gated `F2` page exposes only Mix 1 Enabled plus the eight fields above. It labels every parameter as raw `0..100`; the evidence does not establish engineering-unit conversions for the complete set. The UI contains no mix selector, preset manager, or new send-routing controls. It renders authoritative or latest complete pending state, never substitutes zero for missing state, and consumes only the typed stale/freshness rejection at an interaction race; encoding range and transport errors continue to propagate.
