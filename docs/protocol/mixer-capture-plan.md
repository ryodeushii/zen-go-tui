# Mixer Capture Plan

This plan narrows the next Wireshark captures for the Zen Go mixer so we can decode mixer state without guessing.

## Current Working Model

- There are `2` mixer surfaces: `MIX 1` and `MIX 2`.
- There are `16` mixer strip slots per surface.
- Each strip can be assigned one source from this pool:
  - `Preamp 1`
  - `Preamp 2`
  - `Computer Play 1..8`
  - `Emu Mic 1..2`
  - `SPDIF In 1..2`
  - `Mute`
  - `Oscillator 1..2`
- Link toggles exist only on odd strips and control adjacent pairs:
  - `1-2`, `3-4`, `5-6`, `7-8`, `9-10`, `11-12`, `13-14`, `15-16`
- Per-strip controls in scope now:
  - source assignment
  - pan
  - level
  - mute
  - solo
- Per-strip DSP / AFX on channels `1..4` is intentionally out of scope for this pass.
- `BP ALL` is also deferred for now because it belongs to the AFX path.
- DAW I/O routing is out of scope.

## General Capture Rules

- One control family per capture file.
- Change only one thing at a time.
- Start from a known baseline and write it down before recording.
- Stay on one mixer surface unless the capture is specifically about surface differences.
- Leave `2-3` seconds idle before the first action.
- Leave `2-3` seconds idle between actions.
- Leave `2-3` seconds idle after the last action.
- Speak or write down every action with strip number, target value, and exact order.
- Do not combine assignment, link, pan, mute, solo, and fader moves in one capture unless the capture is specifically about propagation.

## Baseline Setup

Use this baseline before the family captures unless a capture says otherwise.

- Visible page: `Monitors & Headphones`
- Mixer surface: `MIX 1`
- Expanded mixer view: show all `16` strips if possible
- Default strip assignment:
  - `1 = Preamp 1`
  - `2 = Preamp 2`
  - `3..10 = Computer Play 1..8`
  - `11..16 = Mute` or unassigned/silent state
- All link pairs unlinked
- All strip mutes off
- All strip solos off
- Pan values:
  - `Preamp 1` and `Preamp 2` centered
  - stereo-style pairs alternating `-30`, `+30` where applicable
- Faders at visually distinct, written-down values
- No active AFX editing during capture

## Capture Families

### 1. Surface Idle Baseline

Goal:
- determine how `MIX 1` and `MIX 2` differ when no user writes happen
- verify whether passive churn differs by surface

Files:
- `capture_mixer_01_mix1_idle.pcapng`
- `capture_mixer_02_mix2_idle.pcapng`

Actions:
- open `MIX 1`, do nothing for `5-10` seconds
- open `MIX 2`, do nothing for `5-10` seconds

Write down:
- visible strip assignments
- visible link states
- visible pan values
- visible fader values

### 2. Assignment Enum Sweep

Goal:
- map source assignment commands and resulting stable device state
- isolate assignment state from link/pan/fader changes

Recommended strip:
- use strip `11` or another currently silent/unlinked strip outside `1..4`

Files:
- `capture_mixer_03_assignment_core.pcapng`
- `capture_mixer_04_assignment_extended.pcapng`

Capture 03 actions:
- assign strip to `Mute`
- assign strip to `Preamp 1`
- assign strip to `Preamp 2`
- assign strip to `Computer Play 1`
- assign strip to `Computer Play 2`
- assign strip back to `Mute`

Capture 04 actions:
- assign strip to `Computer Play 8`
- assign strip to `Emu Mic 1`
- assign strip to `Emu Mic 2`
- assign strip to `SPDIF In 1`
- assign strip to `SPDIF In 2`
- assign strip to `Oscillator 1`
- assign strip to `Oscillator 2`
- assign strip back to `Mute`

Notes:
- do not move the strip fader
- do not touch pan, mute, solo, or link in these captures

### 3. Link Pair State Only

Goal:
- isolate the stored link state and command family for adjacent pairs
- confirm that odd-slot link buttons represent pair state

