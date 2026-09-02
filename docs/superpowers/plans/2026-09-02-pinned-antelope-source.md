# Pinned Antelope-Ctl Source Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pin reviewed Antelope-Ctl profile sources inside `modules/Antelope-Ctl`, make generator/tests/CI/docs use that checkout, and keep runtime behavior dependent only on checked-in generated artifacts or explicit normalized packs.

**Architecture:** A Git submodule at `modules/Antelope-Ctl` is the build-time canonical source and is pinned to reviewed commit `a40fb9ef7f80cae357196bfaa0c5f612fd281ac5`. The Python generator maps complete raw command, mixer, routing, link, and readback evidence into typed normalized records while preserving raw snapshots and metadata; no optional enrichment field can silently suppress source data. CI initializes and verifies the submodule only in its test job, regenerates/checks committed Rust and JSON artifacts, and runtime never opens or reads the submodule.

**Tech Stack:** Git submodules, GitHub Actions, Python standard library, Rust/Cargo workspace, checked-in generated Rust and normalized JSON profile artifacts, Markdown documentation.

**Spec:** `docs/superpowers/specs/2026-08-31-multi-device-antelope.md` plus approved pinned-source design recorded in the preceding architecture decision.

## Global Constraints

- The canonical source is `modules/Antelope-Ctl/profiles/*.json` at submodule commit `a40fb9ef7f80cae357196bfaa0c5f612fd281ac5`.
- `modules/Antelope-Ctl/profiles/mic_models.json` is excluded because it is microphone-model data, not hardware profile data.
- Revision changes use manual pointer-update PRs; no floating branch, automated update, or unreviewed source fetch is allowed.
- Ordinary Cargo builds and runtime startup must not require, read, or fetch the Antelope-Ctl checkout.
- `src/device/generated.rs` and `src/device/generated_profiles.json` remain checked in and are the runtime built-in source of truth.
- The generator fails on missing or invalid source; it must not fall back to checked-in artifacts when canonical input is unavailable.
- Explicit `--profiles-dir` remains supported for generation and isolated tests; the repository-relative pinned source is the only default source.
- Every non-excluded canonical field is preserved in generated raw snapshots or typed normalized metadata; every control-bearing raw section populates typed output or raises an explicit `ProfileError`, never silently disappears.
- Raw `frame.*`, `mixer`, `buses`, and related command sections are the derivation inputs when `runtime_topology` is absent; confirmed finite geometry becomes actionable, while partial, ambiguous, host-dependent, or unconfirmed evidence remains preserved but non-actionable.
- Runtime external profile support remains normalized JSON through `--profile-pack`; raw Antelope-Ctl JSON is never loaded directly by Rust.
- Orion remains visible but `Disabled` / `RuntimeDriverKind::None`; unknown numbered-report framing and ambiguous input-link independence remain blockers.
- No new validation fields, hardware-validation claims, protocol behavior, Task 8 work, or unrelated warning cleanup.
- Do not create commits unless the user requests them. A submodule pointer is represented by an intentional Git gitlink; do not leave unrelated generated or cache files staged.

---

### Task 1: Add the pinned canonical submodule

**Files:**
- Create: `.gitmodules`
- Add: `modules/Antelope-Ctl` as a Git submodule gitlink pinned to commit `a40fb9ef7f80cae357196bfaa0c5f612fd281ac5`

**Interfaces:**
- Provides canonical source directory `modules/Antelope-Ctl/profiles` for generator and canonical tests.
- Uses remote `https://github.com/diafebus/Antelope-Ctl.git`.
- Does not alter runtime Rust modules or generated artifacts.

- [ ] **Step 1: Add the submodule at the reviewed revision**

Run from repository root:

```bash
git submodule add https://github.com/diafebus/Antelope-Ctl.git modules/Antelope-Ctl
git -C modules/Antelope-Ctl checkout --detach a40fb9ef7f80cae357196bfaa0c5f612fd281ac5
```

Ensure `.gitmodules` contains exactly this configuration:

```ini
[submodule "modules/Antelope-Ctl"]
	path = modules/Antelope-Ctl
	url = https://github.com/diafebus/Antelope-Ctl.git
```

