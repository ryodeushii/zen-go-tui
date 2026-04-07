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
- Default strip assignment on the baseline screenshots / captures:
  - `1 = Preamp 1`
  - `2 = Preamp 2`
  - `3 = Computer Play 1`
  - `4 = Computer Play 2`
  - `5 = Computer Play 3`
  - `6 = Computer Play 4`
  - `7 = Computer Play 5`
  - `8 = Computer Play 6`
  - `9 = Computer Play 7`
  - `10 = Computer Play 8`
  - `11..16 = Mute`
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

Status after captures `03`, `04`, and `18`:

- these files are enough to ground the ordinary-strip assignment family itself: `0x70 / 0x53` with payload prefix `d3 41`
- they are enough to ground the ordinary-strip source enum for the currently observed values and to trust query-side readback from `0x75 03/05..09`
- they were not enough by themselves to safely enable interactive assignment writes across all strips

Files already received under `antelope_pcap/channel_assignments/`:

- the planned focused captures `capture_mixer_21` through `capture_mixer_30`
- additional per-channel spot captures such as `ch1_preamp1_oscillator1_preamp1.pcapng`, `ch2_preamp2_oscillator1_preamp2.pcapng`, `ch3_cp1_oscillator1_cp1.pcapng`, `ch4_cp2_oscillator1_cp2.pcapng`, `ch5_cp3_oscillator1_cp3.pcapng`, `ch11_mute_oscillator1_mute.pcapng`, and `ch16_mute_oscillator1_mute.pcapng`

These should be treated as assignment-focused evidence first, with the numbered `capture_mixer_21..30` files as the canonical implementation set.

What these captures were intended to close:

- the full ordinary-strip write-index map for strips `5..16`
- the dedicated early-strip (`1..4`) write map
- one explicit write-plus-readback cross-check after reassignment so we can tie host writes, UI-visible result, and `0x75 03/*` replies together in the same session

### 2b. Assignment Delta Needed For Implementation

Goal:
- finish only the assignment-side gaps that blocked safe interactive per-channel source selection
- avoid re-capturing already-grounded ordinary-strip enum values unless they are needed as anchors for strip indexing

#### 2b-1. Ordinary Strip Index Map

Goal:
- identify which `d3 41` table entry belongs to each ordinary strip `5..16`
- prove or disprove the current linear-strip-index assumption before enabling writes again

Recommended files:
- `capture_mixer_21_assignment_index_map_5_8.pcapng`
- `capture_mixer_22_assignment_index_map_9_12.pcapng`
- `capture_mixer_23_assignment_index_map_13_16.pcapng`

Setup:
- stay on `MIX 1`
- all ordinary strips `5..16` set to a written-down baseline assignment before recording
- keep links off, pan untouched, faders untouched, no signal present
- use one clearly visible target source that is easy to spot in the UI and already grounded in the ordinary enum, preferably `Oscillator 1` or `SPDIF In 1`

Actions per file:
- for each strip in the file's range:
  - assign strip to the chosen target source
  - wait `2-3` seconds
  - assign the same strip back to baseline `Mute`
  - wait `2-3` seconds
- touch only one strip at a time

Write down:
- exact strip number changed on each step
- source before and after each step
- whether any other visible strip changed unexpectedly in the UI

Why this is needed:
- current captures anchor the write index for strip `11`, but the failed attempt showed that extrapolating that map to every other ordinary strip is not safe enough

#### 2b-2. Early Strip Write Map (`1..4`)

Goal:
- determine how strips `1..4` are encoded in `d3 41`
- separate early-strip indexing from early-strip enum semantics

Recommended files:
- `capture_mixer_24_assignment_early_ch1.pcapng`
- `capture_mixer_25_assignment_early_ch2.pcapng`
- `capture_mixer_26_assignment_early_ch3.pcapng`
- `capture_mixer_27_assignment_early_ch4.pcapng`

Setup:
- stay on `MIX 1`
- no AFX page changes, no BP ALL, no DSP editor interaction during the recording
- keep strips `5..16` untouched for the whole session
- choose two or three source assignments per channel that are visibly valid on that channel and already easy to recognize in the UI

Actions per file:
- start from the written baseline source for that strip
- change the strip to source A
- wait `2-3` seconds
- change the strip to source B
- wait `2-3` seconds
- change the strip back to baseline

Preferred source choices:
- use at least one source that is already grounded in the ordinary enum, such as `Preamp 1`, `Preamp 2`, or `Mute`
- if the UI exposes early-strip-only choices, write them down but capture them in a separate pass only after the baseline two-or-three-source run succeeds

Write down:
- whether the UI exposes a different source list for that strip
- whether any AFX-related control becomes selected or highlighted automatically
- exact visible source label after each click

Why this is needed:
- `capture_mixer_18_surface_independence_assignment.pcapng` already shows an early-strip-shaped `d3 41 05 ...` write, so we know the family exists, but not the full per-channel mapping or whether the source enum differs

