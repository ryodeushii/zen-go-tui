# Orion Normal Support Promotion

- **Date:** 2026-09-02
- **Status:** Architecture approved. Implementation complete; hardware verification pending.
- **Scope:** Orion Studio III (`0x23e5:0xa221`)

> **Superseded meter premise (2026-09-06):** The historical offset-157 physical-preamp meter claims and the state-report meter requirement recorded below were based on evidence later retracted. Current runtime behavior intentionally treats Orion physical-preamp meters as unavailable and does not infer standalone meter offsets; see [the profile-driver contract](../../protocol/profile-driver.md) and [current device-support limits](../../device-support.md). This note supersedes only those meter claims and preserves the approved design history.

## Context

`zen-go-tui` now has a generated profile catalog, a generic `ProfileDriver`, profile-aware HID discovery, and a device picker.

Orion profile data contains command, state, meter, readback, mixer, routing, and confirmed mixer-link mappings.

Its canonical `meter_report` is superseded for per-channel values. Real per-channel meters are
confirmed in `state_report` at offset 157. The current generic driver has no state-report meter
source, so it would otherwise decode known-invalid `meter_report` bytes 33 and later as channels.

Orion remains disabled for three policy reasons:

1. The canonical profile does not confirm HID report-ID framing.
2. The current generic driver has no profile-declared state-report meter source.
3. Raw evidence describes an ambiguous physical and ADAT link space.

The normalized runtime profile exposes no physical or ADAT link action.

It exposes one confirmed mixer link domain at protocol space 3 with 16 pairs.

The user selected normal support before hardware verification.

Runtime support must use an explicit Orion-only policy.

It must not silently change canonical evidence. It must not relax validation for other profiles.

## Decision

Promote Orion to ordinary runtime support through the existing generic profile path.

The generator will apply one explicit Orion runtime policy:

- Match profile id `orion_studio_3` and VID/PID `0x23e5:0xa221`.
- Set effective runtime transport framing to `uses_numbered_reports: false`.
- Keep the canonical raw transport field absent.
- Record the framing assumption in generated support metadata and hazards.
- Promote only source-backed generic command, state, and readback mappings to effective runtime
  status. Leave unsupported `auraverb_command` evidence non-actionable.
- Add a generated `state_report` `physical_meter` indexed operation at base 157, stride 1, and
  the confirmed physical-channel range.
- Treat Orion as `RuntimeReadiness::Supported` only when all other readiness checks pass, including
  the state-report meter mapping.
- Set `RuntimeDriverKind::Profile` for supported Orion.
- Accept either a confirmed `meter_report` source or a confirmed state-report meter source in the
  generic driver. Keep superseded `meter_report` per-channel data non-actionable.
- Populate input meters from state snapshots when the state-report source is selected.
- Accept only the confirmed mixer link domain at protocol space 3.
- Reject any future physical or ADAT link action mapping.
- Preserve identity status `unknown` and all raw evidence text.

The Rust generic driver keeps fail-closed validation and gains one profile-driven meter-source
capability. Profiles with a confirmed `meter_report` keep their current response-meter path.
Profiles with a confirmed state-report `physical_meter` mapping populate meters in `Snapshot`
events and ignore non-readback superseded meter responses.

No descriptor probe, report-framing fallback, blind probe, or dedicated Orion driver will be added.

`RuntimeReadiness::Supported` means that the normal control path is available.

It does not claim hardware validation. Generated support metadata will state that hardware verification remains pending.

## Data flow and ownership

```text
Antelope-Ctl raw profile
        │
        ▼
Python catalog generator
  Orion runtime policy
        │
        ├── generated Rust catalog
        └── normalized test fixture
                │
                ▼
ProfileCatalog and candidate classification
                │
                ▼
ProfileDriver + HidTransport
                │
                ▼
Controller, device picker, and dynamic UI
```

### Generator

The generator owns raw-to-runtime normalization and the Orion policy.

The policy must operate on the normalized runtime record before readiness classification.

This makes the effective framing value explicit without rewriting the canonical source.

The Orion readiness check must use capability data, not a raw note string, for link safety.

It must pass only when the runtime record has no physical or ADAT link action.

It must also have the confirmed mixer link domain at space 3. It must declare 16 pairs.

The policy must not change readiness or framing for any other profile.

### Profile catalog

Generated Orion data must contain:

- `readiness: supported`
- `driver_kind: profile`
- `transport.uses_numbered_reports: false`
- the existing 320-byte report geometry
- the existing confirmed frame, parameter, readback, routing, and mixer mappings
- support metadata that identifies the framing assumption and pending hardware verification

The canonical file `modules/Antelope-Ctl/profiles/orion_studio_3.json` remains unchanged.

### Runtime selection

The existing picker and session flow will consume Orion as a selectable profile.

`driver_for_entry` will select `ProfileDriver` through the existing `RuntimeDriverKind::Profile` branch.

No Orion-specific branch belongs in session startup.

### Codec

`ProfileDriver::new` remains fail closed. It will validate the promoted generated entry before HID open.

The existing Orion command, state, mixer, routing, and readback operation shapes match the generic
codec. The generic driver will accept one of two confirmed meter sources:

- A `meter_report` frame with the existing `physical_meter` indexed layout and confirmed decoder.
- A `state_report` frame with a `physical_meter` indexed layout covering `physical_inputs`.

