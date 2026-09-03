# Task 2 validation report

Date: 2026-09-03
Submodule provenance: `ede7bfd2bb1dfa9fb2252749be6742148a259096`

## Test-first evidence

Command:

```text
cargo test -p antelope-protocol zen_go_profile_model
```

Result: failed as expected before implementation. Compilation reported 11 errors for missing `safe_queries`, `allows`, `layout_for`, candidate-meter helper, and mixer-fader helper.

## Implementation validation

Command:

```text
cargo fmt --all -- --check
```

Result: passed after formatting.

Command:

```text
cargo test -p antelope-protocol --test zen_go_profile_model
```

Result: 3 passed.

Command:

```text
cargo test -p antelope-protocol
```

Result: 150 passed across 5 suites.

Command:

```text
cargo test -p zen-go-tui
```

Result: 369 passed, 2 ignored across 3 suites.

Command:

```text
git diff --check
```

Result: passed.

Generated artifact assertion (canonical profiles loaded through generator with Orion parser compatibility shim, no source edits):

```text
safe_queries=47 layouts=q04/0,q04/1,q18/0 candidate_meters=0xce,0xcf fader=attenuation 0..90 unity=0
submodule HEAD=ede7bfd2bb1dfa9fb2252749be6742148a259096
generated artifacts match canonical profiles under documented Orion parser workaround
```

## Generator validation and blocker

Required command:

```text
python3 tools/generate_device_catalog.py \\
  --profiles-dir modules/Antelope-Ctl/profiles \\
  --output src/device/generated.rs \\
  --pack-output src/device/generated_profiles.json \\
  --check
```

Result: failed at CLI parsing (`--check` currently requires a `PROFILES_DIR` argument; existing CLI check mode also requires `--generated` and `--pack-generated`).

Existing valid check-mode command was also attempted through the test suite. Full result:

```text
Ran 93 tests in 1.148s
FAILED (failures=1, errors=23)
```

All 23 errors and the one failure are pre-existing Orion/source-bank prose parsing failures:

```text
profile error: frame.routing_command.source_banks.0x02 contains a non-contiguous index range
```

This task did not broaden generator parser scope. Generated artifacts were rendered from canonical profiles using a process-local compatibility shim for the known Orion `compplay`/Zen Go S/PDIF prose collision, then compared byte-for-byte against the checked-in outputs.

## Changes

- Owned protocol records: safe query, mixer readback layout, candidate meter, fader semantics, state-report metadata.
- Profile validation for readback safety/layout geometry, candidate offsets, and fader domains.
- Query codec accepts explicit sparse safe pairs or bounded category counts.
- Parent generated definitions, Rust catalog, normalized JSON, and runtime conversion updated.
- Added `antelope-protocol/tests/zen_go_profile_model.rs`.

Open blocker: generator CLI/check path remains blocked by existing Orion/source-bank prose parser issue and mismatched brief check invocation; no parser widening included.