Do not add a floating branch or update policy that follows `main` automatically.

- [ ] **Step 2: Verify the pointer and source shape**

Run:

```bash
test "$(git -C modules/Antelope-Ctl rev-parse HEAD)" = "a40fb9ef7f80cae357196bfaa0c5f612fd281ac5"
git submodule status -- modules/Antelope-Ctl
test -f modules/Antelope-Ctl/profiles/orion_studio_3.json
test -f modules/Antelope-Ctl/profiles/zen_go_sc.json
test -f modules/Antelope-Ctl/profiles/mic_models.json
```

Expected: detached submodule status begins with the pinned hash; all three files exist. The generator, not Git, excludes `mic_models.json`.

- [ ] **Step 3: Record the manual-update contract without committing**

Keep the gitlink needed to represent the submodule pointer, but do not create a commit. Confirm no ordinary build output, Python cache, or unrelated file is staged:

```bash
git diff --cached --name-only
git status --short
```

The only intentional staged path(s), if Git requires staging to materialize the new gitlink, are `.gitmodules` and `modules/Antelope-Ctl`; all source, test, documentation, and generated-artifact edits remain reviewable working-tree changes until the user chooses integration.

---

### Task 2: Normalize complete raw profiles without silent gaps

**Files:**
- Modify: `tools/generate_device_catalog.py: _confirmed_runtime_topology, _runtime_topology, _build_link_domains, _build_mixers, _build_routing_groups, and focused raw-topology helpers`
- Modify: `tools/test_generate_device_catalog.py`
- Modify if generator output changes: `src/device/generated.rs`
- Modify if generator output changes: `src/device/generated_profiles.json`

**Interfaces:**
- Preserve `generate_catalog(profiles_dir: Path)`, `generate_profile_pack(profiles_dir: Path)`, and `check_generated_artifacts(profiles_dir, generated, pack_generated)` signatures.
- Preserve `normalize_profile(data, *, path, profiles_dir, source_bytes) -> NormalizedProfile` and `_normalized_profile_record(profile) -> dict[str, Any]` behavior for explicit fixture paths.
- Keep explicit `runtime_topology` support for existing enriched fixtures, but when it is absent derive topology from raw `frame.*`, `mixer`, `buses`, and parameter evidence instead of returning empty arrays.
- Preserve every mapping-valued non-private `frame.*` record as a typed `RuntimeFrame` with metadata equal to its raw source object; `render_catalog` continues to preserve the complete raw profile snapshot.
- Derive only confirmed finite action domains: Orion emits four 32-strip mixers, all 15 confirmed routing destinations with finite source banks, and only mixer link space 3 with 16 pairs; Orion physical/ADAT/S/PDIF link spaces remain absent from input capabilities. Zen Go emits its confirmed 2×16 mixer geometry, preserves partial routing/link evidence in frame metadata, and emits no unconfirmed link domain or source-domain-backed route.
- Derive raw-confirmed input capabilities without topology loss: Orion `physical_inputs` exposes `gain`, `mode`, `phantom`, and `phase`; Orion `adat_inputs` and `spdif_inputs` expose `gain`; no address space exposes `link` unless an explicit confirmed topology fixture proves it.

- [ ] **Step 1: Write failing raw-to-typed mapping regressions**

Add one test that loads `orion_studio_3.json` and `zen_go_sc.json` from `REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"` through `normalize_profile`, then passes each result to `_normalized_profile_record`. Assert these exact values:

