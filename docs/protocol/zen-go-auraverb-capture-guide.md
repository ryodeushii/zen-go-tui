# Zen Go AuraVerb and settings capture guide

Status: Todo 52 awaits Zen protocol verification. Nothing in this guide authorizes an implementation or protocol write.

The Zen Go manual documents AuraVerb, Monitor Out Trim, and display brightness.
The current Zen profile does not define wire contracts for these controls.
The Orion AuraVerb layout does not establish a Zen layout.

## Evidence checked

- `docs/manuals/Zen-Go-SC-User-Manual.pdf`, SHA-256 `4834fe2e4cb407591fb82750b6b5ad4694e23d854f9057a6afcedb0753f4e64b`
- `docs/manuals/Antelope-Audio-Synergy-Core-FX-User-Manual.pdf`, SHA-256 `b61b751c95722cd353f57fcd1b0bbe6d28bb5dacd6c0369622d41ea626ff7219`
- `modules/Antelope-Ctl/profiles/zen_go_sc.json` at nested revision `d5f9bb9`, SHA-256 `1aca13b208ff8d9af8334e88acfe705235f974f08ffa3925b5cd17f8a543481b`
- [Capture-family inventory](capture-family-inventory.md) for all 127 current Zen captures

The manual establishes visible product features, not HID bytes.
The profile and current capture inventory provide no validated contract for these writes.

Use only the official control panel for control changes. Capture the selected device's full bidirectional USB traffic.
Do not apply a report-magic display filter during capture.

## Record the environment

Record these items before each capture set:

- device model and serial
- device firmware version
- official control-panel version
- operating system and version
- USB or Thunderbolt connection type
- selected device and interface
- initial value of every visible tested-feature control
- all visible preset controls, or an explicit statement that presets are out of scope
- capture start time and local time zone

Use capture names that identify the device, feature, and action order.
Keep a timestamped action log. Take a screenshot at each named value.

## Inventory visible controls

Record every visible AuraVerb, brightness, and output-trim control before capture actions.
Record each panel label, control type, initial value, displayed range, and finite choices.
Record whether preset controls exist. If presets are not tested, mark them as visible and out of scope.
Do not guess a hidden control, range, choice, target, or preset behavior.

## Capture startup and readback

1. Set a nondefault AuraVerb state with the official panel.
2. Record every visible AuraVerb value.
3. Close the official panel without resetting the device.
4. Start the full bidirectional USB capture.
5. Record traffic before you open the official panel.
6. Open the official panel.
7. Wait until all values finish loading.
8. Record the loaded values and timestamps.
9. Close and reopen the panel once more without a reset.
10. Record the second loaded state and timestamps.

This sequence separates startup traffic from nondefault readback.
Do not treat an outbound command as readback.

## Capture AuraVerb controls

Create separate captures when practical. Pause for 3 to 5 seconds after each action.
Restore each control before you test the next control.

1. Set AuraVerb power to off.
2. Set AuraVerb power to on.
3. Set AuraVerb power to off again.
4. Restore the initial power state.
5. Select one visible AuraVerb parameter.
6. Record its actual displayed minimum, midpoint, and maximum.
7. Set the parameter to its displayed minimum.
8. Set the parameter to its displayed midpoint.
9. Set the parameter to its displayed maximum.
10. Restore the parameter to its initial value.
11. Repeat steps 5 through 10 for every visible parameter.

Include these manual-named parameters when the panel exposes them:

- Color
- PreDelay
- Early Reflection Gain
- Late Reflection Delay
- Richness
- Reverb Time
- Room Size
- Reverb Level

Record all other visible AuraVerb controls. Record whether the panel exposes presets.
If presets are not tested, mark them as visible and out of scope.
Do not substitute an assumed `0..100` range. Record the exact panel display for each point.
Do not guess a range for a control that the panel does not expose.
Do not assume that reserved bytes, targets, or parameter order match Orion.

## Capture sends and returns

Capture each exposed send separately from each exposed return.
Record the panel label, mixer, strip, destination, initial value, and displayed range.

1. Select one exposed send.
2. Record its displayed minimum, midpoint, and maximum.
3. Apply those three values with 3 to 5 second pauses.
4. Restore the initial send value.
5. Repeat for each exposed send.
6. Select one exposed return.
7. Record its displayed minimum, midpoint, and maximum.
8. Apply those three values with 3 to 5 second pauses.
9. Restore the initial return value.
10. Repeat for each exposed return.

Do not infer a hidden send or return from the Orion layout.
Do not combine send, return, and AuraVerb parameter sweeps in one unlabeled interval.

## Capture brightness

Use separate captures only if the official panel exposes brightness for this connection type.
Do not invent a range from another connection type or device.

### Brightness nondefault readback

1. Set brightness to a visible nondefault value.
2. Record the exact displayed value.
3. Close the official panel without resetting the device.
4. Start a new full bidirectional capture.
5. Record traffic before you open the panel.
6. Open the official panel.
7. Wait until brightness finishes loading.
8. Record the loaded value and timestamp.
9. Close the official panel without resetting the device.
10. Reopen the official panel.
11. Record the second loaded value and timestamp.

This capture tests brightness readback. Do not count an outbound selector write as readback.

### Brightness sweep

1. Start a separate full bidirectional capture.
2. Record the initial displayed brightness.
3. Record the displayed range or finite choices.
4. Set the displayed minimum.
5. Set the displayed midpoint.
6. Set the displayed maximum.
7. Restore the initial brightness.

Pause for 3 to 5 seconds after each action. Record each timestamp and screenshot.

## Capture output trim

Use separate captures only if the official panel exposes output trim.
Physically power down, disconnect, or externally mute the monitor chain before you start each trim capture.
Do not use a control-panel mute for this safety step. A panel mute write can contaminate the trim trace.
Do not play a loud or full-scale signal.

### Output-trim nondefault readback

1. Set one exposed output trim to a visible nondefault value.
2. Record the exact output label and displayed value.
3. Close the official panel without resetting the device.
4. Start a new full bidirectional capture.
5. Record traffic before you open the panel.
6. Open the official panel.
7. Wait until output trim finishes loading.
8. Record the loaded value and timestamp.
9. Close the official panel without resetting the device.
10. Reopen the official panel.
11. Record the second loaded value and timestamp.
12. Repeat in a separate capture for each independently exposed trim control.

This capture tests output-trim readback. Do not count an outbound selector write as readback.

### Output-trim sweep

1. Start a separate full bidirectional capture.
2. Record the exact output label and initial trim value.
3. Record every displayed trim choice or range endpoint.
4. Set the minimum trim value.
5. Set a midpoint trim value.
6. Set the maximum trim value.
7. Restore the initial trim value.
8. Repeat in a separate capture for each independently exposed trim control.

Pause for 3 to 5 seconds after each action. Record each timestamp and screenshot.
Trim is not volume. Do not use the volume control as a trim substitute.
Do not infer per-output targets when the panel exposes only one Monitor Out Trim control.
Do not change a panel mute during a trim capture.

## Deliverables and acceptance

Deliver the raw capture, action log, screenshots, and metadata together.
Hash each file with SHA-256. Keep capture paths relative to the capture-set root.

A protocol mapping remains pending until analysis identifies all of these facts:

- actual endpoint and direction
- report family and full captured length
- request and readback separation
- exact changed bytes for one control at a time
- target identity and finite value domain
- startup or reopen readback behavior
- stable restoration to the initial value

Do not add Zen AuraVerb, brightness, or output-trim controls before that review.
