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

Zen Go and Orion Studio III are currently selectable for control. Unit and integration tests preserve exact protocol bytes for Zen Go; Orion uses the generic profile driver and source-backed profile mappings. Supported profiles still pass the same selection gate and safety limits described above.

## Orion runtime support and limits

The normalized Orion profile preserves this profile-derived topology:

- 12 physical inputs, 16 ADAT inputs, and 2 S/PDIF inputs;
- 6 output groups;
- 4 mixer surfaces with 32 strips and a master strip each;
- finite routing groups and source domains;
- one confirmed S/PDIF input link domain in protocol space 1 with the single L/R pair;
- one confirmed mixer link domain in protocol space 3 with 16 pairs;
- an exact 113-request startup query order with finite readback bounds.

Orion is `Supported` with `RuntimeDriverKind::Profile`. Its validated Surround global contract adds a capability-gated `F3` page for authoritative 2.0 level (`0..760`, displayed as `-60.0..+16.0 dB`) and lip-sync delay (`6..45`, displayed as `0.6..4.5 ms`). Captured 2.1 state is read-only. Unknown, stale, disconnected, or missing state disables writes, and a timed-out pending write remains session-locked until a fresh controller/device session. The page intentionally omits speakers, EQ, bass management, meters, masks, and format controls. Zen Go has no validated Surround contract, so it has no Surround tab or `F3` page.

The generator applies an Orion-only, non-numbered framing assumption (`transport.uses_numbered_reports: false`). Descriptor and hardware validation of that assumption remains pending. Confirmed profile controls are enabled independently of metering. Physical preamp meters use the confirmed full-report `0x73` bank at offsets 221–232, with count 12, stride 1, and width 1. The profile retains six provisional one-lane output feeds. A user-approved packet-order hypothesis maps `0x73` @157/@158/@159/@160/@177/@178 to output ids 0/1/2/3/4/5. This hypothesis does not establish output ownership, the meter stage, physical post-fader ownership, or L/R geometry. The split of playback-coupled @177/@178 across Reamp and Monitor B remains an ordering assumption. Free-running `0x75` requires byte 1 `0x1f`, while byte 1 `0x00` identifies readback responses. The `0x75` @32/@48 and @33/@49 pairs are route-correlated lanes without fixed ownership. Their meter stage and calibration remain unresolved. See [the bounded Orion meter evidence](protocol/orion-meter-evidence.md).

Output mono is a typed, profile-owned control using `SET_PARAM 0x69` and state-report bus status bit `0x10`. It is exposed only for the four capture-backed buses: Monitor A (`0`), HP1 (`1`), HP2 (`2`), and Monitor B (`5`). Line (`3`) and Reamp (`4`) remain unavailable rather than inheriting the six-slot bus mask; Zen Go declares no output-mono control.

Orion's compact `SET` selector exposes bounded, readback-authoritative settings. Screen brightness is typed global `SET_GLOBAL 0x0e`, range 0–100, with direct state readback at report offset 26. Output trim is a dedicated indexed `SET_PARAM 0x4b` control for exactly Monitor A, Monitor B, and Line Out (targets 0–2), with seven confirmed 20–14 dBu choices and independent packed readback fields at offsets 24–25. Capability requires exact structured `state_report` semantic references and matching compiled frame operations; explanatory readback prose is provenance only. The UI shows `?` until a confirmed snapshot arrives and does not promote requested values after a failed write. Zen Go has no declaration for these settings, so the selector remains hidden there.

Bounded talkback uses that exact `SET_GLOBAL` frame: button `0x1f` is hold-to-talk (`1` press, `0` release), source `0x27` accepts only INT plus Preamps 1–12 (indices 0–12), and active-source gain `0x20` accepts raw 0–96. Button press/release are distinct, non-coalesced writes; the TUI makes a best-effort release on key-up, modal exit, focus loss, and quit, but a physical disconnect can prevent delivery. The explicit Release row does not depend on observed button state. State offset 73 bit `0x40` reports button activity, while source readback is displayed only as its low-two-bit residue modulo 4—never as one of the 13 selected sources—because coexistence with destination assignment is unverified. Offset 74 is labeled active-source gain with unknown source ownership. Destination bits, names, and toggles remain outside this slice.

Only the confirmed S/PDIF link domain at protocol space 1 pair 0 and mixer link domain at protocol space 3 are exposed. Physical and ADAT link semantics remain non-actionable because they share ambiguous protocol space 0, and Zen Go declares no confirmed input-link domain.

Orion provides no device-side S/PDIF link readback. The compact input bank therefore marks its title with `?` and offers separate idempotent `ON` / `OFF` link requests rather than an inferred toggle or confirmed-state indicator. S/PDIF gain writes are not software-mirrored from those requests; each gain edit preserves the existing single-channel behavior.

The root runtime and bundled standalone CLI expose the confirmed Orion physical-input meter bank at full-report `0x73` offsets 221–232. They preserve raw inverted activity without claiming an exact dB calibration or clip state. Output-card meters remain visibly provisional profile assignments and do not establish the physical post-fader stage.

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
