# Zen Go USB Protocol Open Questions

This file tracks the remaining gaps after the current capture set.

## Confirmed Unknowns

### Remaining `0x70 / 0x53` assignment-write questions

The assignment captures now ground the command family well enough to narrow the remaining gap precisely:

- ordinary strip assignment writes use `0x70 / length 0x53` with payload prefix `d3 41`
- the new `capture_mixer_21..23_assignment_index_map_*.pcapng` files confirm that ordinary strips map linearly to table entries: `CH5..16 -> entry 4..15`
- the new `capture_mixer_24..27_assignment_early_ch*.pcapng` files confirm that early strips use `bb = 0x05` with direct entry mapping `CH1..4 -> entry 0..3`
- the `bb = 0x05` early-strip writes use the common source tuples directly for the tested sources, while the ordinary-bank `03 xx` values remain a different representation
- stable assignment effects appear in `0x73` only; current evidence still shows no stable `0x83` delta for assignment changes

What remains unresolved:

- exact semantic role of the ordinary-bank `03 00 .. 03 03` placeholders for the early-strip slots inside `bb = 0x03/06/07/08/09`
- the full valid source-capability subset for each early strip `1..4`, especially any AFX-only or channel-limited sources beyond the now-tested common tuple values
- whether there is any surface-dependent wrapper around `d3 41` for assignment, even though the current evidence still supports assignment itself being shared across surfaces

What is already safe:

- the ordinary-strip enum/value map for `Preamp 1..2`, `Computer Play 1..2,8`, `Emu Mic 1..2`, `SPDIF In 1..2`, `Mute`, and `Oscillator 1..2`
- the candidate interpolation `01 02 .. 01 06 => Computer Play 3 .. 7`
- the full per-strip entry map across both families:
  - early `bb = 0x05`: `CH1..4 -> entry 0..3`
  - ordinary `bb = 0x03/06/07/08/09`: `CH5..16 -> entry 4..15`
- the app-side write rule that preserves existing assignments by serializing a full current table instead of sparse single-entry patches

### Exact semantics inside the late `0x73` table rows

The currently available captures support a stronger structural claim than before:

- repeated row anchors at `0x6e`, `0x8e`, `0xae`, `0xce`
- a shared late shadow cluster at `0xda..0xe5`
- `0x6a` as the current output/mixer surface selector
- row-head bytes at `base+0` behave like coarse enums (`0x60`, `0x5a`, `0x54`, DSP-only `0x51`)
- row-local bytes at `base+1` behave like noisier substate bytes; stronger current evidence supports `0x0cf` as a dense mixed local-status byte rather than one compact level/mode code
- ordinary output/mixer traffic is sparse: it mainly changes `0x6e`, `0x6f`, `0x8e`, `0xce`, `0xcf`, and shadow bytes `0x0da..0x0df`, `0x0e2`, with a now-better-supported split between coarse-shadow `0x0da..0x0dd` + `0x0e2` and secondary local shadow pair `0x0de..0x0df`
- the `0x0ae` band is not a generally active mixer/output row; current strong evidence for it comes only from the extended DSP enter/exit path where `0x0ae`, `0x0af`, `0x0b0`, `0x0b1`, `0x0b3`, `0x0b4`, and `0x0e3` change together

What is still unresolved is the exact meaning of each byte inside those rows.

Additional confirmed boundary from the startup/idle traces:

- the candidate mixer late rows do not settle to one static per-strip table before user interaction; `capture_08`, `capture_10`, and `capture_10_2` show dozens to hundreds of distinct pre-command `0x73` late-row states, and `capture_09` shows the same kind of churn during idle
- because the front output bytes and current-surface byte can remain constant while those late bytes keep moving, the existing captures are not strong enough to justify a real passive startup per-strip level/mute decoder yet

Examples of still-open byte-level questions:

