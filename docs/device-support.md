# Device support and profile validation

This page defines device readiness, profile sources, selection safety, and validation evidence for `zen-go-tui`.

## Support matrix

| Device | Readiness | Runtime driver | Current result |
|---|---|---|---|
| Antelope Zen Go Synergy Core | `Supported` | `ZenGo` | Selectable for control |
| Antelope Orion Studio III | `Supported` | `Profile` | Selectable for control |
| Antelope Discrete 8 Pro Synergy Core | `Partial` | `None` | Visible but not selectable |
| Antelope Discrete 4 Synergy Core | `Unverified` | `None` | Visible but not selectable |
| Antelope Discrete 4 Pro Synergy Core | `Unverified` | `None` | Visible but not selectable |

Only `Supported` entries with a runtime driver can open a control session. The picker sorts these entries before diagnostic-only entries.

`Partial`, `Unverified`, `Disabled`, `Ambiguous`, and `Unsupported` entries cannot activate. Keyboard and mouse input use the same selection gate.

## Profile terms

A **canonical profile** is raw JSON evidence in the Antelope-Ctl repository. Local generation reads these files from:

```text
modules/Antelope-Ctl/profiles
```

The generator excludes `mic_models.json`. That file describes microphone models, not hardware control profiles.

A **normalized profile pack** is validated runtime JSON. The application includes `src/device/generated_profiles.json`. You can also supply a pack with `--profile-pack`.

A **saved-state profile** is a user TOML snapshot managed by the TUI. It stores device control state. It is not a canonical profile or normalized profile pack.

## Generation and drift checks

Generate the Rust catalog and normalized profile pack from the canonical profiles:

```bash
python3 tools/generate_device_catalog.py \
  --profiles-dir modules/Antelope-Ctl/profiles \
  --output src/device/generated.rs \
  --pack-output src/device/generated_profiles.json
```

Check both generated artifacts without changing them:

```bash
python3 tools/generate_device_catalog.py \
  --check modules/Antelope-Ctl/profiles \
  --generated src/device/generated.rs \
  --pack-generated src/device/generated_profiles.json
```

The check fails if either artifact differs from the canonical input. Repository tests also mutate a source profile and each artifact independently.

Each normalized entry records:

- a stable source path relative to the profile directory;
- the SHA-256 hash of the exact canonical source bytes;
- the generator version;
- readiness;
- runtime driver kind;
- a support reason.

These fields show provenance and runtime eligibility. They do not prove physical hardware validation.

## Catalog loading

Without `--profile-pack`, the application uses the checked-in built-in catalog.

With `--profile-pack`, the application loads and validates the external pack before HID discovery. External entries cannot shadow a built-in profile ID or VID/PID identity.

A selectable generic profile must pass all profile-driver safety checks. These checks include explicit report framing, finite operation domains, safe readback bounds, and a confirmed link domain.

## Device selection

Start the picker with the built-in catalog:

```bash
zen-go-tui
```

Select by a unique hexadecimal VID/PID:

```bash
zen-go-tui --device 23e5:a015
```

Select by a unique serial:

```bash
zen-go-tui --device ZEN-SERIAL
zen-go-tui --device serial:ZEN-SERIAL
```

Select by a HID path:

```bash
zen-go-tui --device path:/dev/hidraw4
```

