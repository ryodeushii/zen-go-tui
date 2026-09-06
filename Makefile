SHELL := /bin/sh

PYTHON ?= python3
GIT ?= git
CARGO ?= cargo

MODULE_DIR := modules/Antelope-Ctl
MODULE_GIT := $(GIT) -C $(MODULE_DIR)
PROFILES_DIR := $(MODULE_DIR)/profiles
GENERATOR := tools/generate_device_catalog.py
GENERATED_RUST := src/device/generated.rs
GENERATED_PACK := src/device/generated_profiles.json
GENERATED_LEGACY_CANDIDATES := antelope-protocol/src/legacy_zen_go_candidate_preamp_meters.json

.DEFAULT_GOAL := help

.PHONY: help module-sync module-update generate check-generated release test

help:
	@printf '%s\n' \
		'Available targets:' \
		'  module-sync    Initialize the Antelope-Ctl submodule at the pinned revision.' \
		'  module-update  Fast-forward the clean submodule branch from its configured upstream.' \
		'  generate       Generate checked-in device catalog artifacts.' \
		'  check-generated  Check generated artifacts for drift without writing them.' \
		'  release        Build the locked optimized Rust binary.' \
		'  test           Run the full Cargo workspace test suite.'

module-sync:
	$(GIT) submodule update --init --recursive -- $(MODULE_DIR)

module-update:
	@status="$$($(MODULE_GIT) status --porcelain --untracked-files=all)" || { \
		printf '%s\n' 'module-update: cannot inspect submodule status' >&2; \
		exit 1; \
	}; \
	test -z "$$status" || { \
		printf '%s\n' 'module-update: refusing to update a dirty submodule' >&2; \
		exit 1; \
	}
	@branch="$$($(MODULE_GIT) symbolic-ref --quiet --short HEAD)" && test -n "$$branch" || { \
		printf '%s\n' 'module-update: refusing to update a detached submodule' >&2; \
		exit 1; \
	}
	@upstream="$$($(MODULE_GIT) rev-parse --abbrev-ref --symbolic-full-name '@{upstream}')" && test -n "$$upstream" || { \
		printf '%s\n' 'module-update: submodule branch has no configured upstream' >&2; \
		exit 1; \
	}
	$(MODULE_GIT) pull --ff-only

generate:
	$(PYTHON) $(GENERATOR) \
		--profiles-dir $(PROFILES_DIR) \
		--output $(GENERATED_RUST) \
		--pack-output $(GENERATED_PACK) \
		--legacy-candidate-output $(GENERATED_LEGACY_CANDIDATES)

check-generated:
	$(PYTHON) $(GENERATOR) \
		--check $(PROFILES_DIR) \
		--generated $(GENERATED_RUST) \
		--pack-generated $(GENERATED_PACK) \
		--legacy-candidate-generated $(GENERATED_LEGACY_CANDIDATES)

release:
	$(CARGO) build --release --locked

test:
	$(CARGO) test --workspace