- what exact user-facing concept maps onto the already-better-bounded `0x54` alternate-engaged tier in each workflow
- whether `0x60 -> 0x5a` is simply neutral -> active, or whether row `0x6e` uses that pair as a surface-local view selector rather than a true gain/state transition
- which specific local status facets are packed into `0x0cf` beyond the now-confirmed conclusion that it is a dense row-local status code rather than one direct level/mode code
- the exact per-byte meaning inside the now-better-supported split where `0x0da..0x0dd` plus `0x0e2` follow coarse row-head state while `0x0de..0x0df` behave as a second local shadow pair (`0x0de` more latch-like, `0x0df` more substate-like)
- which exact DSP/preamp page or submode the DSP-only `0x0ae` band represents during the extended `510101` mode; current evidence supports it as page-level state, not an ordinary output/mixer row
- what exact coarse meaning `0x6e` / `0x6f` encode for the current Monitor/HP1 vs HP2 surface group now that the captures support UI-surface partitioning better than output-group or hardware-path partitioning
- which part of the late-row churn is durable mixer state versus transient focus/selection/scan state in the passive startup stream

What is now better supported than before:

- `0x60`, `0x5a`, and `0x54` do not behave like separate direct encodings for normal/mute/dim/linked states; they are better treated as coarse late-row tiers, with `0x54` narrowed further to a stronger alternate-engaged tier rather than a clean mute/dim/link flag
- `0x51` is no longer best treated as a generic DSP-only enum option; the strongest stable evidence is specifically the extended DSP/preamp enter/exit pair, where `0x8e` and `0x0ce` go `0x54 <-> 0x51` while the DSP-only `0x0ae` row goes `0x60 <-> 0x5a`
- `0x0cf` is no longer best treated as a mysterious noisy byte in general; the strongest current evidence supports it as a dense row-local status code that mixes local context/progression rather than a scalar level or direct event marker
- `0x0ae` is now better bounded as DSP/preamp editor-page state rather than as a direct parameter row
- the newer `antelope_pcap/mutes/` captures rule out `0x0e0/0x0e1` as mute-state bytes; they stay pinned at `0x60`, while active-surface pair-local state appears instead in `0x0da..0x0dd` (`MIX 1`) or `0x0de..0x0df` (`MIX 2`)

New boundary from the mute-matrix captures:

- with signal present, those pair-local bytes clearly carry `CH1/CH2` pair state, but the values are mixed with activity/meter behavior and do not yet form a trustworthy mute-only parser
- without signal, the same captures do not produce a clean 4-way static enum either
- the signal-present XOR/codebook view is strong enough to justify a test-only experimental model:
  - `MIX 1`: treat `0x0da/0x0db` as the canonical pair lanes and `0x0dc/0x0dd` as mirrors
  - `MIX 2`: treat `0x0de/0x0df` as the canonical pair lanes
  - current repeated lane codebook:
    - `60/60` both mute
    - `01/01` ch1 mute, ch2 unmute
    - `00/06` ch1 unmute, ch2 mute
    - `0a/05` both unmute
- what remains unresolved is whether those lane codes are direct state values or state-plus-activity overlays that only collapse to this codebook under the tested signal conditions

Recommended follow-up capture:

- One control at a time
- Pause 2-3 seconds between actions
- One capture per control family
- Note the exact channel and target output used
- Add one passive-only capture that opens directly onto the mixer page and then records 5-10 seconds with no user writes, plus a spoken/written note of the visible selected strip and saved strip values

### Exact role of the front bytes in `0x83`

`0x83` is now narrowed substantially:

- completely static during pure idle
- no stable payload changes at all in the mixer/output captures once transient snapshots are ignored
- clock-source changes do not show stable `0x83` deltas in the current captures
- sample-rate and DSP/preamp workflows do show stable `0x83` movement, still concentrated in payload `0x00..0x0d`
- the strongest repeatable moving offsets are `0x00`, `0x02`, `0x04`, `0x05`, `0x06`, `0x08`, `0x09`, and `0x0a`
- bytes `(0,1)`, `(2,3)`, and `(4,5)` often move as three paired fields, consistent with compact little-endian selector/code words

What remains unresolved is what those front bytes *mean*.

The remaining uncertainty is no longer “is `0x83` important state?” but rather:

