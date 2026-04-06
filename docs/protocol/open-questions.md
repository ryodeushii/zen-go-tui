# Zen Go USB Protocol Open Questions

This file tracks the remaining gaps after the current capture set.

## Confirmed Unknowns

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
- byte `2` = selector/target id (`0x01`, `0x11`, `0x12..0x17` observed)
- byte `3` = asserted/cleared state in the confirmed link workflows
- `a204010x` is a companion write seen only adjacent to `a203...`

What remains unresolved:

- exact difference between `a203` and `a204`
- exact UI/control target represented by selector `0x01` vs `0x11`
- whether DSP-only selectors `0x12..0x17` are channel selectors, tabs, or feature toggles
- whether `a204010x` is a master-pair latch while `a203ssx` selects the surface-local target, or whether the split is instead topology-vs-view state

Recommended follow-up capture:

- Link only one pair
- Unlink only one pair
- Do not move any faders in the same capture
- Use both mixer surfaces separately if possible

### DSP / preamp UI-label mapping

The DSP capture now gives stable field effects for several command families, but some UI-level labels are still unresolved.

What is narrowed further:

- payload byte `0x1a` now looks packed rather than monolithic: `4f` selects the low state (`0x10`/`0x11`/`0x12`), `51` toggles bit `0x10`, and `52` toggles bit `0x40`
- `510101` is the clearest entry point into the extended DSP/preamp mode because it changes both `0x73` and the front block of `0x83`
- the DSP-only `0x0ae` band now looks more like page-level state for that richer DSP/preamp mode than like a direct control value, because `510101` / `a2000000` rewrite it as a block while later `a2` selectors mostly leave it unchanged

Still-open mappings include:

- the exact control behind the `4f 00 xx` 3-state selector
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
3. One mixer pan-only capture
4. Signal type only: Mic -> Line -> Hi-Z -> Mic
5. Phase only
6. Phantom only
7. One DSP feature per capture for the families around `4f`, `51`, `52`, `d5`, `d7`
8. A capture that changes only the extended `51 01 01`-style mode and then reverts it cleanly

Those captures should be enough to finish the remaining field map without guesswork.
