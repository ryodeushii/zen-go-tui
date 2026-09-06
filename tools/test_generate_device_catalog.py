"""Tests for profile-driven device catalog generation."""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

TOOLS_DIR = Path(__file__).resolve().parent
REPO_ROOT = TOOLS_DIR.parent
GENERATOR = TOOLS_DIR / "generate_device_catalog.py"
ZEN_GO_PROFILE = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "zen_go_sc.json"
ORION_PROFILE = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
sys.path.insert(0, str(TOOLS_DIR))

import generate_device_catalog as generator  # noqa: E402


def normalized_zen_go() -> dict[str, Any]:
    profile = generator.load_profile(
        ZEN_GO_PROFILE, REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"
    )
    return generator._normalized_profile_record(profile)


def normalized_orion() -> dict[str, Any]:
    profile = generator.load_profile(
        ORION_PROFILE, REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"
    )
    return generator._normalized_profile_record(profile)


EXPECTED_PROFILES = {
    "discrete_4_pro_synergy_core.json",
    "discrete_4_synergy_core.json",
    "discrete_8_pro_synergy_core.json",
    "orion_studio_3.json",
    "zen_go_sc.json",
}


def profile_data(name: str, pid: str, *, status: str = "confirmed") -> dict[str, Any]:
    """Return a minimal canonical hardware profile for hermetic generator tests."""

    return {
        "device": {
            "name": name,
            "vid": "0x23e5",
            "pid": pid,
            "status": status,
            "notes": "source mic_models.json",
        },
        "transport": {
            "type": "hid",
            "report_size": 320,
            "out_endpoint": "0x01",
            "in_endpoint": "0x82",
            "poll_interval_ms": 4,
            "uses_numbered_reports": False,
        },
        "frame": {
            "command": {
                "magic_offset": 0,
                "magic": "0x70",
                "opcode_offset": 4,
                "opcode": "0x13",
                "param_id_offset": 16,
                "channel_offset": 17,
                "value_offset": 18,
                "status": "confirmed",
            },
            "state_report": {
                "magic_offset": 0,
                "magic": "0x73",
                "gain_base_offset": 49,
                "status_base_offset": 61,
                "channel_meter_base_offset": 157,
                "channel_meter_notes": "confirmed synthetic physical channel meters",
                "bus_block": {
                    "base_offset": 28,
                    "bytes_per_bus": 3,
                    "status_bits": {"mute": {"mask": "0x04", "shift": 2}},
                },
                "status": "confirmed",
            },
            "routing_command": {
                "magic_offset": 0,
                "opcode_offset": 4,
                "opcode": "0x53",
                "channel_list_offset": 19,
                "channel_stride": 2,
                "destination_channels": {"0": 16, "1": 2},
                "status": "confirmed",
            },
            "link_command": {
                "magic_offset": 0,
                "magic": "0x70",
                "opcode_offset": 4,
                "opcode": "0x14",
                "pair_index_offset": 18,
                "enabled_offset": 19,
                "allowed_values": [0, 1],
                "status": "confirmed",
            },
        },
        "channels": {
            "count": 2,
            "count_confirmed": 2,
            "confirmed_indices": [0, 1],
            "names": ["A1", "A2"],
            "status": "confirmed",
        },
        "buses": {
            "known": {"0": {"name": "monitor", "aliases": ["mon"]}},
            "status": "confirmed",
        },
        "mixer": {"mixes": 2, "channels_per_mix": 16, "status": "confirmed"},
        "runtime_topology": {
            "status": "confirmed",
            "mixer": {"has_master": False},
            "link_domains": [
                {
                    "protocol_space": 3,
                    "kind": "mixer",
                    "pair_count": 8,
                    "status": "confirmed",
                    "evidence": "synthetic generator fixture assumption",
                }
            ],
            "routing_source_domains": [
                {
                    "id": "fixture_sources",
                    "status": "confirmed",
                    "evidence": "synthetic generator fixture assumption",
                    "banks": [{"bank": 0, "index_count": 2}],
                }
            ],
            "routing_groups": [
                {
                    "destination": 0,
                    "name": "mixer_input_assignments",
                    "channel_count": 16,
                    "source_domain": "fixture_sources",
                }
            ],
        },
        "params": {
            "gain": {
                "id": "0x50",
                "type": "int8",
                "status": "confirmed",
                "frame": "command value @18",
                "readback": "state_report offset 49 + channel",
                "range": [0, 75],
            }
        },
        "constraints": {"allowed_opcodes": ["0x13"]},
        "hazards": {},
    }


def write_profile(directory: Path, filename: str, data: dict[str, Any]) -> Path:
    path = directory / filename
    path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    return path


def fixture_profiles() -> tempfile.TemporaryDirectory[str]:
    """Create five hardware profiles plus excluded model data."""

    temporary = tempfile.TemporaryDirectory()
    directory = Path(temporary.name)
    profiles = [
        ("discrete_4_pro_synergy_core.json", "Antelope Discrete 4 Pro Synergy Core", "0xa2bf", "UNCONFIRMED"),
        ("discrete_4_synergy_core.json", "Antelope Discrete 4 Synergy Core", "0xa2be", "UNCONFIRMED"),
        ("discrete_8_pro_synergy_core.json", "Antelope Discrete 8 Pro Synergy Core", "0xa2b5", "confirmed"),
        ("orion_studio_3.json", "Antelope Orion Studio III", "0xa221", "confirmed"),
        ("zen_go_sc.json", "Antelope Zen Go Synergy Core", "0xa015", "confirmed"),
    ]
    for filename, name, pid, status in profiles:
        write_profile(directory, filename, profile_data(name, pid, status=status))
    write_profile(directory, "mic_models.json", {"models": [{"name": "Edge"}]})
    return temporary


