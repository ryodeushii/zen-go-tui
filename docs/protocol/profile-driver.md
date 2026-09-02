# Profile-driven device protocol

`antelope_protocol::ProfileDriver` encodes and decodes devices whose normalized catalog entry has `readiness: supported` and `driver_kind: profile`. Profiles remain data; runtime protocol decisions are not selected by device name.

## Required confirmed mappings

A supported profile must provide confirmed, report-sized frames named:

- `command`
- `global_command`
- `mix_command`
- `link_command`
- `routing_command`
- `state_report`
- `meter_report`
- `readback`

It must also provide one confirmed decoder for each report family and a confirmed readback definition. Constructor validation rejects absent, duplicate, ambiguous, unconfirmed, out-of-report, or `uncompiled_formula` mappings before I/O.

Scalar and bit-field operations require stable semantic `field` names. One-byte scalars use `not_applicable` endianness. Wider scalars require explicit `little` or `big`; source data without proven wider-scalar endianness is emitted as `uncompiled_formula` and cannot enable a profile driver.

Parameter `frame.offsets` refer to scalar semantic names and must equal those operations' declared offsets. The driver does not infer parameters from names, offsets, or frame order.

## Atomic operations

Mixer strip commands are compound frames. Callers must use `SetMixerStripState` with complete fader, pan, mute, solo, and—when the profile declares it—send state. Partial `SetMixer` actions are rejected and are not queue-coalesced. Profiles without a send operation accept `send: None` and reject `Some`.

Routing commands are complete ordered assignment tables. Callers must use `SetRoutingGroup`; source count, destination range, source banks, and source indexes are checked before any bytes are returned. Routing groups are not queue-coalesced.

## Adding a supported profile

1. Record exact HID report geometry and confirmed command, state, meter, and readback frames in the source profile.
2. Give every scalar/bit operation a unique semantic field and prove wider-scalar endianness.
3. Define confirmed parameters, finite value/range domains, readback category counts, mixer topology, routing counts, allowed source banks, and maximum source index.
4. Add full-frame encoding fixtures and complete readback fixtures. Cover malformed lengths, category/index bounds, truncated records, and unknown categories.
5. Register entry as `driver_kind: profile` only after mappings are complete. Set `readiness: supported` only after constructor validation and fixture tests pass.
6. Regenerate both catalog artifacts and run generator drift checks, workspace tests, and clippy.

## Orion Studio III blocker

Orion Studio III remains disabled. Representative profile fixtures validate generic mechanics only; they do not constitute hardware confirmation. Promotion requires captured, verified command/readback mappings for all required frame families, complete mixer and routing domains, proven multi-byte endianness, and hardware round-trip tests.
