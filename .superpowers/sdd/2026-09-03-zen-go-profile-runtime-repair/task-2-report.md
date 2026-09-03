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

## Fix round 1 review resolution

Review findings resolved:

1. Explicit sparse safe queries now remain authoritative even when `category_counts` is empty or lacks that category. Codec acceptance remains `explicitly_safe OR bounded`; absent pairs reject.
2. Generator always emits computed safe queries, including fallback walks derived from category counts or legacy startup queries. Layouts cannot produce an empty runtime safety list unnoticed.
3. Generator requires both `level_offset` and `state_offset` and reports missing fields as `ProfileError` before rendering.
4. Fader direction is trimmed and lowercased during normalization, producing Rust-compatible `direct`/`attenuation` values.
5. Added codec coverage for Zen Go q04/3 encoding and q04/4 rejection with empty category counts; authoritative 47-entry ordered safe sequence and duplicates remain unchanged.

Fix-round commands and outputs:

```text
cargo test -p antelope-protocol --test zen_go_profile_model
3 passed

cargo test -p antelope-protocol profile_codec::tests::zen_go_sparse_safe_query_codec_preserves_frame_and_rejects_absent_pair
1 passed

python3 -m unittest tools.test_generate_device_catalog.GeneratorTests.test_explicit_sparse_queries_escape_empty_bounds_and_derived_queries_are_emitted tools.test_generate_device_catalog.GeneratorTests.test_fader_direction_is_normalized_for_generated_rust tools.test_generate_device_catalog.GeneratorTests.test_readback_layout_requires_level_and_state_offsets
Ran 3 tests ... OK

cargo test -p antelope-protocol
57 unit tests, 69 profile-driver tests, 21 profile-pack tests, and 3 model tests passed; doc-test passed.

cargo test -p zen-go-tui
329 library tests passed, 40 binary tests passed, 2 ignored.

git diff --check
passed

python3 -m unittest tools.test_generate_device_catalog
Ran 96 tests in 1.103s
FAILED (failures=1, errors=23)
profile error: frame.routing_command.source_banks.0x02 contains a non-contiguous index range
```

Generator full-suite failure remains unrelated pre-existing Orion/source-bank prose parsing. Generated artifact drift comparison passed with process-local compatibility shim and canonical submodule `ede7bfd2bb1dfa9fb2252749be6742148a259096`.

Fix-round concern: no new concerns beyond known Orion parser blocker.

## Fix round 1 resumed: reviewer findings closure

Commands and outputs:

```text
cargo test -p antelope-protocol --test profile_driver profile_driver_encodes_q04_3_and_rejects_q04_4
running 1 test
... ok
test result: ok. 1 passed

cargo test -p antelope-protocol --test zen_go_profile_model
running 3 tests
... ok
test result: ok. 3 passed

cargo test -p antelope-protocol profile_codec::tests::zen_go_sparse_safe_query_codec_preserves_frame_and_rejects_absent_pair
running 1 test
... ok
test result: ok. 1 passed

python3 -m unittest tools.test_generate_device_catalog.GeneratorTests.test_explicit_sparse_queries_escape_empty_bounds_and_derived_queries_are_emitted tools.test_generate_device_catalog.GeneratorTests.test_fader_direction_is_normalized_for_generated_rust tools.test_generate_device_catalog.GeneratorTests.test_readback_layout_requires_level_and_state_offsets
Ran 3 tests ... OK

cargo test -p antelope-protocol
57 unit tests, 70 profile-driver integration tests, 21 profile-pack tests, 3 model tests, and doc-test passed.

cargo test -p zen-go-tui
329 library tests, 40 binary tests passed; 2 ignored.

git diff --check
passed
```

Review closure:

- Explicit safe pairs accepted independently of contiguous category bounds; absent pairs remain rejected.
- Derived safe queries always serialized into normalized readback records; valid layouts cannot silently produce empty runtime safety data.
- `level_offset` and `state_offset` now required before rendering.
- Fader direction normalized to exact `direct`/`attenuation` representation.
- Added `profile_driver_encodes_q04_3_and_rejects_q04_4` integration test; codec unit test covers empty category counts and explicit q04/3 safety.

Residual concern remains unchanged: full generator suite still has known Orion/source-bank prose parsing failure (1 failure, 23 errors). No parser scope expansion performed.

## Fix round 2: Zen Go q04 integration coverage

Reviewer P1 required q04/3 and q04/4 coverage to load Zen Go generated metadata rather than Orion fixture bounds. Applied approved ruling: expose narrow `encode_profile_query` wrapper around existing profile codec; do not alter `ProfileDriver` or Orion/source-bank parsing. Zen Go integration test asserts empty `category_counts`, encodes explicit safe pair q04/3, checks frame bytes, and rejects absent q04/4. Existing redundant Orion-only test and duplicate codec unit coverage removed. Ordered duplicate-containing 47-query startup sequence unchanged.

Exact commands and outputs:

```text
cargo fmt --all
cargo test -p antelope-protocol --test zen_go_profile_model zen_go_profile_codec_encodes_explicit_q04_3_and_rejects_q04_4
running 1 test
... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out

cargo test -p antelope-protocol --test profile_driver query_bounds_and_layout_are_profile_driven
running 1 test
... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 68 filtered out

git diff --check
passed
```

Concern unchanged: full generator suite remains blocked by pre-existing Orion/source-bank prose parser issue (1 failure, 23 errors). Cargo emits pre-existing unused `offset` warning in `profile_codec.rs`; no behavior impact.