class GeneratorTests(unittest.TestCase):
    def test_zen_go_keeps_sparse_safe_queries_explicitly(self) -> None:
        normalized = normalized_zen_go()
        readback = normalized["readback"]

        expected = [
            {"category": 0x01, "index": 0},
            {"category": 0x11, "index": 0},
            {"category": 0x0A, "index": 0},
            {"category": 0x17, "index": 0},
            {"category": 0x18, "index": 0},
            {"category": 0x11, "index": 1},
            {"category": 0x03, "index": 0},
            {"category": 0x03, "index": 1},
            {"category": 0x03, "index": 2},
            {"category": 0x03, "index": 3},
            {"category": 0x03, "index": 4},
            {"category": 0x03, "index": 5},
            {"category": 0x03, "index": 6},
            {"category": 0x03, "index": 7},
            {"category": 0x03, "index": 8},
            {"category": 0x03, "index": 9},
            {"category": 0x0B, "index": 0},
            {"category": 0x16, "index": 0},
            {"category": 0x0A, "index": 0},
            {"category": 0x04, "index": 0},
            {"category": 0x0B, "index": 3},
            {"category": 0x04, "index": 1},
            {"category": 0x0B, "index": 3},
            {"category": 0x04, "index": 2},
            {"category": 0x0B, "index": 3},
            {"category": 0x04, "index": 3},
            {"category": 0x0B, "index": 3},
            {"category": 0x15, "index": 0},
            {"category": 0x19, "index": 0},
            {"category": 0x19, "index": 1},
            {"category": 0x07, "index": 0x27},
            {"category": 0x07, "index": 0x2C},
            {"category": 0x07, "index": 0x09},
            {"category": 0x07, "index": 0x14},
            {"category": 0x07, "index": 0x4C},
            {"category": 0x19, "index": 2},
            {"category": 0x19, "index": 3},
            {"category": 0x19, "index": 4},
            {"category": 0x19, "index": 5},
            {"category": 0x19, "index": 6},
            {"category": 0x19, "index": 7},
            {"category": 0x19, "index": 8},
            {"category": 0x19, "index": 9},
            {"category": 0x19, "index": 10},
            {"category": 0x19, "index": 11},
            {"category": 0x0B, "index": 4},
            {"category": 0x12, "index": 0},
        ]
        self.assertEqual(readback["category_counts"], [])
        self.assertEqual(readback["safe_queries"], expected)
        self.assertEqual(len(readback["safe_queries"]), 47)
        self.assertEqual(readback["safe_queries"].count({"category": 0x0A, "index": 0}), 2)
        self.assertEqual(readback["safe_queries"].count({"category": 0x0B, "index": 3}), 4)
        self.assertIn({"category": 0x0B, "index": 3}, readback["safe_queries"])
        self.assertTrue(all(item in readback["safe_queries"] for item in [
            *[{"category": 0x03, "index": index} for index in range(10)],
            *[{"category": 0x19, "index": index} for index in range(12)],
        ]))

    def test_orion_profile_declares_four_single_lane_mix_master_meters(self) -> None:
        profile = normalized_orion()
        mappings = profile["meter_mappings"]

        self.assertEqual(
            [
                (item["frame_id"], item["target"], item["target_index"], item["lane"], item["offset"])
                for item in mappings
            ],
            [
                ("state_report", "mix_master", 0, 0, 157),
                ("state_report", "mix_master", 1, 0, 158),
                ("state_report", "mix_master", 2, 0, 159),
                ("state_report", "mix_master", 3, 0, 160),
            ],
        )
        self.assertTrue(all(item["status"] == "observed" for item in mappings))
        self.assertTrue(all("mono" in item["evidence"] for item in mappings))
        self.assertTrue(all("no L/R" in item["evidence"] for item in mappings))
        self.assertEqual([mixer["mix_index"] for mixer in profile["mixers"]], [0, 1, 2, 3])
        self.assertTrue(all(mixer["has_master"] for mixer in profile["mixers"]))
        self.assertFalse(any(item["target"] == "physical_output" for item in mappings))

    def test_zen_go_profile_declares_capture_scoped_mixer_layouts(self) -> None:
        layouts = normalized_zen_go()["readback"]["layouts"]

        q040 = next(layout for layout in layouts if layout["category"] == 0x04 and layout["index"] == 0)
        q041 = next(layout for layout in layouts if layout["category"] == 0x04 and layout["index"] == 1)
        q180 = next(layout for layout in layouts if layout["category"] == 0x18 and layout["index"] == 0)

        self.assertEqual(
            (q040["body_size"], q040["record_count"], q040["record_stride"]),
            (34, 16, 2),
        )
        self.assertEqual((q040["level_offset"], q040["state_offset"], q040["surface"]), (2, 3, 0))
        self.assertEqual(q041["surface"], 1)
        self.assertEqual(
            (q180["body_size"], q180["record_count"], q180["record_stride"]),
            (64, 32, 2),
        )
        self.assertEqual(q180["supported_fields"], ["solo"])

    def test_zen_go_profile_declares_candidate_preamp_meter_offsets(self) -> None:
        meters = normalized_zen_go()["state_report"]["candidate_preamp_meters"]

        self.assertEqual(
            [(item["input_index"], item["offset"]) for item in meters],
            [(0, 0xCE), (1, 0xCF)],
        )
        operations = next(
            frame for frame in normalized_zen_go()["frames"] if frame["id"] == "state_report"
        )["operations"]
        self.assertEqual(
            [
                (operation["input_index"], operation["offset"])
                for operation in operations
                if operation.get("field", "").startswith("candidate_preamp_meter_")
            ],
            [(0, 0xDE), (1, 0xDF)],
        )
        self.assertTrue(all(item["status"] == "observed" for item in meters))
        self.assertTrue(all(item["confidence"] == "provisional" for item in meters))
        self.assertTrue(all("mixed-signal" in item["caveat"] for item in meters))
        self.assertTrue(
            all(item["raw_value_ranges"] == [[0x01, 0x49], [0x52, 0x52]] for item in meters)
        )

    def test_candidate_preamp_meter_schema_rejects_unbounded_or_overlapping_values(self) -> None:
        data = profile_data("Test", "0xa001")
        data["frame"]["state_report"]["candidate_preamp_meters"] = [
            {
                "input_index": 0,
                "offset": "0x130",
                "status": "observed",
                "confidence": "provisional",
                "caveat": "synthetic candidate",
                "raw_value_ranges": [["0x01", "0x49"], ["0x40", "0x52"]],
            }
        ]

        with self.assertRaisesRegex(generator.ProfileError, "falls outside the payload"):
            generator.normalize_profile(data)

        data["frame"]["state_report"]["candidate_preamp_meters"][0]["offset"] = "0x10"
        with self.assertRaisesRegex(generator.ProfileError, "overlaps or is out of order"):
            generator.normalize_profile(data)

    def test_zen_go_candidate_meters_reject_reindexed_physical_inputs(self) -> None:
        data = profile_data("Zen Go", "0xa015")
        data["channels"]["count"] = 3
        data["channels"]["count_confirmed"] = 3
        data["channels"]["confirmed_indices"] = [0, 1, 2]
        data["channels"]["names"] = ["A1", "A2", "A3"]
        data["frame"]["state_report"]["candidate_preamp_meters"] = [
            {
                "input_index": 2,
                "offset": "0xd0",
                "status": "observed",
                "confidence": "provisional",
                "caveat": "synthetic candidate",
                "raw_value_ranges": [["0x00", "0x00"]],
            }
        ]

        with self.assertRaisesRegex(generator.ProfileError, "physical input indices 0 and 1"):
            generator.normalize_profile(data)

    def test_legacy_zen_go_candidate_artifact_matches_normalized_profile(self) -> None:
        profiles = generator._load_profiles(generator.DEFAULT_PROFILES_DIR)
        artifact = json.loads(generator.render_legacy_zen_go_candidate_preamp_meters(profiles))

        self.assertEqual(
            artifact,
            normalized_zen_go()["state_report"]["candidate_preamp_meters"],
        )

    def test_zen_go_profile_declares_payload_relative_mix_master_meter_lanes(self) -> None:
        mappings = normalized_zen_go()["meter_mappings"]

        self.assertEqual(
            [
                (item["frame_id"], item["target"], item["target_index"], item["lane"], item["offset"])
                for item in mappings
            ],
            [
                ("state_report", "mix_master", 0, 0, 0xEA),
                ("state_report", "mix_master", 0, 1, 0xEB),
                ("state_report", "mix_master", 1, 0, 0xEE),
                ("state_report", "mix_master", 1, 1, 0xEF),
            ],
        )
        self.assertTrue(all(item["status"] == "observed" for item in mappings))
        self.assertTrue(all(item["target"] == "mix_master" for item in mappings))

    def test_meter_mapping_rejects_competing_target_lane_across_frames(self) -> None:
        data = profile_data("Test", "0xa001")
        data["frame"]["state_report"]["meter_mappings"] = [
            {
                "target": "mix_master",
                "target_index": 0,
                "lane": 0,
                "payload_offset": "0xda",
                "status": "observed",
                "evidence": "synthetic meter lane",
            }
        ]
        data["frame"]["meter_report"] = {
            "magic_offset": 0,
            "magic": "0x75",
            "meter_mappings": [
                {
                    "target": "mix_master",
                    "target_index": 0,
                    "lane": 0,
                    "payload_offset": "0xda",
                    "status": "observed",
                    "evidence": "competing synthetic meter lane",
                }
            ],
        }

        with self.assertRaisesRegex(generator.ProfileError, "target lane is declared more than once"):
            generator.normalize_profile(data)

    def test_zen_go_profile_declares_attenuation_fader_domain(self) -> None:
        fader = normalized_zen_go()["mixers"][0]["fader"]

        self.assertEqual(
            fader,
            {"min": 0, "max": 90, "direction": "attenuation", "unity": 0},
        )

    def test_zen_go_profile_declares_attenuation_output_domain(self) -> None:
        bus_level = next(
            param for param in normalized_zen_go()["params"] if param["name"] == "bus_level"
        )

        self.assertEqual(bus_level["direction"], "attenuation")
        self.assertEqual(bus_level["unity"], 0)
        self.assertEqual(
            json.loads(bus_level["metadata"])["encoding"],
            "raw = dB attenuation below unity: 0 = 0 dB, 96 = -96 dB",
        )

    def test_zen_go_profile_exposes_capture_confirmed_output_dim(self) -> None:
        params = {param["name"]: param for param in normalized_zen_go()["params"]}

        self.assertIn("bus_dim", params)
        self.assertNotIn("bus_param_0x66", params)
        self.assertEqual(params["bus_dim"]["id"], 0x66)
        self.assertEqual(params["bus_dim"]["status"], "confirmed")
        self.assertEqual(params["bus_dim"]["applies_to"], "buses")

    def test_zen_go_profile_preserves_confirmed_ui_labels(self) -> None:
        profile = normalized_zen_go()

        self.assertEqual(
            [
                (input_["index"], input_["name"])
                for input_ in profile["inputs"]
                if input_["space"] == "physical_inputs"
            ],
            [(0, "Preamp 1"), (1, "Preamp 2")],
        )
        self.assertEqual(
            [(output["id"], output["name"]) for output in profile["outputs"]],
            [(0, "Monitor"), (1, "HP1"), (2, "HP2")],
        )
        self.assertEqual(
            [(mixer["mix_index"], mixer["name"]) for mixer in profile["mixers"]],
            [(0, "MIX 1 / Monitor-HP1"), (1, "MIX 2 / HP2")],
        )

    def test_parameter_scalar_domain_requires_complete_valid_metadata(self) -> None:
        for partial in ({"direction": "attenuation"}, {"unity": 0}):
            with self.subTest(partial=partial):
                data = profile_data("Test", "0xa001")
                data["params"]["gain"].update(partial)
                with self.assertRaisesRegex(generator.ProfileError, "direction and unity"):
                    generator._build_params(generator.normalize_profile(data))

        data = profile_data("Test", "0xa001")
        data["params"]["gain"].update({"direction": "attenuation", "unity": 76})
        with self.assertRaisesRegex(generator.ProfileError, "unity.*range"):
            generator._build_params(generator.normalize_profile(data))

    def test_readback_declarations_validate_sparse_query_and_layout_geometry(self) -> None:
        def profile_with_readback(safe_queries: list[dict[str, Any]], layout: dict[str, Any]) -> dict[str, Any]:
            data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
            data["frame"]["readback"] = {
                "request_magic": "0x74",
                "subcmd": "0x10",
                "response_magic": "0x75",
                "response_discriminator_offset": 1,
                "response_discriminator": 0,
                "category_offset": 8,
                "index_offset": 12,
                "data_offset": 16,
                "category_counts": {},
                "safe_queries": safe_queries,
                "layouts": [layout],
            }
            return data

        with self.assertRaisesRegex(generator.ProfileError, r"safe_queries\[0\]\.category"):
            generator.normalize_profile(
                profile_with_readback(
                    [{"category": 0x100, "index": 0}],
                    {"category": 0x18, "index": 0, "kind": "observed", "body_size": 64, "record_count": 32, "record_stride": 2, "status": "observed"},
                )
            )
        with self.assertRaisesRegex(generator.ProfileError, r"is not in frame\.readback\.safe_queries"):
            generator.normalize_profile(
                profile_with_readback(
                    [{"category": 0x18, "index": 0}],
                    {"category": 0x04, "index": 0, "kind": "mixer", "body_size": 34, "record_count": 16, "record_stride": 2, "status": "observed"},
                )
            )
        with self.assertRaisesRegex(generator.ProfileError, r"record_count \* record_stride exceeds body_size"):
            generator.normalize_profile(
                profile_with_readback(
                    [{"category": 0x18, "index": 0}],
                    {"category": 0x18, "index": 0, "kind": "observed", "body_size": 1, "record_count": 32, "record_stride": 2, "status": "observed"},
                )
            )

    def test_explicit_sparse_queries_escape_empty_bounds_and_derived_queries_are_emitted(self) -> None:
        def readback(category_counts: dict[str, int], **extra: Any) -> dict[str, Any]:
            value = {
                "request_magic": "0x74",
                "subcmd": "0x10",
                "response_magic": "0x75",
                "response_discriminator_offset": 1,
                "response_discriminator": 0,
                "category_offset": 8,
                "index_offset": 12,
                "data_offset": 16,
                "category_counts": category_counts,
                **extra,
            }
            return value

        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        data["frame"]["readback"] = readback(
            {},
            safe_queries=[{"category": 0x04, "index": 3}],
            layouts=[{
                "category": 0x04,
                "index": 3,
                "kind": "mixer_state",
                "body_size": 34,
                "record_count": 16,
                "record_stride": 2,
                "level_offset": 2,
                "state_offset": 3,
                "status": "observed",
            }],
        )
        normalized = generator._normalized_profile_record(
            generator.normalize_profile(data, path=Path("zen_go_sc.json"))
        )
        assert normalized["readback"]["safe_queries"] == [{"category": 4, "index": 3}]
        data["frame"]["readback"] = readback(
            {"0x04": 4},
            layouts=[{
                "category": 0x04,
                "index": 0,
                "kind": "mixer_state",
                "body_size": 34,
                "record_count": 16,
                "record_stride": 2,
                "level_offset": 2,
                "state_offset": 3,
                "status": "observed",
            }],
        )
        normalized = generator._normalized_profile_record(
            generator.normalize_profile(data, path=Path("zen_go_sc.json"))
        )
        assert normalized["readback"]["safe_queries"] == [
            {"category": 4, "index": index} for index in range(4)
        ]

    def test_fader_direction_is_normalized_for_generated_rust(self) -> None:
        data = json.loads((REPO_ROOT / "modules/Antelope-Ctl/profiles/zen_go_sc.json").read_text())
        data["mixer"]["fader"]["direction"] = "  ATTENUATION  "
        profile = generator.normalize_profile(data, path=Path("zen_go_sc.json"))
        assert generator._normalized_profile_record(profile)["mixers"][0]["fader"]["direction"] == "attenuation"

    def test_readback_layout_requires_level_and_state_offsets(self) -> None:
        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        data["frame"]["readback"] = {
            "request_magic": "0x74",
            "subcmd": "0x10",
            "response_magic": "0x75",
            "response_discriminator_offset": 1,
            "response_discriminator": 0,
            "category_offset": 8,
            "index_offset": 12,
            "data_offset": 16,
            "category_counts": {},
            "safe_queries": [{"category": 0x04, "index": 0}],
            "layouts": [{
                "category": 0x04,
                "index": 0,
                "kind": "mixer_state",
                "body_size": 34,
                "record_count": 16,
                "record_stride": 2,
                "status": "observed",
            }],
        }
        with self.assertRaisesRegex(generator.ProfileError, r"level_offset is required"):
            generator.normalize_profile(data)

    def test_readback_layout_offsets_and_surface_stride_fit_body_geometry(self) -> None:
        def layout_profile(layout: dict[str, Any]) -> dict[str, Any]:
            data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
            data["frame"]["readback"] = {
                "request_magic": "0x74",
                "subcmd": "0x10",
                "response_magic": "0x75",
                "response_discriminator_offset": 1,
                "response_discriminator": 0,
                "category_offset": 8,
                "index_offset": 12,
                "data_offset": 16,
                "category_counts": {},
                "safe_queries": [{"category": "0x04", "index": 0}],
                "layouts": [layout],
            }
            return data

        for field in ("level_offset", "state_offset"):
            with self.subTest(field=field):
                layout = {
                    "category": "0x04",
                    "index": 0,
                    "kind": "mixer_state",
                    "body_size": 34,
                    "record_count": 16,
                    "record_stride": 2,
                    "level_offset": 2,
                    "state_offset": 3,
                    "status": "capture-confirmed",
                }
                layout[field] = 34
                with self.assertRaisesRegex(generator.ProfileError, field):
                    generator.normalize_profile(layout_profile(layout))

        layout = {
            "category": "0x04",
            "index": 0,
            "kind": "mixer_state",
            "body_size": 64,
            "record_count": 32,
            "record_stride": 2,
            "level_offset": 0,
            "state_offset": 1,
            "surface_stride": 33,
            "status": "observed",
        }
        with self.assertRaisesRegex(generator.ProfileError, "surface_stride"):
            generator.normalize_profile(layout_profile(layout))

    def test_cli_defaults_to_pinned_repository_source(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            generated = root / "generated.rs"
            pack_generated = root / "generated_profiles.json"
            legacy_candidates = root / "legacy_zen_go_candidate_preamp_meters.json"
            result = subprocess.run(
                [
                    sys.executable,
                    str(GENERATOR),
                    "--output",
                    str(generated),
                    "--pack-output",
                    str(pack_generated),
                    "--legacy-candidate-output",
                    str(legacy_candidates),
                ],
                cwd=root,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                generated.read_bytes(),
                (REPO_ROOT / "src/device/generated.rs").read_bytes(),
            )
            self.assertEqual(
                pack_generated.read_bytes(),
                (REPO_ROOT / "src/device/generated_profiles.json").read_bytes(),
            )
            self.assertEqual(
                legacy_candidates.read_bytes(),
                (
                    REPO_ROOT
                    / "antelope-protocol/src/legacy_zen_go_candidate_preamp_meters.json"
                ).read_bytes(),
            )
            self.assertEqual(
                generator.DEFAULT_PROFILES_DIR,
                REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles",
            )
            self.assertNotIn(
                "mic_models.json",
                {
                    profile.path.name
                    for profile in generator.discover_profiles(generator.DEFAULT_PROFILES_DIR)
                },
            )

    def test_discovers_hardware_profiles_but_excludes_mic_models(self) -> None:
        with fixture_profiles() as temporary:
            profiles = generator.discover_profiles(temporary)
            self.assertEqual({profile.path.name for profile in profiles}, EXPECTED_PROFILES)
            self.assertNotIn("mic_models.json", {profile.path.name for profile in profiles})

    def test_parses_hex_and_decimal_integer_values(self) -> None:
        self.assertEqual(generator.parse_int("0x23e5", "vid"), 0x23E5)
        self.assertEqual(generator.parse_int("0X01", "endpoint"), 1)
        self.assertEqual(generator.parse_int("320", "report_size"), 320)
        self.assertEqual(generator.parse_int(4, "poll_interval_ms"), 4)

        with self.assertRaises(generator.ProfileError):
            generator.parse_int("0xnot-an-int", "pid")
        with self.assertRaises(generator.ProfileError):
            generator.parse_int(True, "pid")

    def test_upstream_sc_filenames_keep_stable_runtime_ids(self) -> None:
        profiles_dir = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"
        discrete = generator.load_profile(profiles_dir / "discrete_4_pro_sc.json", profiles_dir)
        self.assertEqual(generator._profile_id(discrete), "discrete_4_pro")

        orion_data = profile_data("Antelope Orion Studio III", "0xa221")
        orion = generator.normalize_profile(orion_data, path=Path("orion_studio_sc.json"))
        self.assertTrue(generator._is_orion(orion))
        self.assertEqual(generator._profile_id(orion), "orion_studio_3")

    def test_records_source_sha256_and_provenance(self) -> None:
        with fixture_profiles() as temporary:
            profile_path = Path(temporary) / "orion_studio_3.json"
            expected_hash = hashlib.sha256(profile_path.read_bytes()).hexdigest()
            profile = generator.load_profile(profile_path, temporary)
            self.assertEqual(profile.provenance.source_sha256, expected_hash)
            self.assertEqual(profile.provenance.generator_version, generator.GENERATOR_VERSION)
            self.assertTrue(profile.provenance.source_path.endswith("profiles/orion_studio_3.json"))
            output = generator.generate_catalog(temporary)
            self.assertIn(expected_hash, output)
            self.assertIn(generator.GENERATOR_VERSION, output)

    def test_readiness_requires_registered_driver_and_complete_profile(self) -> None:
        with fixture_profiles() as temporary:
            profiles = {profile.path.name: profile for profile in generator.discover_profiles(temporary)}
            zen = generator.load_profile(profiles["zen_go_sc.json"].path, temporary)
            orion = generator.load_profile(
                REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json",
                REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles",
            )
            discrete_8 = generator.load_profile(profiles["discrete_8_pro_synergy_core.json"].path, temporary)
            discrete_4 = generator.load_profile(profiles["discrete_4_synergy_core.json"].path, temporary)

            self.assertEqual(generator.classify_readiness(zen), generator.Readiness.SUPPORTED)
            self.assertEqual(generator.classify_readiness(orion), generator.Readiness.SUPPORTED)
            self.assertEqual(generator.classify_readiness(discrete_8), generator.Readiness.PARTIAL)
            self.assertEqual(generator.classify_readiness(discrete_4), generator.Readiness.UNVERIFIED)

            incomplete_orion_data = profile_data("Antelope Orion Studio III", "0xa221")
            incomplete_orion_data["transport"]["report_size"] = None
            incomplete_orion_data["frame"] = {}
            incomplete_orion = generator.normalize_profile(incomplete_orion_data)
            self.assertEqual(generator.classify_readiness(incomplete_orion), generator.Readiness.DISABLED)

    def test_orion_explicit_numbered_reports_is_unrepresentable_blocker(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        data["transport"]["uses_numbered_reports"] = True
        profile = generator.normalize_profile(data, path=path)

        blockers = generator.orion_readiness_blockers(profile)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)
        self.assertTrue(any("unrepresentable" in blocker for blocker in blockers))

    def test_orion_readiness_reports_confirmed_transport_blocker(self) -> None:
        canonical = generator.load_profile(
            REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json",
            REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles",
        )
        blockers = generator.orion_readiness_blockers(canonical)
        self.assertNotIn("transport.uses_numbered_reports is unconfirmed", blockers)
        self.assertEqual(generator.classify_readiness(canonical), generator.Readiness.SUPPORTED)

    def test_builtin_catalog_uses_effective_orion_framing(self) -> None:
        canonical = generator.load_profile(
            REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json",
            REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles",
        )
        rendered = generator.render_catalog([canonical])
        self.assertIn("uses_numbered_reports: Some(false)", rendered)

    def test_orion_normal_support_policy(self) -> None:
        profiles_dir = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"
        canonical_path = profiles_dir / "orion_studio_sc.json"
        canonical = generator.load_profile(canonical_path, profiles_dir)
        normalized = json.loads(generator.render_profile_pack([canonical]))["profiles"][0]
        blockers = generator.orion_readiness_blockers(canonical)

        self.assertEqual(normalized["readiness"], "supported")
        self.assertNotIn("frame.state_report lacks confirmed physical channel meter mapping", blockers)
        self.assertNotIn("frame.readback.category_counts does not match confirmed finite bounds", blockers)
        self.assertEqual(normalized["driver_kind"], "profile")
        self.assertEqual(
            normalized["support_reason"],
            "validated source-backed profile; assumes unnumbered HID reports pending hardware verification",
        )
        state_frame = next(frame for frame in normalized["frames"] if frame["id"] == "state_report")
        self.assertEqual(state_frame["status"], "confirmed")
        self.assertFalse(
            any(
                operation.get("index_field") == "physical_meter"
                for operation in state_frame["operations"]
            )
        )
        self.assertFalse(
            any(
                decoder["frame_id"] == "meter_report"
                and generator._status_variant(decoder["status"]) == "Confirmed"
                for decoder in normalized["decoders"]
            )
        )
        self.assertFalse(normalized["transport"]["uses_numbered_reports"])
        self.assertNotIn("transport.uses_numbered_reports is unconfirmed", blockers)
        self.assertNotIn("physical/ADAT link action has ambiguous space=0 semantics", blockers)
        self.assertEqual(normalized["identity"]["status"], "unknown")
        self.assertEqual(
            normalized["provenance"]["source_sha256"],
            generator.hashlib.sha256(canonical_path.read_bytes()).hexdigest(),
        )

    def test_orion_source_only_params_are_non_actionable(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        profile = generator.load_profile(path, REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles")
        normalized = json.loads(generator.render_profile_pack([profile]))["profiles"][0]
        params = {param["name"]: param for param in normalized["params"]}
        for name in (
            "oscillator",
            "surround_monitor",
            "surround_speaker",
            "dc_coupling",
            "routing_batch_marker",
        ):
            self.assertIsNone(params[name]["id"])
            self.assertIn("id", json.loads(params[name]["metadata"]))

    def test_orion_actionable_params_have_complete_runtime_shape(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        profile = generator.load_profile(path, REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles")
        normalized = json.loads(generator.render_profile_pack([profile]))["profiles"][0]
        params = {param["name"]: param for param in normalized["params"]}
        actionable = {
            "input_mode",
            "gain",
            "phantom",
            "phase_invert",
            "bus_level",
            "bus_dim",
            "bus_mute",
            "bus_mono",
            "sample_rate",
            "screen_brightness",
            "adat_gain",
            "talkback_button",
            "talkback_source",
            "talkback_gain",
            "spdif_gain",
        }
        for name in actionable:
            parameter = params[name]
            self.assertIsNotNone(parameter["id"], name)
            self.assertTrue(parameter["applies_to"].strip(), name)
            self.assertTrue(parameter["frame"]["text"].strip(), name)
            self.assertTrue(parameter["frame"]["offsets"], name)
            self.assertTrue(parameter["readback"]["text"].strip(), name)
            self.assertTrue(parameter["readback"]["offsets"], name)
        for name in (
            "output_trim",
            "routing",
            "oscillator",
            "surround_monitor",
            "dc_coupling",
            "talkback_dest_assign",
        ):
            self.assertIsNone(params[name]["id"], name)

    def test_orion_identity_requires_profile_stem(self) -> None:
        data = json.loads(
            (REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json").read_text()
        )
        data["frame"]["routing_command"]["source_banks"].pop("0x02", None)
        profile = generator.normalize_profile(data, path=Path("same_vid_pid.json"))
        self.assertFalse(generator._is_orion(profile))
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

    def test_orion_frame_promotion_matches_exact_identifiers(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        for value in data["params"].values():
            if not isinstance(value, dict):
                continue
            reference = str(value.get("frame", value.get("runtime_frame", "")))
            if generator._reference_mentions_frame(reference, "command"):
                value["status"] = "unconfirmed"
        data["frame"]["command"]["runtime_status"] = "unconfirmed"
        profile = generator.normalize_profile(data, path=path)
        self.assertNotEqual(
            generator._status_variant(
                generator._effective_frame_status(profile, "command", profile.frame["command"])
            ),
            "Confirmed",
        )

    def test_orion_auraverb_and_micmodeling_never_promote(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        for frame_id in ("auraverb_command", "micmodeling_command"):
            data["frame"][frame_id]["runtime_status"] = "confirmed"
            data["frame"][frame_id]["notes"] = "future confirmed mapping"
            profile = generator.normalize_profile(data, path=path)
            self.assertNotEqual(
                generator._status_variant(
                    generator._effective_frame_status(profile, frame_id, profile.frame[frame_id])
                ),
                "Confirmed",
                frame_id,
            )

    def test_orion_inferred_semantics_are_unique_without_renaming_lookups(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        profile = generator.normalize_profile(data, path=path)
        operations = generator._frame_operations(profile, "state_report", profile.frame["state_report"])
        for kind in ("scalar", "bit_field"):
            names = [operation["field"] for operation in operations if operation["op"] == kind]
            self.assertEqual(len(names), len(set(names)), kind)
        names = {operation.get("field") for operation in operations}
        self.assertTrue({"gain_base", "status_base", "adat_gain_base", "spdif_gain_base"} <= names)
        self.assertNotIn("physical_meter", {operation.get("index_field") for operation in operations})
        self.assertIn("mask__2", names)
        self.assertIn("byte__2", names)

    def test_orion_inferred_scalar_bit_field_collisions_share_namespace(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        profile = generator.normalize_profile(json.loads(path.read_text()), path=path)
        operations = generator._disambiguate_orion_inferred_semantics(
            profile,
            "state_report",
            [
                {"op": "scalar", "field": "shared", "offset": 1, "width": 1},
                {"op": "bit_field", "field": "shared", "offset": 2, "mask": 1, "shift": 0},
                {"op": "scalar", "field": "shared__2", "offset": 3, "width": 1},
                {"op": "bit_field", "field": "shared", "offset": 4, "mask": 2, "shift": 1},
            ],
        )
        self.assertEqual(
            [operation["field"] for operation in operations],
            ["shared", "shared__2", "shared__2__2", "shared__3"],
        )

    def test_orion_parameter_qualifiers_block_referenced_promotion(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        for name, value in data["params"].items():
            if not isinstance(value, dict):
                continue
            reference = str(value.get("frame", value.get("runtime_frame", "")))
            if generator._reference_mentions_frame(reference, "global_command"):
                value["status"] = "unconfirmed"
        data["params"]["sample_rate"]["status"] = "confirmed"
        data["params"]["sample_rate"]["notes"] = "superseded evidence"
        data["frame"]["global_command"]["runtime_status"] = "unconfirmed"
        profile = generator.normalize_profile(data, path=path)
        self.assertNotEqual(
            generator._status_variant(
                generator._effective_frame_status(profile, "global_command", profile.frame["global_command"])
            ),
            "Confirmed",
        )

    def test_orion_geometry_blocker_is_emitted_once(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        data["frame"]["command"]["runtime_operations"] = [
            {"op": "scalar", "field": "bad", "offset": 320, "width": 1, "endian": "not_applicable"}
        ]
        profile = generator.normalize_profile(data, path=path)
        blockers = generator.orion_readiness_blockers(profile)
        self.assertEqual(sum("frame.command operation geometry exceeds" in blocker for blocker in blockers), 1)

    def test_orion_rejects_superseded_evidence_and_out_of_bounds_operation(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        data["frame"]["command"]["notes"] = "superseded mapping"
        profile = generator.normalize_profile(data, path=path)
        self.assertTrue(any("frame.command" in blocker for blocker in generator.orion_readiness_blockers(profile)))

        data = json.loads(path.read_text())
        data["frame"]["command"]["runtime_operations"] = [
            {"op": "scalar", "field": "bad", "offset": 320, "width": 1, "endian": "not_applicable"}
        ]
        profile = generator.normalize_profile(data, path=path)
        self.assertTrue(any("operation geometry exceeds" in blocker for blocker in generator.orion_readiness_blockers(profile)))

    def test_orion_missing_physical_meter_is_capability_limit_not_readiness_blocker(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        data["channels"].pop("confirmed_indices")
        profile = generator.normalize_profile(data, path=path)
        blockers = generator.orion_readiness_blockers(profile)
        self.assertNotIn("frame.state_report lacks confirmed physical channel meter mapping", blockers)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.SUPPORTED)

    def test_orion_disproven_physical_meter_is_not_emitted(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        profile = generator.normalize_profile(data, path=path)
        normalized = generator._normalized_profile_record(profile)

        operations_by_frame = {
            frame["id"]: frame["operations"] for frame in normalized["frames"]
        }
        state_meters = [
            operation
            for operation in operations_by_frame["state_report"]
            if operation.get("op") == "indexed"
            and operation.get("index_field") == "physical_meter"
        ]
        self.assertEqual(state_meters, [])
        leaked_frames = {
            frame_id
            for frame_id, operations in operations_by_frame.items()
            if frame_id != "state_report"
            and any(operation.get("index_field") == "physical_meter" for operation in operations)
        }
        self.assertEqual(leaked_frames, set())

    def test_orion_meter_omits_disproven_channel_mapping(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        data["frame"]["meter_report"]["channel_meter_stride"] = 1
        profile = generator.normalize_profile(data, path=path)
        normalized = generator._normalized_profile_record(profile)
        meter = next(frame for frame in normalized["frames"] if frame["id"] == "meter_report")
        self.assertIn({"op": "fixed_byte", "offset": 0, "value": 0x75}, meter["operations"])
        self.assertFalse(
            any(
                operation.get("field", "").startswith("channel_meter")
                or operation.get("index_field") in {"channel_meter", "physical_meter"}
                for operation in meter["operations"]
            )
        )
        self.assertIn("channel_meter_base_offset", meter["metadata"])

    def test_orion_rejects_out_of_bounds_obsolete_meter_operation(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        data["frame"]["meter_report"]["channel_meter_base_offset"] = 320
        data["frame"]["meter_report"]["channel_meter_stride"] = 1
        profile = generator.normalize_profile(data, path=path)

        blockers = generator.orion_readiness_blockers(profile)
        self.assertIn("frame.meter_report operation geometry exceeds report bounds", blockers)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

        normalized = generator._normalized_profile_record(profile)
        meter = next(frame for frame in normalized["frames"] if frame["id"] == "meter_report")
        self.assertTrue(
            any(
                operation.get("op") == "indexed"
                and operation.get("index_field") == "channel_meter"
                for operation in meter["operations"]
            )
        )

    def test_orion_omits_mixer_link_domains_outside_protocol_space_three(self) -> None:
        data = profile_data("Antelope Orion Studio III", "0xa221")
        data["runtime_topology"]["link_domains"] = [
            {
                "protocol_space": 2,
                "kind": "mixer",
                "pair_count": 8,
                "status": "confirmed",
                "evidence": "confirmed alternate mixer space",
            },
            {
                "protocol_space": 3,
                "kind": "mixer",
                "pair_count": 8,
                "status": "confirmed",
                "evidence": "confirmed protocol mixer space",
            },
        ]
        profile = generator.normalize_profile(data, path=Path("orion_studio_3.json"))
        self.assertEqual(
            [
                (domain["protocol_space"], domain["kind"])
                for domain in generator._build_link_domains(profile)
            ],
            [(3, "mixer")],
        )

    def test_orion_skipped_physical_link_validates_protocol_space(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        data["runtime_topology"] = {
            "status": "confirmed",
            "mixer": {"has_master": False},
            "routing_groups": [],
            "routing_source_domains": [],
            "link_domains": [{
                "protocol_space": 256,
                "kind": "physical",
                "pair_count": 6,
                "status": "confirmed",
                "evidence": "confirmed physical links",
            }],
        }
        with self.assertRaises(generator.ProfileError):
            generator.normalize_profile(data, path=path)

    def test_orion_skipped_adat_link_validates_pair_count(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        data["runtime_topology"] = {
            "status": "confirmed",
            "mixer": {"has_master": False},
            "routing_groups": [],
            "routing_source_domains": [],
            "link_domains": [{
                "protocol_space": 0,
                "kind": "adat",
                "pair_count": 0,
                "status": "confirmed",
                "evidence": "confirmed ADAT links",
            }],
        }
        with self.assertRaises(generator.ProfileError):
            generator.normalize_profile(data, path=path)

    def test_orion_confirmed_physical_adat_link_capabilities_are_actionable_blocker(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        data["runtime_topology"] = {
            "status": "confirmed",
            "mixer": {"has_master": True},
            "routing_groups": [],
            "routing_source_domains": [],
            "input_spaces": [{
                "space": "physical_inputs",
                "controls": [{"kind": "link", "parameter": "channel_link"}],
            }],
        }
        profile = generator.normalize_profile(data, path=path)
        blockers = generator.orion_readiness_blockers(profile)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)
        self.assertTrue(any("physical/ADAT link action" in blocker for blocker in blockers))

    def test_orion_physical_link_domain_is_actionable_blocker(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        data["runtime_topology"] = {
            "status": "confirmed",
            "mixer": {"has_master": False},
            "routing_groups": [],
            "routing_source_domains": [],
            "link_domains": [{
                "protocol_space": 0,
                "kind": "physical",
                "pair_count": 6,
                "status": "confirmed",
                "evidence": "confirmed physical links",
            }],
        }
        profile = generator.normalize_profile(data, path=path)
        blockers = generator.orion_readiness_blockers(profile)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)
        self.assertTrue(any("physical/ADAT link action" in blocker for blocker in blockers))

    def test_orion_combined_physical_adat_space_zero_is_disabled_not_rejected(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        data = json.loads(path.read_text())
        data["runtime_topology"] = {
            "status": "confirmed",
            "mixer": {"has_master": False},
            "routing_groups": [],
            "routing_source_domains": [],
            "link_domains": [
                {
                    "protocol_space": 0,
                    "kind": "physical",
                    "pair_count": 6,
                    "status": "confirmed",
                    "evidence": "confirmed physical links",
                },
                {
                    "protocol_space": 0,
                    "kind": "adat",
                    "pair_count": 8,
                    "status": "confirmed",
                    "evidence": "confirmed ADAT links",
                },
                {
                    "protocol_space": 3,
                    "kind": "mixer",
                    "pair_count": 16,
                    "status": "confirmed",
                    "evidence": "confirmed mixer links",
                },
            ],
        }
        profile = generator.normalize_profile(data, path=path)
        blockers = generator.orion_readiness_blockers(profile)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)
        self.assertIn("physical/ADAT link action has ambiguous space=0 semantics", blockers)

    def test_orion_field_counts_alone_and_one_missing_operation_never_enable(self) -> None:
        data = profile_data("Antelope Orion Studio III", "0xa221")
        data["channels"] = {"count": 12, "status": "confirmed"}
        data["adat"] = {"count": 16, "status": "confirmed"}
        data["spdif"] = {"count": 2, "status": "confirmed"}
        data["buses"]["known"] = {
            str(index): {"name": f"bus_{index}"} for index in range(6)
        }
        data["runtime_topology"]["routing_groups"] = [
            {"destination": index, "name": f"route_{index}", "channel_count": 2}
            for index in range(15)
        ]
        profile = generator.normalize_profile(data, path=Path("orion_studio_3.json"))
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)
        self.assertTrue(generator.orion_readiness_blockers(profile))

        data["frame"]["command"]["status"] = "unconfirmed"
        profile = generator.normalize_profile(data, path=Path("orion_studio_3.json"))
        self.assertTrue(
            any("frame.command" in blocker for blocker in generator.orion_readiness_blockers(profile))
        )

    def test_orion_unsupported_formula_and_unsafe_startup_bound_are_blockers(self) -> None:
        canonical_path = (
            REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json"
        )
        data = json.loads(canonical_path.read_text(encoding="utf-8"))
        data["frame"]["command"]["formula"] = "unknown(channel)"
        data["frame"]["command"]["status"] = "confirmed"
        profile = generator.normalize_profile(data, path=canonical_path)
        self.assertTrue(
            any("uncompiled formula" in blocker for blocker in generator.orion_readiness_blockers(profile))
        )

        data = json.loads(canonical_path.read_text(encoding="utf-8"))
        data["frame"]["readback"]["startup_queries"] = [
            {"category": "0x04", "index": 4}
        ]
        with self.assertRaisesRegex(generator.ProfileError, "outside confirmed finite bounds"):
            generator.normalize_profile(data, path=canonical_path)

    def test_discrete_readiness_policy_is_unchanged(self) -> None:
        with fixture_profiles() as temporary:
            profiles = {profile.path.name: profile for profile in generator.discover_profiles(temporary)}
            expected = {
                "discrete_8_pro_synergy_core.json": generator.Readiness.PARTIAL,
                "discrete_4_synergy_core.json": generator.Readiness.UNVERIFIED,
                "discrete_4_pro_synergy_core.json": generator.Readiness.UNVERIFIED,
            }
            for filename, readiness in expected.items():
                profile = generator.load_profile(profiles[filename].path, temporary)
                self.assertEqual(generator.classify_readiness(profile), readiness)

    def test_canonical_orion_startup_walk_preserves_all_113_markers(self) -> None:
        canonical = generator.load_profile(
            REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json",
            REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles",
        )
        normalized = json.loads(generator.render_profile_pack([canonical]))["profiles"][0]
        startup = [
            (query["query_id"], query["sub_id"])
            for query in normalized["startup_queries"]
        ]
        expected = [
            (int(query["category"], 0), query["index"])
            for query in canonical.raw["frame"]["readback"]["startup_queries"]
        ]
        self.assertEqual(startup, expected)
        self.assertEqual(len(startup), 113)

    def test_raw_control_sections_are_not_dropped(self) -> None:
        profiles = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"

        def record(name: str) -> tuple[dict[str, Any], dict[str, Any]]:
            path = profiles / name
            raw = json.loads(path.read_text())
            normalized = generator.normalize_profile(
                raw,
                path=path,
                profiles_dir=profiles,
                source_bytes=path.read_bytes(),
            )
            return generator._normalized_profile_record(normalized), raw

        orion, orion_raw = record("orion_studio_sc.json")
        self.assertNotIn("runtime_topology", orion_raw)
        self.assertEqual(
            [item["strip_count"] for item in orion["mixers"]], [32, 32, 32, 32]
        )
        self.assertEqual(
            [item["has_master"] for item in orion["mixers"]],
            [True, True, True, True],
        )
        self.assertEqual(
            [group["channel_count"] for group in orion["routing_groups"]],
            [16, 2, 2, 2, 2, 2, 32, 16, 2, 32, 32, 32, 32, 32, 16],
        )
        self.assertEqual(
            [group["destination"] for group in orion["routing_groups"]], list(range(15))
        )
        source_bounds = {
            domain["bank"]: domain["index_count"]
            for domain in orion["routing_groups"][0]["source_domains"]
        }
        self.assertEqual(
            source_bounds,
            {0: 12, 1: 8, 3: 16, 4: 2, 5: 32, 6: 2, 7: 2, 8: 2, 9: 2, 10: 16, 11: 1},
        )
        self.assertEqual(
            [
                (domain["protocol_space"], domain["pair_count"])
                for domain in orion["link_domains"]
            ],
            [(3, 16)],
        )
        expected_startup = [(0x11, 0), (0x11, 1), (0x0B, 1), (0x0B, 2), (0x1B, 0)]
        expected_startup.extend((0x1A, index) for index in range(16))
        expected_startup.extend((0x03, index) for index in range(15))
        for index in range(4):
            expected_startup.extend([(0x04, index), (0x0B, 3)])
        expected_startup.extend([(0x0A, 0), (0x15, 0), (0x0B, 0), (0x16, 0)])
        expected_startup.extend((0x19, index) for index in range(64))
        expected_startup.append((0x0B, 4))
        self.assertEqual(
            [(query["query_id"], query["sub_id"]) for query in orion["startup_queries"]],
            expected_startup,
        )
        self.assertFalse(
            any(
                capability["kind"] == "link"
                for space in orion["address_spaces"]
                for capability in space["input_capabilities"]
            )
        )
        def controls(recorded: dict[str, Any], space_id: str) -> list[str]:
            space = next(item for item in recorded["address_spaces"] if item["id"] == space_id)
            return [item["kind"] for item in space["input_capabilities"]]

        self.assertEqual(
            controls(orion, "physical_inputs"), ["gain", "mode", "phantom", "phase"]
        )
        self.assertEqual(controls(orion, "adat_inputs"), ["gain"])
        self.assertEqual(controls(orion, "spdif_inputs"), ["gain"])

        zen, zen_raw = record("zen_go_sc.json")
        self.assertNotIn("runtime_topology", zen_raw)
        self.assertEqual(
            [item["strip_count"] for item in zen["mixers"]], [16, 16]
        )
        self.assertEqual(
            [item["has_master"] for item in zen["mixers"]], [False, False]
        )
        self.assertEqual(
            [(group["destination"], group["channel_count"]) for group in zen["routing_groups"]],
            [(3, 8), (5, 4), (6, 32), (7, 32), (8, 32), (9, 32)],
        )
        self.assertTrue(all(not group["source_domains"] for group in zen["routing_groups"]))
        self.assertEqual(zen["link_domains"], [])

        for normalized, raw in ((orion, orion_raw), (zen, zen_raw)):
            frame_ids = {frame["id"] for frame in normalized["frames"]}
            raw_frame_ids = {
                name
                for name, value in raw["frame"].items()
                if isinstance(value, dict) and not name.startswith("_")
            }
            self.assertEqual(frame_ids, raw_frame_ids)
            for frame_name in ("mix_command", "routing_command", "link_command", "readback"):
                frame = next(item for item in normalized["frames"] if item["id"] == frame_name)
                self.assertEqual(json.loads(frame["metadata"]), raw["frame"][frame_name])

    def test_malformed_partial_routing_source_banks_raise_profile_error(self) -> None:
        path = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "zen_go_sc.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        data["frame"]["routing_command"]["source_banks"] = {
            "0x00": {"index_count": 2, "evidence": 17}
        }

        with self.assertRaisesRegex(
            generator.ProfileError,
            r"frame\.routing_command\.source_banks\.0x00\.evidence must be a string",
        ):
            generator.normalize_profile(data, path=path)

    def test_negated_routing_evidence_is_not_confirmed(self) -> None:
        self.assertFalse(
            generator._routing_status_is_confirmed(
                {"status": "not fully decoded"}, "confirmed"
            )
        )

    def test_negated_mixer_link_evidence_is_not_confirmed(self) -> None:
        data = profile_data("Antelope Orion Studio III", "0xa221")
        data.pop("runtime_topology")
        data["frame"]["routing_command"]["status"] = "partial"
        data["frame"]["routing_command"]["addressable_destinations"] = {"0": "mixer"}
        data["frame"]["routing_command"]["destination_channels"] = {"0": 16}
        data["frame"]["link_command"]["space_values"] = {
            "3": "not confirmed mixer link pairs"
        }
        profile = generator.normalize_profile(data)

        self.assertEqual(generator._derived_link_domains(profile), [])

    def test_sparse_or_duplicate_indices_are_rejected(self) -> None:
        for indices in ([0, 2], [0, 1, 1]):
            with self.assertRaisesRegex(generator.ProfileError, "indices"):
                generator._index_count_from_raw(
                    {"indices": indices, "evidence": "confirmed source indices"},
                    "source_bank",
                )

    def test_partial_destination_groups_are_not_ignored(self) -> None:
        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        data.pop("runtime_topology")
        routing = data["frame"]["routing_command"]
        routing["status"] = "partial"
        routing["addressable_destinations"] = {"0": "mixer", "1": "aux"}
        routing["destination_channels"] = {"0": 16, "1": 2}
        routing["destination_groups"] = {
            "2": {"name": "extra_target", "channel_count": 4}
        }
        profile = generator.normalize_profile(data)

        groups, confirmed = generator._derived_routing_records(profile)

        self.assertFalse(confirmed)
        self.assertEqual(
            [(group["destination"], group["channel_count"]) for group in groups],
            [(0, 16), (1, 2), (2, 4)],
        )

    def test_malformed_partial_destination_groups_are_rejected(self) -> None:
        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        data.pop("runtime_topology")
        routing = data["frame"]["routing_command"]
        routing["status"] = "partial"
        routing["addressable_destinations"] = {"0": "mixer", "1": "aux"}
        routing["destination_channels"] = {"0": 16, "1": 2}
        routing["destination_groups"] = {"2": {"name": "missing_count"}}

        with self.assertRaisesRegex(
            generator.ProfileError,
            r"frame\.routing_command\.destination_groups\.2\.channel_count is required",
        ):
            generator.normalize_profile(data)

    def test_negative_source_bank_evidence_is_not_confirmed(self) -> None:
        for evidence in (
            "partial source indices 0-1 confirmed",
            "ambiguous source indices 0-1 confirmed",
            "incomplete source indices 0-1 confirmed",
            "host-dependent source indices 0-1 confirmed",
        ):
            with self.subTest(evidence=evidence):
                data = profile_data("Antelope Orion Studio III", "0xa221")
                data.pop("runtime_topology")
                routing = data["frame"]["routing_command"]
                routing.update(
                    {
                        "status": "confirmed",
                        "addressable_destinations": {"0": "mixer"},
                        "destination_channels": {"0": 16},
                        "source_banks": {
                            "0x00": {"index_count": 2, "evidence": evidence}
                        },
                    }
                )
                profile = generator.normalize_profile(data)

                self.assertEqual(
                    generator._derived_routing_source_domains(profile, routing, True), []
                )

    def test_orion_host_dependent_source_bank_remains_metadata_only(self) -> None:
        profiles_dir = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"
        profile = generator.load_profile(ORION_PROFILE, profiles_dir)
        domains = generator._derived_routing_source_domains(
            profile, profile.frame["routing_command"], True
        )

        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.SUPPORTED)
        self.assertTrue(domains)
        self.assertNotIn(
            0x02,
            {
                bank["bank"]
                for domain in domains
                for bank in domain["banks"]
            },
        )

    def test_orion_readback_domain_is_observed_and_write_distinct(self) -> None:
        profiles_dir = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"
        profile = generator.load_profile(ORION_PROFILE, profiles_dir)
        normalized = generator._normalized_profile_record(profile)
        groups = normalized["routing_groups"]

        expected_readback = [{
            "bank": 2,
            "indices": list(range(24)),
            "status": "observed",
            "evidence": (
                "Verified existing Orion captures (ep130 320-byte reports, magic 0x7500): "
                "destination 13 records AntelopeINIT.json frame 16648; "
                "macos-antelopeINIT-poweroff-on2-itsavedstate.json frames 39355,42453; "
                "macos-antelopeINIT-poweroff-on3-itsavedstate.json frames 15840,18966; "
                "macos-antelopeINIT-poweron.json frames 12884,18118; "
                "macos-antelopeINIT-poweron1-itresettopreviousstate.json frames 12852,15696 "
                "all contain category 0x03/index 13 bytes 17..64 = "
                "0200020102020203020402050206020702080209020a020b020c020d020e020f02100211021202130214021502160217 "
                "(bank 0x02 indices 0..23). AntelopeINIT.json frame 16406 contains "
                "category 0x03/index 2 bytes 17..20 = 02020203 (bank 0x02 indices 2,3); "
                "no captured bank 0x02 index >=24."
            ),
        }]
        self.assertEqual(groups[1]["readback_source_domains"], expected_readback)
        self.assertTrue(all(group["readback_source_domains"] == expected_readback for group in groups))
        self.assertNotIn(2, {domain["bank"] for domain in groups[1]["source_domains"]})
        self.assertNotIn(12, {domain["bank"] for domain in groups[1]["source_domains"]})

    def test_malformed_observed_readback_domains_are_rejected(self) -> None:
        for value in (
            {"indices": [0, 0], "status": "observed", "evidence": "capture"},
            {"indices": [1, 0], "status": "observed", "evidence": "capture"},
            {"indices": [0, 1], "status": "confirmed", "evidence": "capture"},
            {"indices": [0, 1], "status": "observed", "evidence": ""},
        ):
            with self.subTest(value=value):
                data = profile_data("Antelope Orion Studio III", "0xa221")
                data.pop("runtime_topology")
                routing = data["frame"]["routing_command"]
                routing.update(
                    {
                        "status": "confirmed",
                        "addressable_destinations": {"0": "mixer"},
                        "destination_channels": {"0": 16},
                        "source_banks": {
                            "0x00": {"index_count": 2, "evidence": "idx 0-1 confirmed"}
                        },
                        "readback_source_banks": {"0x02": value},
                    }
                )
                with self.assertRaisesRegex(generator.ProfileError, "readback_source_banks"):
                    generator.normalize_profile(data)

    def test_observed_readback_does_not_promote_unknown_bank(self) -> None:
        data = profile_data("Antelope Orion Studio III", "0xa221")
        data.pop("runtime_topology")
        routing = data["frame"]["routing_command"]
        routing.update(
            {
                "status": "confirmed",
                "addressable_destinations": {"0": "mixer"},
                "destination_channels": {"0": 16},
                "source_banks": {
                    "0x00": {"index_count": 2, "evidence": "idx 0-1 confirmed"}
                },
                "readback_source_banks": {
                    "0x0c": {
                        "indices": [0, 1],
                        "status": "observed",
                        "evidence": "unidentified source observation",
                    }
                },
            }
        )
        profile = generator.normalize_profile(data)
        self.assertEqual(
            generator._derived_routing_readback_source_domains(profile, routing, True), []
        )

    def test_duplicate_normalized_source_banks_are_rejected(self) -> None:
        data = profile_data("Antelope Orion Studio III", "0xa221")
        data.pop("runtime_topology")
        routing = data["frame"]["routing_command"]
        routing.update(
            {
                "status": "confirmed",
                "addressable_destinations": {"0": "mixer"},
                "destination_channels": {"0": 16},
                "source_banks": {
                    "0x03": {"index_count": 2, "evidence": "idx 0-1 confirmed"},
                    "3": {"index_count": 2, "evidence": "idx 0-1 confirmed"},
                },
            }
        )

        with self.assertRaisesRegex(generator.ProfileError, "duplicate source bank"):
            generator.normalize_profile(data)

    def test_co_present_sparse_indices_are_rejected(self) -> None:
        for value in (
            {"index_count": 2, "indices": [0, 2], "evidence": "confirmed"},
            {"range": [0, 2], "indices": [0, 1, 1], "evidence": "confirmed"},
        ):
            with self.subTest(value=value):
                with self.assertRaisesRegex(generator.ProfileError, "indices"):
                    generator._index_count_from_raw(value, "source_bank")

    def test_textual_index_ranges_are_zero_based_and_contiguous(self) -> None:
        for evidence in (
            "idx 1-3",
            "idx 0-1 and idx 4-5",
            "idx 0-2 and idx 2-4",
        ):
            with self.subTest(evidence=evidence):
                with self.assertRaisesRegex(generator.ProfileError, "index"):
                    generator._index_count_from_text(evidence, "source_bank")
        self.assertEqual(
            generator._index_count_from_text("idx 0/1", "source_bank"), 2
        )

    def test_co_present_index_evidence_must_agree(self) -> None:
        for value in (
            {"index_count": 2, "range": [0, 2], "evidence": "confirmed"},
            {"index_count": 2, "evidence": "idx 0-2 confirmed"},
            {"count": 2, "index_count": 3, "evidence": "confirmed"},
        ):
            with self.subTest(value=value):
                with self.assertRaisesRegex(generator.ProfileError, r"index evidence|indices"):
                    generator._index_count_from_raw(value, "source_bank")

    def test_null_destination_groups_are_rejected(self) -> None:
        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        data.pop("runtime_topology")
        routing = data["frame"]["routing_command"]
        routing.update(
            {
                "status": "partial",
                "addressable_destinations": {"0": "mixer"},
                "destination_channels": {"0": 16},
                "destination_groups": None,
            }
        )

        with self.assertRaisesRegex(
            generator.ProfileError,
            r"frame\.routing_command\.destination_groups must be an object",
        ):
            generator.normalize_profile(data)

    def test_negative_partial_destination_groups_do_not_inherit_source_domains(self) -> None:
        for qualification in (
            {"status": "partial"},
            {"evidence": "not confirmed destination channels"},
        ):
            with self.subTest(qualification=qualification):
                data = profile_data("Antelope Orion Studio III", "0xa221")
                data.pop("runtime_topology")
                routing = data["frame"]["routing_command"]
                routing.update(
                    {
                        "status": "confirmed",
                        "addressable_destinations": {"0": "mixer"},
                        "destination_channels": {"0": 16},
                        "source_banks": {
                            "0x00": {
                                "index_count": 2,
                                "evidence": "idx 0-1 confirmed",
                            }
                        },
                        "destination_groups": {
                            "2": {"name": "partial", "channel_count": 4, **qualification}
                        },
                    }
                )
                profile = generator.normalize_profile(data)
                normalized = json.loads(generator.render_profile_pack([profile]))["profiles"][0]
                groups = {
                    group["destination"]: group
                    for group in normalized["routing_groups"]
                }

                self.assertTrue(groups[0]["source_domains"])
                self.assertEqual(groups[2]["source_domains"], [])

    def test_duplicate_normalized_link_spaces_are_rejected(self) -> None:
        data = profile_data("Antelope Orion Studio III", "0xa221")
        data.pop("runtime_topology")
        routing = data["frame"]["routing_command"]
        routing.update(
            {
                "status": "partial",
                "addressable_destinations": {"0": "mixer"},
                "destination_channels": {"0": 16},
            }
        )
        data["frame"]["link_command"]["space_values"] = {
            "0x03": "not confirmed mixer link pairs",
            "3": "confirmed mixer link pairs",
        }

        with self.assertRaisesRegex(generator.ProfileError, "duplicate link space"):
            generator.normalize_profile(data)

    def test_contradictory_route_qualifiers_are_not_confirmed(self) -> None:
        route = {
            "status": "confirmed",
            "notes": "not confirmed routing map",
        }
        self.assertFalse(generator._routing_status_is_confirmed(route, "unknown"))

    def test_contradictory_link_qualifiers_are_not_confirmed(self) -> None:
        data = profile_data("Antelope Orion Studio III", "0xa221")
        data.pop("runtime_topology")
        data["frame"]["routing_command"].update(
            {
                "status": "partial",
                "addressable_destinations": {"0": "mixer"},
                "destination_channels": {"0": 16},
            }
        )
        data["frame"]["link_command"].update(
            {
                "status": "confirmed",
                "notes": "not confirmed link space",
                "space_values": {"3": "confirmed mixer link pairs"},
            }
        )

        profile = generator.normalize_profile(data)
        self.assertEqual(generator._derived_link_domains(profile), [])

    def test_contradictory_source_qualifiers_are_not_confirmed(self) -> None:
        data = profile_data("Antelope Orion Studio III", "0xa221")
        data.pop("runtime_topology")
        routing = data["frame"]["routing_command"]
        routing.update(
            {
                "status": "confirmed",
                "addressable_destinations": {"0": "mixer"},
                "destination_channels": {"0": 16},
                "source_banks": {
                    "0x00": {
                        "index_count": 2,
                        "evidence": "idx 0-1 confirmed",
                        "notes": "host-dependent source bound",
                    }
                },
            }
        )

        profile = generator.normalize_profile(data)
        self.assertEqual(
            generator._derived_routing_source_domains(profile, routing, True), []
        )

    def test_known_zen_requires_confirmed_hid_and_required_frame_geometry(self) -> None:
        base = profile_data("Antelope Zen Go Synergy Core", "0xa015")

        for mutate in (
            lambda data: data["transport"].update({"type": "usb"}),
            lambda data: data["transport"].update({"report_size": None}),
            lambda data: data["transport"].update({"status": "unconfirmed"}),
            lambda data: data["frame"].update({"command": {**data["frame"]["command"], "opcode": None}}),
            lambda data: data["frame"].update({"command": {key: value for key, value in data["frame"]["command"].items() if key != "value_offset"}}),
            lambda data: data["frame"].update({"state_report": {**data["frame"]["state_report"], "magic": None}}),
            lambda data: data["frame"].update({"state_report": {**data["frame"]["state_report"], "status": "observed"}}),
        ):
            data = json.loads(json.dumps(base))
            mutate(data)
            profile = generator.normalize_profile(data)
            self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

        pid_only = json.loads(json.dumps(base))
        pid_only["transport"] = {"type": "hid", "report_size": 320, "out_endpoint": 1, "in_endpoint": 130, "poll_interval_ms": 4}
        pid_only["frame"] = {}
        self.assertEqual(
            generator.classify_readiness(generator.normalize_profile(pid_only)),
            generator.Readiness.DISABLED,
        )

    def test_known_zen_rejects_optional_frame_offsets_outside_report(self) -> None:
        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        data["frame"]["routing_command"]["optional_geometry"] = {"payload_offset": 320}
        profile = generator.normalize_profile(data)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

    def test_known_zen_rejects_parameter_reference_offsets_outside_report(self) -> None:
        for reference_name, reference in (
            ("frame", "command value @320"),
            ("readback", "state_report offset 320 + channel"),
        ):
            data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
            data["params"]["gain"][reference_name] = reference
            profile = generator.normalize_profile(data)
            self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

    def test_known_zen_rejects_textual_parameter_reference_offsets_outside_report(self) -> None:
        for reference in ("command byte 320", "command bytes 0x140"):
            data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
            data["params"]["gain"]["frame"] = reference
            profile = generator.normalize_profile(data)
            self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

    def test_known_zen_rejects_nested_plural_frame_offsets_outside_report(self) -> None:
        for field_name, field_value in (
            ("param_offsets", {"outer": [{"inner": 320}]}),
            ("byte_offsets", [{"outer": {"inner": 320}}]),
        ):
            data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
            data["frame"]["routing_command"][field_name] = field_value
            profile = generator.normalize_profile(data)
            self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        data["frame"]["routing_command"]["values"] = [320]
        profile = generator.normalize_profile(data)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.SUPPORTED)

    def test_known_zen_requires_explicit_numbered_report_metadata(self) -> None:
        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        del data["transport"]["uses_numbered_reports"]
        profile = generator.normalize_profile(data)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

    def test_transport_interface_expectations_are_explicit_or_safely_inferred(self) -> None:
        explicit = profile_data("Test", "0xa001")
        explicit["transport"].update(
            {
                "expected_interface_number": "3",
                "expected_usage_page": "0xffa0",
                "expected_usage": "0x0003",
            }
        )
        profile = generator.normalize_profile(explicit)
        self.assertEqual(profile.transport.expected_interface_number, 3)
        self.assertEqual(profile.transport.expected_usage_page, 0xffa0)
        self.assertEqual(profile.transport.expected_usage, 3)

        inferred = profile_data("Test", "0xa002")
        inferred["transport"]["notes"] = "Confirmed vendor HID control interface 3."
        profile = generator.normalize_profile(inferred)
        self.assertEqual(profile.transport.expected_interface_number, 3)
        self.assertIsNone(profile.transport.expected_usage_page)
        self.assertIsNone(profile.transport.expected_usage)

        ambiguous = profile_data("Test", "0xa003")
        ambiguous["transport"]["notes"] = "HID control interface 3 or 4 may be present."
        profile = generator.normalize_profile(ambiguous)
        self.assertIsNone(profile.transport.expected_interface_number)

        rendered = generator.render_catalog([generator.normalize_profile(explicit)])
        self.assertIn("expected_interface_number: Some(3)", rendered)
        self.assertIn("expected_usage_page: Some(65440u16)", rendered)
        self.assertIn("expected_usage: Some(3u16)", rendered)

    def test_known_zen_rejects_required_offsets_and_bus_span_outside_report(self) -> None:
        base = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        for mutate in (
            lambda data: data["frame"]["command"].update({"value_offset": 320}),
            lambda data: data["frame"]["state_report"].update({"gain_base_offset": 320}),
            lambda data: data["frame"]["state_report"]["bus_block"].update({"base_offset": 319}),
            lambda data: data["frame"]["state_report"]["bus_block"].update({"bytes_per_bus": 321}),
        ):
            data = json.loads(json.dumps(base))
            mutate(data)
            profile = generator.normalize_profile(data)
            self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

        boundary = json.loads(json.dumps(base))
        boundary["buses"]["known"].update(
            {
                "1": {"name": "line_1"},
                "2": {"name": "line_2"},
            }
        )
        boundary["frame"]["state_report"]["bus_block"].update(
            {"base_offset": 314, "bytes_per_bus": 2}
        )
        profile = generator.normalize_profile(boundary)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.SUPPORTED)
        bus_block = profile.frame["state_report"]["bus_block"]
        bus_count = generator._profile_bus_count(profile)
        self.assertGreater(bus_count, 1)
        self.assertEqual(
            bus_block["base_offset"] + bus_block["bytes_per_bus"] * bus_count,
            profile.transport.report_size,
        )

        one_byte_over = json.loads(json.dumps(boundary))
        one_byte_over["frame"]["state_report"]["bus_block"]["base_offset"] = 315
        profile = generator.normalize_profile(one_byte_over)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

    def test_constraint_bounds_and_scalar_interval_are_typed_without_enum_guessing(self) -> None:
        data = profile_data("Test", "0xa001")
        data["constraints"] = {
            "adat_gain_bounds": [-6, 12],
            "spdif_gain_bounds": ["-6", "0xc"],
            "bus_level_bounds": [0, 96],
            "gain_bounds": [0, 75],
            "input_mode_allowed_values": [0, 1],
            "two_value_enum": [7, 9],
            "min_write_interval_ms": 250,
        }
        profile = generator.normalize_profile(data)
        constraints = {item["name"]: item for item in generator._build_constraints(profile)}
        for name, expected in {
            "adat_gain_bounds": (-6, 12),
            "spdif_gain_bounds": (-6, 12),
            "bus_level_bounds": (0, 96),
            "gain_bounds": (0, 75),
        }.items():
            self.assertEqual(constraints[name]["range"], expected)
            self.assertEqual(constraints[name]["values"], [])
        self.assertEqual(constraints["input_mode_allowed_values"]["values"], [0, 1])
        self.assertEqual(constraints["two_value_enum"]["values"], [7, 9])
        self.assertEqual(constraints["min_write_interval_ms"]["scalar"], 250)
        self.assertEqual(constraints["min_write_interval_ms"]["text"], "")

    def test_parameter_range_forms_and_mapping_references_are_typed(self) -> None:
        data = profile_data("Test", "0xa001")
        data["params"]["gain"] = {
            "id": "0x50",
            "type": "int8",
            "status": "confirmed",
            "per_mode_range": {"mic": [0, 75], "line": "narrow"},
            "range_by_mode": {"hiz": [0, 45]},
            "frame": {"frame": "command", "offset": "value_offset", "formula": "18 + channel"},
            "readback": {"frame": "state_report", "offset": 49, "formula": "49 + channel"},
        }
        profile = generator.normalize_profile(data)
        param = generator._build_params(profile)[0]
        self.assertEqual(param["range_by_mode"]["mic"], (0, 75))
        self.assertEqual(param["range_by_mode"]["line"], "narrow")
        self.assertEqual(param["range_by_mode"]["hiz"], (0, 45))
        self.assertEqual(param["frame_reference"]["text"], "{\"formula\":\"18 + channel\",\"frame\":\"command\",\"offset\":\"value_offset\"}")
        self.assertEqual(param["frame_reference"]["offsets"][0]["offset"], 18)
        self.assertEqual(param["frame_reference"]["offsets"][0]["formula"], "18 + channel")
        self.assertEqual(param["readback_reference"]["offsets"][0]["offset"], 49)
        self.assertEqual(param["readback_reference"]["offsets"][0]["formula"], "49 + channel")

    def test_textual_byte_parameter_references_are_typed(self) -> None:
        reference = generator._parameter_reference(
            "command byte 0x12, bytes 19, offset 0x14, value @21",
            "params.gain.frame",
        )
        self.assertEqual(
            [offset["offset"] for offset in reference["offsets"]],
            [0x12, 19, 0x14, 21],
        )

    def test_nested_numeric_frame_fields_are_typed(self) -> None:
        data = profile_data("Test", "0xa001")
        data["frame"]["nested_geometry"] = {
            "magic": "0x70",
            "opcode": "0x17",
            "param_id": "0xd4",
            "id": "0x01",
            "count": "2",
            "stride": "3",
            "offset": "19",
            "mask": "0x3f",
            "bit": "0x40",
            "width": "2",
            "pan_center": "32",
            "mix_wet_constant": "100",
            "values": ["0", "0xff"],
        }
        profile = generator.normalize_profile(data)
        frame = next(item for item in generator._build_frames(profile) if item["id"] == "nested_geometry")
        fields = {item["name"]: item for item in frame["fields"]}
        self.assertEqual(fields["magic"]["value"], 0x70)
        self.assertEqual(fields["opcode"]["value"], 0x17)
        self.assertEqual(fields["param_id"]["value"], 0xD4)
        self.assertEqual(fields["id"]["value"], 1)
        self.assertEqual(fields["count"]["value"], 2)
        self.assertEqual(fields["stride"]["stride"], 3)
        self.assertEqual(fields["offset"]["offset"], 19)
        self.assertEqual(fields["mask"]["mask"], 0x3F)
        self.assertEqual(fields["bit"]["mask"], 0x40)
        self.assertEqual(fields["width"]["width"], 2)
        self.assertEqual(fields["pan_center"]["value"], 32)
        self.assertEqual(fields["mix_wet_constant"]["value"], 100)
        self.assertEqual(fields["values"]["values"], [0, 0xFF])

    def test_numeric_width_overflow_and_malformed_lists_raise_profile_error(self) -> None:
        cases = []
        data = profile_data("Test", "0xa001")
        data["frame"]["command"]["opcode"] = 256
        cases.append(data)
        data = profile_data("Test", "0xa001")
        data["frame"]["nested"] = {"mask": 256}
        cases.append(data)
        data = profile_data("Test", "0xa001")
        data["frame"]["nested"] = {"values": [2**31]}
        cases.append(data)
        data = profile_data("Test", "0xa001")
        data["hazards"] = {"unsafe": {"opcodes": [256]}}
        cases.append(data)
        data = profile_data("Test", "0xa001")
        data["constraints"] = {"allowed_opcodes": [0, 256]}
        cases.append(data)
        for invalid in cases:
            with self.assertRaises(generator.ProfileError):
                generator.normalize_profile(invalid)

    def test_renders_normalized_profile_pack_with_typed_operations(self) -> None:
        with fixture_profiles() as temporary:
            profiles = [
                generator.load_profile(source.path, temporary)
                for source in generator.discover_profiles(temporary)
                if source.path.name in {"orion_studio_3.json", "zen_go_sc.json"}
            ]
            pack = json.loads(generator.render_profile_pack(profiles))

            self.assertEqual(pack["schema_version"], 1)
            self.assertEqual(pack["generator_version"], generator.GENERATOR_VERSION)
            self.assertEqual(
                [profile["id"] for profile in pack["profiles"]],
                ["orion_studio_3", "zen_go_sc"],
            )
            by_id = {profile["id"]: profile for profile in pack["profiles"]}
            self.assertEqual(by_id["zen_go_sc"]["readiness"], "supported")
            self.assertEqual(by_id["orion_studio_3"]["readiness"], "disabled")
            for profile in by_id.values():
                self.assertIn("startup_queries", profile)
                self.assertIn("readback", profile)
                operation_kinds = {
                    operation["op"]
                    for frame in profile["frames"]
                    for operation in frame["operations"]
                }
                self.assertTrue(operation_kinds)
                self.assertIn("fixed_byte", operation_kinds)
                self.assertIn("scalar", operation_kinds)
                self.assertIn("indexed", operation_kinds)
                self.assertIn("bit_field", operation_kinds)
                self.assertIn("pair_index", operation_kinds)
                self.assertIn("allowed_values", operation_kinds)
                scalar_operations = [
                    operation
                    for frame in profile["frames"]
                    for operation in frame["operations"]
                    if operation["op"] == "scalar"
                ]
                self.assertTrue(all(operation["field"] for operation in scalar_operations))
                self.assertTrue(
                    all(
                        operation["endian"] == "not_applicable"
                        for operation in scalar_operations
                        if operation["width"] == 1
                    )
                )
                bit_operations = [
                    operation
                    for frame in profile["frames"]
                    for operation in frame["operations"]
                    if operation["op"] == "bit_field"
                ]
                self.assertTrue(all(operation["field"] for operation in bit_operations))

    def test_unproven_multibyte_scalar_endianness_is_unavailable(self) -> None:
        data = profile_data("Test", "0xa001")
        data["frame"]["command"] = {
            "magic_offset": 0,
            "magic": "0x70",
            "opcode_offset": 4,
            "opcode": "0x13",
            "value_offset": 16,
            "value_width": 2,
            "status": "confirmed",
        }
        profile = generator.normalize_profile(data)
        normalized = json.loads(generator.render_profile_pack([profile]))["profiles"][0]
        operations = next(
            frame["operations"]
            for frame in normalized["frames"]
            if frame["id"] == "command"
        )
        self.assertIn(
            {
                "op": "uncompiled_formula",
                "formula": "unproven endianness for value width 2",
            },
            operations,
        )

    def test_input_space_capabilities_are_typed_and_preserve_idless_link(self) -> None:
        data = profile_data("Typed Inputs", "0xa111")
        data["params"]["channel_link"] = {
            "status": "confirmed",
            "type": "per-pair bool",
            "frame": "link_command",
        }
        data["runtime_topology"]["input_spaces"] = [
            {
                "space": "physical_inputs",
                "controls": [
                    {"kind": "gain", "parameter": "gain"},
                    {"kind": "link", "parameter": "channel_link"},
                ],
            }
        ]
        profile = generator.normalize_profile(data)
        normalized = json.loads(generator.render_profile_pack([profile]))["profiles"][0]
        capabilities = next(
            space["input_capabilities"]
            for space in normalized["address_spaces"]
            if space["id"] == "physical_inputs"
        )
        self.assertEqual(
            capabilities,
            [
                {
                    "kind": "gain",
                    "parameter": "gain",
                    "parameter_id": 0x50,
                    "label": "GAIN",
                },
                {
                    "kind": "link",
                    "parameter": "channel_link",
                    "parameter_id": None,
                    "label": "LINK",
                },
            ],
        )
        rust = generator.render_catalog([profile])
        self.assertIn("InputControlKind::Gain", rust)
        self.assertIn("InputControlKind::Link", rust)
        self.assertIn('parameter: "channel_link", parameter_id: None', rust)

    def test_input_space_capability_kind_key_mapping_is_closed(self) -> None:
        legal_pairs = {
            "gain": ("gain", "adat_gain", "spdif_gain"),
            "mode": ("input_mode",),
            "phantom": ("phantom",),
            "phase": ("phase_invert",),
            "link": ("channel_link", "adat_channel_link", "spdif_channel_link"),
        }
        parameter_ids = {
            "gain": 0x50,
            "adat_gain": 0x5B,
            "spdif_gain": 0x5C,
            "input_mode": 0x4F,
            "phantom": 0x51,
            "phase_invert": 0x52,
            "channel_link": None,
            "adat_channel_link": None,
            "spdif_channel_link": None,
        }
        for kind, parameter_keys in legal_pairs.items():
            for parameter_key in parameter_keys:
                with self.subTest(kind=kind, parameter=parameter_key):
                    data = profile_data("Valid Inputs", "0xa112")
                    data["params"][parameter_key] = {
                        "id": parameter_ids[parameter_key],
                        "type": "int8",
                        "status": "confirmed",
                        "frame": "command value @18",
                        "readback": "state_report offset 49 + channel",
                        "range": [0, 75],
                    }
                    supplied_parameter_id = parameter_ids[parameter_key]
                    data["runtime_topology"]["input_spaces"] = [{
                        "space": "physical_inputs",
                        "controls": [{
                            "kind": kind,
                            "parameter": parameter_key,
                            "parameter_id": (
                                hex(supplied_parameter_id)
                                if supplied_parameter_id is not None
                                else None
                            ),
                        }],
                    }]
                    generator.normalize_profile(data)

        for kind, parameter_key in (("phantom", "adat_gain"), ("mode", "spdif_gain")):
            with self.subTest(kind=kind, parameter=parameter_key):
                data = profile_data("Invalid Inputs", "0xa113")
                data["params"][parameter_key] = {
                    "id": parameter_ids[parameter_key],
                    "type": "int8",
                    "status": "confirmed",
                    "frame": "command value @18",
                    "readback": "state_report offset 49 + channel",
                    "range": [0, 75],
                }
                data["runtime_topology"]["input_spaces"] = [{
                    "space": "physical_inputs",
                    "controls": [{"kind": kind, "parameter": parameter_key}],
                }]
                with self.assertRaises(generator.ProfileError) as raised:
                    generator.normalize_profile(data)
                message = str(raised.exception)
                self.assertIn("physical_inputs", message)
                self.assertIn(kind, message)
                self.assertIn(parameter_key, message)

    def test_input_space_capability_supplied_parameter_id_is_strict(self) -> None:
        cases = [
            ({"kind": "gain", "parameter": "gain", "parameter_id": "0x51"}, "gain"),
            ({"kind": "link", "parameter": "channel_link", "parameter_id": "0xa2"}, "channel_link"),
            ({"kind": "gain", "parameter": "gain", "parameter_id": "not-a-number"}, "gain"),
        ]
        for control, parameter_key in cases:
            with self.subTest(control=control):
                data = profile_data("Invalid Inputs", "0xa114")
                if parameter_key == "channel_link":
                    data["params"][parameter_key] = {
                        "status": "confirmed",
                        "type": "per-pair bool",
                        "frame": "link_command",
                    }
                data["runtime_topology"]["input_spaces"] = [{
                    "space": "physical_inputs",
                    "controls": [control],
                }]
                with self.assertRaises(generator.ProfileError) as raised:
                    generator.normalize_profile(data)
                message = str(raised.exception)
                self.assertIn("physical_inputs", message)
                self.assertIn(control["kind"], message)
                self.assertIn(parameter_key, message)

    def test_input_space_capabilities_reject_malformed_duplicate_and_unknown_records(self) -> None:
        cases = [
            ([{"space": "missing", "controls": []}], "unknown address space"),
            (
                [{"space": "physical_inputs", "controls": [{"kind": "gain", "parameter": "missing"}]}],
                "unknown parameter",
            ),
            (
                [{"space": "physical_inputs", "controls": [
                    {"kind": "gain", "parameter": "gain"},
                    {"kind": "gain", "parameter": "gain"},
                ]}],
                "duplicate control kind",
            ),
            (
                [{"space": "physical_inputs", "controls": [{"kind": "gain", "parameter": "gain", "label": ""}]}],
                "label must be non-empty",
            ),
            (
                [{"space": "physical_inputs", "controls": [{"kind": "unknown", "parameter": "gain"}]}],
                "unknown input control kind",
            ),
        ]
        for input_spaces, message in cases:
            with self.subTest(message=message):
                data = profile_data("Invalid Inputs", "0xa112")
                data["runtime_topology"]["input_spaces"] = input_spaces
                with self.assertRaisesRegex(generator.ProfileError, message):
                    generator.normalize_profile(data)

    def test_structured_link_and_routing_domains_are_closed_and_finite(self) -> None:
        data = profile_data("Typed Domains", "0xa120")
        profile = generator.normalize_profile(data)
        normalized = json.loads(generator.render_profile_pack([profile]))["profiles"][0]
        self.assertEqual(
            normalized["link_domains"],
            [{
                "protocol_space": 3,
                "kind": "mixer",
                "pair_count": 8,
                "status": "confirmed",
                "evidence": "synthetic generator fixture assumption",
            }],
        )
        self.assertEqual(normalized["routing_groups"][0]["source_domains"][0]["bank"], 0)
        self.assertEqual(normalized["routing_groups"][0]["source_domains"][0]["index_count"], 2)

        mutations = [
            lambda topology: topology["link_domains"].append(dict(topology["link_domains"][0])),
            lambda topology: topology["link_domains"][0].update({"pair_count": 0}),
            lambda topology: topology["routing_source_domains"][0]["banks"].append(
                dict(topology["routing_source_domains"][0]["banks"][0])
            ),
            lambda topology: topology["routing_source_domains"][0]["banks"][0].update(
                {"index_count": 257}
            ),
            lambda topology: topology["routing_groups"][0].update({"source_domain": "missing"}),
        ]
        for mutate in mutations:
            invalid = profile_data("Invalid Domains", "0xa121")
            mutate(invalid["runtime_topology"])
            with self.assertRaises(generator.ProfileError):
                generator.normalize_profile(invalid)

    def test_canonical_orion_emits_no_input_links_and_only_confirmed_mixer_link_domain(self) -> None:
        canonical = generator.load_profile(
            REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles" / "orion_studio_sc.json",
            REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles",
        )
        normalized = json.loads(generator.render_profile_pack([canonical]))["profiles"][0]
        for space in normalized["address_spaces"]:
            if space["id"] in {"physical_inputs", "adat_inputs", "spdif_inputs"}:
                self.assertNotIn(
                    "link",
                    [capability["kind"] for capability in space["input_capabilities"]],
                )
        self.assertEqual(
            [(domain["protocol_space"], domain["kind"], domain["pair_count"])
             for domain in normalized["link_domains"]],
            [(3, "mixer", 16)],
        )
        self.assertEqual(len(normalized["routing_groups"]), 15)
        self.assertTrue(all(group["source_domains"] for group in normalized["routing_groups"]))
        self.assertTrue(all(
            all(domain["bank"] != 0x0c for domain in group["source_domains"])
            for group in normalized["routing_groups"]
        ))

    def test_orion_normalization_preserves_capability_sections(self) -> None:
        data = profile_data("Antelope Orion Studio III", "0xa221")
        data["adat"] = {"count": 16, "status": "confirmed"}
        data["spdif"] = {"count": 2, "status": "confirmed"}
        data["frame"]["readback"] = {
            "request_magic": "0x74",
            "subcmd": "0x10",
            "response_magic": "0x75",
            "response_discriminator_offset": 1,
            "response_discriminator": 0,
            "category_offset": 8,
            "index_offset": 12,
            "data_offset": 16,
            "category_counts": {"0x04": 4},
            "status": "confirmed",
        }
        data["hazards"] = {
            "unsafe_query": {"status": "confirmed", "rule": "bound index", "effect": "crash"}
        }
        profile = generator.normalize_profile(data, path=Path("orion_studio_3.json"))
        normalized = json.loads(generator.render_profile_pack([profile]))["profiles"][0]

        self.assertEqual(
            {space["id"] for space in normalized["address_spaces"]},
            {"physical_inputs", "adat_inputs", "spdif_inputs", "outputs"},
        )
        self.assertEqual(len(normalized["outputs"]), 1)
        self.assertEqual(len(normalized["mixers"]), 2)
        self.assertTrue(any(frame["id"] == "routing_command" for frame in normalized["frames"]))
        self.assertTrue(normalized["params"])
        self.assertTrue(normalized["hazards"])

    def test_check_detects_stale_rust_and_json_artifacts(self) -> None:
        with fixture_profiles() as temporary:
            artifacts = Path(temporary) / "artifacts"
            artifacts.mkdir()
            generated = artifacts / "generated.rs"
            pack_generated = artifacts / "generated_profiles.json"
            legacy_candidates = artifacts / "legacy_zen_go_candidate_preamp_meters.json"
            generated.write_text(generator.generate_catalog(temporary), encoding="utf-8")
            pack_generated.write_text(generator.generate_profile_pack(temporary), encoding="utf-8")
            legacy_candidates.write_text(
                generator.generate_legacy_zen_go_candidate_preamp_meters(temporary),
                encoding="utf-8",
            )
            self.assertTrue(
                generator.check_generated_artifacts(
                    temporary, generated, pack_generated, legacy_candidates
                )
            )

            zen_path = Path(temporary) / "zen_go_sc.json"
            old_hash = hashlib.sha256(zen_path.read_bytes()).hexdigest()
            zen_path.write_bytes(zen_path.read_bytes() + b" ")
            new_hash = hashlib.sha256(zen_path.read_bytes()).hexdigest()
            self.assertNotEqual(old_hash, new_hash)
            self.assertFalse(generator.check_catalog(temporary, generated))
            self.assertFalse(generator.check_profile_pack(temporary, pack_generated))
            self.assertFalse(
                generator.check_generated_artifacts(
                    temporary, generated, pack_generated, legacy_candidates
                )
            )
            regenerated_rust = generator.generate_catalog(temporary)
            regenerated_pack = generator.generate_profile_pack(temporary)
            self.assertIn("profiles/zen_go_sc.json", regenerated_rust)
            self.assertIn("profiles/zen_go_sc.json", regenerated_pack)
            self.assertIn(new_hash, regenerated_rust)
            self.assertIn(new_hash, regenerated_pack)
            self.assertNotIn(old_hash, regenerated_rust)
            self.assertNotIn(old_hash, regenerated_pack)

    def test_cli_check_detects_source_and_individual_artifact_drift(self) -> None:
        with fixture_profiles() as temporary:
            profiles_dir = Path(temporary)
            artifacts = profiles_dir / "artifacts"
            artifacts.mkdir()
            generated = artifacts / "generated.rs"
            pack_generated = artifacts / "generated_profiles.json"
            legacy_candidates = artifacts / "legacy_zen_go_candidate_preamp_meters.json"
            generated_text = generator.generate_catalog(profiles_dir)
            pack_text = generator.generate_profile_pack(profiles_dir)
            legacy_text = generator.generate_legacy_zen_go_candidate_preamp_meters(profiles_dir)
            generated.write_text(generated_text, encoding="utf-8")
            pack_generated.write_text(pack_text, encoding="utf-8")
            legacy_candidates.write_text(legacy_text, encoding="utf-8")

            command = [
                sys.executable,
                str(TOOLS_DIR / "generate_device_catalog.py"),
                "--check",
                str(profiles_dir),
                "--generated",
                str(generated),
                "--pack-generated",
                str(pack_generated),
                "--legacy-candidate-generated",
                str(legacy_candidates),
            ]
            self.assertEqual(subprocess.run(command, check=False).returncode, 0)

            generated.write_text(generated_text + "// stale\n", encoding="utf-8")
            self.assertNotEqual(subprocess.run(command, check=False).returncode, 0)
            generated.write_text(generated_text, encoding="utf-8")

            pack_generated.write_text(pack_text + " ", encoding="utf-8")
            self.assertNotEqual(subprocess.run(command, check=False).returncode, 0)
            pack_generated.write_text(pack_text, encoding="utf-8")

            legacy_candidates.write_text(legacy_text + " ", encoding="utf-8")
            self.assertNotEqual(subprocess.run(command, check=False).returncode, 0)
            legacy_candidates.write_text(legacy_text, encoding="utf-8")

            source = profiles_dir / "zen_go_sc.json"
            source.write_bytes(source.read_bytes() + b" ")
            self.assertNotEqual(subprocess.run(command, check=False).returncode, 0)

    def test_section_status_uses_positive_confirmation_only(self) -> None:
        self.assertEqual(generator._section_status({"notes": "confirmed capture"}, "unknown"), "confirmed")
        self.assertEqual(generator._section_status({"notes": "not confirmed"}, "unknown"), "unknown")
        self.assertEqual(generator._section_status({"notes": "unconfirmed"}, "unknown"), "unknown")
        self.assertEqual(generator._section_status({}, "observed"), "observed")

    def test_negative_section_evidence_cannot_inherit_confirmed_fallback(self) -> None:
        for phrase in (
            "not confirmed",
            "not yet confirmed",
            "not independently confirmed",
            "never confirmed",
            "unconfirmed",
            "unverified",
        ):
            self.assertEqual(
                generator._section_status({"notes": phrase}, "confirmed"),
                "unconfirmed",
            )

        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        data["frame"]["command"].pop("status")
        data["frame"]["command"]["notes"] = "not confirmed"
        profile = generator.normalize_profile(data)
        self.assertNotEqual(generator.classify_readiness(profile), generator.Readiness.SUPPORTED)

    def test_negative_transport_evidence_cannot_inherit_confirmed_fallback(self) -> None:
        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        data["transport"]["notes"] = "not confirmed"
        profile = generator.normalize_profile(data)
        self.assertEqual(profile.transport.status, "unconfirmed")
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

    def test_known_zen_requires_explicit_positive_channel_and_bus_geometry(self) -> None:
        base = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        cases = []

        missing_channels = json.loads(json.dumps(base))
        missing_channels["channels"].pop("count")
        cases.append(missing_channels)

        zero_channels = json.loads(json.dumps(base))
        zero_channels["channels"]["count"] = 0
        cases.append(zero_channels)

        missing_buses = json.loads(json.dumps(base))
        missing_buses["buses"] = {"status": "confirmed"}
        cases.append(missing_buses)

        zero_buses = json.loads(json.dumps(base))
        zero_buses["buses"]["count"] = 0
        cases.append(zero_buses)

        for data in cases:
            profile = generator.normalize_profile(data)
            self.assertEqual(generator.classify_readiness(profile), generator.Readiness.DISABLED)

        missing_bus_profile = generator.normalize_profile(missing_buses)
        self.assertEqual(generator._profile_bus_count(missing_bus_profile), 0)

    def test_transport_geometry_rejects_zero_values(self) -> None:
        for field in ("report_size", "out_endpoint", "in_endpoint", "poll_interval_ms"):
            data = profile_data("Test", "0xa001")
            data["transport"][field] = 0
            with self.assertRaises(generator.ProfileError):
                generator.normalize_profile(data)

    def test_magic_and_opcode_offsets_are_u16_frame_fields(self) -> None:
        data = profile_data("Antelope Zen Go Synergy Core", "0xa015")
        data["frame"]["command"].update({"magic_offset": 256, "opcode_offset": 319})
        profile = generator.normalize_profile(data)
        self.assertEqual(generator.classify_readiness(profile), generator.Readiness.SUPPORTED)
        frame = next(item for item in generator._build_frames(profile) if item["id"] == "command")
        fields = {item["name"]: item for item in frame["fields"]}

        self.assertEqual(fields["magic_offset"]["offset"], 256)
        self.assertEqual(fields["opcode_offset"]["offset"], 319)
        self.assertEqual(frame["magic"], 0x70)
        self.assertEqual(frame["opcode"], 0x13)

        rendered = generator.render_catalog([profile])
        self.assertIn("offset: Some(256u16)", rendered)
        self.assertIn("offset: Some(319u16)", rendered)
        self.assertIn("magic_offset: Some(256u16), magic: Some(112u8)", rendered)
        self.assertIn("opcode_offset: Some(319u16), opcode: Some(19u8)", rendered)

    def test_malformed_numeric_fields_are_rejected(self) -> None:
        with self.assertRaises(generator.ProfileError):
            generator._try_int("not-a-number", "frame.command.opcode")
        with self.assertRaises(generator.ProfileError):
            generator._range(["bad", 4], "params.gain.range")
        with self.assertRaises(generator.ProfileError):
            generator.normalize_profile(
                {
                    **profile_data("Test", "0xa001"),
                    "frame": {"command": {"opcode": "bad"}},
                }
            )
        with self.assertRaises(generator.ProfileError):
            generator.normalize_profile(
                {
                    **profile_data("Test", "0xa001"),
                    "params": {"gain": {"id": "bad"}},
                }
            )

    def test_missing_output_verification_defaults_false(self) -> None:
        profile = generator.normalize_profile(profile_data("Test", "0xa001"))
        outputs = generator._build_outputs(profile)
        self.assertEqual(len(outputs), 1)
        self.assertFalse(outputs[0]["verified"])

    def test_inferred_mixer_geometry_is_not_confirmed(self) -> None:
        orion_data = profile_data("Antelope Orion Studio III", "0xa221")
        orion_data["mixer"] = {}
        orion_data["frame"]["mix_command"] = {
            "notes": "CONFIRMED Mix 1-4 capture; each mix has 32 input strips; Mix 2-4 presumed",
        }
        orion = generator.normalize_profile(orion_data, path=Path("orion_studio_3.json"))
        orion_mixers = generator._build_mixers(orion)
        self.assertEqual(len(orion_mixers), 4)
        self.assertTrue(
            all(generator._status_variant(item["status"]) in {"Observed", "Unconfirmed"} for item in orion_mixers)
        )
        rendered = generator.render_catalog([orion])
        mixer_start = rendered.index("static ORION_STUDIO_3_MIXERS")
        mixer_end = rendered.index("static ORION_STUDIO_3_LINK_DOMAINS", mixer_start)
        mixer_output = rendered[mixer_start:mixer_end]
        self.assertNotIn("status: Status::Confirmed", mixer_output)

        zen = generator.normalize_profile(profile_data("Antelope Zen Go Synergy Core", "0xa015"))
        zen_mixers = generator._build_mixers(zen)
        self.assertTrue(all(generator._status_variant(item["status"]) == "Confirmed" for item in zen_mixers))

    def test_nested_frame_geometry_and_parameter_references_are_typed(self) -> None:
        with fixture_profiles() as temporary:
            profile = generator.load_profile(Path(temporary) / "orion_studio_3.json", temporary)
            frames = generator._build_frames(profile)
            state = next(frame for frame in frames if frame["id"] == "state_report")
            bus_block = next(field for field in state["fields"] if field["name"] == "bus_block")
            base = next(field for field in bus_block["children"] if field["name"] == "base_offset")
            stride = next(field for field in bus_block["children"] if field["name"] == "bytes_per_bus")
            self.assertEqual(base["offset"], 28)
            self.assertEqual(stride["value"], 3)

            routing = next(frame for frame in frames if frame["id"] == "routing_command")
            destination_channels = next(
                field for field in routing["fields"] if field["name"] == "destination_channels"
            )
            line_out = next(field for field in destination_channels["children"] if field["name"] == "0")
            self.assertEqual(line_out["value"], 16)

            param = generator._build_params(profile)[0]
            self.assertEqual(param["frame_reference"]["offsets"][0]["offset"], 18)
            self.assertEqual(param["readback_reference"]["offsets"][0]["offset"], 49)

    def test_generated_catalog_has_all_hardware_and_preserves_raw_source(self) -> None:
        with fixture_profiles() as temporary:
            output = generator.generate_catalog(temporary)
            for profile_name in EXPECTED_PROFILES:
                self.assertIn(profile_name, output)
            self.assertIn("mic_models.json", output)  # raw hardware notes remain exact
            self.assertNotIn('source_path: "profiles/mic_models.json"', output)
            profile_text = (Path(temporary) / "orion_studio_3.json").read_text(encoding="utf-8")
            self.assertEqual(generator.load_profile(Path(temporary) / "orion_studio_3.json", temporary).raw_text, profile_text)
            self.assertIn(generator._rust_string(profile_text), output)
            self.assertIn("Antelope Orion Studio III", output)
            self.assertIn("Antelope Zen Go Synergy Core", output)

    def test_generator_rejects_non_object_profile(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "broken.json"
            path.write_text(json.dumps([]), encoding="utf-8")
            with self.assertRaises(generator.ProfileError):
                generator.load_profile(path, Path(temporary))


if __name__ == "__main__":
    unittest.main()
