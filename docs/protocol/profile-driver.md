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
- `readback`

Metering is optional. A supported profile may omit both meter sources; the constructor then keeps metering unavailable and does not emit meter events. If a meter mapping is declared, it must be a confirmed, unambiguous, finite, in-report mapping: a confirmed `meter_report` also requires exactly one confirmed decoder, while a state-report `physical_meter` mapping must cover its finite input space. Constructor validation rejects malformed, duplicate, ambiguous, unconfirmed, out-of-report, or `uncompiled_formula` mappings before I/O.

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

## Orion Studio III status and limits

Orion Studio III is `Supported` with `RuntimeDriverKind::Profile`. Its normalized profile uses a source-backed non-numbered framing assumption (`transport.uses_numbered_reports: false`); descriptor and hardware validation of that assumption remains pending. Confirmed controls are enabled independently of metering. Physical preamp meters remain unavailable: the upstream state-report mapping at offset 157 was retracted because those bytes are mix-master data, and no physical meter decoder is enabled. Superseded `meter_report` bytes 33 and later are also not interpreted as channel meters; the UI renders those physical meter values as unknown (`?`).

Only mixer link space 3 is described as actionable in the profile. Physical and ADAT links remain non-actionable, while confirmed output, input, mixer, and routing controls use the generic profile driver.

Profile-driver fixtures validate codec, bounds, and framing mechanics only. They do not constitute physical Orion validation.