Recommended pairs:
- pair `1-2`
- pair `7-8`

Files:
- `capture_mixer_05_link_pair_1_2_only.pcapng`
- `capture_mixer_06_link_pair_7_8_only.pcapng`

Actions per capture:
- ensure pair is unlinked
- link the pair
- wait
- unlink the pair

Notes:
- no pan, mute, solo, assignment, or fader changes in the same file

### 4. Pan Only

Goal:
- decode pan writes and stable state separately from fader and link changes
- compare mono-source pan behavior with playback-pair members that start from default left/right pan values

Files:
- `capture_mixer_07_pan_mono_strip.pcapng`
- `capture_mixer_08_pan_pair_strip.pcapng`

Capture 07 recommended strip:
- one unlinked strip assigned to `Preamp 1` or another mono source

Capture 07 actions:
- set pan to `0`
- set pan to `-30`
- set pan to `+30`
- set pan back to `0`

Capture 08 recommended pair:
- one adjacent unlinked pair assigned to `Computer Play 1` and `Computer Play 2`
- start from the visible default pair state: left member at `-30`, right member at `+30`

Capture 08 actions:
- set left member pan from `-30` to `0`
- wait
- set left member pan from `0` to `+30`
- wait
- set left member pan back to `-30`
- wait
- set right member pan from `+30` to `0`
- wait
- set right member pan from `0` to `-30`
- wait
- set right member pan back to `+30`

Notes:
- this capture is about the stereo playback pair, not one standalone strip interpreted as stereo by itself
- keep the pair unlinked unless specifically testing linked pan later
- do not move faders in these captures

### 5. Fader Level Only

Goal:
- refine level-state mapping without interference from mute or link changes

Files:
- `capture_mixer_09_fader_unlinked.pcapng`
- `capture_mixer_10_fader_linked_pair.pcapng`

Capture 09 actions:
- choose one unlinked strip
- move fader through three visibly different values
- return to baseline

Capture 10 actions:
- link one pair first before recording, or record link setup separately before the level moves
- move left member fader once
- move right member fader once
- return to baseline

Write down:
- which member was dragged
- whether the partner followed visually
- final fader values on both members

### 6. Mute Only

Goal:
- isolate mute state and confirm linked propagation behavior separately from level changes

Files:
- `capture_mixer_11_mute_unlinked.pcapng`
- `capture_mixer_12_mute_linked_pair.pcapng`

Actions:
- toggle mute on
- wait
- toggle mute off

For linked capture:
- note whether muting either member affects both members

### 7. Solo Only

Goal:
- identify solo command/state behavior and any interaction with adjacent linked pairs

Files:
- `capture_mixer_13_solo_single.pcapng`
- `capture_mixer_14_solo_pair_interaction.pcapng`

Capture 13 actions:
- solo one strip
- unsolo it

Capture 14 actions:
- solo strip `1`
- solo strip `2`
- clear solos

Notes:
- do not mix mute changes into these files
- if the UI has exclusive-solo behavior, say so in the notes

### 8. Metering Only

Goal:
- identify which traffic carries per-strip mixer meters and master mix meters
- separate streaming meter behavior from durable state writes
- distinguish ordinary strip metering from the separate preamp-input meter path

Files:
- `capture_mixer_15_meter_single_strip_playback.pcapng`
- `capture_mixer_16_meter_same_signal_different_strip.pcapng`
- `capture_mixer_17_preamp_panel_and_strip.pcapng`
- `capture_mixer_17_preamp_panel_only.pcapng`

Setup:
- use one stable signal source at a time
- avoid touching mixer controls during recording

Capture 15 setup:
- assign one steady playback source to a single visible strip
- keep all other strips visually idle if possible

Capture 15 actions:
- start signal
- record `5-10` seconds idle
- stop signal

Capture 16 setup:
- use the same steady source as capture `15`
- assign it to a different visible strip in a separate capture

Capture 16 actions:
- start signal
- record `5-10` seconds idle
- stop signal

Capture 17a setup:
- feed stable signal into one preamp input
- assign that preamp source to one mixer strip
- keep the preamp panel visible in notes if the UI shows separate input metering there