State-report meter sources are applied to `DynamicInputState.meter` while decoding a `Snapshot`.
A profile without either source fails validation. A profile with only a state source does not decode
superseded meter responses as per-channel meters. Existing profiles with confirmed meter reports
retain their current behavior.

The implementation will not duplicate Orion mappings in a device-specific driver.

### Transport

The effective `false` framing value selects the current non-numbered HID path:

- Logical 320-byte output reports receive the existing leading zero report byte.
- Input reports accept the existing logical or zero-prefixed forms.
- The transport performs no alternate framing attempt.

Hardware verification must confirm this assumption later.

## Failure handling

- If generated Orion mappings fail `ProfileDriver::new`, the session fails before HID open.
- If report sizes, fixed bytes, offsets, category bounds, or startup query bounds are invalid, the driver returns an error.
- The driver sends no command when those mappings are invalid.
- If the device uses different report framing, transport validation or report I/O returns a device error.
- The runtime loop marks the session disconnected and returns to discovery.
- If an action targets an unsupported physical or ADAT link, the driver returns `UnsupportedAction`.
- The controller must not enqueue or write that command.
- A superseded non-readback meter response is ignored when no confirmed meter-response source is
  available. It is never decoded as per-channel input data.
- Unknown response magic follows current decoder behavior and does not issue a write.
- Orion remains unavailable when candidate identity, interface, usage, path, or transport checks fail.

## Alternatives rejected

### Dedicated Orion driver

Rejected. Current normalized operation shapes already fit `ProfileDriver`.

A second codec would duplicate mappings and create divergent validation.

### Experimental opt-in flag

Rejected. The user selected normal support. Orion will appear in the ordinary picker when its generated entry is supported.

### Generic validation relaxation

Rejected as a broad change. Unknown framing and incomplete evidence must remain blocked for every
non-Orion profile. The generic driver gets only the narrow meter-source alternative described above.

`ProfileDriver::new` keeps fail-closed validation and requires one confirmed meter source.

### HID descriptor probe or framing fallback

Rejected. Local HID API support does not establish descriptor semantics on every backend.

Automatic probing could send reports with the wrong shape. Hardware verification will validate the explicit policy later.

### Physical and ADAT link enablement

Rejected. Raw evidence cannot distinguish those links. The runtime will expose only the confirmed mixer link domain.

## Migration steps

1. Fast-forward the root branch to profile-support commit `04a79e2`.
2. Add the Orion-only generator policy.
3. Regenerate the Rust catalog and normalized fixture.
4. Update generator, catalog, generic meter-driver, picker, and transport tests as needed.
5. Update device-support documentation with the normal-support status and hardware-verification note.
6. Run formatting, generator drift checks, workspace tests, LSP diagnostics, `lens_diagnostics` with `mode=all`, and whitespace checks.
7. Review the final diff and confirm that the canonical raw profile is unchanged.
8. Commit the promotion on `feature/multi-device-antelope`.
9. Push the branch to its configured origin.

## Verification strategy

### Generator checks

- Generate Orion from the canonical raw profile.
- Assert `Supported`, `Profile`, and `uses_numbered_reports == false`.
- Assert the generated support reason names the framing assumption and pending hardware verification.
- Assert only mixer link space 3 is actionable.
- Assert physical and ADAT link mappings are absent.
- Assert non-Orion readiness and driver mappings remain unchanged.
- Assert generated output is deterministic and has no uncommitted drift after regeneration.

### Rust checks

- Load the regenerated Orion fixture through profile validation.
- Construct `ProfileDriver` from the Orion entry.
- Exercise confirmed input, output, global, mixer, mixer-link, routing, query, state, meter, and readback paths.
- Assert Orion state snapshots decode per-channel meters from state-report offset 157.
- Assert superseded meter responses are not decoded as per-channel values.
- Assert profiles with confirmed meter-report sources retain their existing meter path.
- Assert unsupported physical and ADAT link actions return `UnsupportedAction` without a transport write.
- Assert transport tests cover the selected non-numbered framing.
- Preserve existing Zen Go and picker tests.

### Repository gates

Run the repository-prescribed generator tests and drift check, then run:

```text
cargo fmt --all -- --check
cargo test --workspace
git diff --check
```

Run LSP diagnostics on edited source files and `lens_diagnostics` with `mode=all` before completion.

These checks do not replace hardware verification.

The implementation must not claim hardware validation until a later physical test confirms report framing and readback behavior.

The test must also confirm control writes and reconnect behavior.

## Acceptance criteria

- Orion is selectable through the ordinary device picker.
- Orion uses `ProfileDriver` without a device-specific codec.
- The generic driver accepts Orion’s confirmed state-report meter source and populates input meters
  from offset 157.
- Superseded `meter_report` bytes 33 and later are never treated as per-channel values.
- Profiles with normal confirmed meter reports retain their existing decode path.
- The generated Orion entry uses explicit non-numbered framing.
- The canonical raw Orion profile remains unchanged.
- Only confirmed mixer linking is actionable.
- Unknown physical and ADAT link semantics cannot produce writes.
- Other profiles retain existing fail-closed readiness and driver checks.
- Generator, Rust, formatting, diagnostics, drift, and whitespace checks pass.
- The promotion is committed and pushed after verification.