#### 2b-3. Write/Readback Cross-Check

Goal:
- confirm that the same session shows matching host write, visible UI result, and startup/query readback for both an ordinary strip and an early strip

Recommended files:
- `capture_mixer_28_assignment_write_readback_ordinary.pcapng`
- `capture_mixer_29_assignment_write_readback_early.pcapng`

Setup:
- start from a non-default but clearly written baseline
- keep only one target strip changing in each file
- after each assignment change, let the control panel settle long enough to emit its readback traffic

Actions per file:
- change one strip assignment
- wait `3-5` seconds
- if needed, switch away and back to the mixer page or surface only if that is required to trigger `0x75` refreshes, and note it explicitly
- leave `3-5` seconds idle after the last action

Write down:
- exact strip changed
- exact visible source after the write
- whether the control panel needed a page/surface refresh before the expected readback appeared

Why this is needed:
- it gives one authoritative end-to-end anchor tying together `d3 41` host writes and the `0x75 03/05..09` query-side state we already decode

#### 2b-4. Surface Propagation Sanity Check

Goal:
- verify assignment sharing across surfaces with one ordinary strip and one early strip after the new focused captures, without mixing in link or level work

Recommended file:
- `capture_mixer_30_assignment_surface_propagation_sanity.pcapng`

Actions:
- on `MIX 1`, change one ordinary strip assignment
- switch to `MIX 2` and confirm the same assignment is visible
- back on `MIX 1`, change one early strip assignment
- switch to `MIX 2` and confirm the same assignment is visible

Write down:
- whether propagation is immediate or only visible after the page settles
- whether the early strip behaves differently from the ordinary strip during the switch

Why this is still useful even though `capture_mixer_18` exists:
- the failed implementation suggests we need one cleaner post-index-map sanity capture where assignment is the only thing under test

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

Status after analyzing captures `15..17` and the two new `capture_mixer_20_*` files:

- existing metering captures are enough to ground the current documentation boundary:
  - metering lives in device-originated traffic, not host writes
  - stable `0x83` does not participate in the tested meter workflows
  - current evidence is still too noisy / under-labeled for a real parser-field map
- the new `capture_mixer_20_mix1_master_and_chan2.pcapng` and `capture_mixer_20_mix2_master_and_chan2.pcapng` files confirm that per-surface strip mute/level asymmetry is the correct isolation method
- they still do not provide enough UI-labeled evidence to identify master-only bytes confidently
- no additional metering capture is required **yet** for documentation-only work

Status now that meter parser work is the next implementation target:

- additional metering capture **is** required before claiming a parser-ready strip or master meter map
- the current late-row candidates are still too ambiguous to promote safely into app code
- the existing `A2` observed meter path should be treated as a separate unresolved meter-like signal, not as proven preamp metering

### 8b. Parser-Ready Meter Follow-Up

Goal:
- isolate strip-local mixer metering from master/output metering
- distinguish direct preamp metering from the existing weird meter-like path currently surfaced near `A2`
- add enough UI-labeled evidence to promote at least a narrow passive meter parser safely

General setup for all files:
- record exact visible page and active surface in notes
- baseline mixer setup: all strips muted on both mixes unless the file explicitly says otherwise
- baseline source setup: every strip source assignment set to `Mute` unless the file explicitly says otherwise
- prefer using `Oscillator` as the signal source for strip-meter mapping, because it should light only the assigned strip meters by default
- keep source assignment unchanged during a given file unless the file is explicitly about strip-slot remapping
- do not touch pan, mute, solo, link, or assignment while signal is present unless the file explicitly requires it
- use one steady signal source only
- keep `0x70` host traffic out of the meter window whenever possible
- write down exactly which on-screen meter moved: strip meter, monitor/hp1 master, hp2 master, preamp input meter, or some combination

Grounded operating assumptions for these follow-up files:
- strip meter display follows strip assignment and is **not** suppressed by per-strip volume or mute alone
- if all strips stay muted on both mixes, the safest expectation is strip-meter movement without output/master movement
- if all strip sources start at `Mute`, accidental preamp or mic noise should stay out of the capture until one strip is explicitly reassigned for the test
- if one or more strips are unmuted in a given mix, that mix's output/master meter may also move
- this makes all-strips-muted the preferred baseline for strip-only captures, and deliberate per-mix unmute differences the preferred baseline for output/master isolation captures

#### 8b-1. Strip-Slot Meter Map

Goal:
- prove which late-row movement follows strip slot rather than source identity
- compare an early strip, an adjacent early strip, and at least one late ordinary strip with the same source and level conditions

Recommended files:
- `capture_mixer_31_strip_meter_map_ch1.pcapng`
- `capture_mixer_32_strip_meter_map_ch2.pcapng`
- `capture_mixer_33_strip_meter_map_ch11.pcapng`
- `capture_mixer_34_strip_meter_map_ch12.pcapng`