```python
def test_raw_control_sections_are_not_dropped(self):
    profiles = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"

    def record(name):
        path = profiles / name
        raw = json.loads(path.read_text())
        normalized = generator.normalize_profile(
            raw,
            path=path,
            profiles_dir=profiles,
            source_bytes=path.read_bytes(),
        )
        return generator._normalized_profile_record(normalized), raw

    orion, orion_raw = record("orion_studio_3.json")
    self.assertEqual([item["strip_count"] for item in orion["mixers"]], [32, 32, 32, 32])
    self.assertEqual([item["has_master"] for item in orion["mixers"]], [True, True, True, True])
    self.assertEqual(
        [group["channel_count"] for group in orion["routing_groups"]],
        [16, 2, 2, 2, 2, 2, 32, 16, 2, 32, 32, 32, 32, 32, 16],
    )
    self.assertEqual([group["destination"] for group in orion["routing_groups"]], list(range(15)))
    source_bounds = {domain["bank"]: domain["index_count"] for domain in orion["routing_groups"][0]["source_domains"]}
    self.assertEqual(source_bounds, {0: 12, 1: 8, 3: 16, 4: 2, 5: 32, 6: 2, 7: 2, 8: 2, 9: 2, 10: 16, 11: 1})
    self.assertEqual(
        [(domain["protocol_space"], domain["pair_count"]) for domain in orion["link_domains"]],
        [(3, 16)],
    )
    self.assertEqual(len(orion["startup_queries"]), 113)
    self.assertFalse(
        any(
            capability["kind"] == "link"
            for space in orion["address_spaces"]
            for capability in space["input_capabilities"]
        )
    )
    def controls(recorded, space_id):
        space = next(item for item in recorded["address_spaces"] if item["id"] == space_id)
        return [item["kind"] for item in space["input_capabilities"]]
    self.assertEqual(controls(orion, "physical_inputs"), ["gain", "mode", "phantom", "phase"])
    self.assertEqual(controls(orion, "adat_inputs"), ["gain"])
    self.assertEqual(controls(orion, "spdif_inputs"), ["gain"])

    zen, zen_raw = record("zen_go_sc.json")
    self.assertEqual([item["strip_count"] for item in zen["mixers"]], [16, 16])
    self.assertEqual([item["has_master"] for item in zen["mixers"]], [False, False])
    self.assertEqual(
        [(group["destination"], group["channel_count"]) for group in zen["routing_groups"]],
        [(3, 8), (5, 4), (6, 32), (7, 32), (8, 32), (9, 32)],
    )
    self.assertTrue(all(not group["source_domains"] for group in zen["routing_groups"]))
    self.assertEqual(zen["link_domains"], [])

    for normalized, raw in ((orion, orion_raw), (zen, zen_raw)):
        frame_ids = {frame["id"] for frame in normalized["frames"]}
        raw_frame_ids = {
            name for name, value in raw["frame"].items()
            if isinstance(value, dict) and not name.startswith("_")
        }
        self.assertEqual(frame_ids, raw_frame_ids)
        for frame_name in ("mix_command", "routing_command", "link_command", "readback"):
            frame = next(item for item in normalized["frames"] if item["id"] == frame_name)
            self.assertEqual(json.loads(frame["metadata"]), raw["frame"][frame_name])
```

The test must also assert `"runtime_topology" not in orion_raw` and `"runtime_topology" not in zen_raw`. Keep raw link space 0/1 and Zen Go partial route/link notes in frame metadata; do not assert or create input-link capabilities for them.

- [ ] **Step 2: Run focused tests and verify the topology test fails**

Run:

```bash
python3 -m unittest tools.test_generate_device_catalog.GeneratorTests.test_raw_control_sections_are_not_dropped -v
```

Expected: failure because absent top-level `runtime_topology` currently makes normalized mixers, routing groups, and link domains empty, even though frame metadata and readback are present.

- [ ] **Step 3: Implement raw-section derivation with explicit safety boundaries**

Add a deterministic derivation path used only when explicit confirmed `runtime_topology` is absent:

```python
def _confirmed_runtime_topology(profile: NormalizedProfile) -> Mapping[str, Any] | None:
    explicit = profile.raw.get("runtime_topology")
    if explicit is not None:
        if not isinstance(explicit, Mapping):
            raise ProfileError("runtime_topology must be an object")
        if _normalized_status(str(explicit.get("status", ""))) == "confirmed":
            return explicit
    return _derive_runtime_topology(profile)
```

`_derive_runtime_topology` must:

