# Cross-device profile mapping: Orion Studio SC and Zen Go SC

## Scope

This report compares how Antelope-Ctl describes readback queries, mixer state, preamps, and related runtime metadata for Orion Studio SC and Zen Go SC. It also identifies which names can be shared and which values must remain device-specific.

Primary sources:

- `modules/Antelope-Ctl/profiles/orion_studio_sc.json`
- `modules/Antelope-Ctl/profiles/zen_go_sc.json`
- `modules/Antelope-Ctl/docs/profile-schema.md`
- `modules/Antelope-Ctl/antelope/protocol.py`
- `tools/generate_device_catalog.py`
- `antelope-protocol/src/profile_driver.rs`

## Conclusion

The two profiles share substantial semantic vocabulary. They do not share one wire layout.

Shared concepts should use common names: input mode, gain, phantom power, phase inversion, mixer fader, pan, mute, routing, readback categories, and state-report offsets. Device profiles should supply different report magics, offsets, counts, ranges, and record layouts.

Orion currently encodes much of this information through established scalar fields, bounded category counts, and an implicit canonical mixer layout. Zen Go needs explicit sparse query and mixer-layout records because its observed startup walk and mixer records do not match Orion's assumptions.

## Mapping summary

| Concept | Orion Studio SC | Zen Go SC | Recommended common model |
|---|---|---|---|
| Readback transport | `0x74` request, `0x75` response, category/index addressing | Same transport and addressing | Shared readback header fields |
| Safe startup queries | Derived from confirmed `category_counts` | Explicit `safe_queries` from observed traffic | Permit bounded counts and explicit sparse pairs |
| Mixer readback | Category `0x04`; four records; implicit 33-slot, three-byte layout | Explicit `layouts`; two-byte strip records and device-specific categories | Shared typed layout records with device-specific values |
| Preamp commands | Parameters `0x4f` through `0x52` | Same parameter IDs | Shared parameter names and control kinds |
| Preamp state | Gain/status arrays at Orion-specific offsets | Gain/status arrays at Zen-specific offsets | Shared field names with per-device offsets |
| Preamp status bits | Mode `0x03`, phantom `0x10`, phase `0x40` | Same masks | Shared semantic bit names |
| Gain ranges | Mic `0..75`, line `-6..20`, Hi-Z `0..65` | Mic `0..65`, line ungrounded, Hi-Z `0..45` | Per-device and per-mode ranges |
| Mixer names | Derived as `Mix N` | Explicit display names | Optional display metadata |
| Meter source | Orion state report and `0x75` meter family | Zen `0x83` meter report plus observed state-report candidates | Shared meter semantics, device-specific source |

## Readback queries

### Orion

Orion stores a generic readback request/response definition and confirmed finite `category_counts`. Category `0x04`, for example, has four records. The profile warns that an out-of-range index can crash the device. The counts therefore serve both as enumeration data and safety bounds.

The parent generator expands each confirmed category count into ordered `(category, index)` pairs when no explicit query list exists. This behavior is in `tools/generate_device_catalog.py::_readback_definition` near line 3959.

Relevant sources:

- Readback header and safety notes: `modules/Antelope-Ctl/profiles/orion_studio_sc.json` near lines 440-477
- Category inventory: the same file near lines 465-530
- Documented schema: `modules/Antelope-Ctl/docs/profile-schema.md` near lines 120-130

### Zen Go

Zen Go uses the same readback transport, but its captured Launcher startup walk does not provide complete contiguous category bounds. Its `category_counts` object is empty. Sweeping inferred indices would therefore be unsafe.

The profile supplies 47 observed `safe_queries` across 13 categories. These are an explicit sparse allow-list, not a claim that every index below a maximum is valid. Duplicate pairs preserve the observed startup order.

Relevant source:

- `modules/Antelope-Ctl/profiles/zen_go_sc.json` near lines 153-235

### Recommended schema rule

Use one shared readback model with two safety representations:

1. `category_counts` for capture-confirmed contiguous ranges.
2. `safe_queries` for capture-confirmed sparse or ordered pairs when contiguous bounds are unknown.

An explicit list must remain authoritative when both forms exist. This is already how the parent generator behaves. Antelope-Ctl's schema documentation does not yet describe `safe_queries`.

## Mixer layouts

### Orion

Orion's category `0x04` prose describes four mixer records. Each record contains 33 three-byte slots:

- Slot 0: mixer master.
- Slots 1-32: input strips.
- Bytes per slot: fader, packed pan/mute/solo, send.