Bare values that start with `hid`, `/`, or `\` are paths. Other bare text is a serial unless it has a hexadecimal `VID:PID` shape.

Use `serial:` when a serial looks like a path or hexadecimal identity. Use `path:` when a path needs explicit interpretation.

The parser resolves each selector to one criterion. Its syntax precedence is explicit prefix, hexadecimal `VID:PID`, path-shaped text, then serial text.

A serial or VID/PID must match one candidate. Multiple matches produce an ambiguity error that lists candidate paths. The application never selects the first ambiguous device.

## Exact-path and reconnect safety

Discovery reads HID metadata before the application opens a transport. The session validates readiness, driver kind, profile identity, report geometry, and report framing before HID creation.

The transport opens the selected raw HID path. It does not substitute another matching path.

Automatic reconnect requires all of these identity facts:

- a nonempty serial that matches the previous device;
- matching VID and PID;
- matching interface number;
- matching usage page and usage;
- a unique selectable candidate.

The application prefers the previous exact path. It still applies the full identity check if the operating system reuses that path. If a path changes, the same unique identity checks apply.

Ambiguous or incomplete identity returns the application to the picker. The old controller and transport worker stop before a replacement session opens.

## Supported Zen Go controls

The generated Zen Go topology defines:

- 2 physical inputs with gain, mode, phantom, and phase controls;
- 3 output buses;
- 2 mixer surfaces with 16 strips each;
- 16 mixer input-assignment destinations.

The built-in `ZenGo` driver also supports the tested global, output, preamp, mixer, routing, readback, raw-view, and metering behavior described in [Zen Go Synergy Core TUI](zen-go-tui.md).

Zen Go and Orion Studio III are currently selectable for control. Unit and integration tests preserve exact protocol bytes for Zen Go. Supported profiles still pass the same selection gate and safety limits described above.

## Orion runtime support and limits

The normalized Orion profile preserves this profile-derived topology:

- 12 physical inputs, 16 ADAT inputs, and 2 S/PDIF inputs;
- 6 output groups;
- 4 mixer surfaces with 32 strips and a master strip each;
- finite routing groups and source domains;
- one confirmed mixer link domain in protocol space 3 with 16 pairs;
- an exact 113-request startup query order with finite readback bounds.

Orion is `Supported` with `RuntimeDriverKind::Profile`. The generator applies an Orion-only, non-numbered framing assumption: `transport.uses_numbered_reports: false`. The canonical raw profile remains unchanged; generated support metadata records this assumption and states that hardware verification remains pending. No descriptor or hardware validation establishes this framing yet.

Orion per-channel input meters come from `state_report` at offset 157, one byte per physical channel. The superseded `meter_report` bytes 33 and later are not interpreted as per-channel meters. The generic driver uses the confirmed state-report source and ignores those superseded channel values.

Only the confirmed mixer link domain at protocol space 3 is exposed. Physical and ADAT link semantics remain non-actionable, so the UI exposes no controls for those links. The catalog does not add unconfirmed link actions.

Generic profile-driver tests cover representative writes, typed decoding, routing bounds, whole-state behavior, state-report meter decoding, superseded meter handling, and profile-derived fixtures. These tests are not physical Orion validation.

## Discrete profile status

Antelope Discrete 8 Pro Synergy Core is `Partial`. Its profile data is incomplete for safe read/write control.

Antelope Discrete 4 Synergy Core and Antelope Discrete 4 Pro Synergy Core are `Unverified`. Their transport or frame geometry is unverified.

All three entries use `RuntimeDriverKind::None`. The picker shows their support reasons and prevents activation.

## Evidence levels

**Captured evidence** comes from recorded USB traffic or descriptor data. A canonical profile must name the source and status for each claim.

**Profile-derived fixtures** are generated or written from canonical values. They test codecs and bounds without a connected device. They are not packet captures.

**Physical hardware validation** requires a separate run with the named device. Do not infer this validation from generated artifacts, mock tests, or profile-derived fixtures.

No physical multi-device validation was performed for this implementation.

## Hardware validation procedure

Use this procedure only with hardware you control. Record the application revision and canonical profile hashes before the run.

1. Check generated artifacts.

   ```bash
   python3 tools/generate_device_catalog.py \
     --check modules/Antelope-Ctl/profiles \
     --generated src/device/generated.rs \
     --pack-generated src/device/generated_profiles.json
   ```

2. Run non-hardware verification.

   ```bash
   cargo test --workspace
   python3 -m unittest discover -s tools -p 'test_*.py' -v
   ```

3. Connect one target device and start the picker.

   ```bash
   cargo run
   ```

4. Record product, serial, exact path, VID/PID, interface, usage page, usage, readiness, and diagnostic text.

5. For a supported device, open the exact candidate and verify startup state before changing a control.

6. Test one control from each advertised capability family. Record the requested value, encoded write, readback, and visible result.

7. Unplug the device. Confirm that the old worker stops and no stale write succeeds.

8. Reconnect the same serial. Confirm exact-path or unique changed-path recovery.

9. Present a duplicate or incomplete identity when possible. Confirm that the picker reports ambiguity instead of opening a device.

10. Save logs and failures with the application revision, profile hash, generator version, operating system, and device firmware.

A successful run must identify every tested control and reconnect condition. A failed or incomplete run must remain pending. Documentation can claim hardware validation only after this evidence exists and receives review.
