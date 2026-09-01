"""Tests for profile-driven device catalog generation."""

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

TOOLS_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS_DIR))

import generate_device_catalog as generator  # noqa: E402


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
        },
        "channels": {"count": 2, "names": ["A1", "A2"], "status": "confirmed"},
        "buses": {
            "known": {"0": {"name": "monitor", "aliases": ["mon"]}},
            "status": "confirmed",
        },
        "mixer": {"mixes": 2, "channels_per_mix": 16, "status": "confirmed"},
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


class GenerateDeviceCatalogTests(unittest.TestCase):
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
            orion = generator.load_profile(profiles["orion_studio_3.json"].path, temporary)
            discrete_8 = generator.load_profile(profiles["discrete_8_pro_synergy_core.json"].path, temporary)
            discrete_4 = generator.load_profile(profiles["discrete_4_synergy_core.json"].path, temporary)

            self.assertEqual(generator.classify_readiness(zen), generator.Readiness.SUPPORTED)
            self.assertEqual(generator.classify_readiness(orion), generator.Readiness.DISABLED)
            self.assertEqual(generator.classify_readiness(discrete_8), generator.Readiness.PARTIAL)
            self.assertEqual(generator.classify_readiness(discrete_4), generator.Readiness.UNVERIFIED)

            incomplete_orion_data = profile_data("Antelope Orion Studio III", "0xa221")
            incomplete_orion_data["transport"]["report_size"] = None
            incomplete_orion_data["frame"] = {}
            incomplete_orion = generator.normalize_profile(incomplete_orion_data)
            self.assertEqual(generator.classify_readiness(incomplete_orion), generator.Readiness.DISABLED)

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

    def test_check_detects_stale_generated_catalog(self) -> None:
        with fixture_profiles() as temporary:
            generated = Path(temporary) / "generated.rs"
            generated.write_text(generator.generate_catalog(temporary), encoding="utf-8")
            self.assertTrue(generator.check_catalog(temporary, generated))
            zen_path = Path(temporary) / "zen_go_sc.json"
            zen_path.write_text(zen_path.read_text(encoding="utf-8").replace('"0xa015"', '"0xa016"'), encoding="utf-8")
            self.assertFalse(generator.check_catalog(temporary, generated))

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
        mixer_end = rendered.index("static ORION_STUDIO_3_FRAME", mixer_start)
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