The Rust profile driver has a compatibility fallback for this canonical layout: base offset 16, stride 3, width 3, maximum slot 32. See `antelope-protocol/src/profile_driver.rs` near lines 1213-1250 and `decode_mixer` near lines 1604-1687.

Orion therefore works without a profile `layouts` array. The executable layout partly lives in decoder defaults rather than entirely in profile data.

### Zen Go

Zen Go's observed mixer bodies are not Orion-shaped. Its capture-confirmed category `0x04` records describe 16 two-byte strips, and category `0x18` has a separate observed record shape. The explicit `frame.readback.layouts` entries carry body size, record count, stride, field offsets, surface mapping, supported fields, and evidence status.

Relevant source:

- `modules/Antelope-Ctl/profiles/zen_go_sc.json` near lines 236-304

### Recommended schema rule

`frame.readback.layouts` is a reasonable cross-device abstraction, but it should be documented and generalized before it becomes shared upstream vocabulary. Orion could then declare its four canonical category `0x04` layouts explicitly and the decoder fallback could eventually be removed.

Until that happens, Zen's layout records are an additive extension used by the parent Rust normalizer. Antelope-Ctl's Python code does not consume them.

## Preamps

Preamps show the strongest cross-device naming.

Both profiles use these parameter IDs:

| Semantic control | Parameter |
|---|---:|
| Input mode | `0x4f` |
| Gain | `0x50` |
| Phantom power | `0x51` |
| Phase inversion | `0x52` |

Both state reports use the same status-bit meanings:

| State | Mask |
|---|---:|
| Input mode | `0x03` |
| Phantom power | `0x10` |
| Phase inversion | `0x40` |

The layout values differ:

- Orion: gain base 49, status base 61, physical input meter base 157.
- Zen Go: gain base 40, status base 42, observed preamp-meter candidates at `0xce` and `0xcf`.

Gain ranges also differ by hardware and evidence quality. Zen's line-mode range remains intentionally ungrounded and must not inherit Orion's `-6..20` range.

Relevant sources:

- Orion state report: `modules/Antelope-Ctl/profiles/orion_studio_sc.json` near lines 241-310
- Zen state report: `modules/Antelope-Ctl/profiles/zen_go_sc.json` near lines 115-152
- Zen parameters: the same file near lines 319-380

The parent generator converts these common names into canonical `Gain`, `Mode`, `Phantom`, and `Phase` capabilities. See `_build_address_spaces` near line 1275 and `_build_input_capabilities` near line 1467 in `tools/generate_device_catalog.py`.

## Mixer names and value semantics

Orion has no top-level `mixer` object. Its geometry is inferred from readback counts, command evidence, and canonical defaults. The parent generator creates display names `Mix 1`, `Mix 2`, and so on when no explicit names exist.

Zen's `mixer.names` only overrides those display labels. It is not wire-protocol data. If upstream wants profiles to stay protocol-focused, this field can remain in a parent-side UI overlay.

Fader `direction` and `unity` describe interpretation, not layout. They prevent a raw attenuation value from being displayed as positive gain. These concepts are cross-device, but the schema should define them consistently for mixer faders and output levels rather than add one-off fields to a single profile.

Relevant source:

- `tools/generate_device_catalog.py::_build_mixers` near lines 2623-2678

## Compatibility

Antelope-Ctl loads profiles with plain `json.load` and does not reject unknown keys. See `modules/Antelope-Ctl/antelope/protocol.py` near line 23. The Zen extensions are therefore backward-compatible for current Python consumers.

Compatibility does not make them established schema. `frame.readback.safe_queries`, `frame.readback.layouts`, `frame.state_report.candidate_preamp_meters`, and `mixer.names` remain undocumented extensions and have no exact counterpart in other profiles.

## Recommendation

Use three layers:

1. **Shared semantic vocabulary** — stable names for controls, capabilities, state bits, query pairs, and mixer fields.
2. **Per-device evidence** — magics, offsets, masks, ranges, category bounds, layouts, and confidence status.
3. **Consumer/UI metadata** — display names and client-specific startup policy when upstream does not want those concerns in protocol profiles.

For the open Zen Go PR, either:

- document `safe_queries` and `layouts` as reusable profile-schema features and add an explicit Orion example, or
- move these arrays and display-only names into a zen-go-tui-owned overlay while leaving confirmed raw offsets and protocol facts upstream.

Do not force Zen into Orion's implicit mixer layout or infer contiguous query bounds from sparse captures. Those values are device-specific and safety-sensitive.
