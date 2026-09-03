# Task 3 implementation report

## Scope

Implemented profile-backed `ZenGoDriver` construction and normalized Zen Go readback. Legacy Zen Go command encoding remains in `ZenGoDriver`; runtime session construction now passes selected `RuntimeProfile`. Added capture-scoped q04/q18 patch conversion, profile-shaped snapshots, candidate 0x73 preamp meters, raw 0x83 auxiliary preservation, and attenuation-domain validation.

Nested profile submodule was not changed. It remains separate at `ede7bfd2bb1dfa9fb2252749be6742148a259096`.

## Changed files

- `antelope-protocol/src/zen_go.rs`
- `antelope-protocol/src/query.rs`
- `antelope-protocol/src/mixer.rs`
- `antelope-protocol/tests/profile_driver.rs`
- `antelope-protocol/tests/zen_go_profile_driver.rs`
- `antelope-protocol/tests/fixtures/zen_go/q04_mix1_reply.hex`
- `antelope-protocol/tests/fixtures/zen_go/q04_mix2_reply.hex`
- `antelope-protocol/tests/fixtures/zen_go/q18_reply.hex`
- `antelope-protocol/tests/fixtures/zen_go/state_with_candidate_meters.hex`
- `antelope-protocol/tests/fixtures/zen_go/meter_83_auxiliary.hex`
- `src/device/session.rs`
- `src/device/mod.rs`
- `src/app/controller.rs`
- `src/app/mod.rs`
- `src/app/dynamic_state_tests.rs`
- `src/command_queue.rs`
- `src/main.rs`
- `src/runtime.rs`
- `src/ui/tests.rs`

## Tests added/updated

`antelope-protocol/tests/zen_go_profile_driver.rs` adds 7 tests covering:

- ordered 47-pair profile startup requests
- attenuation fader wire value and range rejection
- q04/0 and q04/1 normalized mixer patches
- q18 supported-field-only solo patch
- profile candidate meters `0x41` and `0x52`
- 0x83 auxiliary classification
- constructor identity/topology rejection

Five 320-byte, 640-hex-digit fixtures were generated for those tests.

## Commands and outputs

### Profile fixture generation

```text
$ python3 - <<'PY' ...
...
PY
wrote 5 fixtures; each 320 bytes / 640 hex digits
 641 antelope-protocol/tests/fixtures/zen_go/meter_83_auxiliary.hex
 641 antelope-protocol/tests/fixtures/zen_go/q04_mix1_reply.hex
 641 antelope-protocol/tests/fixtures/zen_go/q04_mix2_reply.hex
 641 antelope-protocol/tests/fixtures/zen_go/q18_reply.hex
 641 antelope-protocol/tests/fixtures/zen_go/state_with_candidate_meters.hex
3205 total
```

### Formatting

```text
$ cargo fmt --all -- --check
[failed before formatting: reported formatting diffs in modified Rust files]
$ cargo fmt --all
[no output]
```

### Focused driver tests

```text
$ cargo test -p antelope-protocol --test zen_go_profile_driver
running 7 tests
...
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### Profile model tests

```text
$ cargo test -p antelope-protocol --test zen_go_profile_model
running 4 tests
...
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### Full workspace tests

```text
$ cargo test --workspace
...
test result: ok. 56 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
...
test result: ok. 69 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
...
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
...
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
...
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
...
test result: ok. 329 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
...
test result: ok. 40 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
...
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
...
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

Warnings remain from pre-existing `profile_codec.rs` unused variable and unrelated dead code. No test failures.

### Final formatting check

```text
$ cargo fmt --all -- --check && echo FORMAT_OK
FORMAT_OK
```

### Repository checks and commits

```text
$ git diff --check
[no output]
$ git commit -m "feat: restore profile-backed Zen Go readback"
[feature/multi-device-antelope 58fb9fd] feat: restore profile-backed Zen Go readback
 19 files changed, 572 insertions(+), 113 deletions(-)
 create mode 100644 antelope-protocol/tests/fixtures/zen_go/meter_83_auxiliary.hex
 create mode 100644 antelope-protocol/tests/fixtures/zen_go/q04_mix1_reply.hex
 create mode 100644 antelope-protocol/tests/fixtures/zen_go/q04_mix2_reply.hex
 create mode 100644 antelope-protocol/tests/fixtures/zen_go/q18_reply.hex
 create mode 100644 antelope-protocol/tests/fixtures/zen_go/state_with_candidate_meters.hex
 create mode 100644 antelope-protocol/tests/zen_go_profile_driver.rs
$ git rev-parse HEAD
58fb9fd4158c3cd1aa250f8221ce18e688d1fba9
$ git -C modules/Antelope-Ctl rev-parse HEAD
ede7bfd2bb1dfa9fb2252749be6742148a259096
```

Final root status:

```text
?? docs/superpowers/plans/2026-09-03-zen-go-profile-runtime-repair.md
```

The untracked plan file predated Task 3 and was not staged or committed. No staged files remain.

## Concerns

- `RuntimeProfile` has no `driver_kind`; per supervisor ruling, constructor validates canonical Zen Go VID/PID and required topology, while `RuntimeEntry.driver_kind` remains factory selection.
- `DynamicStatePatch` can carry one mixer surface. q18 exposes first 16 records as normalized surface 0; complete q18 body remains preserved in `DeviceEvent::QueryReply.body` and `raw`. A later seam can expose second surface without changing wire handling.
- Existing profile-codec unused-variable warning remains unrelated to Task 3.