1. derive mixer count, strip count, master presence, and offsets from top-level `mixer` when present or confirmed `frame.mix_command` fields/notes/readback evidence when it is absent; use no PID/device-name defaults;
2. derive Orion routing destinations from `frame.routing_command.addressable_destinations` and `destination_channels`, derive finite source-bank domains from explicit raw index-range evidence, exclude host-dependent bank `0x02` from confirmed typed bounds, and retain all command/readback evidence in frame metadata;
3. derive only Orion mixer link domain `{protocol_space: 3, pair_count: 16, status: "confirmed"}` from confirmed mixer-link evidence; preserve physical/ADAT/S/PDIF space values but never turn them into input capabilities;
4. retain Zen Go partial route/link records as non-actionable typed groups/metadata without source domains or confirmed link domains; unconfirmed or host-dependent bounds must not become confirmed action domains;
5. raise `ProfileError` when a recognized control record is malformed or claims a typed field that cannot be represented, instead of returning an empty list that hides the source record.

Keep `_readback_definition` independent and unchanged except for tests proving `frame.readback` still maps. Do not add a raw JSON field to Rust; generated Rust already embeds each raw profile snapshot.

- [ ] **Step 4: Regenerate checked-in artifacts from the pinned source**

Run:

```bash
python3 tools/generate_device_catalog.py \\
  --profiles-dir modules/Antelope-Ctl/profiles \\
  --output src/device/generated.rs \\
  --pack-output src/device/generated_profiles.json
```

Inspect generated changes. They may add the typed topology that raw profiles already prove, but must not enable Orion, invent report framing, expose ambiguous input links, or change unrelated Zen Go/Discrete protocol behavior. Do not hand-edit generated files.

- [ ] **Step 5: Run mapping, generator, and artifact checks**

Run:

```bash
python3 -m unittest tools.test_generate_device_catalog.GeneratorTests.test_raw_control_sections_are_not_dropped -v
python3 -m unittest discover -s tools -p 'test_*.py' -v
python3 tools/generate_device_catalog.py --check modules/Antelope-Ctl/profiles --generated src/device/generated.rs --pack-generated src/device/generated_profiles.json
```

Expected: focused mapping test, full Python generator suite, and explicit canonical artifact check pass. Readiness/driver matrix remains unchanged; all raw control sections are either typed or preserved as metadata with a deliberate non-actionable status.

---

### Task 3: Make generator source resolution repository-relative

**Files:**
- Modify: `tools/generate_device_catalog.py: module constants, _parse_args, main`
- Test: `tools/test_generate_device_catalog.py`

**Interfaces:**
- Preserve `generate_catalog(profiles_dir: Path)`, `generate_profile_pack(profiles_dir: Path)`, and `check_generated_artifacts(profiles_dir, generated, pack_generated)` signatures.
- Preserve explicit generation flags `--profiles-dir`, `--output`, `--pack-output` and check flags `--check`, `--generated`, `--pack-generated`.
- Add a module-level repository-relative default equivalent to `Path(__file__).resolve().parents[1] / "modules" / "Antelope-Ctl" / "profiles"`.

- [ ] **Step 1: Write the failing default-source regression**

Add a subprocess test that runs the real generator from a temporary current directory with output paths but without `--profiles-dir`, then compares both generated files with the checked-in artifacts:

```python
def test_cli_defaults_to_pinned_repository_source(self):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        generated = root / "generated.rs"
        pack_generated = root / "generated_profiles.json"
        result = subprocess.run(
            [
                sys.executable,
                str(GENERATOR),
                "--output",
                str(generated),
                "--pack-output",
                str(pack_generated),
            ],
            cwd=root,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(generated.read_bytes(), (REPO_ROOT / "src/device/generated.rs").read_bytes())
        self.assertEqual(
            pack_generated.read_bytes(),
            (REPO_ROOT / "src/device/generated_profiles.json").read_bytes(),
        )
```

Also assert the default path is repository-relative and that `mic_models.json` is not treated as a hardware profile:

```python
self.assertEqual(
    generator.DEFAULT_PROFILES_DIR,
    REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles",
)
self.assertNotIn("mic_models", generator.discover_profiles(generator.DEFAULT_PROFILES_DIR))
```

Use the test module's existing `REPO_ROOT`, subprocess, and temporary-directory conventions; do not hard-code `/home/ryodeushii`.

- [ ] **Step 2: Run the focused test and verify it fails before implementation**