Actions for each file:
- start from a quiet baseline with all other visible strips idle
- route the same already-grounded source to the target strip before recording, preferably `Oscillator`
- start steady signal
- record `8-10` seconds with no UI interaction
- stop signal
- leave `2-3` seconds idle before ending capture

Write down:
- exact target strip number
- whether only that strip meter moved or whether another visible strip also moved
- whether either master meter moved
- which surface was visible

Why these files:
- `capture_mixer_15` and `capture_mixer_16` already showed slot-sensitive movement, but not enough adjacent-slot comparison to promote a parser

#### 8b-2. Master/Output Isolation

Goal:
- separate strip-local meter bytes from visible master/output meter bytes
- decide whether `Monitor+HP1` and `HP2` have different passive meter footprints

Recommended files:
- `capture_mixer_35_master_isolation_mix1_chan2_only.pcapng`
- `capture_mixer_36_master_isolation_mix2_chan2_only.pcapng`
- `capture_mixer_37_master_isolation_mix1_master_only_if_possible.pcapng`
- `capture_mixer_38_master_isolation_mix2_master_only_if_possible.pcapng`

Actions:
- keep assignment shared exactly as in the current working setup
- start from the all-strips-muted baseline
- make one surface clearly pass the signal by unmuting only the intended strip on that mix
- make the other surface clearly suppress the same strip by keeping it muted on that mix
- start steady signal and record `8-10` seconds idle
- if the UI allows a clean master-only visible state with no strip meter movement, record that as a separate file
- stop signal and leave `2-3` seconds idle

Write down:
- whether `Monitor+HP1` moved
- whether `HP2` moved
- whether the visible strip meter also moved
- whether the suppressed surface still showed master movement

Why these files:
- the current `capture_mixer_20_*` pair proves the right experiment shape, but not enough UI-side labeling to claim master-only bytes

#### 8b-3. Direct Preamp Meter Versus Mixer Meter

Goal:
- separate true preamp-input metering from mixer-strip metering
- determine whether the existing weird meter-like path near `A2` is direct-preamp, output/master-related, or a mixed proxy

Recommended files:
- `capture_mixer_39_preamp_only_a1_signal.pcapng`
- `capture_mixer_40_preamp_only_a2_signal.pcapng`
- `capture_mixer_41_preamp_and_strip_a1_to_ch1.pcapng`
- `capture_mixer_42_preamp_and_strip_a2_to_ch2.pcapng`

Actions:
- for preamp-only files, feed steady signal into one preamp input without assigning it to an active strip if possible
- for combined files, feed the same signal and assign it to exactly one strip before recording
- record `8-10` seconds idle per file with no UI interaction
- stop signal and leave `2-3` seconds idle

Write down:
- whether the preamp input meter moved
- whether any strip meter moved
- whether either master meter moved
- whether the weird existing meter-like path appears to follow `A2`, output/master activity, or both

Why these files:
- `capture_mixer_17_preamp_panel_and_strip` and `capture_mixer_17_preamp_panel_only` prove coexistence, but not enough to identify the current `A2` path safely

#### 8b-4. Silent Baselines For Every Follow-Up Family

Goal:
- pin stable baseline values for the same late-row windows without signal

Recommended files:
- `capture_mixer_43_strip_meter_baseline_silent.pcapng`
- `capture_mixer_44_master_meter_baseline_silent.pcapng`
- `capture_mixer_45_preamp_meter_baseline_silent.pcapng`

Actions:
- repeat the same visual setup as the corresponding signal-present files
- record `8-10` seconds with no signal and no interaction

Write down:
- which on-screen meters stayed fully still
- whether any meter-like flicker remained visible in the UI

Why these files:
- clean silent baselines make it easier to separate true activity bytes from static row-state placeholders

Deferred only if parser work becomes the next task:

### 10. Surface-Isolated Master Meter Capture

Goal:
- isolate master-meter movement from strip-meter movement
- decide whether any device-side meter bytes are dedicated to the visible master meter rather than strip-local paths
- separate `MIX 1` vs `MIX 2` master movement even though source assignment is shared across surfaces

Recommended only if meter parsing becomes an implementation target.

Files now obtained:
- `capture_mixer_20_mix1_master_and_chan2.pcapng`
- `capture_mixer_20_mix2_master_and_chan2.pcapng`

Actions:
- keep source assignment unchanged
- set one surface to pass the test strip clearly
- set the other surface to suppress the same strip via level at minimum or mute
- feed one steady signal that clearly hits the assigned strip
- record `5-10` seconds idle with signal present
- swap the per-surface mute/level roles and repeat if possible in the same session notes
- stop signal

Write down:
- which master meter moved
- whether both master meters still moved despite the surface-local mute/level difference
- whether any strip meter also moved on screen
- which mixer surface and page were visible

Status after the current `capture_mixer_20_*` pair:

- the setup is now confirmed to be the right kind of experiment
- passive traffic stays limited to `0x73` and `0x81`, with stable `0x83`
- the candidate late-row window is narrower than before, but the captures still do not justify a master-only byte claim without better UI-side notes

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
