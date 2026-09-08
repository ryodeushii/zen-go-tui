# Orion Surround speaker EQ readback

The runtime's read-only contract is category `0x1a`, indices `0..15`, with complete 320-byte responses. Bytes `0..15` are the exact readback header (index at byte 12), bytes `16..131` are the 116 meaningful record bytes, and bytes `132..319` must be zero.

The record is **four opaque candidate-head bytes followed by sixteen 7-byte EQ bands**. This corrects the older claim that the speaker head was absent. The four bytes align positionally with the speaker OUT frame's delay and packed level/invert, but all four init captures contain only default values there. Their geometry is proven; dynamic refresh and engineering meaning are not. The runtime therefore omits them. Todo 45's isolated recapture is the gate for any future whole-record read-modify-write.

EQ begins at response byte 20, not 16. Each band is frequency LE `u16` in Hz, Q LE `u16 / 100`, gain LE signed `i16 / 100` dB, and one raw mode byte. Endpoint shelf/pass names are not proven, so the TUI displays `RAW 0xNN`. Unknown mode bytes remain readable and never grant write authority.

Only captured identities are exposed: index 0 `L`, index 1 `R`, and index 2 `LFE`. Fresh known 2.0 owns `L/R`; fresh known 2.1 owns `L/R/LFE`. No labels for indices 3–15 or higher-format table are inferred. There is no opcode-`0x87` runtime action or capability in this implementation.

Fixture provenance: `macos-antelopeINIT-poweron.pcapng` SHA-256 `195fea4caefea43d3de5141b9457c50eaf72fe093899e47f8d46c645c7e40d80`, frames 17676/17686/17696. Full-report fixture hashes are recorded and mechanically checked in `tools/test_generate_device_catalog.py`.