Run:

```bash
python3 -m unittest tools.test_generate_device_catalog.GeneratorTests.test_cli_defaults_to_pinned_repository_source -v
```

Expected: failure because generation currently requires `--profiles-dir` and no `DEFAULT_PROFILES_DIR` exists.

- [ ] **Step 3: Implement the smallest source-resolution change**

Define the default from `__file__`, not the process working directory:

```python
REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PROFILES_DIR = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"
```

Keep `--profiles-dir` defaulting to `DEFAULT_PROFILES_DIR` only for generation. Determine generation mode from output flags (and an explicitly supplied `--profiles-dir`) so check mode remains unambiguous. Generation with `--output PATH --pack-output PATH` uses the pinned source; generation with `--profiles-dir PATH --output PATH --pack-output PATH` uses the explicit source. Missing default input must propagate through the existing `OSError`/profile-error return path; never use generated artifacts as fallback.

- [ ] **Step 4: Run focused and existing generator tests**

Run:

```bash
python3 -m unittest tools.test_generate_device_catalog.GeneratorTests.test_cli_defaults_to_pinned_repository_source -v
python3 -m unittest tools/test_generate_device_catalog.py -v
```

Expected: the new test passes and all existing tests remain green, including explicit temporary-source generation and `mic_models.json` exclusion.

---

### Task 4: Migrate canonical generator tests to the pinned path

**Files:**
- Modify: `tools/test_generate_device_catalog.py`

**Interfaces:**
- All canonical-source tests consume `REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"`.
- Temporary-fixture tests continue passing an explicit `--profiles-dir` or `Path` argument.
- No test may depend on `/home/ryodeushii/repos/Antelope-Ctl`.

- [ ] **Step 1: Add the canonical-path contract test**

Add a test that checks the test suite's canonical directory is the checked-out submodule and that a known profile exists:

```python
def test_canonical_profiles_use_pinned_submodule(self):
    self.assertEqual(
        CANONICAL_PROFILES,
        REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles",
    )
    self.assertTrue((CANONICAL_PROFILES / "orion_studio_3.json").is_file())
```

- [ ] **Step 2: Run the new test before path migration**

Run:

```bash
python3 -m unittest tools.test_generate_device_catalog.GeneratorTests.test_canonical_profiles_use_pinned_submodule -v
```

Expected: failure if the suite still points at the old absolute checkout or has no `CANONICAL_PROFILES` helper.

- [ ] **Step 3: Replace canonical absolute paths with one repository-relative helper**

Define once near existing test constants:

```python
REPO_ROOT = Path(__file__).resolve().parents[1]
CANONICAL_PROFILES = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"
```

Replace every canonical test use of `/home/ryodeushii/repos/Antelope-Ctl/profiles` with `CANONICAL_PROFILES`. Keep test-created profile directories separate. Do not alter expected profile semantics, readiness matrix, Orion blocker assertions, provenance checks, or artifact contents.

- [ ] **Step 4: Run full Python tests and search for stale paths**

Run:

```bash
python3 -m unittest discover -s tools -p 'test_*.py' -v
python3 - <<'PY'
from pathlib import Path
root = Path('.')
needle = '/home/ryodeushii/repos/Antelope-Ctl'
paths = [
    *root.glob('tools/*.py'),
    *root.glob('README.md'),
    *root.glob('docs/*.md'),
    *root.glob('.github/workflows/*.yml'),
]
for path in paths:
    if needle in path.read_text():
        raise SystemExit(f'stale absolute canonical path: {path}')
print('canonical path search: pass')
PY
```

Expected: all Python tests pass and the scoped search prints `canonical path search: pass`.

---

### Task 5: Initialize and verify the pin in CI

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Build/lint jobs remain independent of the submodule.
- Test job checks out this repository with recursive submodules before running canonical generator tests.
- CI verifies the exact gitlink commit before using `modules/Antelope-Ctl/profiles`.

- [ ] **Step 1: Add CI configuration regression coverage**