- is it DSP/preamp auxiliary metadata?
- a compact capability/selection block that also tracks sample-rate family?
- a lookup/index block consumed by the UI?
- why advanced DSP writes (`510101`, `d50a...`, `d711...`) and sample-rate writes perturb it while ordinary mixer/output and clock-source writes do not

### `0x81` 6-byte notification field meaning

`0x81` is now narrowed to an event/heartbeat channel rather than a state channel.

What is confirmed:

- it continues during idle at roughly millisecond cadence
- byte `5` is usually `0x00`, but rare nonzero bursts do exist: `0xff` is visible in `capture_11`, and `0x01` / `0xff` appear in a few packets in `capture_07`
- bytes `0..4` are highly dynamic and not a direct mirror of UI state
- byte `0` is usually `0x00`, with rare one-packet pulses to `0x01` or `0x02` that do not match any stable `0x73`/`0x83` change
- bytes `1..4` are not a simple monotonic counter in either endianness, and do not cleanly decompose into two monotonic 16-bit counters either

Additional confirmed boundary:

- `0x81` traffic gets denser around many write-heavy windows in mixer/output captures, but most notifications still occur outside narrow write-adjacent windows, so it is event-adjacent timing data rather than a stable state or commit channel

What remains unresolved is the exact encoding of bytes `0..4`:

- monotonic counter fragments
- timer/tick material
- event ids / sequence fragments
- some combination of those

### Exact target mapping inside `0x70 / 0x14` (`A2`) family

This family is now narrowed to a clean byte split:

- byte `1` = subfamily (`0x03` / `0x04`)
- `a203`: byte `2` = selector/target id, byte `3` = asserted/cleared state in the confirmed link workflows
- `a204`: byte `2` = companion bank/context byte (`0x00` and `0x01` observed in the dedicated mixer link captures), byte `3` = asserted/cleared state
- `a204<bank><x>` is a companion write seen only adjacent to `a203...`

What remains unresolved:

- exact difference between `a203` and `a204`
- exact UI/control target represented by selector `0x01` vs older mixed-capture selector `0x11`
- whether DSP-only selectors `0x12..0x17` are channel selectors, tabs, or feature toggles
- whether `a204<bank><x>` is a master-pair latch while `a203ssx` selects the surface-local target, or whether the split is instead topology-vs-view state

Recommended follow-up capture:

- Link only one pair
- Unlink only one pair
- Do not move any faders in the same capture
- Use both mixer surfaces separately if possible

What is now grounded enough to remove from the unknown bucket:

- tested mixer-pair selectors are now partially mapped:
  - `a203000x` = tested `MIX 1` pair `1-2`
  - `a203030x` = tested `MIX 1` pair `7-8`
  - `a2030101` = tested `MIX 2` link target in the surface-independence capture
- `a204000x` and `a204010x` are better bounded as helper/companion writes because they show no stable `0x73`/`0x83` delta by themselves in the tested captures

What remains unresolved:

- the complete selector map for all adjacent pairs across both surfaces
- whether selector numbering is row-local, surface-banked, or follows another internal table order

### Passive solo-state decode

The dedicated solo captures narrow the write-side encoding enough to remove that part from the unknown bucket:

- solo uses the ordinary mixer `d4 04 <mixer> <channel> <level> <pan/state>` host family
- the final byte carries a solo bit `0x80` on top of the ordinary pan/state byte
- grounded examples:
  - `d4 04 00 01 00 a0` = centered strip-1 solo on
  - `d4 04 00 01 00 20` = centered strip-1 solo off
  - `d4 04 00 02 00 a0` = centered strip-2 solo on
  - `d4 04 00 02 00 20` = centered strip-2 solo off

What remains unresolved:

- which exact late `0x73` byte or tuple is the durable passive solo field for a given strip
- whether the same passive pattern generalizes cleanly beyond the tested strip-`1` / strip-`2` solo captures
- whether any late-row changes seen in the solo captures are pure solo state versus selection/focus churn in the same cluster

### Exact passive `0x73` pan-field decode

The dedicated pan captures now ground the host encoding well enough to narrow this precisely:

- pan uses the ordinary `d4 04 <mixer> <channel> <level> <pan>` host family
- the final byte is a scalar pan value over the observed range `0x02 .. 0x3e`
- `0x20` is the grounded center value
- the same raw encoding is used for the tested mono strip and playback-pair members