Capture 17a actions:
- start signal
- record `5-10` seconds idle
- stop signal

Capture 17b setup:
- feed stable signal into one preamp input
- keep focus on the preamp-only metering path without the mixer strip view changing

Capture 17b actions:
- start signal
- record `5-10` seconds idle
- stop signal

Write down:
- which strip shows movement
- whether movement follows the strip assignment rather than the source class
- whether a separate preamp-only meter is also moving
- which master meter shows movement
- which mixer surface is visible

Notes:
- ordinary mixer metering should be treated as strip-local until captures prove otherwise
- preamp inputs are special because they may show both mixer-strip metering and separate direct preamp metering
- do not try to decode meter values and durable strip settings from the same capture; metering is likely transient/noisy traffic

Status after analyzing captures `15..17`:

- existing metering captures are enough to ground the current documentation boundary:
  - metering lives in device-originated traffic, not host writes
  - stable `0x83` does not participate in the tested meter workflows
  - current evidence is still too noisy / under-labeled for a real parser-field map
- no additional metering capture is required **yet** for documentation-only work
- if parser work is attempted later, the next minimal capture should be one explicitly labeled master-meter-only recording so strip and master movement can be separated without guessing

Deferred only if parser work becomes the next task:

### 10. Master Meter Isolation

Goal:
- isolate master-meter movement from strip-meter movement
- decide whether any device-side meter bytes are dedicated to the visible master meter rather than strip-local paths

Recommended only if meter parsing becomes an implementation target.

File:
- `capture_mixer_20_master_meter_only.pcapng`

Actions:
- feed one steady signal that clearly hits the visible master meter
- keep strip assignments and mixer controls unchanged during recording
- record `5-10` seconds idle with signal present
- stop signal

Write down:
- which master meter moved
- whether any strip meter also moved on screen
- which mixer surface and page were visible

### 9. Surface Duplication / Independence

Goal:
- verify the already-known split between shared strip assignment and mix-local link/level state between `MIX 1` and `MIX 2`

Files:
- `capture_mixer_18_surface_independence_assignment.pcapng`
- `capture_mixer_19_surface_independence_link.pcapng`
- `capture_mixer_20_surface_independence_level.pcapng`

Actions:
- change one strip assignment on `MIX 1`
- switch to `MIX 2`
- verify that the same strip assignment is visible on `MIX 2`

- change one odd-slot pair link state on `MIX 1`
- switch to `MIX 2`
- verify that `MIX 2` keeps an independent link state for the same pair

- change one strip level on `MIX 1`
- switch to `MIX 2`
- verify that `MIX 2` keeps an independent level for the same strip

Notes:
- strip source assignment is expected to be shared across surfaces
- pair link state is expected to be stored independently per surface
- strip level is expected to be stored independently per surface
- do not combine assignment, link, and level in the same file if it can be avoided

## Recommended Order

1. `capture_mixer_01_mix1_idle`
2. `capture_mixer_02_mix2_idle`
3. `capture_mixer_05_link_pair_1_2_only`
4. `capture_mixer_07_pan_mono_strip`
5. `capture_mixer_09_fader_unlinked`
6. `capture_mixer_11_mute_unlinked`
7. `capture_mixer_13_solo_single`
8. `capture_mixer_03_assignment_core`
9. `capture_mixer_04_assignment_extended`
10. `capture_mixer_15_meter_preamp_signal`

This order gets the most likely durable-state families first and leaves the noisier families for later.

## What To Record Alongside Each Capture

- file name
- date/time
- control panel page
- mixer surface
- whether mixer is expanded to show all strips
- strip number touched
- pair number touched if relevant
- source assignment before and after
- link state before and after
- pan before and after
- level before and after
- mute/solo before and after
- whether signal was present
- any visible meter movement

## First-Pass Decode Targets After Capture

- strip-count confirmation: `15` vs `16`
- source-assignment enum mapping
- pair-link state representation
- pan command family and value encoding
- durable level bytes vs transient meter bytes
- mute and solo command/state representation
- which fields are surface-local vs shared across surfaces