Extend the existing CI YAML test or add a Python test in `tools/test_generate_device_catalog.py` that parses `.github/workflows/ci.yml` and asserts the test job contains `submodules: recursive`, the pinned hash verification, and a canonical generator check command. The assertion must inspect YAML structure/text already used by repository tests; do not invoke GitHub Actions.

Expected contract:

```yaml
- uses: actions/checkout@v4
  with:
    submodules: recursive
```

and a test-job command equivalent to:

```bash
test "$(git -C modules/Antelope-Ctl rev-parse HEAD)" = "a40fb9ef7f80cae357196bfaa0c5f612fd281ac5"
python3 -m unittest discover -s tools -p 'test_*.py' -v
python3 tools/generate_device_catalog.py --check modules/Antelope-Ctl/profiles --generated src/device/generated.rs --pack-generated src/device/generated_profiles.json
```

- [ ] **Step 2: Run the CI regression before editing workflow**

Run the focused YAML/CI test. Expected: failure because checkout does not initialize submodules and the test job only runs the hermetic subset.

- [ ] **Step 3: Update only the test job**

Add `with: submodules: recursive` to the test job checkout. Add an explicit pinned-revision check, full local Python generator suite, and canonical `--check` command after Python setup and before or alongside existing Cargo checks. Preserve existing Rust test, formatter, and Clippy commands. Do not add an external checkout, `git fetch`, floating branch, or absolute workstation path.

- [ ] **Step 4: Validate workflow structure and local equivalents**

Run:

```bash
python3 -m unittest tools.test_generate_device_catalog -v
python3 - <<'PY'
from pathlib import Path
import yaml
workflow = yaml.safe_load(Path('.github/workflows/ci.yml').read_text())
assert workflow['jobs']['test']['steps']
print('CI YAML: pass')
PY
python3 -m unittest discover -s tools -p 'test_*.py' -v
python3 tools/generate_device_catalog.py --check modules/Antelope-Ctl/profiles --generated src/device/generated.rs --pack-generated src/device/generated_profiles.json
```

Expected: CI parse, Python tests, and canonical artifact check pass locally.

---

### Task 6: Document pinned source ownership and manual updates

**Files:**
- Modify: `README.md`
- Modify: `docs/zen-go-tui.md`
- Modify: `docs/device-support.md`

**Interfaces:**
- Documentation names `modules/Antelope-Ctl` and the pinned commit as build-time canonical source.
- Documentation distinguishes raw canonical JSON, normalized runtime JSON, and saved-state TOML.
- Documentation retains external normalized `--profile-pack` support and runtime independence.

- [ ] **Step 1: Add documentation assertions before prose edits**

Extend the existing Markdown/documentation checks with assertions for:

```text
modules/Antelope-Ctl
mic_models.json
https://github.com/diafebus/Antelope-Ctl.git
a40fb9ef7f80cae357196bfaa0c5f612fd281ac5
manual pointer update
```

Also assert no authored README/device-guide command contains `/home/ryodeushii/repos/Antelope-Ctl`.

- [ ] **Step 2: Run documentation checks and verify missing pinned-source text**

Run the existing Markdown-link and documentation self-lints. Expected: the new pinned-source assertions fail before documentation is updated.

- [ ] **Step 3: Update all three documents**

Document these exact facts:

1. `modules/Antelope-Ctl` is pinned to `a40fb9ef7f80cae357196bfaa0c5f612fd281ac5` from `https://github.com/diafebus/Antelope-Ctl.git`.
2. `profiles/*.json` is canonical hardware evidence; `mic_models.json` is excluded.
3. The generator commands use `modules/Antelope-Ctl/profiles`, emit checked-in `src/device/generated.rs` and `src/device/generated_profiles.json`, and fail instead of falling back when source is missing.
4. Ordinary Cargo/runtime operation uses checked-in artifacts; runtime does not read the submodule.
5. Manual pointer updates fetch/review an exact commit, update the gitlink, regenerate both artifacts, run full Python/Cargo/CI checks, and land in a focused PR. No floating branch or automated update is used.
6. `--profile-pack` accepts normalized external JSON, not raw canonical profiles.
7. `--device` selectors, picker gating, reconnect identity, all five readiness states, Zen Go controls, Orion `Disabled/None` blockers, Discrete status, evidence distinctions, hardware procedure, and saved-state TOML terminology remain accurate.