What remains unresolved:

- exact mapping from intermediate raw values to UI labels/steps
- which exact late `0x73` byte or tuple is the durable passive pan field for a given strip
- whether a cleaner passive pan decode needs a slower one-step-per-capture workflow to avoid late-row churn

What is no longer fully open:

- the app now uses a narrow passive decoder for the active-surface strip-1 cluster, but only for grounded center/near-center anchors
- this does **not** promote passive pan to solved status for arbitrary strips or arbitrary raw pan positions

### Exact meter-field mapping

The metering captures now narrow this further than before:

- stable meter-related changes are visible in `0x73`, not `0x83`
- ordinary playback meter movement follows strip slot / row placement rather than source identity alone
- preamp-panel metering is distinct from mixer-strip metering and can coexist with it
- the new surface-isolated files are passive-only and keep `0x83` fully stable while narrowing the candidate `0x73` windows further:
  - `capture_mixer_20_mix1_master_and_chan2.pcapng`: `0x6e`, `0x8e`, `0xce`, `0xcf`, `0xe2`
  - `capture_mixer_20_mix2_master_and_chan2.pcapng`: `0x8e`, `0xce`, `0xcf`, `0xda..0xdd`, `0xe2`

What remains unresolved:

- exact master-meter separation
- exact user-facing strip-meter scaling beyond the currently grounded raw-byte lane
- whether `0x81` contributes compact meter-side timing/stream data or is only adjacent notification traffic; the current broader cross-plane pass still does not justify treating it as a parser-ready meter source

What is better bounded now:

- the shared strip-meter raw lane is now grounded at `0x8e..0x9d`, mapping directly to `CH1..16`
- `0x83` remains fully stable in the dedicated passive meter captures, so it is not the missing mixer meter/state plane
- `0x81` clearly reacts to activity level, but the current passive meter captures do not give a stable strip-addressable mapping from its six bytes to visible meter values

Recommended follow-up only if parser work becomes the next target:

- a better-labeled repeat of the same surface-isolated setup, with written UI notes stating exactly which visible master meter moved and whether any strip meter moved at the same time; the current `capture_mixer_20_*` pair improves the boundary but is still not enough to disambiguate master-only bytes
- the focused follow-up matrix now lives in `docs/protocol/mixer-capture-plan.md` under `8b. Parser-Ready Meter Follow-Up`

### DSP / preamp UI-label mapping

The DSP capture now gives stable field effects for several command families, but some UI-level labels are still unresolved.

What is narrowed further:

- payload byte `0x1a` now looks packed rather than monolithic: `4f` selects the low state (`0x10`/`0x11`/`0x12`), `51` toggles bit `0x10`, and `52` toggles bit `0x40`
- `510101` is the clearest entry point into the extended DSP/preamp mode because it changes both `0x73` and the front block of `0x83`
- the DSP-only `0x0ae` band now looks more like page-level state for that richer DSP/preamp mode than like a direct control value, because `510101` / `a2000000` rewrite it as a block while later `a2` selectors mostly leave it unchanged

Still-open mappings include:

- the user-facing meaning of the extended `51 01 01` mode
- the actual feature changed by `d5 0a ...`
- whether `d7 11 ...` is preset/application/commit traffic or a hidden control family

Recommended follow-up capture:

- One DSP feature per capture
- Speak or write down the exact action time
- Avoid mixing phantom, impedance, phase, link, and source-type changes in one session

## Best Next Captures

1. Link-only / unlink-only on one stereo pair, no fader moves
2. One output `DIM` capture with no volume changes
3. One better-labeled follow-up to the `capture_mixer_20_*` surface-isolated master-meter setup if meter parser work becomes necessary
4. Signal type only: Mic -> Line -> Hi-Z -> Mic
5. Phase only
6. Phantom only
7. One DSP feature per capture for the families around `4f`, `51`, `52`, `d5`, `d7`
8. A capture that changes only the extended `51 01 01`-style mode and then reverts it cleanly

Those captures should be enough to finish the remaining field map without guesswork.