Use repository-relative commands, for example:

```bash
git submodule update --init --recursive
python3 tools/generate_device_catalog.py \
  --profiles-dir modules/Antelope-Ctl/profiles \
  --output src/device/generated.rs \
  --pack-output src/device/generated_profiles.json
python3 tools/generate_device_catalog.py \
  --check modules/Antelope-Ctl/profiles \
  --generated src/device/generated.rs \
  --pack-generated src/device/generated_profiles.json
```

- [ ] **Step 4: Run documentation checks**

Run the repository's Markdown local-link checker, Python documentation self-lints, and the new pinned-source assertions. Expected: all pass with no stale absolute canonical path in authored docs.

---

### Task 7: Regenerate artifacts and perform release verification

**Files:**
- Modify if generator output changes: `src/device/generated.rs`
- Modify if generator output changes: `src/device/generated_profiles.json`
- Do not modify runtime source solely to integrate the submodule.

**Interfaces:**
- Generated Rust and normalized JSON remain byte-consistent with the pinned source.
- Orion remains `Disabled` / `None`; no generated readiness or protocol behavior is upgraded.

- [ ] **Step 1: Generate from the pinned source**

Run:

```bash
python3 tools/generate_device_catalog.py \
  --profiles-dir modules/Antelope-Ctl/profiles \
  --output src/device/generated.rs \
  --pack-output src/device/generated_profiles.json
```

Inspect the diff. If generated readiness, provenance, or profile content changes beyond the approved pinned source, stop and resolve the source/artifact mismatch before continuing; do not hand-edit generated files.

- [ ] **Step 2: Run artifact and source guards**

Run:

```bash
python3 tools/generate_device_catalog.py \
  --check modules/Antelope-Ctl/profiles \
  --generated src/device/generated.rs \
  --pack-generated src/device/generated_profiles.json
cargo fmt --all -- --check
python3 -m py_compile tools/generate_device_catalog.py tools/test_generate_device_catalog.py
```

Expected: artifact check, formatter, and Python compilation pass.

- [ ] **Step 3: Run full release verification**

Run:

```bash
cargo test --workspace --quiet
python3 -m unittest discover -s tools -p 'test_*.py' -v
cargo clippy -p zen-go-tui -p antelope-protocol --all-targets
git diff --check
git diff --cached --check
```

Also run primary LSP diagnostics over every changed Rust/Python file and `lens_diagnostics` with `mode=all` for edited source files. Record strict `-D warnings` as blocked if only the known cumulative baseline warnings remain; do not claim strict Clippy clean without zero errors.

- [ ] **Step 4: Verify final pin, runtime independence, and repository hygiene**

Run:

```bash
test "$(git -C modules/Antelope-Ctl rev-parse HEAD)" = "a40fb9ef7f80cae357196bfaa0c5f612fd281ac5"
git submodule status -- modules/Antelope-Ctl
! grep -R --exclude-dir=.git --exclude='*.jsonl' '/home/ryodeushii/repos/Antelope-Ctl' tools README.md docs .github
! grep -R --exclude-dir=.git 'ZEN_GO_PID\|HidTransport::open(' src/cli.rs src/main.rs src/runtime.rs src/device/session.rs
test ! -d tools/__pycache__
```

Confirm the worktree has no unrelated staged files, no cache artifacts, no commit was created, and canonical Antelope-Ctl has no staged changes. Confirm final report states no hardware was accessed and Orion remains intentionally disabled.

---

## Plan self-review

- Spec coverage: Tasks 1–2 pin and resolve canonical input; Task 3 migrates local coverage; Task 4 makes CI deterministic with the pin; Task 5 documents ownership and manual advancement; Task 6 regenerates and verifies runtime-independent artifacts.
- Safety coverage: missing-source failure, no runtime submodule dependency, no raw-pack loading, no generated fallback, exact pointer verification, and preserved Orion-disabled policy are explicit.
- No new validation metadata or protocol operations are introduced.
- No commits are required; the Git submodule gitlink is the only intentional index exception if staging is necessary to represent it.
