#!/usr/bin/env python3
"""Generate a self-contained Rust catalog from Antelope-Ctl profiles.

The profile directory is an input to this tool only.  Runtime Rust code consumes
checked-in ``src/device/generated.rs`` and never reads JSON from Antelope-Ctl.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Any, Iterable, Mapping, Sequence


GENERATOR_VERSION = "1.4.0"
PROFILE_PACK_SCHEMA_VERSION = 1
EXCLUDED_PROFILE_NAMES = frozenset({"mic_models.json"})
REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PROFILES_DIR = REPO_ROOT / "modules" / "Antelope-Ctl" / "profiles"


class ProfileError(ValueError):
    """Raised when canonical profile data cannot be represented safely."""


class Readiness(Enum):
    SUPPORTED = "supported"
    PARTIAL = "partial"
    UNVERIFIED = "unverified"
    DISABLED = "disabled"


@dataclass(frozen=True)
class ProfileSource:
    path: Path


@dataclass(frozen=True)
class Provenance:
    source_path: str
    source_sha256: str
    generator_version: str


@dataclass(frozen=True)
class Identity:
    name: str
    vid: int
    pid: int
    bcd_device: str | None
    status: str
    notes: str
    evidence: str


@dataclass(frozen=True)
class Transport:
    kind: str
    report_size: int | None
    out_endpoint: int | None
    in_endpoint: int | None
    poll_interval_ms: int | None
    uses_numbered_reports: bool | None
    expected_interface_number: int | None
    expected_usage_page: int | None
    expected_usage: int | None
    status: str
    notes: str
    evidence: str


@dataclass(frozen=True)
class NormalizedProfile:
    path: Path
    raw: Mapping[str, Any]
    raw_text: str
    identity: Identity
    transport: Transport
    frame: Mapping[str, Any]
    channels: Mapping[str, Any]
    adat: Mapping[str, Any]
    spdif: Mapping[str, Any]
    buses: Mapping[str, Any]
    mixer: Mapping[str, Any]
    params: Mapping[str, Any]
    constraints: Mapping[str, Any]
    hazards: Mapping[str, Any]
    provenance: Provenance

    @property
    def profile_status(self) -> str:
        return self.identity.status


# Known profile identities are the policy boundary between profile existence and
# runtime readiness.  A future profile is not implicitly selectable merely because
# it happens to contain a complete-looking transport section.
_KNOWN_READINESS: dict[tuple[int, int], Readiness] = {
    (0x23E5, 0xA015): Readiness.SUPPORTED,  # Zen Go Synergy Core
    # Orion becomes selectable only when its dedicated driver is registered.
    (0x23E5, 0xA221): Readiness.DISABLED,
    (0x23E5, 0xA2B5): Readiness.PARTIAL,  # Discrete 8 Pro, no readback
    (0x23E5, 0xA2BE): Readiness.UNVERIFIED,  # Discrete 4
    (0x23E5, 0xA2BF): Readiness.UNVERIFIED,  # Discrete 4 Pro
}


_REQUIRED_RUNTIME_FRAME_FIELDS: dict[str, tuple[str, ...]] = {
    # SET_PARAM is the only command shape used by the existing Zen Go driver.
    "command": (
        "magic_offset",
        "magic",
        "opcode_offset",
        "opcode",
        "param_id_offset",
        "channel_offset",
        "value_offset",
    ),
    # These two parallel arrays are required by the existing readback decoder.
    "state_report": ("magic_offset", "magic", "gain_base_offset", "status_base_offset"),
}


def _report_span_fits(offset: int, span: int, report_size: int) -> bool:
    """Return whether byte range starting at offset fits in report."""

    return span > 0 and 0 <= offset and offset + span <= report_size


def _is_frame_offset_name(name: str) -> bool:
    normalized = name.lower()
    return normalized == "offset" or normalized.endswith("_offset")


def _is_frame_offset_container_name(name: str) -> bool:
    normalized = name.lower()
    return normalized == "offsets" or normalized.endswith("_offsets")


def _frame_offsets_fit_report(profile: NormalizedProfile, report_size: int) -> bool:
    """Return whether every typed raw frame offset addresses one report byte."""

    def visit(value: Any, context: str, offset_field: bool = False) -> None:
        if isinstance(value, Mapping):
            for key, child in value.items():
                key_text = str(key)
                if key_text.startswith("_"):
                    continue
                child_context = f"{context}.{key_text}"
                child_is_offset = (
                    offset_field
                    or _is_frame_offset_name(key_text)
                    or _is_frame_offset_container_name(key_text)
                )
                visit(child, child_context, child_is_offset)
            return
        if isinstance(value, list):
            for index, child in enumerate(value):
                visit(child, f"{context}[{index}]", offset_field)
            return
        if offset_field and value is not None:
            offset = _checked_u16(value, context)
            if not _report_span_fits(offset, 1, report_size):
                raise ProfileError(f"{context} falls outside report size {report_size}")

    try:
        visit(profile.frame, "frame")
    except ProfileError:
        return False
    return True


def _parameter_reference_offsets_fit_report(profile: NormalizedProfile, report_size: int) -> bool:
    """Return whether typed parameter reference offsets address report bytes."""

    try:
        for parameter in _build_params(profile):
            for reference_name in ("frame_reference", "readback_reference"):
                reference = parameter[reference_name]
                for offset in reference["offsets"]:
                    if not _report_span_fits(offset["offset"], 1, report_size):
                        return False
    except (KeyError, ProfileError, TypeError):
        return False
    return True


def _profile_bus_count(profile: NormalizedProfile) -> int:
    """Return number of bus slots represented by profile output geometry."""

    count = _count(profile.buses, ("count", "count_confirmed", "count_assumed_total"), "buses")
    if count is not None:
        return count
    known = profile.buses.get("known")
    if isinstance(known, Mapping) and known:
        bus_ids = [_checked_u16(key, "buses.known.id") for key in known]
        return max(bus_ids) + 1
    return 0


def _profile_has_confirmed_runtime_shape(profile: NormalizedProfile) -> bool:
    """Return whether profile contains enough confirmed shape to be opened safely."""

    transport = profile.transport
    if _kind_variant(transport.kind) != "Hid":
        return False
    if not isinstance(transport.uses_numbered_reports, bool):
        return False
    report_size = transport.report_size
    out_endpoint = transport.out_endpoint
    in_endpoint = transport.in_endpoint
    poll_interval_ms = transport.poll_interval_ms
    if (
        report_size is None
        or out_endpoint is None
        or in_endpoint is None
        or poll_interval_ms is None
    ):
        return False
    if (
        report_size <= 0
        or out_endpoint <= 0
        or in_endpoint <= 0
        or poll_interval_ms <= 0
    ):
        return False
    if _status_variant(transport.status) != "Confirmed":
        return False

    if "count" not in profile.channels or profile.channels["count"] is None:
        return False
    channel_count = _count(
        profile.channels,
        ("count", "count_confirmed", "count_assumed_total"),
        "channels",
    )
    if channel_count is None or channel_count <= 0:
        return False
    if _status_variant(_section_status(profile.channels, profile.profile_status)) != "Confirmed":
        return False

    bus_count = _profile_bus_count(profile)
    if bus_count <= 0:
        return False
    if _status_variant(_section_status(profile.buses, profile.profile_status)) != "Confirmed":
        return False
    if not _frame_offsets_fit_report(profile, report_size):
        return False
    if not _parameter_reference_offsets_fit_report(profile, report_size):
        return False

    for frame_name, required_fields in _REQUIRED_RUNTIME_FRAME_FIELDS.items():
        frame = profile.frame.get(frame_name)
        if not isinstance(frame, Mapping):
            return False
        if _status_variant(_section_status(frame, profile.profile_status)) != "Confirmed":
            return False
        for field_name in required_fields:
            value = frame.get(field_name)
            if value is None:
                return False
            try:
                if field_name in {"magic", "opcode"}:
                    _checked_u8(value, f"frame.{frame_name}.{field_name}")
                else:
                    offset = _checked_u16(value, f"frame.{frame_name}.{field_name}")
                    if field_name.endswith("_offset") and not _report_span_fits(offset, 1, report_size):
                        return False
            except ProfileError:
                return False

        if frame_name == "state_report":
            channel_span = channel_count
            for field_name in ("gain_base_offset", "status_base_offset"):
                try:
                    base_offset = _checked_u16(frame[field_name], f"frame.state_report.{field_name}")
                except ProfileError:
                    return False
                if not _report_span_fits(base_offset, channel_span, report_size):
                    return False

            bus_block = frame.get("bus_block")
            bus_base = frame.get("bus_block_offset")
            bus_stride = frame.get("bus_block_stride")
            if isinstance(bus_block, Mapping):
                bus_base = bus_block.get("base_offset", bus_base)
                bus_stride = bus_block.get("bytes_per_bus", bus_stride)
            if bus_base is None or bus_stride is None:
                return False
            try:
                bus_base = _checked_u16(bus_base, "frame.state_report.bus_block_offset")
                bus_stride = _checked_u16(bus_stride, "frame.state_report.bus_block_stride")
            except ProfileError:
                return False
            if not _report_span_fits(bus_base, bus_stride * bus_count, report_size):
                return False
            if isinstance(bus_block, Mapping):
                for field_name, value in bus_block.items():
                    if field_name.endswith("_offset") and field_name != "base_offset":
                        try:
                            relative_offset = _checked_u16(
                                value, f"frame.state_report.bus_block.{field_name}"
                            )
                        except ProfileError:
                            return False
                        if relative_offset >= bus_stride:
                            return False
    return True


def parse_int(value: Any, field: str = "value") -> int:
    """Parse strict decimal/hex integer profile values.

    JSON booleans are integers in Python, but accepting them for USB geometry or
    identifiers would silently produce a wrong catalog, so bool is rejected.
    """

    if isinstance(value, bool):
        raise ProfileError(f"{field} must be an integer, not boolean")
    if isinstance(value, int):
        return value
    if not isinstance(value, str):
        raise ProfileError(f"{field} must be an integer or string, got {type(value).__name__}")
    text = value.strip()
    if not text:
        raise ProfileError(f"{field} must not be empty")
    sign = 1
    unsigned = text
    if unsigned[0] in "+-":
        if unsigned[0] == "-":
            sign = -1
        unsigned = unsigned[1:]
    try:
        if unsigned.lower().startswith("0x"):
            digits = unsigned[2:]
            if not digits or not re.fullmatch(r"[0-9a-fA-F]+", digits):
                raise ValueError
            return sign * int(digits, 16)
        if not re.fullmatch(r"[0-9]+", unsigned):
            raise ValueError
        return sign * int(unsigned, 10)
    except ValueError as exc:
        raise ProfileError(f"{field} has invalid integer value {value!r}") from exc


def _checked_int(value: Any, field: str, minimum: int, maximum: int) -> int:
    parsed = parse_int(value, field)
    if not minimum <= parsed <= maximum:
        raise ProfileError(f"{field} must fit in {minimum}..{maximum}")
    return parsed


def _checked_i32(value: Any, field: str) -> int:
    return _checked_int(value, field, -(2**31), 2**31 - 1)


def _checked_u8(value: Any, field: str) -> int:
    return _checked_int(value, field, 0, 0xFF)


def _checked_u16(value: Any, field: str) -> int:
    return _checked_int(value, field, 0, 0xFFFF)


def _required_mapping(parent: Mapping[str, Any], key: str) -> Mapping[str, Any]:
    value = parent.get(key)
    if not isinstance(value, Mapping):
        raise ProfileError(f"profile field {key!r} must be an object")
    return value


def _required_text(parent: Mapping[str, Any], key: str, context: str) -> str:
    value = parent.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ProfileError(f"{context}.{key} must be a non-empty string")
    return value


def _optional_text(parent: Mapping[str, Any], key: str, default: str = "") -> str:
    value = parent.get(key, default)
    if value is None:
        return ""
    if not isinstance(value, str):
        raise ProfileError(f"profile field {key!r} must be a string when present")
    return value


def _optional_int(parent: Mapping[str, Any], key: str, context: str) -> int | None:
    if key not in parent or parent[key] is None:
        return None
    return parse_int(parent[key], f"{context}.{key}")


def _validate_optional_nonnegative(value: int | None, field: str, *, zero_allowed: bool = True) -> None:
    if value is None:
        return
    if value < 0 or (not zero_allowed and value == 0):
        qualifier = "non-negative" if zero_allowed else "positive"
        raise ProfileError(f"{field} must be {qualifier}")


def _validate_optional_byte(value: int | None, field: str) -> None:
    if value is not None and not 0 <= value <= 0xFF:
        raise ProfileError(f"{field} must fit in one byte")


def _transport_explicit_values(
    transport: Mapping[str, Any],
    direct_keys: Sequence[str],
    nested_keys: Sequence[str],
    *,
    allow_scalar_container: bool = False,
) -> list[tuple[str, Any]]:
    """Collect supported explicit HID-interface metadata spellings."""

    values: list[tuple[str, Any]] = []
    for key in direct_keys:
        if key in transport:
            values.append((f"transport.{key}", transport[key]))

    for container_key in ("control_interface", "expected_control_interface", "hid_interface"):
        if container_key not in transport:
            continue
        container = transport[container_key]
        if isinstance(container, Mapping):
            for key in nested_keys:
                if key in container:
                    values.append((f"transport.{container_key}.{key}", container[key]))
        elif allow_scalar_container:
            values.append((f"transport.{container_key}", container))
        elif container is not None and not isinstance(container, (str, int, bool)):
            raise ProfileError(f"transport.{container_key} must be an object")
    return values


def _resolve_transport_optional_int(
    transport: Mapping[str, Any],
    direct_keys: Sequence[str],
    nested_keys: Sequence[str],
    context: str,
    minimum: int,
    maximum: int,
    *,
    allow_scalar_container: bool = False,
) -> tuple[int | None, bool]:
    """Resolve aliases, rejecting conflicting explicit interface metadata."""

    values = _transport_explicit_values(
        transport,
        direct_keys,
        nested_keys,
        allow_scalar_container=allow_scalar_container,
    )
    if not values:
        return None, False

    parsed = [
        None if value is None else _checked_int(value, field, minimum, maximum)
        for field, value in values
    ]
    distinct = set(parsed)
    if len(distinct) > 1:
        fields = ", ".join(field for field, _ in values)
        raise ProfileError(f"conflicting {context} values in {fields}")
    return parsed[0], True


_INTERFACE_REFERENCE_RE = re.compile(
    r"\b(?:interface|iface)\s*(?:number\s*)?(?:[_-]\s*)?(\d+)\b",
    re.IGNORECASE,
)


def _infer_control_interface_number(profile: Mapping[str, Any]) -> int | None:
    """Infer one control interface only from unambiguous profile evidence."""

    raw_device = profile.get("device")
    raw_transport = profile.get("transport")
    device = raw_device if isinstance(raw_device, Mapping) else {}
    transport = raw_transport if isinstance(raw_transport, Mapping) else {}
    texts = [
        str(transport.get("notes", "")),
        str(transport.get("evidence", "")),
        str(device.get("notes", "")),
        str(device.get("evidence", "")),
    ]
    numbers: set[int] = set()
    ambiguous = False
    for text in texts:
        for match in _INTERFACE_REFERENCE_RE.finditer(text):
            prefix = text[max(0, match.start() - 48) : match.start()].lower()
            suffix = text[match.end() : match.end() + 24].lower()
            context = f"{prefix} {suffix}"
            # Ignore class-compliant audio interfaces unless evidence also
            # identifies that same interface as HID/vendor control.
            if re.search(r"\b(?:audio|uac|isochronous|stream)\b", context) and not re.search(
                r"\b(?:hid|control|vendor)\b", context
            ):
                continue
            if re.match(r"\s*(?:[/,]|-|–|\bor\b|\band\b)\s*\d", suffix):
                ambiguous = True
                continue
            numbers.add(int(match.group(1)))
    return next(iter(numbers)) if len(numbers) == 1 and not ambiguous else None


def _transport_interface_expectations(
    data: Mapping[str, Any], transport: Mapping[str, Any]
) -> tuple[int | None, int | None, int | None]:
    """Read explicit interface metadata, then use conservative text inference."""

    interface_number, explicit_interface = _resolve_transport_optional_int(
        transport,
        ("expected_interface_number", "expected_control_interface_number", "control_interface_number", "interface_number"),
        ("interface_number", "number", "interface"),
        "expected control interface number",
        0,
        0x7FFF_FFFF,
        allow_scalar_container=True,
    )
    usage_page, _ = _resolve_transport_optional_int(
        transport,
        ("expected_usage_page", "control_usage_page", "usage_page"),
        ("usage_page",),
        "expected HID usage page",
        0,
        0xFFFF,
    )
    usage, _ = _resolve_transport_optional_int(
        transport,
        ("expected_usage", "control_usage", "usage"),
        ("usage",),
        "expected HID usage",
        0,
        0xFFFF,
    )
    if not explicit_interface:
        interface_number = _infer_control_interface_number(data)
    return interface_number, usage_page, usage


def _validate_section_objects(data: Mapping[str, Any]) -> None:
    for key in (
        "frame",
        "channels",
        "adat",
        "spdif",
        "buses",
        "mixer",
        "params",
        "constraints",
        "hazards",
    ):
        if key in data and data[key] is not None and not isinstance(data[key], Mapping):
            raise ProfileError(f"profile field {key!r} must be an object")


def _profile_source_path(path: Path, profiles_dir: Path | None) -> str:
    """Return stable source provenance independent of checkout absolute path."""

    if profiles_dir is not None:
        try:
            relative = path.resolve().relative_to(profiles_dir.resolve())
            return (Path("profiles") / relative).as_posix()
        except ValueError:
            pass
    return path.name


def normalize_profile(
    data: Mapping[str, Any],
    *,
    path: Path | None = None,
    profiles_dir: Path | None = None,
    source_bytes: bytes | None = None,
) -> NormalizedProfile:
    """Validate and normalize one loose JSON profile.

    Required identity and transport keys are validated strictly.  Explicit null
    transport geometry is retained for profiles such as Discrete 4; omitted keys
    are rejected so incomplete data cannot acquire invented defaults.
    """

    if not isinstance(data, Mapping):
        raise ProfileError("profile root must be a JSON object")
    _validate_section_objects(data)

    device = _required_mapping(data, "device")
    transport = _required_mapping(data, "transport")

    name = _required_text(device, "name", "device")
    if "vid" not in device:
        raise ProfileError("device.vid is required")
    if "pid" not in device:
        raise ProfileError("device.pid is required")
    vid = parse_int(device["vid"], "device.vid")
    pid = parse_int(device["pid"], "device.pid")
    if not 0 <= vid <= 0xFFFF:
        raise ProfileError("device.vid must fit in a USB vendor identifier")
    if not 0 <= pid <= 0xFFFF:
        raise ProfileError("device.pid must fit in a USB product identifier")

    bcd_device = device.get("bcdDevice")
    if bcd_device is not None and not isinstance(bcd_device, str):
        raise ProfileError("device.bcdDevice must be a string when present")
    profile_status = _optional_text(device, "status", "unknown")

    transport_type = _required_text(transport, "type", "transport")
    # These keys are required even when value is null.  Null is meaningful
    # evidence for unverified profiles; omission is schema incompleteness.
    for key in ("report_size", "out_endpoint", "in_endpoint", "poll_interval_ms"):
        if key not in transport:
            raise ProfileError(f"transport.{key} is required (use null when unverified)")
    report_size = _optional_int(transport, "report_size", "transport")
    out_endpoint = _optional_int(transport, "out_endpoint", "transport")
    in_endpoint = _optional_int(transport, "in_endpoint", "transport")
    poll_interval_ms = _optional_int(transport, "poll_interval_ms", "transport")
    _validate_optional_nonnegative(report_size, "transport.report_size", zero_allowed=False)
    _validate_optional_nonnegative(out_endpoint, "transport.out_endpoint", zero_allowed=False)
    _validate_optional_nonnegative(in_endpoint, "transport.in_endpoint", zero_allowed=False)
    _validate_optional_nonnegative(poll_interval_ms, "transport.poll_interval_ms", zero_allowed=False)
    if report_size is not None:
        _checked_u16(report_size, "transport.report_size")
    if out_endpoint is not None:
        _checked_u8(out_endpoint, "transport.out_endpoint")
    if in_endpoint is not None:
        _checked_u8(in_endpoint, "transport.in_endpoint")
    if poll_interval_ms is not None:
        _checked_u16(poll_interval_ms, "transport.poll_interval_ms")
    uses_numbered_reports = transport.get("uses_numbered_reports")
    if uses_numbered_reports is not None and not isinstance(uses_numbered_reports, bool):
        raise ProfileError("transport.uses_numbered_reports must be boolean when present")
    (
        expected_interface_number,
        expected_usage_page,
        expected_usage,
    ) = _transport_interface_expectations(data, transport)

    if path is None:
        path = Path("profile.json")
    if source_bytes is None:
        source_text = json.dumps(data, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
        source_bytes = source_text.encode("utf-8")
    else:
        source_text = source_bytes.decode("utf-8")
    provenance = Provenance(
        source_path=_profile_source_path(path, profiles_dir),
        source_sha256=hashlib.sha256(source_bytes).hexdigest(),
        generator_version=GENERATOR_VERSION,
    )

    def section(name_: str) -> Mapping[str, Any]:
        value = data.get(name_, {})
        return value if isinstance(value, Mapping) else {}

    frame_data = section("frame")
    params_data = section("params")
    _validate_frame_numeric_fields(frame_data, "frame")
    for param_name, param_value in params_data.items():
        if not isinstance(param_value, Mapping):
            continue
        if "id" in param_value and param_value["id"] is not None:
            _checked_u16(param_value["id"], f"params.{param_name}.id")
        if "range" in param_value:
            _range(param_value["range"], f"params.{param_name}.range")

    normalized = NormalizedProfile(
        path=path,
        raw=data,
        raw_text=source_text,
        identity=Identity(
            name=name,
            vid=vid,
            pid=pid,
            bcd_device=bcd_device,
            status=profile_status,
            notes=_optional_text(device, "notes"),
            evidence=_optional_text(device, "evidence"),
        ),
        transport=Transport(
            kind=transport_type,
            report_size=report_size,
            out_endpoint=out_endpoint,
            in_endpoint=in_endpoint,
            poll_interval_ms=poll_interval_ms,
            uses_numbered_reports=uses_numbered_reports,
            expected_interface_number=expected_interface_number,
            expected_usage_page=expected_usage_page,
            expected_usage=expected_usage,
            status=_section_status(transport, profile_status),
            notes=_optional_text(transport, "notes"),
            evidence=_optional_text(transport, "evidence"),
        ),
        frame=frame_data,
        channels=section("channels"),
        adat=section("adat"),
        spdif=section("spdif"),
        buses=section("buses"),
        mixer=section("mixer"),
        params=params_data,
        constraints=section("constraints"),
        hazards=section("hazards"),
        provenance=provenance,
    )
    # Run all typed builders once during normalization so malformed values and
    # values that cannot fit generated Rust types fail as ProfileError before
    # rendering or catalog classification.
    _build_frames(normalized)
    _build_params(normalized)
    _build_constraints(normalized)
    _build_hazards(normalized)
    _build_address_spaces(normalized)
    _build_input_capabilities(normalized)
    _build_inputs(normalized)
    _build_outputs(normalized)
    _build_mixers(normalized)
    _build_link_domains(normalized)
    _build_routing_groups(normalized)
    _readback_definition(normalized)
    return normalized


def load_profile(path: Path | str, profiles_dir: Path | str | None = None) -> NormalizedProfile:
    path = Path(path)
    root = Path(profiles_dir) if profiles_dir is not None else path.parent
    try:
        source_bytes = path.read_bytes()
    except OSError as exc:
        raise ProfileError(f"cannot read profile {path}: {exc}") from exc
    try:
        data = json.loads(source_bytes.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise ProfileError(f"profile {path} is not valid UTF-8 JSON: {exc}") from exc
    if not isinstance(data, Mapping):
        raise ProfileError(f"profile {path} root must be a JSON object")
    return normalize_profile(data, path=path, profiles_dir=root, source_bytes=source_bytes)


def discover_profiles(profiles_dir: Path | str) -> list[ProfileSource]:
    """Discover canonical hardware JSON files in deterministic order."""

    root = Path(profiles_dir)
    if not root.is_dir():
        raise ProfileError(f"profiles directory does not exist: {root}")
    return [
        ProfileSource(path=path)
        for path in sorted(root.glob("*.json"), key=lambda item: item.name)
        if path.name not in EXCLUDED_PROFILE_NAMES
    ]


def source_sha256(path: Path | str) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def _is_orion(profile: NormalizedProfile) -> bool:
    return (
        profile.path.stem == "orion_studio_3"
        and (profile.identity.vid, profile.identity.pid) == (0x23E5, 0xA221)
    )


def _effective_framing(profile: NormalizedProfile) -> bool | None:
    """Apply Orion's source-backed default while retaining raw values elsewhere."""

    if not _is_orion(profile):
        return profile.transport.uses_numbered_reports
    if profile.transport.uses_numbered_reports is None:
        return False
    return profile.transport.uses_numbered_reports


def _reference_mentions_frame(text: str, frame_id: str) -> bool:
    """Match complete frame identifiers, not identifiers containing one another."""

    match = re.match(r"\s*(?:frame\.)?([A-Za-z_][A-Za-z0-9_]*)", text)
    return match is not None and match.group(1) == frame_id


def _parameter_blocks_promotion(parameter: Mapping[str, Any]) -> bool:
    """Treat qualifiers anywhere in referenced parameter evidence as blocking."""

    return _evidence_has_negative_qualifier(
        " ".join((str(parameter.get("status", "")), str(parameter.get("metadata", ""))))
    )


def _effective_frame_status(profile: NormalizedProfile, frame_id: str, frame: Mapping[str, Any]) -> str:
    """Promote allowlisted Orion mappings only with confirmed source evidence."""

    raw_status = _section_status(frame, profile.profile_status or "unknown")
    allowlisted = {
        "command",
        "global_command",
        "mix_command",
        "link_command",
        "state_report",
        "readback",
    }
    if _is_orion(profile) and frame_id in {"auraverb_command", "micmodeling_command"}:
        return "unknown"
    if not _is_orion(profile) or frame_id not in allowlisted:
        return raw_status
    explicit_status = str(frame.get("status", ""))
    explicit_decoded = frame_id == "readback" and re.match(
        r"\s*decoded\b", explicit_status, re.IGNORECASE
    ) is not None
    parameters = _build_params(profile)
    has_confirmed_mapping = any(
        _reference_mentions_frame(
            parameter["frame_reference"]["text"] + " " + parameter["readback_reference"]["text"], frame_id
        )
        and _normalized_status(parameter["status"]) == "confirmed"
        and not _parameter_blocks_promotion(parameter)
        for parameter in parameters
    )
    if frame_id == "state_report":
        has_confirmed_mapping = has_confirmed_mapping or _evidence_is_confirmed(
            str(frame.get("channel_meter_notes", ""))
        )
    explicit_evidence = " ".join(str(frame.get(key, "")) for key in ("status", "runtime_status", "notes", "evidence"))
    if (not has_confirmed_mapping and not explicit_decoded) or _evidence_has_negative_qualifier(explicit_evidence):
        return raw_status
    try:
        operations = _frame_operations(profile, frame_id, frame)
    except ProfileError:
        return raw_status
    if any(operation["op"] == "uncompiled_formula" for operation in operations):
        return raw_status
    if not _operations_fit_report(profile, operations):
        return raw_status
    return "confirmed"


def _operations_fit_report(profile: NormalizedProfile, operations: list[dict[str, Any]]) -> bool:
    report_size = profile.transport.report_size
    if report_size is None:
        return False
    try:
        for operation in operations:
            kind = operation["op"]
            if kind in {"fixed_byte", "scalar", "bit_field"}:
                span = operation.get("width", 1)
                if not _report_span_fits(operation["offset"], span, report_size):
                    return False
            elif kind in {"indexed", "pair_index"}:
                span = operation["width"]
                end = operation["base"] + operation["stride"] * operation["max_index"]
                if not _report_span_fits(end, span, report_size):
                    return False
        return True
    except (KeyError, TypeError):
        return False


def _orion_state_meter_operations(profile: NormalizedProfile) -> list[dict[str, Any]]:
    state = profile.frame.get("state_report")
    if not _is_orion(profile) or not isinstance(state, Mapping):
        return []
    evidence = str(state.get("channel_meter_notes", ""))
    if not _evidence_is_confirmed(evidence):
        return []
    count = profile.channels.get("count_confirmed")
    indices = profile.channels.get("confirmed_indices")
    if (
        not isinstance(count, int)
        or isinstance(count, bool)
        or count <= 0
        or not isinstance(indices, list)
        or indices != list(range(count))
    ):
        return []
    base = state.get("channel_meter_base_offset")
    if count is None or count <= 0 or base is None:
        return []
    try:
        base = _checked_u16(base, "frame.state_report.channel_meter_base_offset")
        if not _report_span_fits(base, count, profile.transport.report_size or 0):
            return []
    except ProfileError:
        return []
    return [{"op": "indexed", "base": base, "stride": 1, "index_field": "physical_meter", "width": 1, "max_index": count - 1}]


def _orion_has_confirmed_ambiguous_input_link(profile: NormalizedProfile) -> bool:
    """Block confirmed physical/ADAT links whose shared space byte is unresolved."""

    link_parameters = {"channel_link", "adat_channel_link"}
    topology = profile.raw.get("runtime_topology")
    if not isinstance(topology, Mapping):
        return []
    if _normalized_status(str(topology.get("status", ""))) != "confirmed":
        return []
    records = topology.get("input_spaces", [])
    for record in records:
        if not isinstance(record, Mapping) or record.get("space") not in {
            "physical_inputs",
            "adat_inputs",
        }:
            continue
        controls = record.get("controls", [])
        if any(
            isinstance(control, Mapping)
            and control.get("kind") == "link"
            and control.get("parameter") in link_parameters
            for control in controls
        ):
            return True
    return False


def orion_readiness_blockers(profile: NormalizedProfile) -> list[str]:
    """Return confirmed-only Orion promotion blockers without inferring protocol facts."""

    if not _is_orion(profile):
        return []
    blockers: list[str] = []
    framing = _effective_framing(profile)
    if framing is True:
        blockers.append("transport.uses_numbered_reports=true is unrepresentable for Orion")
    elif framing is None:
        blockers.append("transport.uses_numbered_reports is unconfirmed")
    if profile.transport.report_size != 320:
        blockers.append("transport.report_size is not confirmed as 320")

    expected_geometry = {"channels": 12, "adat": 16, "spdif": 2}
    for section_name, expected_count in expected_geometry.items():
        section = getattr(profile, section_name)
        count = _count(
            section,
            ("count", "count_confirmed", "count_assumed_total"),
            section_name,
        )
        if count != expected_count:
            blockers.append(f"{section_name}.count is not confirmed as {expected_count}")
    if len(_build_outputs(profile)) != 6:
        blockers.append("buses.known does not contain six finite output buses")
    mixers = _build_mixers(profile)
    if len(mixers) != 4 or any(
        mixer["strip_count"] != 32 or not mixer["has_master"] for mixer in mixers
    ):
        blockers.append("runtime_topology.mixer is not four master-plus-32 surfaces")
    link_domains = _build_link_domains(profile)
    if [
        (domain["protocol_space"], domain["kind"], domain["pair_count"])
        for domain in link_domains
    ] != [(3, "mixer", 16)]:
        blockers.append("runtime_topology.link_domains does not contain only confirmed mixer space 3")
    routing = _build_routing_groups(profile)
    if len(routing) != 15 or [group["destination"] for group in routing] != list(range(15)):
        blockers.append("runtime_topology.routing_groups is not the finite 0..14 topology")
    elif any(not group["source_domains"] for group in routing):
        blockers.append("runtime_topology.routing_groups lacks per-destination source domains")

    required_frames = {
        "command",
        "global_command",
        "mix_command",
        "link_command",
        "routing_command",
        "state_report",
        "readback",
    }
    built_frames = {frame["id"]: frame for frame in _build_frames(profile)}
    for frame_id in sorted(required_frames):
        frame = built_frames.get(frame_id)
        if frame is None:
            blockers.append(f"frame.{frame_id} is missing or unconfirmed")
            continue
        operations = _frame_operations(profile, frame_id, profile.frame.get(frame_id, {}))
        if not _operations_fit_report(profile, operations):
            blockers.append(f"frame.{frame_id} operation geometry exceeds report bounds")
        if _status_variant(_effective_frame_status(profile, frame_id, profile.frame.get(frame_id, {}))) != "Confirmed":
            blockers.append(f"frame.{frame_id} is missing or unconfirmed")
            continue
        if any(operation["op"] == "uncompiled_formula" for operation in operations):
            blockers.append(f"frame.{frame_id} has an uncompiled formula")

    meter_report = profile.frame.get("meter_report")
    if isinstance(meter_report, Mapping):
        meter_operations = _frame_operations(profile, "meter_report", meter_report)
        obsolete_operations = [
            operation
            for operation in meter_operations
            if _obsolete_orion_channel_meter_operation(profile, "meter_report", operation)
        ]
        if obsolete_operations and not _operations_fit_report(profile, obsolete_operations):
            blockers.append("frame.meter_report operation geometry exceeds report bounds")

    readback, startup = _readback_definition(profile)
    expected_counts = {
        0x03: 15,
        0x04: 4,
        0x0A: 1,
        0x0B: 8,
        0x11: 2,
        0x15: 1,
        0x16: 1,
        0x19: 64,
        0x1A: 16,
        0x1B: 1,
    }
    actual_counts = (
        {item["category"]: item["count"] for item in readback["category_counts"]}
        if readback is not None
        else {}
    )
    if actual_counts != expected_counts:
        blockers.append("frame.readback.category_counts does not match confirmed finite bounds")
    if len(startup) != 113:
        blockers.append("frame.readback.startup_queries is not the explicit 113-request walk")
    for query in startup:
        count = actual_counts.get(query["query_id"])
        if count is None or query["sub_id"] >= count:
            blockers.append(
                f"frame.readback startup query {query['query_id']:#04x}:{query['sub_id']} is unsafe"
            )
            break

    if not _orion_state_meter_operations(profile):
        blockers.append("frame.state_report lacks confirmed physical channel meter mapping")
    raw_topology = profile.raw.get("runtime_topology")
    raw_link_domains = raw_topology.get("link_domains", []) if isinstance(raw_topology, Mapping) else []
    if _orion_has_confirmed_ambiguous_input_link(profile) or any(
        isinstance(domain, Mapping)
        and domain.get("kind") in {"physical", "adat"}
        and _normalized_status(str(domain.get("status", ""))) == "confirmed"
        for domain in raw_link_domains
    ):
        blockers.append("physical/ADAT link action has ambiguous space=0 semantics")
    return blockers


def classify_readiness(profile: NormalizedProfile) -> Readiness:
    """Classify runtime readiness without conflating it with source status."""

    identity = (profile.identity.vid, profile.identity.pid)
    if _is_orion(profile):
        return Readiness.SUPPORTED if not orion_readiness_blockers(profile) else Readiness.DISABLED
    if identity == (0x23E5, 0xA221):
        return Readiness.DISABLED
    known = _KNOWN_READINESS.get(identity)
    if known is not None:
        if known is Readiness.SUPPORTED and not _profile_has_confirmed_runtime_shape(profile):
            return Readiness.DISABLED
        return known
    if "unconfirm" in profile.identity.status.lower():
        return Readiness.UNVERIFIED
    return Readiness.DISABLED


def _status_variant(text: str | None) -> str:
    normalized = (text or "").strip().lower()
    if normalized.startswith("confirm") or re.match(r"fully\s+decoded\b", normalized):
        return "Confirmed"
    if normalized.startswith("observ"):
        return "Observed"
    if "unavailable" in normalized or "does not exist" in normalized or normalized.startswith("forbidden"):
        return "Unavailable"
    if "unconfirm" in normalized:
        return "Unconfirmed"
    return "Unknown"


def _kind_variant(text: str) -> str:
    normalized = text.lower()
    if normalized == "hid":
        return "Hid"
    return "Unknown"


def _addressing_variant(text: str | None) -> str:
    normalized = (text or "").lower()
    if "0-index" in normalized or "0 based" in normalized or "zero-based" in normalized:
        return "ZeroBased"
    if "1-index" in normalized or "1 based" in normalized or "one-based" in normalized:
        return "OneBased"
    return "Unknown"


def _space_kind(name: str) -> str:
    normalized = name.lower()
    if normalized == "channels":
        return "PhysicalInputs"
    if normalized == "adat":
        return "AdatInputs"
    if normalized == "spdif":
        return "SpdifInputs"
    if normalized == "buses":
        return "Outputs"
    if normalized == "mixer":
        return "Mixer"
    return "Unknown"


def _frame_kind(name: str) -> str:
    normalized = name.lower()
    if "state_report" in normalized:
        return "StateReport"
    if "meter_report" in normalized:
        return "MeterReport"
    if "name_report" in normalized:
        return "NameReport"
    if "init_enumeration" in normalized:
        return "InitEnumerationReport"
    if "error_response" in normalized:
        return "ErrorResponse"
    if "response" in normalized:
        return "Response"
    if normalized.endswith("command") or normalized in {"command", "opcodes"}:
        return "Command"
    return "Decoder"


def _param_type(value: Any) -> str:
    normalized = str(value or "").lower()
    if normalized == "bool":
        return "Bool"
    if normalized == "enum":
        return "Enum"
    if "int8" in normalized:
        return "Int8"
    if normalized in {"int", "integer"} or normalized.startswith("int"):
        return "Int"
    if normalized.startswith("uint") or normalized.startswith("unsigned"):
        return "UInt"
    return "Unknown"


def _try_int(value: Any, field: str) -> int | None:
    """Parse optional numeric value and reject malformed numeric input."""

    if value is None:
        return None
    return parse_int(value, field)


def _json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def _rust_string(value: Any) -> str:
    """Render a valid Rust string literal without relying on JSON escapes."""

    text = "" if value is None else str(value)
    output = ['"']
    for char in text:
        codepoint = ord(char)
        if char == "\\":
            output.append("\\\\")
        elif char == '"':
            output.append('\\"')
        elif char == "\n":
            output.append("\\n")
        elif char == "\r":
            output.append("\\r")
        elif char == "\t":
            output.append("\\t")
        elif codepoint < 0x20:
            output.append(f"\\u{{{codepoint:x}}}")
        else:
            output.append(char)
    output.append('"')
    return "".join(output)


def _rust_option(value: int | bool | str | None, renderer: Any = str) -> str:
    return "None" if value is None else f"Some({renderer(value)})"


def _rust_i32(value: int) -> str:
    return str(value)


def _rust_u8(value: int) -> str:
    return f"{value}u8"


def _rust_u16(value: int) -> str:
    return f"{value}u16"


def _rust_slice(values: Iterable[str]) -> str:
    rendered = list(values)
    return "&[]" if not rendered else "&[" + ", ".join(rendered) + "]"


def _count(section: Mapping[str, Any], keys: Sequence[str], context: str) -> int | None:
    for key in keys:
        if key in section and section[key] is not None:
            count = _checked_u16(section[key], f"{context}.{key}")
            return count
    return None


def _int_list(
    value: Any,
    context: str,
    *,
    minimum: int = -(2**31),
    maximum: int = 2**31 - 1,
) -> list[int]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ProfileError(f"{context} must be an array")
    return [
        _checked_int(item, f"{context}[{index}]", minimum, maximum)
        for index, item in enumerate(value)
    ]


def _range(value: Any, context: str) -> tuple[int, int] | None:
    """Parse optional two-sided range without accepting malformed values."""

    if value is None:
        return None
    if isinstance(value, Mapping):
        if "min" not in value or "max" not in value:
            raise ProfileError(f"{context} must contain integer min and max")
        first = _checked_i32(value["min"], f"{context}.min")
        second = _checked_i32(value["max"], f"{context}.max")
    elif isinstance(value, list) and len(value) == 2:
        first = _checked_i32(value[0], f"{context}[0]")
        second = _checked_i32(value[1], f"{context}[1]")
    else:
        raise ProfileError(f"{context} must be a two-item integer array or min/max object")
    if first > second:
        raise ProfileError(f"{context} lower bound must not exceed upper bound")
    return first, second


def _metadata(value: Any) -> str:
    """Serialize source metadata without changing canonical text."""

    return _json(value)


def _section_status(section: Mapping[str, Any], fallback: str) -> str:
    """Use explicit typed status, source status, or canonical evidence/notes."""

    if "runtime_status" in section:
        explicit = section["runtime_status"]
        if explicit is None:
            return "unknown"
        return str(explicit)
    if "status" in section:
        explicit = section["status"]
        if explicit is None:
            return "unknown"
        return str(explicit)
    evidence = f"{section.get('notes', '')} {section.get('evidence', '')}".lower()
    if re.search(
        r"\b(?:unconfirmed|unverified|never\s+confirmed|not\s+(?:(?:yet|independently)\s+)?confirmed)\b",
        evidence,
    ):
        return "unconfirmed" if _status_variant(fallback) == "Confirmed" else "unknown"
    if re.search(r"\bconfirmed\b", evidence):
        return "confirmed"
    return fallback


def _build_address_spaces(profile: NormalizedProfile) -> list[dict[str, Any]]:
    sections = (
        ("physical_inputs", "Physical inputs", "channels", profile.channels),
        ("adat_inputs", "ADAT inputs", "adat", profile.adat),
        ("spdif_inputs", "S/PDIF inputs", "spdif", profile.spdif),
        ("outputs", "Outputs", "buses", profile.buses),
    )
    result: list[dict[str, Any]] = []
    for identifier, display_name, section_name, section in sections:
        if not section:
            continue
        count_keys = ("count", "count_confirmed", "count_assumed_total")
        count = _count(section, count_keys, section_name)
        status = _section_status(section, profile.profile_status or "unknown")
        result.append(
            {
                "id": identifier,
                "name": display_name,
                "kind": _space_kind(section_name),
                "count": count,
                "addressing": _addressing_variant(section.get("addressing")),
                "status": status,
                "status_text": status,
                "notes": str(section.get("notes", "")),
                "metadata": _metadata(section),
            }
        )
    return result


_INPUT_CONTROL_LABELS = {
    "gain": "GAIN",
    "mode": "MODE",
    "phantom": "48V",
    "phase": "Ø",
    "link": "LINK",
}

_INPUT_CONTROL_PARAMETERS = {
    "gain": frozenset(("gain", "adat_gain", "spdif_gain")),
    "mode": frozenset(("input_mode",)),
    "phantom": frozenset(("phantom",)),
    "phase": frozenset(("phase_invert",)),
    "link": frozenset(("channel_link", "adat_channel_link", "spdif_channel_link")),
}

# Raw profiles intentionally do not repeat a device-specific topology block.
# These mappings describe canonical parameter semantics only; section presence,
# finite count, and confirmed parameter evidence still gate every derived record.
_RAW_INPUT_CAPABILITY_SPECS = (
    ("physical_inputs", "gain", "gain", None),
    ("physical_inputs", "mode", "input_mode", None),
    ("physical_inputs", "phantom", "phantom", None),
    ("physical_inputs", "phase", "phase_invert", None),
    ("adat_inputs", "gain", "adat_gain", "adat"),
    ("spdif_inputs", "gain", "spdif_gain", "spdif"),
)


def _derive_raw_input_capability_records(
    profile: NormalizedProfile,
) -> list[dict[str, Any]]:
    """Derive safe input controls from canonical sections and parameters."""

    spaces = {space["id"]: space for space in _build_address_spaces(profile)}
    params = {param["name"]: param for param in _build_params(profile)}
    records: list[dict[str, Any]] = []
    for space, kind, parameter, evidence_token in _RAW_INPUT_CAPABILITY_SPECS:
        space_record = spaces.get(space)
        param_record = params.get(parameter)
        if (
            space_record is None
            or space_record["count"] is None
            or param_record is None
            or _normalized_status(param_record["status"]) != "confirmed"
        ):
            continue
        if evidence_token is not None:
            applies_to = re.sub(r"[^a-z0-9]", "", param_record["applies_to"].lower())
            if evidence_token not in applies_to:
                continue
        records.append(
            {
                "space": space,
                "controls": [
                    {
                        "kind": kind,
                        "parameter": parameter,
                        "label": _INPUT_CONTROL_LABELS[kind],
                    }
                ],
            }
        )

    # Combine fixed semantic records into one validated record per address
    # space.  Link parameters are deliberately absent: raw link-space values
    # do not distinguish physical from ADAT input links.
    combined: dict[str, dict[str, Any]] = {}
    for record in records:
        combined.setdefault(record["space"], {"space": record["space"], "controls": []})[
            "controls"
        ].extend(record["controls"])
    return list(combined.values())


def _validate_input_capability_records(
    profile: NormalizedProfile,
    records: list[Any],
    *,
    context_prefix: str,
) -> dict[str, list[dict[str, Any]]]:
    """Validate explicit or raw-derived input capability records."""

    known_spaces = {space["id"] for space in _build_address_spaces(profile)}
    params = {param["name"]: param for param in _build_params(profile)}
    result: dict[str, list[dict[str, Any]]] = {}
    for space_index, record in enumerate(records):
        context = f"{context_prefix}[{space_index}]"
        if not isinstance(record, Mapping):
            raise ProfileError(f"{context} must be an object")
        space = record.get("space")
        if not isinstance(space, str) or not space.strip() or space not in known_spaces:
            raise ProfileError(f"{context}.space references unknown address space")
        if space in result:
            raise ProfileError(f"{context}.space duplicates input address space")
        controls = record.get("controls")
        if not isinstance(controls, list):
            raise ProfileError(f"{context}.controls must be an array")
        typed: list[dict[str, Any]] = []
        kinds: set[str] = set()
        parameter_ids: set[int] = set()
        for control_index, control in enumerate(controls):
            control_context = f"{context}.controls[{control_index}]"
            if not isinstance(control, Mapping):
                raise ProfileError(f"{control_context} must be an object")
            kind = control.get("kind")
            if not isinstance(kind, str) or kind not in _INPUT_CONTROL_LABELS:
                raise ProfileError(f"{control_context}.kind is an unknown input control kind")
            parameter = control.get("parameter")
            if not isinstance(parameter, str):
                raise ProfileError(f"{control_context}.parameter references unknown parameter")
            capability_context = (
                f"address space {space!r}, kind {kind!r}, parameter key {parameter!r}"
            )
            if parameter not in params:
                raise ProfileError(
                    f"{control_context}.parameter for {capability_context} references unknown parameter"
                )
            if kind in kinds:
                raise ProfileError(
                    f"{control_context}.kind is a duplicate control kind for {capability_context}"
                )
            if parameter not in _INPUT_CONTROL_PARAMETERS[kind]:
                raise ProfileError(f"{control_context} {capability_context} is not a legal input capability")
            label = control.get("label", _INPUT_CONTROL_LABELS[kind])
            if not isinstance(label, str) or not label.strip():
                raise ProfileError(
                    f"{control_context}.label must be non-empty for {capability_context}"
                )
            parameter_id = params[parameter]["id"]
            if "parameter_id" in control:
                supplied_raw = control["parameter_id"]
                supplied_parameter_id = (
                    None
                    if supplied_raw is None
                    else _checked_u16(
                        supplied_raw,
                        f"{control_context}.parameter_id for {capability_context}",
                    )
                )
                if supplied_parameter_id != parameter_id:
                    raise ProfileError(
                        f"{control_context}.parameter_id for {capability_context} must equal "
                        f"referenced parameter id {parameter_id!r}"
                    )
            if parameter_id is not None and parameter_id in parameter_ids:
                raise ProfileError(f"{control_context}.parameter has a duplicate parameter id")
            kinds.add(kind)
            if parameter_id is not None:
                parameter_ids.add(parameter_id)
            typed.append(
                {
                    "kind": kind,
                    "parameter": parameter,
                    "parameter_id": parameter_id,
                    "label": label,
                }
            )
        result[space] = typed
    return result


def _build_input_capabilities(profile: NormalizedProfile) -> dict[str, list[dict[str, Any]]]:
    if "runtime_topology" not in profile.raw:
        records = _derive_raw_input_capability_records(profile)
        return _validate_input_capability_records(
            profile, records, context_prefix="derived_input_spaces"
        )

    topology = profile.raw.get("runtime_topology")
    if topology is None:
        return {}
    if not isinstance(topology, Mapping):
        raise ProfileError("runtime_topology must be an object")
    if _normalized_status(str(topology.get("status", ""))) != "confirmed":
        return {}
    records = topology.get("input_spaces", [])
    if not isinstance(records, list):
        raise ProfileError("runtime_topology.input_spaces must be an array")
    return _validate_input_capability_records(
        profile, records, context_prefix="runtime_topology.input_spaces"
    )


def _build_inputs(profile: NormalizedProfile) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    physical_count = _count(profile.channels, ("count", "count_confirmed", "count_assumed_total"), "channels")
    raw_names = profile.channels.get("names")
    if raw_names is None:
        names: list[Any] = []
    elif isinstance(raw_names, list):
        names = raw_names
    else:
        raise ProfileError("channels.names must be an array when present")
    hiz = set(_int_list(profile.channels.get("hiz_channels"), "channels.hiz_channels"))
    if physical_count is not None:
        for index in range(physical_count):
            label = names[index] if index < len(names) and isinstance(names[index], str) else f"Input {index + 1}"
            result.append(
                {
                    "id": f"physical_{index + 1}",
                    "space": "physical_inputs",
                    "index": index,
                    "name": label,
                    "hiz": index in hiz,
                    "status": _section_status(profile.channels, profile.profile_status or "unknown"),
                    "metadata": _metadata({"source": "channels", "index": index}),
                }
            )

    adat_count = _count(profile.adat, ("count",), "adat")
    if adat_count is not None:
        for index in range(adat_count):
            result.append(
                {
                    "id": f"adat_{index + 1}",
                    "space": "adat_inputs",
                    "index": index,
                    "name": f"ADAT {index + 1}",
                    "hiz": False,
                    "status": _section_status(profile.adat, profile.profile_status or "unknown"),
                    "metadata": _metadata({"source": "adat", "index": index}),
                }
            )

    spdif_count = _count(profile.spdif, ("count",), "spdif")
    spdif_names = ["S/PDIF L", "S/PDIF R"]
    if spdif_count is not None:
        for index in range(spdif_count):
            label = spdif_names[index] if index < len(spdif_names) else f"S/PDIF {index + 1}"
            result.append(
                {
                    "id": f"spdif_{index + 1}",
                    "space": "spdif_inputs",
                    "index": index,
                    "name": label,
                    "hiz": False,
                    "status": _section_status(profile.spdif, profile.profile_status or "unknown"),
                    "metadata": _metadata({"source": "spdif", "index": index}),
                }
            )
    return result


def _build_outputs(profile: NormalizedProfile) -> list[dict[str, Any]]:
    buses = profile.buses
    known = buses.get("known", {})
    if known is not None and not isinstance(known, Mapping):
        raise ProfileError("buses.known must be an object when present")
    result: list[dict[str, Any]] = []
    if isinstance(known, Mapping) and known:
        items: list[tuple[int, Any]] = []
        for key, value in known.items():
            items.append((_checked_u16(key, "buses.known.id"), value))
        items.sort(key=lambda item: item[0])
    else:
        count = _count(buses, ("count",), "buses")
        items = [] if count is None else [(index, {}) for index in range(count)]
    for bus_id, value in items:
        details = value if isinstance(value, Mapping) else {}
        name = details.get("name", f"bus_{bus_id}")
        if not isinstance(name, str) or not name:
            raise ProfileError(f"buses.known.{bus_id}.name must be a non-empty string")
        aliases = details.get("aliases", [])
        if aliases is None:
            aliases = []
        if not isinstance(aliases, list) or any(not isinstance(alias, str) for alias in aliases):
            raise ProfileError(f"buses.known.{bus_id}.aliases must be an array of strings")
        verified = details.get("verified", False)
        if not isinstance(verified, bool):
            raise ProfileError(f"buses.known.{bus_id}.verified must be boolean")
        result.append(
            {
                "id": bus_id,
                "name": name,
                "aliases": aliases,
                "verified": verified,
                "status": _section_status(buses, profile.profile_status or "unknown"),
                "metadata": _metadata(details),
            }
        )
    return result


_TOPOLOGY_CONTROL_FRAME_NAMES = frozenset(
    {"mix_command", "routing_command", "link_command", "readback"}
)


def _raw_control_frame(
    profile: NormalizedProfile, frame_name: str
) -> Mapping[str, Any] | None:
    """Return one recognized raw control record, rejecting malformed records."""

    if frame_name not in profile.frame:
        return None
    value = profile.frame[frame_name]
    if not isinstance(value, Mapping):
        raise ProfileError(f"frame.{frame_name} must be an object")
    return value


def _mixer_text(profile: NormalizedProfile, frame_mixer: Mapping[str, Any]) -> str:
    """Collect only source evidence used to infer mixer geometry."""

    mixer_parts = [
        str(profile.mixer.get(key, ""))
        for key in ("notes", "evidence", "readback")
        if key in profile.mixer
    ]
    frame_parts = [
        str(frame_mixer.get(key, ""))
        for key in ("notes", "evidence")
        if key in frame_mixer
    ]
    readback = _raw_control_frame(profile, "readback")
    if readback is not None:
        mixer_parts.extend(
            str(readback.get(key, ""))
            for key in ("notes", "evidence")
            if key in readback
        )
    return " ".join((*mixer_parts, *frame_parts))


def _readback_category_count(
    profile: NormalizedProfile, category: int, context: str
) -> int | None:
    """Read one explicitly enumerated readback category count."""

    readback = _raw_control_frame(profile, "readback")
    if readback is None or "category_counts" not in readback:
        return None
    counts = readback["category_counts"]
    if not isinstance(counts, Mapping):
        raise ProfileError("frame.readback.category_counts must be an object")
    for raw_category, raw_count in counts.items():
        parsed_category = _checked_u8(raw_category, f"{context}.category")
        if parsed_category == category:
            count = _checked_u16(raw_count, f"{context}.count")
            if count == 0:
                raise ProfileError(f"{context}.count must be positive")
            return count
    return None


def _mixer_geometry_evidence(
    profile: NormalizedProfile,
) -> tuple[int | None, int | None, bool | None, Mapping[str, Any], bool]:
    """Derive mixer dimensions only from typed sections or positive evidence."""

    mixer = profile.mixer
    frame_mixer = _raw_control_frame(profile, "mix_command") or {}
    mix_count = _count(mixer, ("mixes", "count"), "mixer") if mixer else None
    strip_count = _count(
        mixer, ("channels_per_mix", "strip_count", "strips"), "mixer"
    ) if mixer else None
    inferred_mix_count = False
    inferred_strip_count = False
    text = _mixer_text(profile, frame_mixer)

    if mix_count is None:
        readback_count = _readback_category_count(
            profile, 0x04, "frame.readback.category_counts[0x04]"
        )
        if readback_count is not None:
            mix_count = readback_count
            inferred_mix_count = True
    if mix_count is None:
        matches = list(
            re.finditer(
                r"\bmix(?:es)?\s*(?:1\s*[-–]\s*)?(\d+)\b|\b(\d+)\s+mix(?:es)?\b",
                text,
                re.IGNORECASE,
            )
        )
        if matches:
            raw_count = next(
                value for match in matches for value in (match.group(1), match.group(2)) if value
            )
            mix_count = _checked_u16(raw_count, "frame.mix_command.mix_count")
            inferred_mix_count = True

    if strip_count is None:
        # Explicit prose such as "32 input strips" is preferred over broad
        # number matching, because frame notes contain many unrelated offsets.
        match = re.search(r"\b(\d+)\s+input\s+strips?\b", text, re.IGNORECASE)
        if match:
            strip_count = _checked_u16(match.group(1), "frame.mix_command.strip_count")
            inferred_strip_count = True
    if strip_count is None:
        match = re.search(
            r"\bchannel\s*\(?\s*0\s*[-–]\s*(\d+)\b[^.]{0,80}\b(?:input\s+)?strips?\b",
            text,
            re.IGNORECASE,
        )
        if match:
            strip_count = _checked_u16(match.group(1), "frame.mix_command.strip_count")
            inferred_strip_count = True
    if strip_count is None:
        # Orion readback evidence describes 33 slots: slot zero is its master,
        # leaving 32 input strips.  Do not infer this form without master proof.
        match = re.search(r"\b(\d+)\s+(?:three-byte\s+)?slots?\b", text, re.IGNORECASE)
        if match:
            slot_count = _checked_u16(match.group(1), "frame.mix_command.slot_count")
            if slot_count == 0:
                raise ProfileError("frame.mix_command.slot_count must be positive")
            master_hint = bool(
                re.search(r"\bmaster\b", text, re.IGNORECASE)
            )
            if master_hint and slot_count > 1:
                strip_count = slot_count - 1
                inferred_strip_count = True

    has_master: bool | None = None
    if "has_master" in mixer:
        raw_master = mixer["has_master"]
        if not isinstance(raw_master, bool):
            raise ProfileError("mixer.has_master must be boolean")
        has_master = raw_master
    else:
        positive_master = bool(
            re.search(
                r"\b(?:mix\s+)?master\b|\bmaster\s+(?:strip|fader)\b",
                text,
                re.IGNORECASE,
            )
        )
        negative_master = bool(
            re.search(r"\b(?:no|without|excluding)\s+(?:a\s+)?master\b", text, re.IGNORECASE)
        )
        if positive_master and not negative_master:
            has_master = True
        elif not positive_master:
            # A range explicitly described as all input strips proves absence
            # of a separate master slot (Zen Go's 0..15 geometry).
            input_range = re.search(
                r"\bchannel\s*\(?\s*0\s*[-–]\s*(\d+)\b[^.]{0,80}\binput\s+strips?\b",
                text,
                re.IGNORECASE,
            )
            if input_range and strip_count is not None:
                last_channel = _checked_u16(
                    input_range.group(1), "frame.mix_command.strip_count"
                )
                if last_channel + 1 == strip_count:
                    has_master = False
            elif re.search(r"\b\d+\s+input\s+strips?\s+each\b", text, re.IGNORECASE):
                has_master = False

    for key in (
        "mix_offset",
        "channel_offset",
        "fader_offset",
        "pan_flags_offset",
        "send_offset",
    ):
        if key in frame_mixer and frame_mixer[key] is not None:
            _checked_u16(frame_mixer[key], f"frame.mix_command.{key}")

    if mix_count is not None and mix_count == 0:
        raise ProfileError("mixer mix count must be positive")
    if strip_count is not None and strip_count == 0:
        raise ProfileError("mixer strip count must be positive")
    if mix_count is not None and mix_count > 256:
        raise ProfileError("mixer mix count must fit in 1..=256")
    if strip_count is not None and strip_count > 256:
        raise ProfileError("mixer strip count must fit in 1..=256")
    return (
        mix_count,
        strip_count,
        has_master,
        frame_mixer,
        inferred_mix_count or inferred_strip_count,
    )


def _mixer_geometry(
    profile: NormalizedProfile,
) -> tuple[int, int, Mapping[str, Any], bool]:
    mix_count, strip_count, _has_master, frame_mixer, inferred = _mixer_geometry_evidence(profile)
    return mix_count or 0, strip_count or 0, frame_mixer, inferred


def _profile_param_range(profile: NormalizedProfile, names: Sequence[str]) -> tuple[int, int] | None:
    for name in names:
        value = profile.params.get(name)
        if isinstance(value, Mapping) and "range" in value:
            return _range(value.get("range"), f"params.{name}.range")
    return None


def _bounded_count(value: Any, context: str) -> int:
    count = _checked_u16(value, context)
    if not 1 <= count <= 256:
        raise ProfileError(f"{context} must be within 1..=256")
    return count


def _destination_ids(value: Any, context: str) -> list[int]:
    """Parse one raw destination key, including Zen Go's grouped key form."""

    if isinstance(value, bool):
        raise ProfileError(f"{context} must contain integer destination ids")
    text = str(value).strip()
    if not text:
        raise ProfileError(f"{context} must contain integer destination ids")
    parts = [part.strip() for part in text.split(",")]
    if any(not part for part in parts):
        raise ProfileError(f"{context} must contain integer destination ids")
    result = [_checked_u16(part, f"{context}.{index}") for index, part in enumerate(parts)]
    if len(set(result)) != len(result):
        raise ProfileError(f"{context} contains duplicate destination ids")
    return result


def _index_count_from_text(text: str, context: str) -> int | None:
    """Extract finite index bounds explicitly stated beside idx/index labels."""

    number = r"(?:0[xX][0-9a-fA-F]+|[0-9]+)"
    candidates: list[int] = []
    ranges: list[tuple[int, int]] = []
    for label in re.finditer(r"\b(?:idx|index|indices)\b", text, re.IGNORECASE):
        # Parse the first value attached to each label.  Later prose often has
        # unrelated dates or source numbers (for example "preamp 1-12").
        tail = text[label.end() : label.end() + 100]
        immediate = re.match(
            rf"\s*[,=:]?\s*({number})(?:\s*(?:[-–]|\.\.|\bto\b)\s*({number})|\s*/\s*({number}))?",
            tail,
            re.IGNORECASE,
        )
        if not immediate:
            continue
        first = parse_int(immediate.group(1), f"{context}.range.min")
        if first < 0:
            raise ProfileError(f"{context} contains an invalid index")
        second = immediate.group(2) or immediate.group(3)
        if second is None:
            last = first
        else:
            last = parse_int(second, f"{context}.range.max")
            if last < first:
                raise ProfileError(f"{context} contains an invalid index range")
        ranges.append((first, last))
        candidates.append(last + 1)
        # "up to N" is an explicit safe upper bound even when the first
        # sentence names only index zero (the Orion compplay evidence).
        upper = re.search(
            rf"\b(?:up\s+to|max(?:imum)?)\s*({number})\b", tail, re.IGNORECASE
        )
        if upper:
            upper_value = parse_int(upper.group(1), f"{context}.max")
            if upper_value < 0:
                raise ProfileError(f"{context} contains an invalid index bound")
            upper_context = tail[max(0, upper.start() - 24) : upper.end() + 24]
            # "up to 32 channels" gives a count; "idx up to 0x1f"
            # gives an inclusive index bound.
            candidates.append(
                upper_value
                if re.search(r"\bchannels?\b|\bdriver\b", upper_context, re.IGNORECASE)
                else upper_value + 1
            )
        # Some source notes spell a two-entry bank as "0 = L, 1 = R".
        if second is None:
            enum_pair = re.search(rf",\s*({number})\s*=", tail, re.IGNORECASE)
            if enum_pair:
                enum_value = parse_int(enum_pair.group(1), f"{context}.range.max")
                if enum_value != first + 1:
                    raise ProfileError(f"{context} contains a non-contiguous index range")
                ranges[-1] = (first, enum_value)
                candidates.append(enum_value + 1)
    if not candidates:
        return None
    expected = 0
    ordered_ranges = sorted(ranges)
    for range_index, (first, last) in enumerate(ordered_ranges):
        for previous_first, previous_last in ordered_ranges[:range_index]:
            overlaps = first <= previous_last and previous_first <= last
            contained = (first >= previous_first and last <= previous_last) or (
                previous_first >= first and previous_last <= last
            )
            if overlaps and not contained:
                raise ProfileError(f"{context} contains overlapping index ranges")
        if first > expected:
            raise ProfileError(f"{context} contains a non-contiguous index range")
        expected = max(expected, last + 1)
    return _bounded_count(max(candidates), context)


def _index_count_from_raw(value: Any, context: str) -> tuple[int | None, str]:
    """Read string or structured finite source-bank evidence."""

    if isinstance(value, str):
        return _index_count_from_text(value, context), value
    if not isinstance(value, Mapping):
        raise ProfileError(f"{context} must be a string or object")
    evidence = value.get("evidence", value.get("notes", _metadata(value)))
    if evidence is None:
        evidence = _metadata(value)
    if not isinstance(evidence, str):
        raise ProfileError(f"{context}.evidence must be a string")

    indices = value.get("indices")
    parsed_indices: list[int] | None = None
    if indices is not None:
        if not isinstance(indices, list) or not indices:
            raise ProfileError(f"{context}.indices must be a non-empty array")
        parsed_indices = [
            _checked_u16(item, f"{context}.indices[{index}]")
            for index, item in enumerate(indices)
        ]
        if sorted(parsed_indices) != list(range(len(parsed_indices))):
            raise ProfileError(
                f"{context}.indices must contain each zero-based index exactly once"
            )

    candidates: list[tuple[int, str]] = []
    if parsed_indices is not None:
        candidates.append((len(parsed_indices), f"{context}.indices"))

    for key in ("index_count", "count"):
        if key in value and value[key] is not None:
            candidates.append(
                (_bounded_count(value[key], f"{context}.{key}"), f"{context}.{key}")
            )
    for key in ("index_range", "range"):
        if key not in value or value[key] is None:
            continue
        raw_range = value[key]
        if isinstance(raw_range, str):
            count = _index_count_from_text(raw_range, f"{context}.{key}")
            if count is not None:
                candidates.append((count, f"{context}.{key}"))
            continue
        parsed = _range(raw_range, f"{context}.{key}")
        if parsed is None or parsed[0] < 0:
            raise ProfileError(f"{context}.{key} must contain a non-negative finite range")
        candidates.append((
            _bounded_count(parsed[1] + 1, f"{context}.{key}"),
            f"{context}.{key}",
        ))

    evidence_count = _index_count_from_text(evidence, context)
    if evidence_count is not None:
        candidates.append((evidence_count, f"{context}.evidence"))
    if not candidates:
        return None, evidence
    expected = candidates[0][0]
    if any(count != expected for count, _field in candidates[1:]):
        fields = ", ".join(field for _count, field in candidates)
        raise ProfileError(f"{context} index evidence must agree across {fields}")
    return expected, evidence


def _evidence_has_negative_qualifier(
    text: str, *, include_untested: bool = True
) -> bool:
    negative_tokens = (
        r"\b(?:partial|partly|unconfirmed|unverified|unknown|ambiguous|incomplete|conflicting|superseded"
        + (r"|untested" if include_untested else "")
        + r")\b|\bhost[-\s]+dependent\b"
    )
    return bool(
        re.search(negative_tokens, text, re.IGNORECASE)
        or re.search(
            r"\b(?:not|never|no)\b(?:\W+\w+){0,4}\W+\b(?:confirmed|fully\s+decoded)\b",
            text,
            re.IGNORECASE,
        )
    )


def _evidence_is_confirmed(text: str) -> bool:
    """Accept positive confirmation only when evidence has no negative qualifier."""

    if _evidence_has_negative_qualifier(text):
        return False
    return bool(
        re.search(r"\bconfirmed\b", text, re.IGNORECASE)
        or re.search(r"\bfully\s+decoded\b", text, re.IGNORECASE)
    )


def _evidence_negates_subject(text: str, subject: str) -> bool:
    if re.search(
        r"\b(?:not|never|no)\b(?:\W+\w+){0,4}\W+\b(?:confirmed|fully\s+decoded)\b",
        text,
        re.IGNORECASE,
    ):
        return True
    if not _evidence_has_negative_qualifier(text):
        return False
    subject_terms = {
        "routing": r"\b(?:route|routing|destination|source|matrix)\b",
        "link": r"\b(?:mixer\s+link|virtual[-\s]+mixer|space\s*(?:0x03|3))\b",
    }
    return bool(re.search(subject_terms[subject], text, re.IGNORECASE))


def _source_bound_evidence_is_negative(text: str) -> bool:
    return _evidence_has_negative_qualifier(text, include_untested=False) or bool(
        re.search(
            r"\buntested\b(?:\W+\w+){0,4}\W+\b(?:idx|index|indices)\b",
            text,
            re.IGNORECASE,
        )
    )


def _routing_status_is_confirmed(route: Mapping[str, Any], fallback: str) -> bool:
    raw_status = route.get("status")
    status_text = str(raw_status) if raw_status is not None else _section_status(route, fallback)
    if not _evidence_is_confirmed(status_text):
        return False
    for field in ("runtime_status", "notes", "evidence"):
        if field not in route:
            continue
        qualifier = route[field]
        if not isinstance(qualifier, str):
            return False
        if field == "runtime_status":
            if not _evidence_is_confirmed(qualifier):
                return False
        elif _evidence_negates_subject(qualifier, "routing"):
            return False
    return True


def _derived_routing_records(
    profile: NormalizedProfile,
) -> tuple[list[dict[str, Any]], bool]:
    """Derive route groups while keeping partial Zen records non-actionable."""

    route = _raw_control_frame(profile, "routing_command")
    if route is None:
        return [], False
    fallback = profile.profile_status or "unknown"
    confirmed = _routing_status_is_confirmed(route, fallback)
    names: dict[int, str] = {}
    counts: dict[int, int] = {}

    if "addressable_destinations" in route:
        raw_names = route["addressable_destinations"]
        if not isinstance(raw_names, Mapping):
            raise ProfileError("frame.routing_command.addressable_destinations must be an object")
        for raw_destination, raw_name in raw_names.items():
            destination_ids = _destination_ids(
                raw_destination, "frame.routing_command.addressable_destinations"
            )
            if len(destination_ids) != 1:
                raise ProfileError(
                    "frame.routing_command.addressable_destinations keys must identify one destination"
                )
            if not isinstance(raw_name, str) or not raw_name.strip():
                raise ProfileError(
                    "frame.routing_command.addressable_destinations values must be non-empty strings"
                )
            destination = destination_ids[0]
            if destination in names:
                raise ProfileError(
                    "frame.routing_command.addressable_destinations contains duplicate destinations"
                )
            names[destination] = raw_name

    if "destination_channels" in route:
        raw_counts = route["destination_channels"]
        if not isinstance(raw_counts, Mapping):
            raise ProfileError("frame.routing_command.destination_channels must be an object")
        for raw_destination, raw_count in raw_counts.items():
            destination_ids = _destination_ids(
                raw_destination, "frame.routing_command.destination_channels"
            )
            if len(destination_ids) != 1:
                raise ProfileError(
                    "frame.routing_command.destination_channels keys must identify one destination"
                )
            destination = destination_ids[0]
            if destination in counts:
                raise ProfileError(
                    "frame.routing_command.destination_channels contains duplicate destinations"
                )
            counts[destination] = _bounded_count(
                raw_count, f"frame.routing_command.destination_channels.{destination}"
            )

    groups: list[dict[str, Any]] = []
    if names or counts:
        if not names or not counts:
            raise ProfileError(
                "frame.routing_command requires addressable_destinations and destination_channels"
            )
        if set(names) != set(counts):
            raise ProfileError(
                "frame.routing_command destination maps must cover the same groups"
            )
        for destination in sorted(names):
            groups.append(
                {
                    "destination": destination,
                    "name": names[destination],
                    "channel_count": counts[destination],
                    "source_domains": [],
                    "_source_domains_confirmed": confirmed,
                }
            )

    if "destination_groups" in route:
        raw_partial_groups = route["destination_groups"]
        if not isinstance(raw_partial_groups, Mapping):
            raise ProfileError("frame.routing_command.destination_groups must be an object")
        for raw_destination, raw_details in raw_partial_groups.items():
            destination_ids = _destination_ids(
                raw_destination, "frame.routing_command.destination_groups"
            )
            for destination in destination_ids:
                source_domains_confirmed = False
                if isinstance(raw_details, Mapping):
                    if "channel_count" not in raw_details:
                        raise ProfileError(
                            f"frame.routing_command.destination_groups.{destination}.channel_count is required"
                        )
                    channel_count = _bounded_count(
                        raw_details["channel_count"],
                        f"frame.routing_command.destination_groups.{destination}.channel_count",
                    )
                    name = raw_details.get("name", f"destination_{destination}")
                    if not isinstance(name, str) or not name.strip():
                        raise ProfileError(
                            f"frame.routing_command.destination_groups.{destination}.name must be non-empty"
                        )
                    qualifications = [
                        raw_details[field]
                        for field in ("status", "runtime_status", "evidence", "notes")
                        if field in raw_details
                    ]
                    for qualification in qualifications:
                        if not isinstance(qualification, str):
                            raise ProfileError(
                                f"frame.routing_command.destination_groups.{destination} qualification must be text"
                            )
                    source_domains_confirmed = confirmed and bool(qualifications) and all(
                        _evidence_is_confirmed(qualification)
                        for qualification in qualifications
                    )
                elif isinstance(raw_details, str):
                    match = re.search(
                        r"\b(\d+)\s+(?:slots?|channels?)\b", raw_details, re.IGNORECASE
                    )
                    if not match:
                        raise ProfileError(
                            f"frame.routing_command.destination_groups.{destination} lacks finite channel evidence"
                        )
                    channel_count = _bounded_count(
                        match.group(1),
                        f"frame.routing_command.destination_groups.{destination}.channel_count",
                    )
                    name = f"destination_{destination}"
                    source_domains_confirmed = confirmed and _evidence_is_confirmed(raw_details)
                else:
                    raise ProfileError(
                        f"frame.routing_command.destination_groups.{destination} must be a string or object"
                    )
                if any(group["destination"] == destination for group in groups):
                    raise ProfileError(
                        f"frame.routing_command.destination_groups.{destination} duplicates a destination"
                    )
                groups.append(
                    {
                        "destination": destination,
                        "name": name,
                        "channel_count": channel_count,
                        "source_domains": [],
                        "_source_domains_confirmed": source_domains_confirmed,
                    }
                )

    groups.sort(key=lambda group: group["destination"])
    return groups, confirmed


def _derived_routing_source_domains(
    profile: NormalizedProfile, route: Mapping[str, Any], confirmed: bool
) -> list[dict[str, Any]]:
    """Build finite source banks only for a confirmed routing map."""

    if "source_banks" not in route:
        return []
    raw_banks = route["source_banks"]
    if not isinstance(raw_banks, Mapping):
        raise ProfileError("frame.routing_command.source_banks must be an object")
    banks: list[dict[str, Any]] = []
    seen_banks: set[int] = set()
    for raw_bank, raw_evidence in raw_banks.items():
        bank = _checked_u8(raw_bank, "frame.routing_command.source_banks.bank")
        if bank in seen_banks:
            raise ProfileError(
                f"frame.routing_command.source_banks contains duplicate source bank {bank:#04x}"
            )
        seen_banks.add(bank)
        bank_context = f"frame.routing_command.source_banks.{bank:#04x}"
        count, evidence = _index_count_from_raw(raw_evidence, bank_context)
        if count is None:
            # Source record remains available in frame metadata, but no safe
            # action domain can be generated without a finite bound.
            continue
        qualifications = [evidence]
        status_qualifications: list[str] = []
        if isinstance(raw_evidence, Mapping):
            for field in ("status", "runtime_status", "evidence", "notes"):
                if field not in raw_evidence:
                    continue
                qualifier = raw_evidence[field]
                if not isinstance(qualifier, str):
                    raise ProfileError(f"{bank_context}.{field} must be a string")
                qualifications.append(qualifier)
                if field in ("status", "runtime_status"):
                    status_qualifications.append(qualifier)
        if any(not _evidence_is_confirmed(qualifier) for qualifier in status_qualifications):
            continue
        if any(_source_bound_evidence_is_negative(qualifier) for qualifier in qualifications):
            continue
        # Bank 0x02 is host-dependent: Windows exposes 24 channels while
        # native macOS exposes up to 32.  Keep its raw evidence metadata-only.
        evidence_text = evidence.lower()
        if bank == 0x02:
            continue
        # Oscillator has a finite-looking note but no confirmation.  It is a
        # pseudo-source, so keep it metadata-only until raw evidence confirms it.
        if bank == 0x0C and not re.search(r"\bconfirmed\b", evidence_text):
            continue
        banks.append(
            {
                "bank": bank,
                "index_count": count,
                "status": "confirmed",
                "evidence": evidence,
            }
        )
    if not confirmed or not banks:
        return []
    return [
        {
            "id": "derived_routing_sources",
            "status": "confirmed",
            "evidence": "frame.routing_command.source_banks finite index evidence",
            "banks": banks,
        }
    ]


def _derived_link_domains(
    profile: NormalizedProfile,
) -> list[dict[str, Any]]:
    """Derive only confirmed mixer link space; preserve other spaces as metadata."""

    link = _raw_control_frame(profile, "link_command")
    if link is None or "space_values" not in link:
        return []
    for field in ("status", "runtime_status", "notes", "evidence"):
        if field not in link:
            continue
        qualifier = link[field]
        if not isinstance(qualifier, str):
            raise ProfileError(f"frame.link_command.{field} must be a string")
        if field in ("status", "runtime_status"):
            if not _evidence_is_confirmed(qualifier):
                return []
        elif _evidence_negates_subject(qualifier, "link"):
            return []

    raw_spaces = link["space_values"]
    if not isinstance(raw_spaces, Mapping):
        raise ProfileError("frame.link_command.space_values must be an object")
    mixer_evidence: str | None = None
    seen_spaces: set[int] = set()
    for raw_space, raw_meaning in raw_spaces.items():
        space = _checked_u8(raw_space, "frame.link_command.space_values.space")
        if space in seen_spaces:
            raise ProfileError(
                f"frame.link_command.space_values contains duplicate link space {space:#04x}"
            )
        seen_spaces.add(space)
        if not isinstance(raw_meaning, str) or not raw_meaning.strip():
            raise ProfileError(
                f"frame.link_command.space_values.{space:#04x} must be a non-empty string"
            )
        if space == 3:
            mixer_evidence = raw_meaning
    if mixer_evidence is None:
        return []
    if not _evidence_is_confirmed(mixer_evidence):
        return []
    _mix_count, strip_count, _has_master, _frame_mixer, _inferred = _mixer_geometry_evidence(profile)
    if strip_count is None or strip_count < 2:
        return []
    if strip_count % 2:
        raise ProfileError("frame.link_command mixer geometry must contain complete pairs")
    return [
        {
            "protocol_space": 3,
            "kind": "mixer",
            "pair_count": _bounded_count(strip_count // 2, "frame.link_command.pair_count"),
            "status": "confirmed",
            "evidence": mixer_evidence,
        }
    ]


def _derive_runtime_topology(profile: NormalizedProfile) -> Mapping[str, Any] | None:
    """Derive topology from raw sections when no confirmed topology block exists."""

    # Validate recognized records even when their evidence is incomplete.  A
    # malformed record must never disappear into an empty typed section.
    for frame_name in _TOPOLOGY_CONTROL_FRAME_NAMES:
        _raw_control_frame(profile, frame_name)

    topology: dict[str, Any] = {}
    mix_count, strip_count, has_master, frame_mixer, _inferred = _mixer_geometry_evidence(profile)
    if mix_count is not None and strip_count is not None and has_master is not None:
        offsets = {
            key: _checked_u16(frame_mixer[key], f"frame.mix_command.{key}")
            for key in (
                "mix_offset",
                "channel_offset",
                "fader_offset",
                "pan_flags_offset",
                "send_offset",
            )
            if key in frame_mixer and frame_mixer[key] is not None
        }
        topology["mixer"] = {
            "has_master": has_master,
            "mix_count": mix_count,
            "strip_count": strip_count,
            "offsets": offsets,
        }

    groups, routing_confirmed = _derived_routing_records(profile)
    source_domains = _derived_routing_source_domains(
        profile, _raw_control_frame(profile, "routing_command") or {}, routing_confirmed
    )
    if source_domains:
        for group in groups:
            if not group.get("_source_domains_confirmed", False):
                continue
            group["source_domains"] = [
                dict(domain) for domain in source_domains[0]["banks"]
            ]
    if groups or _raw_control_frame(profile, "routing_command") is not None:
        topology["routing_groups"] = groups
    if source_domains:
        topology["routing_source_domains"] = source_domains
    link_domains = _derived_link_domains(profile)
    if link_domains or _raw_control_frame(profile, "link_command") is not None:
        topology["link_domains"] = link_domains
    return topology if topology else None


def _confirmed_runtime_topology(profile: NormalizedProfile) -> Mapping[str, Any] | None:
    explicit = profile.raw.get("runtime_topology")
    if explicit is not None:
        if not isinstance(explicit, Mapping):
            raise ProfileError("runtime_topology must be an object")
        if _normalized_status(str(explicit.get("status", ""))) == "confirmed":
            return explicit
    return _derive_runtime_topology(profile)


def _build_link_domains(profile: NormalizedProfile) -> list[dict[str, Any]]:
    topology = _confirmed_runtime_topology(profile)
    if topology is None:
        return []
    records = topology.get("link_domains", [])
    if not isinstance(records, list):
        raise ProfileError("runtime_topology.link_domains must be an array")
    result: list[dict[str, Any]] = []
    seen: dict[int, Any] = {}
    for index, record in enumerate(records):
        context = f"runtime_topology.link_domains[{index}]"
        if not isinstance(record, Mapping):
            raise ProfileError(f"{context} must be an object")
        protocol_space = _checked_u8(record.get("protocol_space"), f"{context}.protocol_space")
        kind = record.get("kind")
        pair_count = _checked_u16(record.get("pair_count"), f"{context}.pair_count")
        status = str(record.get("status", ""))
        evidence = record.get("evidence")
        if kind not in {"mixer", "physical", "adat"}:
            raise ProfileError(f"{context}.kind is not a supported closed link-domain kind")
        if (
            protocol_space in seen
            and not (
                _is_orion(profile)
                and protocol_space == 0
                and kind in {"physical", "adat"}
                and seen[protocol_space] in {"physical", "adat"}
            )
        ) or (
            not 1 <= pair_count <= 256
            or _normalized_status(status) != "confirmed"
            or not isinstance(evidence, str)
            or not evidence.strip()
        ):
            raise ProfileError(
                f"{context} requires a unique protocol space, confirmed evidence, and pair_count within 1..=256"
            )
        seen[protocol_space] = kind
        if _is_orion(profile) and kind == "mixer" and protocol_space != 3:
            continue
        if kind in {"physical", "adat"}:
            if _is_orion(profile):
                continue
            raise ProfileError(f"{context}.kind is not a supported closed link-domain kind")
        result.append(
            {
                "protocol_space": protocol_space,
                "kind": kind,
                "pair_count": pair_count,
                "status": status,
                "evidence": evidence,
            }
        )
    return result


def _routing_source_domain_sets(
    topology: Mapping[str, Any],
) -> dict[str, list[dict[str, Any]]]:
    records = topology.get("routing_source_domains", [])
    if not isinstance(records, list):
        raise ProfileError("runtime_topology.routing_source_domains must be an array")
    result: dict[str, list[dict[str, Any]]] = {}
    for set_index, record in enumerate(records):
        context = f"runtime_topology.routing_source_domains[{set_index}]"
        if not isinstance(record, Mapping):
            raise ProfileError(f"{context} must be an object")
        identifier = record.get("id")
        status = str(record.get("status", ""))
        evidence = record.get("evidence")
        banks = record.get("banks")
        if (
            not isinstance(identifier, str)
            or not identifier.strip()
            or identifier in result
            or _normalized_status(status) != "confirmed"
            or not isinstance(evidence, str)
            or not evidence.strip()
            or not isinstance(banks, list)
            or not banks
        ):
            raise ProfileError(f"{context} requires unique id, confirmed evidence, and banks")
        typed: list[dict[str, Any]] = []
        seen_banks: set[int] = set()
        for bank_index, bank_record in enumerate(banks):
            bank_context = f"{context}.banks[{bank_index}]"
            if not isinstance(bank_record, Mapping):
                raise ProfileError(f"{bank_context} must be an object")
            bank = _checked_u8(bank_record.get("bank"), f"{bank_context}.bank")
            index_count = _checked_u16(
                bank_record.get("index_count"), f"{bank_context}.index_count"
            )
            bank_evidence = bank_record.get("evidence", evidence)
            if (
                bank in seen_banks
                or not 1 <= index_count <= 256
                or not isinstance(bank_evidence, str)
                or not bank_evidence.strip()
            ):
                raise ProfileError(
                    f"{bank_context} requires unique bank, finite index_count within 1..=256, and evidence"
                )
            seen_banks.add(bank)
            typed.append(
                {
                    "bank": bank,
                    "index_count": index_count,
                    "status": status,
                    "evidence": bank_evidence,
                }
            )
        result[identifier] = typed
    return result


def _runtime_topology(profile: NormalizedProfile) -> tuple[bool | None, list[dict[str, Any]]]:
    topology = _confirmed_runtime_topology(profile)
    if topology is None:
        return None, []
    explicit = profile.raw.get("runtime_topology")
    explicit_confirmed = isinstance(explicit, Mapping) and _normalized_status(
        str(explicit.get("status", ""))
    ) == "confirmed"
    mixer = topology.get("mixer")
    if explicit_confirmed:
        if not isinstance(mixer, Mapping) or not isinstance(mixer.get("has_master"), bool):
            raise ProfileError("confirmed runtime_topology.mixer.has_master must be boolean")
        source_domain_sets = _routing_source_domain_sets(topology)
        groups = topology.get("routing_groups")
        if not isinstance(groups, list):
            raise ProfileError("confirmed runtime_topology.routing_groups must be an array")
        result: list[dict[str, Any]] = []
        seen: set[int] = set()
        for index, group in enumerate(groups):
            context = f"runtime_topology.routing_groups[{index}]"
            if not isinstance(group, Mapping):
                raise ProfileError(f"{context} must be an object")
            destination = _checked_u16(group.get("destination"), f"{context}.destination")
            name = group.get("name")
            if not isinstance(name, str) or not name.strip():
                raise ProfileError(f"{context}.name must be non-empty")
            channel_count = _checked_u16(group.get("channel_count"), f"{context}.channel_count")
            if channel_count == 0 or destination in seen:
                raise ProfileError(
                    "runtime_topology routing destinations must be unique with positive channel counts"
                )
            source_domain_id = group.get("source_domain")
            if source_domain_id is None:
                source_domains: list[dict[str, Any]] = []
            elif not isinstance(source_domain_id, str) or source_domain_id not in source_domain_sets:
                raise ProfileError(f"{context}.source_domain references an unknown confirmed domain")
            else:
                source_domains = [dict(domain) for domain in source_domain_sets[source_domain_id]]
            seen.add(destination)
            result.append(
                {
                    "destination": destination,
                    "name": name,
                    "channel_count": channel_count,
                    "source_domains": source_domains,
                }
            )
        return mixer["has_master"], result

    # Derived records already passed raw-evidence validation.  Keep incomplete
    # sections typed but non-actionable instead of requiring a synthetic master.
    has_master: bool | None = None
    if mixer is not None:
        if not isinstance(mixer, Mapping):
            raise ProfileError("derived runtime mixer must be an object")
        if "has_master" in mixer:
            if not isinstance(mixer["has_master"], bool):
                raise ProfileError("derived runtime mixer.has_master must be boolean")
            has_master = mixer["has_master"]
    groups = topology.get("routing_groups", [])
    if not isinstance(groups, list):
        raise ProfileError("derived runtime routing_groups must be an array")
    result: list[dict[str, Any]] = []
    seen: set[int] = set()
    for index, group in enumerate(groups):
        context = f"derived runtime_topology.routing_groups[{index}]"
        if not isinstance(group, Mapping):
            raise ProfileError(f"{context} must be an object")
        destination = _checked_u16(group.get("destination"), f"{context}.destination")
        name = group.get("name")
        if not isinstance(name, str) or not name.strip():
            raise ProfileError(f"{context}.name must be non-empty")
        channel_count = _checked_u16(group.get("channel_count"), f"{context}.channel_count")
        if channel_count == 0 or destination in seen:
            raise ProfileError(
                "derived runtime routing destinations must be unique with positive channel counts"
            )
        source_domains = group.get("source_domains", [])
        if not isinstance(source_domains, list):
            raise ProfileError(f"{context}.source_domains must be an array")
        seen.add(destination)
        result.append(
            {
                "destination": destination,
                "name": name,
                "channel_count": channel_count,
                "source_domains": [dict(domain) for domain in source_domains],
            }
        )
    return has_master, result


def _build_routing_groups(profile: NormalizedProfile) -> list[dict[str, Any]]:
    return _runtime_topology(profile)[1]


def _fader_domain(profile: NormalizedProfile) -> dict[str, Any] | None:
    fader = profile.mixer.get("fader", {})
    if not isinstance(fader, Mapping):
        raise ProfileError("mixer.fader must be an object")
    domain_keys = {"min", "max", "direction", "unity"}
    if not domain_keys.intersection(fader):
        return None
    missing = domain_keys - fader.keys()
    if missing:
        missing_text = ", ".join(sorted(missing))
        raise ProfileError(f"mixer.fader missing required domain fields: {missing_text}")
    minimum = _checked_i32(fader["min"], "mixer.fader.min")
    maximum = _checked_i32(fader["max"], "mixer.fader.max")
    if minimum > maximum:
        raise ProfileError("mixer.fader.min must not exceed mixer.fader.max")
    direction = fader["direction"]
    if not isinstance(direction, str) or not direction.strip():
        raise ProfileError("mixer.fader.direction must be a non-empty string")
    direction = direction.strip().lower()
    if direction not in {"direct", "attenuation"}:
        raise ProfileError("mixer.fader.direction must be direct or attenuation")
    unity = _checked_i32(fader["unity"], "mixer.fader.unity")
    if not minimum <= unity <= maximum:
        raise ProfileError("mixer.fader.unity must fit mixer.fader min/max")
    return {"min": minimum, "max": maximum, "direction": direction, "unity": unity}


def _build_mixers(profile: NormalizedProfile) -> list[dict[str, Any]]:
    has_master, _ = _runtime_topology(profile)
    if has_master is None:
        return []
    mix_count, strip_count, frame_mixer, inferred_geometry = _mixer_geometry(profile)
    if not mix_count or not strip_count:
        return []
    fader = profile.mixer.get("fader", {}) if isinstance(profile.mixer.get("fader", {}), Mapping) else {}
    pan = profile.mixer.get("pan", {}) if isinstance(profile.mixer.get("pan", {}), Mapping) else {}
    fader_range = _range(fader.get("range"), "mixer.fader.range")
    fader_range = fader_range or _profile_param_range(profile, ("mix_fader",))
    fader_domain = _fader_domain(profile)
    pan_range = _range(pan.get("range_deg"), "mixer.pan.range_deg")
    pan_range = pan_range or _profile_param_range(profile, ("mix_pan",))
    pan_center_value = pan.get("center", frame_mixer.get("pan_center"))
    pan_center = _try_int(pan_center_value, "mixer.pan.center")
    if pan_center is not None:
        pan_center = _checked_i32(pan_center, "mixer.pan.center")
    send_range = _profile_param_range(profile, ("mix_send",))
    mixer_status = (
        _section_status(profile.mixer, "unknown")
        if profile.mixer
        else _section_status(frame_mixer, profile.profile_status or "unknown")
    )
    status = "observed" if inferred_geometry else mixer_status
    result = []
    for mix_index in range(mix_count):
        _checked_u8(mix_index, "mixer.mix_index")
        result.append(
            {
                "id": f"mix_{mix_index + 1}",
                "name": f"Mix {mix_index + 1}",
                "mix_index": mix_index,
                "strip_count": strip_count,
                "has_master": has_master,
                "fader_range": fader_range,
                "fader": fader_domain,
                "pan_range": pan_range,
                "pan_center": pan_center,
                "send_range": send_range,
                "status": status,
                "status_text": status,
                "notes": str(profile.mixer.get("notes", frame_mixer.get("notes", ""))),
                "metadata": _metadata({"mixer": profile.mixer, "frame": frame_mixer}),
            }
        )
    return result


_NUMERIC_FIELD_NAMES = {
    "bit",
    "bits",
    "bytes_per_bus",
    "channel_bias",
    "count",
    "id",
    "ids",
    "magic",
    "mask",
    "mix_wet_constant",
    "mixes",
    "opcode",
    "opcodes",
    "offset",
    "param_id",
    "pan_center",
    "shift",
    "stride",
    "strip_count",
    "subcmd",
    "value",
    "values",
    "width",
}


def _has_numeric_variant(name: str, stem: str) -> bool:
    return bool(re.search(rf"(?:^|_){re.escape(stem)}(?:_|$)", name.lower()))


def _is_numeric_field(name: str) -> bool:
    normalized = name.lower()
    if normalized.endswith(("_name", "_note", "_notes")):
        return normalized in _NUMERIC_FIELD_NAMES
    return (
        normalized in _NUMERIC_FIELD_NAMES
        or any(
            _has_numeric_variant(normalized, stem)
            for stem in ("offset", "opcode", "magic", "id", "ids", "stride", "mask", "bit", "bits", "count", "index", "width")
        )
        or bool(re.fullmatch(r"(?:0x)?[0-9a-f]+", normalized))
    )


def _is_integer_literal(value: Any) -> bool:
    if isinstance(value, bool):
        return False
    if isinstance(value, int):
        return True
    if not isinstance(value, str):
        return False
    text = value.strip()
    return bool(re.fullmatch(r"[+-]?(?:0[xX][0-9a-fA-F]+|[0-9]+)", text))


def _numeric_field_width(name: str, context: str) -> tuple[int, int]:
    """Return target bounds for a generated frame field."""

    normalized = name.lower()
    context_parts = context.lower().split(".")
    if normalized == "channel_bias":
        return -(2**31), 2**31 - 1
    if (
        normalized == "offset"
        or normalized.endswith("_offset")
        or normalized == "stride"
        or normalized.endswith("_stride")
    ):
        return 0, 0xFFFF
    if (
        normalized in {"mask", "bit", "bits", "magic", "opcode", "subcmd"}
        or _has_numeric_variant(normalized, "mask")
        or _has_numeric_variant(normalized, "bit")
        or _has_numeric_variant(normalized, "bits")
        or _has_numeric_variant(normalized, "magic")
        or _has_numeric_variant(normalized, "opcode")
    ):
        return 0, 0xFF
    if (
        normalized in {"id", "ids", "offset", "stride", "width", "count"}
        or any(
            _has_numeric_variant(normalized, stem)
            for stem in ("id", "ids", "offset", "stride", "width", "count", "index")
        )
        or "param_offsets" in context_parts
    ):
        return 0, 0xFFFF
    return -(2**31), 2**31 - 1


def _parse_frame_number(value: Any, name: str, context: str) -> int:
    minimum, maximum = _numeric_field_width(name, context)
    return _checked_int(value, context, minimum, maximum)


def _frame_field(name: str, value: Any, context: str) -> dict[str, Any]:
    """Normalize one scalar or nested frame field without dropping geometry."""

    children: list[dict[str, Any]] = []
    values: list[int] = []
    text = ""
    numeric: int | None = None
    normalized_name = name.lower()
    if isinstance(value, Mapping):
        children = [
            _frame_field(str(child_name), child_value, f"{context}.{child_name}")
            for child_name, child_value in value.items()
            if not str(child_name).startswith("_")
        ]
    elif isinstance(value, list):
        if all(not isinstance(item, (Mapping, list)) for item in value):
            numeric_list = _is_numeric_field(name) or all(
                isinstance(item, int) and not isinstance(item, bool) for item in value
            ) or all(_is_integer_literal(item) for item in value)
            if numeric_list:
                values = [
                    _parse_frame_number(item, name, f"{context}[{index}]")
                    for index, item in enumerate(value)
                ]
            else:
                text = _json(value)
        else:
            text = _json(value)
    elif _is_numeric_field(name) or (
        isinstance(value, int) and not isinstance(value, bool)
    ) or _is_integer_literal(value):
        if value is None:
            numeric = None
        elif re.fullmatch(r"(?:0x)?[0-9a-f]+", normalized_name) and not _is_integer_literal(value):
            text = str(value)
        else:
            numeric = _parse_frame_number(value, name, context)
    else:
        text = "" if value is None else str(value)

    is_offset = _is_frame_offset_name(normalized_name)
    is_stride = normalized_name == "stride" or normalized_name.endswith("_stride")
    is_mask = (
        normalized_name in {"mask", "bit", "bits"}
        or normalized_name.endswith("_mask")
        or normalized_name.endswith("_bit")
        or normalized_name.endswith("_bits")
    )
    is_width = normalized_name == "width" or normalized_name.endswith("_width")
    offset = numeric if is_offset else None
    stride = numeric if is_stride else None
    mask = numeric if is_mask else None
    return {
        "name": name,
        "offset": offset,
        "stride": stride,
        "width": numeric if is_width else None,
        "value": numeric if numeric is not None and not (is_offset or is_stride or is_mask or is_width) else None,
        "mask": mask,
        "values": values,
        "formula": str(value) if normalized_name == "formula" else "",
        "text": text,
        "children": children,
    }


def _frame_fields(frame: Mapping[str, Any], frame_name: str) -> list[dict[str, Any]]:
    ignored = {"status", "notes", "evidence"}
    return [
        _frame_field(key, value, f"frame.{frame_name}.{key}")
        for key, value in frame.items()
        if not key.startswith("_") and key not in ignored
    ]


def _validate_frame_numeric_fields(frame: Mapping[str, Any], context: str) -> None:
    """Reject malformed and out-of-width values in nested frame geometry."""

    for key, value in frame.items():
        key_text = str(key)
        if key_text.startswith("_"):
            continue
        field_context = f"{context}.{key_text}"
        if isinstance(value, Mapping):
            _validate_frame_numeric_fields(value, field_context)
            continue
        if isinstance(value, list):
            if any(isinstance(item, Mapping) for item in value):
                for index, item in enumerate(value):
                    if isinstance(item, Mapping):
                        _validate_frame_numeric_fields(item, f"{field_context}[{index}]")
            if _is_numeric_field(key_text):
                for index, item in enumerate(value):
                    if isinstance(item, (Mapping, list)):
                        continue
                    _parse_frame_number(item, key_text, f"{field_context}[{index}]")
            continue
        if _is_numeric_field(key_text):
            # Null is incomplete evidence, not malformed geometry; readiness
            # classification handles it as disabled for known identities.
            if value is None:
                continue
            # Numeric object keys (for example status_values["0x10"]) can
            # legitimately point at descriptive strings.
            if re.fullmatch(r"(?:0x)?[0-9a-f]+", key_text.lower()) and not _is_integer_literal(value):
                continue
            _parse_frame_number(value, key_text, field_context)
        elif isinstance(value, int) and not isinstance(value, bool):
            _checked_i32(value, field_context)


def _build_frames(profile: NormalizedProfile) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    for frame_name, value in profile.frame.items():
        if frame_name.startswith("_"):
            continue
        if not isinstance(value, Mapping):
            if frame_name in _TOPOLOGY_CONTROL_FRAME_NAMES:
                raise ProfileError(f"frame.{frame_name} must be an object")
            continue
        status = _effective_frame_status(profile, frame_name, value)
        kind = _frame_kind(frame_name)
        # Selectable packs reject unconfirmed command frames.  These whole-state
        # Orion shapes have no generic parameter mapping; retain their raw frame
        # metadata as decoder records instead of exposing unsafe actions.
        if (
            _is_orion(profile)
            and kind == "Command"
            and _status_variant(status) != "Confirmed"
            and frame_name in {"auraverb_command", "micmodeling_command"}
        ):
            kind = "Decoder"
        magic_offset_value = value.get("magic_offset")
        magic_value = value.get("magic")
        opcode_offset_value = value.get("opcode_offset")
        opcode_value = value.get("opcode")
        magic_offset = (
            _checked_u16(magic_offset_value, f"frame.{frame_name}.magic_offset")
            if magic_offset_value is not None
            else None
        )
        magic = _checked_u8(magic_value, f"frame.{frame_name}.magic") if magic_value is not None else None
        opcode_offset = (
            _checked_u16(opcode_offset_value, f"frame.{frame_name}.opcode_offset")
            if opcode_offset_value is not None
            else None
        )
        opcode = _checked_u8(opcode_value, f"frame.{frame_name}.opcode") if opcode_value is not None else None
        result.append(
            {
                "id": frame_name,
                "kind": kind,
                "status": status,
                "status_text": status,
                "magic_offset": magic_offset,
                "magic": magic,
                "opcode_offset": opcode_offset,
                "opcode": opcode,
                "opcode_name": str(value.get("opcode_name", "")),
                "fields": _frame_fields(value, frame_name),
                "metadata": _metadata(value),
            }
        )
    return result


def _build_decoders(frames: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    return [
        {
            "id": f"decode_{frame['id']}",
            "frame_id": frame["id"],
            "kind": frame["kind"],
            "status": frame["status"],
            "metadata": frame["metadata"],
        }
        for frame in frames
        if frame["kind"] != "Command"
    ]


_REFERENCE_OFFSET_RE = re.compile(
    r"(?:@|(?:\b|\.)[A-Za-z0-9_]*offset|\bbytes?)\s*"
    r"(?:\(\s*)?(0[xX][0-9a-fA-F]+|[0-9]+)",
    re.IGNORECASE,
)
_REFERENCE_FORMULA_NUMBER_RE = re.compile(r"(?<![A-Za-z0-9_])(0[xX][0-9a-fA-F]+|[0-9]+)(?![A-Za-z0-9_])")


def _reference_offset(value: Any, context: str) -> int:
    return _checked_u16(value, context)


def _parameter_reference(value: Any, context: str) -> dict[str, Any]:
    """Normalize string or structured reference while exposing offsets/formulas."""

    if value is None:
        return {"text": "", "formula": "", "offsets": []}
    if isinstance(value, str):
        text = value
        formula = ""
        offset_sources = [
            (f"offset_{index}", match.group(1), "")
            for index, match in enumerate(_REFERENCE_OFFSET_RE.finditer(text))
        ]
    elif isinstance(value, Mapping):
        text = _json(value)
        formula_value = value.get("formula", "")
        if formula_value is None:
            formula = ""
        elif isinstance(formula_value, str):
            formula = formula_value
        else:
            raise ProfileError(f"{context}.formula must be a string when present")
        offset_sources: list[tuple[str, Any, str]] = []

        def add_mapping_offset(name: str, raw: Any, offset_context: str, offset_formula: str = "") -> None:
            if raw is None:
                return
            if isinstance(raw, str) and not _is_integer_literal(raw):
                # Symbolic references such as ``value_offset`` are resolved by
                # their accompanying formula/text, not mistaken for numbers.
                if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", raw):
                    raise ProfileError(f"{offset_context} must be an integer or symbolic offset")
                return
            offset_sources.append((name, raw, offset_formula))

        for key, raw in value.items():
            key_text = str(key)
            if key_text == "offset" or key_text.endswith("_offset"):
                add_mapping_offset(key_text, raw, f"{context}.{key_text}", formula)
            elif key_text == "offsets":
                if isinstance(raw, Mapping):
                    for offset_name, offset_value in raw.items():
                        add_mapping_offset(
                            str(offset_name), offset_value, f"{context}.offsets.{offset_name}", formula
                        )
                elif isinstance(raw, list):
                    for index, offset_value in enumerate(raw):
                        if isinstance(offset_value, Mapping):
                            offset_name = str(offset_value.get("name", f"offset_{index}"))
                            offset_raw = offset_value.get("offset")
                            offset_formula = str(offset_value.get("formula", formula))
                            add_mapping_offset(
                                offset_name,
                                offset_raw,
                                f"{context}.offsets[{index}].offset",
                                offset_formula,
                            )
                        else:
                            add_mapping_offset(
                                f"offset_{index}",
                                offset_value,
                                f"{context}.offsets[{index}]",
                                formula,
                            )
                else:
                    raise ProfileError(f"{context}.offsets must be an object or array")
        # A mapping formula can carry a literal base offset even when its
        # offset member names a symbolic profile field.
        if formula:
            for match in _REFERENCE_FORMULA_NUMBER_RE.finditer(formula):
                offset_sources.append((f"offset_{len(offset_sources)}", match.group(1), formula))
    else:
        raise ProfileError(f"{context} must be a string or object when present")

    offsets: list[dict[str, Any]] = []
    seen: set[tuple[int, str]] = set()
    for index, (name, raw_offset, offset_formula) in enumerate(offset_sources):
        offset = _reference_offset(raw_offset, f"{context}.{name}")
        key = (offset, offset_formula)
        if key in seen:
            continue
        seen.add(key)
        offsets.append(
            {
                "name": name or f"offset_{index}",
                "offset": offset,
                "formula": offset_formula,
            }
        )
    return {"text": text, "formula": formula, "offsets": offsets}


def _normalize_range_form(value: Any, context: str) -> tuple[int, int] | str:
    if isinstance(value, (list, Mapping)):
        parsed = _range(value, context)
        assert parsed is not None
        return parsed
    if isinstance(value, str):
        # Textual forms (for example Zen Go's approximate line-gain range) are
        # still typed as range-form text instead of being dropped to metadata.
        return value
    raise ProfileError(f"{context} must be a range array, min/max object, or explanatory string")


def _range_form_entry(name: str, value: Any, context: str) -> dict[str, Any]:
    normalized = _normalize_range_form(value, context)
    if isinstance(normalized, tuple):
        return {"name": name, "range": normalized, "text": ""}
    return {"name": name, "range": None, "text": normalized}


def _orion_runtime_parameter_defaults(name: str) -> dict[str, Any] | None:
    """Return source-backed runtime shape for Orion command parameters.

    Orion's canonical parameter records predate this typed runtime schema.  Keep
    those records untouched, but fill only mappings whose command and readback
    offsets are explicitly described by the profile evidence.
    """

    command = {
        "text": "frame.command SET_PARAM: parameter @16, target @17, value @18",
        "offsets": [
            {"name": "param_id", "offset": 16},
            {"name": "channel", "offset": 17},
            {"name": "value", "offset": 18},
        ],
    }
    global_command = {
        "text": "frame.global_command SET_GLOBAL: parameter @16, value @17",
        "offsets": [
            {"name": "param_id", "offset": 16},
            {"name": "value", "offset": 17},
        ],
    }
    state = {
        "input_mode": ("state_report status byte offset 61 + channel, bits 0-1", 61),
        "gain": ("state_report gain_base_offset 49 + channel", 49),
        "phantom": ("state_report status byte offset 61 + channel, bit 0x10", 61),
        "phase_invert": ("state_report status byte offset 61 + channel, bit 0x40", 61),
        "bus_level": ("state_report bus_block level offset 28 + 3 * bus_id", 28),
        "bus_dim": ("state_report bus_block status offset 29 + 3 * bus_id, bit 0x08", 29),
        "bus_mute": ("state_report bus_block status offset 29 + 3 * bus_id, bit 0x04", 29),
        "bus_mono": ("state_report bus_block status offset 29 + 3 * bus_id, bit 0x10", 29),
        "sample_rate": ("state_report sample-rate byte offset 18", 18),
        "screen_brightness": ("state_report screen-brightness byte offset 26", 26),
        "adat_gain": ("state_report adat_gain_base_offset 75 + ADAT channel", 75),
        "talkback_button": ("state_report talkback status offset 73, bit 0x40", 73),
        "talkback_source": ("state_report talkback status offset 73, source bits 0-1", 73),
        "talkback_gain": ("state_report talkback gain offset 74", 74),
        "talkback_dest_assign": ("state_report talkback status offset 73, destination bits 2-5", 73),
        "spdif_gain": ("state_report spdif_gain_base_offset 91 + S/PDIF channel", 91),
    }
    applies_to = {
        "input_mode": "physical_inputs",
        "gain": "physical_inputs",
        "phantom": "physical_inputs",
        "phase_invert": "physical_inputs",
        "bus_level": "outputs",
        "bus_dim": "outputs",
        "bus_mute": "outputs",
        "bus_mono": "outputs",
        "sample_rate": "globals",
        "screen_brightness": "globals",
        "adat_gain": "adat_inputs",
        "talkback_button": "globals",
        "talkback_source": "globals",
        "talkback_gain": "globals",
        "talkback_dest_assign": "globals",
        "spdif_gain": "spdif_inputs",
    }
    readback = state.get(name)
    target = applies_to.get(name)
    if readback is None or target is None:
        return None
    text, offset = readback
    return {
        "applies_to": target,
        "frame": command if target not in {"globals"} else global_command,
        "readback": {
            "text": text,
            "offsets": [{"name": "offset", "offset": offset}],
        },
    }


def _orion_source_only_parameter(name: str) -> bool:
    """Return whether Orion ID lacks a safe generic command mapping."""

    return name in {
        "output_trim",
        "routing",
        "oscillator",
        "surround_monitor",
        "dc_coupling",
        "talkback_dest_assign",
    }


def _build_params(profile: NormalizedProfile) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    runtime_profile = profile.raw.get("runtime_profile", {})
    compile_confirmed_only = isinstance(runtime_profile, Mapping) and runtime_profile.get(
        "compile_confirmed_only"
    ) is True
    for name, value in profile.params.items():
        if name.startswith("_"):
            continue
        if not isinstance(value, Mapping):
            raise ProfileError(f"params.{name} must be an object")
        if compile_confirmed_only and value.get("runtime_available") is not True:
            continue
        status = _section_status(value, profile.profile_status or "unknown")
        runtime_defaults = (
            _orion_runtime_parameter_defaults(name) if _is_orion(profile) else None
        )
        range_source = value.get("runtime_range", value.get("range"))
        if range_source is None and name == "gain" and _is_orion(profile):
            range_source = [0, 75]
        parameter_range = _range(range_source, f"params.{name}.runtime_range")
        range_by_mode: dict[str, tuple[int, int] | str] = {}
        per_mode_range: dict[str, tuple[int, int] | str] = {}
        range_forms: list[dict[str, Any]] = []
        for range_key in ("per_mode_range", "range_by_mode"):
            raw_ranges = value.get(range_key)
            if raw_ranges is None:
                continue
            if isinstance(raw_ranges, Mapping) and not ("min" in raw_ranges or "max" in raw_ranges):
                for mode, raw_range in raw_ranges.items():
                    mode_name = str(mode)
                    normalized_range = _normalize_range_form(
                        raw_range, f"params.{name}.{range_key}.{mode_name}"
                    )
                    range_by_mode[mode_name] = normalized_range
                    if range_key == "per_mode_range":
                        per_mode_range[mode_name] = normalized_range
                    range_forms.append(
                        _range_form_entry(
                            f"{range_key}.{mode_name}",
                            raw_range,
                            f"params.{name}.{range_key}.{mode_name}",
                        )
                    )
            else:
                range_forms.append(
                    _range_form_entry(range_key, raw_ranges, f"params.{name}.{range_key}")
                )
        # Preserve any future named range form without silently reducing it to
        # raw JSON metadata.  The two canonical mode aliases above share the
        # convenient range_by_mode view.
        for range_key, raw_ranges in value.items():
            if "range" not in str(range_key).lower() or range_key in {
                "range",
                "per_mode_range",
                "range_by_mode",
            }:
                continue
            if isinstance(raw_ranges, Mapping) and not ("min" in raw_ranges or "max" in raw_ranges):
                for mode, raw_range in raw_ranges.items():
                    range_forms.append(
                        _range_form_entry(
                            f"{range_key}.{mode}",
                            raw_range,
                            f"params.{name}.{range_key}.{mode}",
                        )
                    )
            else:
                range_forms.append(
                    _range_form_entry(range_key, raw_ranges, f"params.{name}.{range_key}")
                )
        values: list[dict[str, Any]] = []
        enum_values = value.get("values")
        if enum_values is None and _is_orion(profile) and name in {"output_trim", "talkback_source"}:
            if name == "output_trim":
                enum_values = {str(index): f"raw_{index}" for index in range(7)}
            else:
                enum_values = {"0": "INT"} | {
                    str(index): f"input_{index}" for index in range(1, 13)
                }
        if isinstance(enum_values, Mapping):
            for raw_key, raw_value in enum_values.items():
                number = _checked_i32(raw_key, f"params.{name}.values.key")
                label = str(raw_value)
                values.append({"value": number, "name": label})
            values.sort(key=lambda item: item["value"])
        elif enum_values is not None:
            raise ProfileError(f"params.{name}.values must be an object")
        frame_value = value.get("runtime_frame", value.get("frame"))
        readback_value = value.get("runtime_readback", value.get("readback"))
        applies_to = str(value.get("runtime_applies_to", value.get("applies_to", "")))
        if runtime_defaults is not None:
            if frame_value is None:
                frame_value = runtime_defaults["frame"]
            if readback_value is None:
                readback_value = runtime_defaults["readback"]
            if not applies_to:
                applies_to = runtime_defaults["applies_to"]
        if readback_value is None and "state_report_offset_formula" in value:
            readback_value = value.get("state_report_offset_formula")
        result.append(
            {
                "name": name,
                # Unconfirmed Orion IDs are source evidence only. Omitting ID keeps
                # strict selectable-pack validation from treating them as actions;
                # raw ID remains in metadata above.
                "id": (
                    _checked_u16(value["id"], f"params.{name}.id")
                    if value.get("id") is not None
                    and (not _is_orion(profile) or (
                        _status_variant(status) == "Confirmed"
                        and not _orion_source_only_parameter(name)
                    ))
                    else None
                ),
                "value_type": _param_type(value.get("type")),
                "status": status,
                "status_text": status,
                "applies_to": applies_to,
                "range": parameter_range,
                "range_by_mode": range_by_mode,
                "per_mode_range": per_mode_range,
                "range_forms": range_forms,
                "values": values,
                "frame_reference": _parameter_reference(frame_value, f"params.{name}.frame"),
                "readback_reference": _parameter_reference(readback_value, f"params.{name}.readback"),
                "encoding": str(value.get("encoding", "")),
                "metadata": _metadata(value),
            }
        )
    return result


def _constraint_is_bounds(name: str) -> bool:
    return any(part.lower().endswith("_bounds") for part in name.split("."))


def _constraint_is_opcode_list(name: str) -> bool:
    return name.split(".")[-1].lower() in {"opcodes", "allowed_opcodes", "forbidden_opcodes"}


def _constraint_scalar(value: Any, context: str) -> int:
    return _checked_i32(value, context)


def _empty_constraint(
    name: str,
    *,
    status: str,
    range_value: tuple[int | None, int | None] | None = None,
    values: Sequence[int] = (),
    scalar: int | None = None,
    text: str = "",
    metadata: Any = None,
) -> dict[str, Any]:
    return {
        "name": name,
        "status": status,
        "range": range_value,
        "values": list(values),
        "scalar": scalar,
        "text": text,
        "metadata": _metadata(metadata),
    }


def _flatten_constraints(
    name: str, value: Any, *, range_list: bool = False
) -> list[dict[str, Any]]:
    """Turn nested bounds into independently typed constraint records."""

    if isinstance(value, Mapping):
        if "min" in value or "max" in value:
            minimum = (
                _checked_i32(value["min"], f"constraints.{name}.min") if "min" in value else None
            )
            maximum = (
                _checked_i32(value["max"], f"constraints.{name}.max") if "max" in value else None
            )
            if minimum is not None and maximum is not None and minimum > maximum:
                raise ProfileError(f"constraints.{name} lower bound must not exceed upper bound")
            return [
                _empty_constraint(
                    name,
                    status=str(value.get("status", "confirmed")),
                    range_value=(minimum, maximum),
                    text=str(value.get("note", value.get("notes", ""))),
                    metadata=value,
                )
            ]
        nested: list[dict[str, Any]] = []
        for child_name, child_value in value.items():
            if str(child_name).startswith("_"):
                continue
            child_full_name = f"{name}.{child_name}"
            if isinstance(child_value, (Mapping, list)):
                nested.extend(
                    _flatten_constraints(
                        child_full_name,
                        child_value,
                        range_list=range_list or _constraint_is_bounds(name),
                    )
                )
        if nested:
            return nested
    if isinstance(value, list):
        if all(not isinstance(item, (Mapping, list)) for item in value):
            numeric_limit = 0xFF if _constraint_is_opcode_list(name) else 2**31 - 1
            numeric_minimum = 0 if _constraint_is_opcode_list(name) else -(2**31)
            numbers = [
                _checked_int(item, f"constraints.{name}[{index}]", numeric_minimum, numeric_limit)
                for index, item in enumerate(value)
            ]
            if (range_list or _constraint_is_bounds(name)) and len(numbers) == 2:
                if numbers[0] > numbers[1]:
                    raise ProfileError(f"constraints.{name} lower bound must not exceed upper bound")
                return [
                    _empty_constraint(
                        name,
                        status="confirmed",
                        range_value=(numbers[0], numbers[1]),
                        metadata=value,
                    )
                ]
            return [
                _empty_constraint(name, status="confirmed", values=numbers, metadata=value)
            ]
    if isinstance(value, (int, str)) and not isinstance(value, bool):
        # Numeric constraint scalars stay typed.  Known scalar names reject a
        # malformed value instead of silently moving it into descriptive text.
        scalar_names = {"min_write_interval_ms"}
        if isinstance(value, int) or name.split(".")[-1].lower() in scalar_names:
            scalar = _constraint_scalar(value, f"constraints.{name}")
            return [_empty_constraint(name, status="confirmed", scalar=scalar, metadata=value)]
    return [_empty_constraint(name, status="unknown", text=str(value), metadata=value)]


def _build_constraints(profile: NormalizedProfile) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    for name, value in profile.constraints.items():
        if name.startswith("_"):
            continue
        result.extend(_flatten_constraints(name, value))
    return result


def _build_hazards(profile: NormalizedProfile) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    for name, value in profile.hazards.items():
        if name.startswith("_"):
            continue
        details = value if isinstance(value, Mapping) else {}
        opcodes = (
            _int_list(details.get("opcodes"), f"hazards.{name}.opcodes", minimum=0, maximum=0xFF)
            if details
            else []
        )
        result.append(
            {
                "name": name,
                "status": str(details.get("status", "confirmed")) if details else "unknown",
                "rule": str(details.get("rule", "")),
                "effect": str(details.get("effect", "")),
                "notes": str(details.get("notes", "")),
                "opcodes": opcodes,
                "metadata": _metadata(value),
            }
        )
    return result


def _orion_generated_hazards(profile: NormalizedProfile) -> list[dict[str, Any]]:
    if not _is_orion(profile):
        return []
    return [{
        "name": "orion_framing_assumption",
        "status": "pending",
        "rule": "uses_numbered_reports=false",
        "effect": "hardware verification pending",
        "notes": "Source-backed runtime assumption; verify Orion HID report framing on hardware.",
        "opcodes": [],
        "metadata": _metadata({"raw": "transport.uses_numbered_reports is absent", "verification": "pending"}),
    }]


def _normalized_status(text: str | None) -> str:
    return _status_variant(text).lower()


def _normalized_kind(variant: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", variant).lower()


def _profile_id(profile: NormalizedProfile) -> str:
    identifier = re.sub(r"[^a-z0-9]+", "_", profile.path.stem.lower()).strip("_")
    if not identifier:
        raise ProfileError(f"cannot derive stable profile id from {profile.path}")
    return identifier


def _runtime_driver_kind(profile: NormalizedProfile, readiness: Readiness) -> str:
    if readiness is Readiness.SUPPORTED and (profile.identity.vid, profile.identity.pid) == (0x23E5, 0xA015):
        return "zen_go"
    if readiness is Readiness.SUPPORTED and _is_orion(profile):
        return "profile"
    return "none"


def _support_reason(profile: NormalizedProfile, readiness: Readiness) -> str:
    if _is_orion(profile) and readiness is Readiness.SUPPORTED:
        return "validated source-backed profile; assumes unnumbered HID reports pending hardware verification"
    return {
        Readiness.SUPPORTED: "validated built-in driver",
        Readiness.PARTIAL: "profile data is incomplete for safe read/write control",
        Readiness.UNVERIFIED: "transport or frame geometry is unverified",
        Readiness.DISABLED: "profile is not enabled for control",
    }[readiness]


def _operation_max_index(
    profile: NormalizedProfile,
    frame_id: str,
    index_field: str,
    frame: Mapping[str, Any],
) -> int:
    """Return finite reachable index bound from confirmed normalized geometry."""

    field = index_field.lower()
    if "routing" in frame_id:
        destinations = frame.get("destination_channels")
        if isinstance(destinations, Mapping) and destinations:
            counts = [_checked_u16(value, f"frame.{frame_id}.destination_channels") for value in destinations.values()]
            if counts and min(counts) > 0:
                return max(counts) - 1
    if "bus" in field or frame_id == "state_report":
        outputs = _build_outputs(profile)
        if outputs:
            return max(output["id"] for output in outputs)
    if "mix" in field or "channel" in field:
        mixers = _build_mixers(profile)
        if "mix" in frame_id and mixers:
            return max(mixer["strip_count"] for mixer in mixers)
        inputs = _build_inputs(profile)
        if inputs:
            return max(input["index"] for input in inputs)
    for key in ("max_index", "max_safe_index"):
        if frame.get(key) is not None:
            return _checked_u16(frame[key], f"frame.{frame_id}.{key}")
    raise ProfileError(
        f"frame.{frame_id}.{index_field} has stride geometry without a proven finite domain"
    )


def _pair_max_index(profile: NormalizedProfile, frame_id: str) -> int:
    pair_counts: list[int] = []
    for section_name, section in (
        ("channels", profile.channels),
        ("adat", profile.adat),
        ("spdif", profile.spdif),
    ):
        count = _count(section, ("count", "count_confirmed", "count_assumed_total"), section_name)
        if count:
            pair_counts.append(count // 2)
    pair_counts.extend(
        mixer["strip_count"] // 2 for mixer in _build_mixers(profile) if mixer["strip_count"]
    )
    if not pair_counts or max(pair_counts) == 0:
        raise ProfileError(f"frame.{frame_id}.pair_index has no proven finite pair domain")
    return max(pair_counts) - 1


def _obsolete_orion_channel_meter_operation(
    profile: NormalizedProfile, frame_id: str, operation: Mapping[str, Any]
) -> bool:
    """Exclude superseded 0x75 channel indexing, while retaining other geometry."""

    return (
        _is_orion(profile)
        and frame_id == "meter_report"
        and operation.get("op") == "indexed"
        and operation.get("index_field") == "channel_meter"
        and operation.get("max_index", 0) > 0
    )


def _disambiguate_orion_inferred_semantics(
    profile: NormalizedProfile, frame_id: str, operations: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    """Suffix only Orion's inferred, non-lookup duplicate semantic fields."""

    if not _is_orion(profile):
        return operations
    lookup_names = {
        "gain_base",
        "status_base",
        "adat_gain_base",
        "spdif_gain_base",
        "channel_meter_base",
        "physical_meter",
        "gain",
        "state",
        "input_mode",
        "input_phantom",
        "input_phase",
        "output_mute",
        "output_dim",
    }
    seen: dict[str, int] = {}
    # Reserve lookup names so generated aliases cannot collide with runtime semantics
    # emitted later in traversal order.
    used: set[str] = set(lookup_names)
    result: list[dict[str, Any]] = []
    for operation in operations:
        if operation.get("op") not in {"scalar", "bit_field"}:
            result.append(operation)
            continue
        key = operation.get("field")
        if key is None or key in lookup_names:
            if key is not None:
                used.add(key)
            result.append(operation)
            continue
        count = seen.get(key, 0)
        candidate = key
        while candidate in used:
            count += 1
            candidate = f"{key}__{count + 1}"
        seen[key] = count
        used.add(candidate)
        if candidate != key:
            operation = dict(operation)
            operation["field"] = candidate
        result.append(operation)
    return result


def _retain_meter_report_operations(
    profile: NormalizedProfile, frame_id: str, operations: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    obsolete_operations = [
        operation
        for operation in operations
        if _obsolete_orion_channel_meter_operation(profile, frame_id, operation)
    ]
    # Keep malformed superseded geometry visible so readiness can fail closed;
    # only suppress it after report-bound validation succeeds.
    if obsolete_operations and not _operations_fit_report(profile, obsolete_operations):
        return operations
    return [
        operation
        for operation in operations
        if not _obsolete_orion_channel_meter_operation(profile, frame_id, operation)
    ]


def _frame_operations(
    profile: NormalizedProfile, frame_id: str, frame: Mapping[str, Any]
) -> list[dict[str, Any]]:
    """Compile allowlisted canonical frame geometry into finite-domain operations."""

    explicit = frame.get("runtime_operations")
    if explicit is not None:
        if not isinstance(explicit, list):
            raise ProfileError(f"frame.{frame_id}.runtime_operations must be an array")
        operations: list[dict[str, Any]] = []
        for index, raw in enumerate(explicit):
            context = f"frame.{frame_id}.runtime_operations[{index}]"
            if not isinstance(raw, Mapping):
                raise ProfileError(f"{context} must be an object")
            kind = str(raw.get("op", ""))
            if kind == "fixed_byte":
                operation = {
                    "op": kind,
                    "offset": _checked_u16(raw.get("offset"), f"{context}.offset"),
                    "value": _checked_u8(raw.get("value"), f"{context}.value"),
                }
            elif kind == "scalar":
                field = str(raw.get("field", "")).strip()
                if not field:
                    raise ProfileError(f"{context}.field must be non-empty")
                width = _checked_u8(raw.get("width", 1), f"{context}.width")
                endian = str(raw.get("endian", "not_applicable"))
                if endian not in {"not_applicable", "little", "big"}:
                    raise ProfileError(f"{context}.endian is invalid")
                operation = {
                    "op": kind,
                    "field": field,
                    "offset": _checked_u16(raw.get("offset"), f"{context}.offset"),
                    "width": width,
                    "endian": endian,
                }
            elif kind == "indexed":
                field = str(raw.get("index_field", "")).strip()
                if not field:
                    raise ProfileError(f"{context}.index_field must be non-empty")
                operation = {
                    "op": kind,
                    "base": _checked_u16(raw.get("base"), f"{context}.base"),
                    "stride": _checked_u16(raw.get("stride"), f"{context}.stride"),
                    "index_field": field,
                    "width": _checked_u8(raw.get("width"), f"{context}.width"),
                    "max_index": _checked_u16(raw.get("max_index"), f"{context}.max_index"),
                }
            elif kind == "bit_field":
                field = str(raw.get("field", "")).strip()
                if not field:
                    raise ProfileError(f"{context}.field must be non-empty")
                operation = {
                    "op": kind,
                    "field": field,
                    "offset": _checked_u16(raw.get("offset"), f"{context}.offset"),
                    "mask": _checked_u8(raw.get("mask"), f"{context}.mask"),
                    "shift": _checked_u8(raw.get("shift"), f"{context}.shift"),
                }
            elif kind == "pair_index":
                field = str(raw.get("pair_field", "")).strip()
                if not field:
                    raise ProfileError(f"{context}.pair_field must be non-empty")
                operation = {
                    "op": kind,
                    "base": _checked_u16(raw.get("base"), f"{context}.base"),
                    "stride": _checked_u16(raw.get("stride"), f"{context}.stride"),
                    "pair_field": field,
                    "width": _checked_u8(raw.get("width"), f"{context}.width"),
                    "max_index": _checked_u16(raw.get("max_index"), f"{context}.max_index"),
                }
            elif kind == "allowed_values":
                values = raw.get("values")
                if not isinstance(values, list) or not values:
                    raise ProfileError(f"{context}.values must be a non-empty array")
                operation = {
                    "op": kind,
                    "values": [_checked_i32(value, f"{context}.values") for value in values],
                }
            elif kind == "uncompiled_formula":
                formula = str(raw.get("formula", "")).strip()
                if not formula:
                    raise ProfileError(f"{context}.formula must be non-empty")
                operation = {"op": kind, "formula": formula}
            else:
                raise ProfileError(f"{context}.op {kind!r} is unsupported")
            operations.append(operation)
        if frame_id == "state_report":
            operations.extend(_orion_state_meter_operations(profile))
            operations.extend(_candidate_preamp_meter_operations(profile))
        return _retain_meter_report_operations(profile, frame_id, operations)

    operations: list[dict[str, Any]] = []
    seen: set[str] = set()

    def add(operation: dict[str, Any]) -> None:
        key = _json(operation)
        if key not in seen:
            seen.add(key)
            operations.append(operation)

    def walk(value: Mapping[str, Any], context: str, inherited_offset: int | None = None) -> None:
        local_offset = inherited_offset
        for key in ("offset", "byte_offset", "base_offset"):
            if key in value and value[key] is not None:
                local_offset = _checked_u16(value[key], f"{context}.{key}")
                break

        for key, raw in value.items():
            key_text = str(key)
            if key_text.startswith("_"):
                continue
            if key_text.endswith("_offset") and raw is not None and not isinstance(raw, (Mapping, list)):
                offset = _checked_u16(raw, f"{context}.{key_text}")
                stem = key_text[: -len("_offset")]
                fixed = value.get(stem)
                if fixed is not None and _is_integer_literal(fixed):
                    number = parse_int(fixed, f"{context}.{stem}")
                    if 0 <= number <= 0xFF:
                        add({"op": "fixed_byte", "offset": offset, "value": number})
                        continue
                if "pair_index" in stem:
                    add(
                        {
                            "op": "pair_index",
                            "base": offset,
                            "stride": 1,
                            "pair_field": stem,
                            "width": 1,
                            "max_index": _pair_max_index(profile, frame_id),
                        }
                    )
                    continue
                width_raw = value.get(f"{stem}_width", 1)
                width = _checked_u8(width_raw, f"{context}.{stem}_width")
                if width == 1:
                    add(
                        {
                            "op": "scalar",
                            "field": stem,
                            "offset": offset,
                            "width": width,
                            "endian": "not_applicable",
                        }
                    )
                else:
                    add(
                        {
                            "op": "uncompiled_formula",
                            "formula": f"unproven endianness for {stem} width {width}",
                        }
                    )
            elif key_text in {"offset", "byte_offset", "base_offset"} and raw is not None and not isinstance(raw, (Mapping, list)):
                add(
                    {
                        "op": "scalar",
                        "field": context.rsplit(".", 1)[-1],
                        "offset": _checked_u16(raw, f"{context}.{key_text}"),
                        "width": 1,
                        "endian": "not_applicable",
                    }
                )
            elif (key_text == "values" or key_text.endswith("_values")) and isinstance(raw, list):
                values = [_checked_i32(item, f"{context}.{key_text}") for item in raw]
                if values:
                    add({"op": "allowed_values", "values": values})
            elif (key_text in {"mask", "bit", "bits"} or key_text.endswith(("_mask", "_bit", "_bits"))) and raw is not None and not isinstance(raw, (Mapping, list)) and local_offset is not None:
                mask = _checked_u8(raw, f"{context}.{key_text}")
                shift_key = key_text.removesuffix("_mask") + "_shift"
                shift = _checked_u8(value.get(shift_key, (mask & -mask).bit_length() - 1 if mask else 0), f"{context}.{shift_key}")
                field = key_text
                for suffix in ("_mask", "_bit", "_bits"):
                    field = field.removesuffix(suffix)
                add(
                    {
                        "op": "bit_field",
                        "field": field,
                        "offset": local_offset,
                        "mask": mask,
                        "shift": shift,
                    }
                )
            elif key_text == "formula" and isinstance(raw, str) and raw.strip():
                if "// 2" in raw and local_offset is not None:
                    add(
                        {
                            "op": "pair_index",
                            "base": local_offset,
                            "stride": 1,
                            "pair_field": context,
                            "width": 1,
                            "max_index": _pair_max_index(profile, frame_id),
                        }
                    )
                else:
                    add({"op": "uncompiled_formula", "formula": raw})
            if isinstance(raw, Mapping):
                walk(raw, f"{context}.{key_text}", local_offset)

        offset_fields = [
            (str(key), raw)
            for key, raw in value.items()
            if (str(key) in {"offset", "base_offset"} or str(key).endswith("_offset"))
            and raw is not None
            and not isinstance(raw, (Mapping, list))
        ]
        stride_fields = [
            (str(key), raw)
            for key, raw in value.items()
            if (str(key) == "stride" or str(key).endswith("_stride") or str(key) == "bytes_per_bus")
            and raw is not None
        ]
        for stride_name, stride_raw in stride_fields:
            stem = stride_name.removesuffix("_stride")
            base = next(
                (
                    raw
                    for name, raw in offset_fields
                    if name == "base_offset" or (stem and name.startswith(stem))
                ),
                None,
            )
            if base is None:
                base = next(
                    (raw for name, raw in offset_fields if not name.startswith(("magic", "opcode"))),
                    None,
                )
            if base is not None:
                add(
                    {
                        "op": "indexed",
                        "base": _checked_u16(base, f"{context}.base_offset"),
                        "stride": _checked_u16(stride_raw, f"{context}.{stride_name}"),
                        "index_field": stem or context,
                        "width": 1,
                        "max_index": _operation_max_index(
                            profile, frame_id, stem or context, frame
                        ),
                    }
                )

    walk(frame, "frame")
    if frame_id == "state_report":
        operations.extend(_orion_state_meter_operations(profile))
        operations.extend(_candidate_preamp_meter_operations(profile))
    operations = _disambiguate_orion_inferred_semantics(profile, frame_id, operations)
    return _retain_meter_report_operations(profile, frame_id, operations)


def _candidate_preamp_meters(profile: NormalizedProfile) -> list[dict[str, Any]]:
    state = profile.frame.get("state_report")
    if not isinstance(state, Mapping) or "candidate_preamp_meters" not in state:
        return []
    raw_meters = state["candidate_preamp_meters"]
    if not isinstance(raw_meters, list):
        raise ProfileError("frame.state_report.candidate_preamp_meters must be an array")
    meters: list[dict[str, Any]] = []
    for index, raw_meter in enumerate(raw_meters):
        context = f"frame.state_report.candidate_preamp_meters[{index}]"
        if not isinstance(raw_meter, Mapping):
            raise ProfileError(f"{context} must be an object")
        input_index = _checked_u8(raw_meter.get("input_index"), f"{context}.input_index")
        offset = _checked_u16(raw_meter.get("offset"), f"{context}.offset")
        status = raw_meter.get("status")
        caveat = raw_meter.get("caveat")
        if not isinstance(status, str) or not status.strip():
            raise ProfileError(f"{context}.status must be a non-empty string")
        if not isinstance(caveat, str) or not caveat.strip():
            raise ProfileError(f"{context}.caveat must be a non-empty string")
        meters.append(
            {
                "input_index": input_index,
                "offset": offset,
                "status": status,
                "caveat": caveat,
            }
        )
    return meters


def _candidate_preamp_meter_operations(profile: NormalizedProfile) -> list[dict[str, Any]]:
    return [
        {
            "op": "scalar",
            "field": f"candidate_preamp_meter_{meter['input_index']}",
            "offset": meter["offset"],
            "width": 1,
            "endian": "not_applicable",
            "input_index": meter["input_index"],
            "status": meter["status"],
            "caveat": meter["caveat"],
        }
        for meter in _candidate_preamp_meters(profile)
    ]


def _readback_definition(profile: NormalizedProfile) -> tuple[dict[str, Any] | None, list[dict[str, int]]]:
    raw = profile.frame.get("readback")
    if not isinstance(raw, Mapping):
        return None, []
    required = {
        "request_magic": raw.get("request_magic"),
        "request_subcommand": raw.get("request_subcommand", raw.get("subcmd")),
        "response_magic": raw.get("response_magic"),
        "response_discriminator_offset": raw.get("response_discriminator_offset"),
        "response_discriminator": raw.get("response_discriminator"),
        "category_offset": raw.get("category_offset"),
        "index_offset": raw.get("index_offset"),
        "data_offset": raw.get("data_offset"),
    }
    if any(value is None for value in required.values()):
        return None, []
    counts_raw = raw.get("category_counts", {})
    if not isinstance(counts_raw, Mapping):
        raise ProfileError("frame.readback.category_counts must be an object")
    counts = sorted(
        (
            {
                "category": _checked_u8(category, "frame.readback.category_counts.category"),
                "count": _checked_u16(count, "frame.readback.category_counts.count"),
            }
            for category, count in counts_raw.items()
        ),
        key=lambda item: item["category"],
    )
    readback = {
        "request_magic": _checked_u8(required["request_magic"], "frame.readback.request_magic"),
        "request_subcommand": _checked_int(required["request_subcommand"], "frame.readback.request_subcommand", 0, 0xFFFF_FFFF),
        "response_magic": _checked_u8(required["response_magic"], "frame.readback.response_magic"),
        "response_discriminator_offset": _checked_u16(required["response_discriminator_offset"], "frame.readback.response_discriminator_offset"),
        "response_discriminator": _checked_u8(required["response_discriminator"], "frame.readback.response_discriminator"),
        "category_offset": _checked_u16(required["category_offset"], "frame.readback.category_offset"),
        "index_offset": _checked_u16(required["index_offset"], "frame.readback.index_offset"),
        "data_offset": _checked_u16(required["data_offset"], "frame.readback.data_offset"),
        "category_counts": counts,
    }
    bounds = {item["category"]: item["count"] for item in counts}

    def parse_queries(raw_queries: Any, field: str) -> list[dict[str, int]]:
        if not isinstance(raw_queries, list):
            raise ProfileError(f"{field} must be an array")
        queries: list[dict[str, int]] = []
        for record_index, record in enumerate(raw_queries):
            context = f"{field}[{record_index}]"
            if not isinstance(record, Mapping):
                raise ProfileError(f"{context} must be an object")
            category = _checked_u8(record.get("category"), f"{context}.category")
            index = _checked_u8(record.get("index"), f"{context}.index")
            count = bounds.get(category)
            if count is not None and index >= count:
                raise ProfileError(
                    f"{context} category {category:#04x} index {index} is outside confirmed finite bounds"
                )
            queries.append({"category": category, "index": index})
        return queries

    has_explicit_safe_queries = "safe_queries" in raw
    safe_queries = parse_queries(raw["safe_queries"], "frame.readback.safe_queries") if has_explicit_safe_queries else None
    if safe_queries is None:
        explicit_startup = raw.get("startup_queries")
        if explicit_startup is not None:
            safe_queries = parse_queries(explicit_startup, "frame.readback.startup_queries")
        else:
            safe_queries = [
                {"category": item["category"], "index": index}
                for item in counts
                for index in range(item["count"])
                if index <= 0xFF
            ]
    # Preserve ordered startup walks even when derived from category counts or
    # legacy startup_queries. Explicit sparse pairs remain authoritative.
    readback["safe_queries"] = safe_queries

    layouts_raw = raw.get("layouts", [])
    if not isinstance(layouts_raw, list):
        raise ProfileError("frame.readback.layouts must be an array")
    safe_pairs = {(item["category"], item["index"]) for item in safe_queries}
    layouts: list[dict[str, Any]] = []
    seen_layout_pairs: set[tuple[int, int]] = set()
    for layout_index, raw_layout in enumerate(layouts_raw):
        context = f"frame.readback.layouts[{layout_index}]"
        if not isinstance(raw_layout, Mapping):
            raise ProfileError(f"{context} must be an object")
        category = _checked_u8(raw_layout.get("category"), f"{context}.category")
        index = _checked_u8(raw_layout.get("index"), f"{context}.index")
        if (category, index) not in safe_pairs:
            raise ProfileError(f"{context} query {category:#04x}:{index} is not in frame.readback.safe_queries")
        if (category, index) in seen_layout_pairs:
            raise ProfileError(f"{_profile_id(profile)}: {context} duplicates query {category:#04x}:{index}")
        seen_layout_pairs.add((category, index))
        kind = raw_layout.get("kind")
        status = raw_layout.get("status")
        if not isinstance(kind, str) or not kind.strip():
            raise ProfileError(f"{context}.kind must be a non-empty string")
        if not isinstance(status, str) or not status.strip():
            raise ProfileError(f"{context}.status must be a non-empty string")
        body_size = _checked_u16(raw_layout.get("body_size"), f"{context}.body_size")
        record_count = _checked_u16(raw_layout.get("record_count"), f"{context}.record_count")
        record_stride = _checked_u16(raw_layout.get("record_stride"), f"{context}.record_stride")
        if body_size == 0 or record_count == 0 or record_stride == 0:
            raise ProfileError(f"{context} body_size, record_count, and record_stride must be positive")
        if record_count * record_stride > body_size:
            raise ProfileError(f"{context}.record_count * record_stride exceeds body_size")
        normalized_layout = dict(raw_layout)
        normalized_layout.update(
            {
                "category": category,
                "index": index,
                "body_size": body_size,
                "record_count": record_count,
                "record_stride": record_stride,
            }
        )
        for field in ("level_offset", "state_offset"):
            if field not in raw_layout:
                raise ProfileError(f"{context}.{field} is required")
            offset = _checked_u16(raw_layout[field], f"{context}.{field}")
            final_byte = (record_count - 1) * record_stride + offset + 1
            if final_byte > body_size:
                raise ProfileError(f"{context}.{field} final record span exceeds body_size")
            normalized_layout[field] = offset
        if "surface_stride" in raw_layout:
            surface_stride = _checked_u16(raw_layout["surface_stride"], f"{context}.surface_stride")
            if surface_stride == 0:
                raise ProfileError(f"{context}.surface_stride must be positive")
            if surface_stride > record_count or surface_stride * record_stride > body_size:
                raise ProfileError(f"{context}.surface_stride final span exceeds body_size")
            normalized_layout["surface_stride"] = surface_stride
        if "surface" in raw_layout:
            normalized_layout["surface"] = _checked_u8(raw_layout["surface"], f"{context}.surface")
        if "supported_fields" in raw_layout:
            fields = raw_layout["supported_fields"]
            if not isinstance(fields, list) or not all(isinstance(field, str) and field.strip() for field in fields):
                raise ProfileError(f"{context}.supported_fields must be an array of non-empty strings")
        layouts.append(normalized_layout)
    if "layouts" in raw:
        readback["layouts"] = layouts
    return readback, [{"query_id": item["category"], "sub_id": item["index"]} for item in safe_queries]


def _normalized_profile_record(profile: NormalizedProfile) -> dict[str, Any]:
    readiness = classify_readiness(profile)
    spaces = _build_address_spaces(profile)
    input_capabilities = _build_input_capabilities(profile)
    space_ids = {space["id"]: index for index, space in enumerate(spaces)}
    frames = _build_frames(profile)
    readback, startup_queries = _readback_definition(profile)
    return {
        "id": _profile_id(profile),
        "identity": {
            "name": profile.identity.name,
            "vid": profile.identity.vid,
            "pid": profile.identity.pid,
            "bcd_device": profile.identity.bcd_device,
            "status": _normalized_status(profile.identity.status),
            "status_text": profile.identity.status,
            "notes": profile.identity.notes,
            "evidence": profile.identity.evidence,
        },
        "transport": {
            "kind": profile.transport.kind.lower(),
            "report_size": profile.transport.report_size,
            "out_endpoint": profile.transport.out_endpoint,
            "in_endpoint": profile.transport.in_endpoint,
            "poll_interval_ms": profile.transport.poll_interval_ms,
            "uses_numbered_reports": _effective_framing(profile),
            "expected_interface_number": profile.transport.expected_interface_number,
            "expected_usage_page": profile.transport.expected_usage_page,
            "expected_usage": profile.transport.expected_usage,
            "status": _normalized_status(profile.transport.status),
            "status_text": profile.transport.status,
            "notes": profile.transport.notes,
            "evidence": profile.transport.evidence,
        },
        "address_spaces": [
            {
                "id": item["id"],
                "space_id": space_ids[item["id"]],
                "name": item["name"],
                "kind": _normalized_kind(item["kind"]),
                "count": item["count"],
                "addressing": _normalized_kind(item["addressing"]),
                "status": _normalized_status(item["status"]),
                "status_text": item["status_text"],
                "notes": item["notes"],
                "metadata": item["metadata"],
                "input_capabilities": input_capabilities.get(item["id"], []),
            }
            for item in spaces
        ],
        "inputs": [
            {
                "id": item["id"],
                "space": item["space"],
                "space_id": space_ids[item["space"]],
                "index": item["index"],
                "name": item["name"],
                "hiz_capable": item["hiz"],
                "status": _normalized_status(item["status"]),
                "metadata": item["metadata"],
            }
            for item in _build_inputs(profile)
        ],
        "outputs": [
            {**item, "status": _normalized_status(item["status"])}
            for item in _build_outputs(profile)
        ],
        "mixers": [
            {**item, "status": _normalized_status(item["status"])}
            for item in _build_mixers(profile)
        ],
        "link_domains": [
            {**item, "status": _normalized_status(item["status"])}
            for item in _build_link_domains(profile)
        ],
        "routing_groups": [
            {
                **group,
                "source_domains": [
                    {**domain, "status": _normalized_status(domain["status"])}
                    for domain in group["source_domains"]
                ],
            }
            for group in _build_routing_groups(profile)
        ],
        **(
            {"state_report": {"candidate_preamp_meters": _candidate_preamp_meters(profile)}}
            if _candidate_preamp_meters(profile)
            else {}
        ),
        "frames": [
            {
                "id": frame["id"],
                "kind": _normalized_kind(frame["kind"]),
                "status": _normalized_status(frame["status"]),
                "report_size": profile.transport.report_size,
                "operations": _frame_operations(
                    profile, frame["id"], profile.frame.get(frame["id"], {})
                ),
                "metadata": frame["metadata"],
            }
            for frame in frames
        ],
        "decoders": [
            {
                "id": decoder["id"],
                "frame_id": decoder["frame_id"],
                "kind": _normalized_kind(decoder["kind"]),
                "status": _normalized_status(decoder["status"]),
                "metadata": decoder["metadata"],
            }
            for decoder in _build_decoders(frames)
        ],
        "params": [
            {
                "name": param["name"],
                "id": param["id"],
                "value_type": _normalized_kind(param["value_type"]),
                "status": _normalized_status(param["status"]),
                "applies_to": param["applies_to"],
                "range": param["range"],
                "values": [[value["value"], value["name"]] for value in param["values"]],
                "frame": {
                    "text": param["frame_reference"]["text"],
                    "formula": param["frame_reference"]["formula"],
                    "offsets": [[offset["name"], offset["offset"]] for offset in param["frame_reference"]["offsets"]],
                },
                "readback": {
                    "text": param["readback_reference"]["text"],
                    "formula": param["readback_reference"]["formula"],
                    "offsets": [[offset["name"], offset["offset"]] for offset in param["readback_reference"]["offsets"]],
                },
                "metadata": param["metadata"],
            }
            for param in _build_params(profile)
        ],
        "constraints": [
            {
                "name": item["name"],
                "status": _normalized_status(item["status"]),
                "range": item["range"] if item["range"] and all(value is not None for value in item["range"]) else None,
                "values": item["values"],
                "scalar": item["scalar"],
                "text": item["text"],
                "metadata": item["metadata"],
            }
            for item in _build_constraints(profile)
        ],
        "hazards": [
            {**item, "status": _normalized_status(item["status"])}
            for item in (_build_hazards(profile) + _orion_generated_hazards(profile))
        ],
        "startup_queries": startup_queries,
        "readback": readback,
        "provenance": {
            "source_path": profile.provenance.source_path,
            "source_sha256": profile.provenance.source_sha256,
            "generator_version": profile.provenance.generator_version,
        },
        "readiness": readiness.value,
        "driver_kind": _runtime_driver_kind(profile, readiness),
        "support_reason": _support_reason(profile, readiness),
    }


def render_profile_pack(profiles: Sequence[NormalizedProfile]) -> str:
    """Render deterministic compact normalized JSON with one trailing newline."""

    records = sorted((_normalized_profile_record(profile) for profile in profiles), key=lambda item: item["id"])
    pack = {
        "schema_version": PROFILE_PACK_SCHEMA_VERSION,
        "generator_version": GENERATOR_VERSION,
        "profiles": records,
    }
    return json.dumps(pack, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n"


def _support_level(readiness: Readiness) -> str:
    return {
        Readiness.SUPPORTED: "Supported",
        Readiness.PARTIAL: "Partial",
        Readiness.UNVERIFIED: "Unverified",
        Readiness.DISABLED: "Unsupported",
    }[readiness]


def _profile_status(profile: NormalizedProfile) -> str:
    # Device-level status is authoritative when present.  Orion intentionally
    # omits it, so retain Unknown instead of inventing Confirmed.
    return profile.identity.status


def _ident(value: str) -> str:
    text = re.sub(r"[^A-Za-z0-9_]", "_", value).upper()
    text = re.sub(r"_+", "_", text).strip("_")
    if not text or text[0].isdigit():
        text = "PROFILE_" + text
    return text


def _render_aliases(aliases: Sequence[str]) -> str:
    return _rust_slice(_rust_string(alias) for alias in aliases)


def _render_range(value: tuple[int, int] | None) -> str:
    if value is None:
        return "None"
    return f"Some(({value[0]}, {value[1]}))"


def _render_fader(value: Mapping[str, Any] | None) -> str:
    if value is None:
        return "None"
    direction = {"direct": "Direct", "attenuation": "Attenuation"}.get(
        str(value["direction"]).strip().lower()
    )
    if direction is None:
        raise ProfileError(f"unsupported mixer.fader.direction {value['direction']!r}")
    return (
        "Some(FaderSemanticsDefinition { "
        f"min: {value['min']}, max: {value['max']}, "
        f"direction: FaderDirectionDefinition::{direction}, unity: {value['unity']} }})"
    )


def _render_values(values: Sequence[Mapping[str, Any]], helper: str, lines: list[str]) -> str:
    if not values:
        return "&[]"
    lines.append(f"static {helper}: &[ParamValueDefinition] = &[")
    for item in values:
        lines.append(
            "    ParamValueDefinition { value: "
            + str(item["value"])
            + ", name: "
            + _rust_string(item["name"])
            + " },"
        )
    lines.append("];\n")
    return helper


def _render_param_ranges(
    ranges: Sequence[Mapping[str, Any]], helper: str, lines: list[str]
) -> str:
    if not ranges:
        return "&[]"
    lines.append(f"static {helper}: &[ParamRangeDefinition] = &[")
    for item in ranges:
        lines.append(
            "    ParamRangeDefinition { "
            f"name: {_rust_string(item['name'])}, range: {_render_range(item['range'])}, "
            f"text: {_rust_string(item.get('text', ''))} }},"
        )
    lines.append("];\n")
    return helper


def _render_frame_fields(frame: Mapping[str, Any], helper: str, lines: list[str]) -> str:
    """Render recursive frame fields, including numeric nested geometry."""

    fields = frame["fields"]
    if not fields:
        return "&[]"
    child_helpers: list[str] = []
    value_helpers: list[str] = []
    for index, field in enumerate(fields):
        child_helper = f"{helper}_{index}_CHILDREN"
        children = field.get("children", [])
        child_helpers.append(_render_frame_field_slice(children, child_helper, lines))
        value_helper = f"{helper}_{index}_VALUES"
        values = field.get("values", [])
        if values:
            lines.append(f"static {value_helper}: &[i32] = &[{', '.join(str(value) for value in values)}];")
            value_helpers.append(value_helper)
        else:
            value_helpers.append("&[]")
    lines.append(f"static {helper}: &[FrameFieldDefinition] = &[")
    for index, field in enumerate(fields):
        lines.append(
            "    FrameFieldDefinition { "
            f"name: {_rust_string(field['name'])}, "
            f"offset: {_rust_option(field['offset'], _rust_u16)}, "
            f"stride: {_rust_option(field.get('stride'), _rust_u16)}, "
            f"width: {_rust_option(field['width'], _rust_u16)}, "
            f"value: {_rust_option(field['value'], _rust_i32)}, "
            f"mask: {_rust_option(field['mask'], _rust_u8)}, "
            f"values: {value_helpers[index]}, formula: {_rust_string(field.get('formula', ''))}, "
            f"text: {_rust_string(field['text'])}, children: {child_helpers[index]} }},"
        )
    lines.append("];\n")
    return helper


def _render_frame_field_slice(fields: Sequence[Mapping[str, Any]], helper: str, lines: list[str]) -> str:
    """Render nested field slice using same recursive layout as top-level fields."""

    if not fields:
        return "&[]"
    return _render_frame_fields({"fields": list(fields)}, helper, lines)


def _render_frame_operations(
    operations: Sequence[Mapping[str, Any]], helper: str, lines: list[str]
) -> str:
    if not operations:
        return "&[]"
    allowed_helpers: dict[int, str] = {}
    for index, operation in enumerate(operations):
        if operation["op"] == "allowed_values":
            values_helper = f"{helper}_{index}_VALUES"
            lines.append(
                f"static {values_helper}: &[i32] = &[{', '.join(str(value) for value in operation['values'])}];"
            )
            allowed_helpers[index] = values_helper
    lines.append(f"static {helper}: &[FrameOperationDefinition] = &[")
    for index, operation in enumerate(operations):
        kind = operation["op"]
        if kind == "fixed_byte":
            rendered = f"FixedByte {{ offset: {operation['offset']}u16, value: {operation['value']}u8 }}"
        elif kind == "scalar":
            endian = {
                "not_applicable": "NotApplicable",
                "little": "Little",
                "big": "Big",
            }.get(operation["endian"])
            if endian is None:
                rendered = f"UncompiledFormula {{ formula: {_rust_string('unproven scalar endianness')} }}"
            else:
                rendered = (
                    f"Scalar {{ field: {_rust_string(operation['field'])}, offset: {operation['offset']}u16, "
                    f"width: {operation['width']}u8, endian: FrameEndianDefinition::{endian} }}"
                )
        elif kind == "indexed":
            rendered = (
                f"Indexed {{ base: {operation['base']}u16, stride: {operation['stride']}u16, "
                f"index_field: {_rust_string(operation['index_field'])}, width: {operation['width']}u8, "
                f"max_index: Some({operation['max_index']}u16) }}"
            )
        elif kind == "bit_field":
            rendered = (
                f"BitField {{ field: {_rust_string(operation['field'])}, offset: {operation['offset']}u16, "
                f"mask: {operation['mask']}u8, shift: {operation['shift']}u8 }}"
            )
        elif kind == "pair_index":
            rendered = (
                f"PairIndex {{ base: {operation['base']}u16, stride: {operation['stride']}u16, "
                f"pair_field: {_rust_string(operation['pair_field'])}, width: {operation['width']}u8, "
                f"max_index: Some({operation['max_index']}u16) }}"
            )
        elif kind == "allowed_values":
            rendered = f"AllowedValues {{ values: {allowed_helpers[index]} }}"
        elif kind == "uncompiled_formula":
            rendered = f"UncompiledFormula {{ formula: {_rust_string(operation['formula'])} }}"
        else:
            raise ProfileError(f"unsupported normalized frame operation {kind!r}")
        lines.append(f"    FrameOperationDefinition::{rendered},")
    lines.append("];")
    return helper


def _render_opcodes(opcodes: Sequence[int], helper: str, lines: list[str]) -> str:
    if not opcodes:
        return "&[]"
    values = ", ".join(f"{opcode}u8" for opcode in opcodes)
    lines.append(f"static {helper}: &[u8] = &[{values}];\n")
    return helper


def _render_parameter_reference(
    reference: Mapping[str, Any], helper: str, lines: list[str]
) -> str:
    offsets = reference["offsets"]
    if offsets:
        lines.append(f"static {helper}_OFFSETS: &[ParamOffsetDefinition] = &[")
        for offset in offsets:
            lines.append(
                "    ParamOffsetDefinition { "
                f"name: {_rust_string(offset['name'])}, offset: {offset['offset']}u16, "
                f"formula: {_rust_string(offset['formula'])} }},"
            )
        lines.append("];\n")
        offsets_name = f"{helper}_OFFSETS"
    else:
        offsets_name = "&[]"
    lines.append(
        f"static {helper}: ParamReference = ParamReference {{ text: "
        f"{_rust_string(reference['text'])}, formula: {_rust_string(reference.get('formula', ''))}, "
        f"offsets: {offsets_name} }};"
    )
    return helper


def render_catalog(profiles: Sequence[NormalizedProfile]) -> str:
    """Render normalized profiles into compilable Rust source."""

    lines = [
        "// rustfmt::skip_file",
        "// @generated by tools/generate_device_catalog.py; DO NOT EDIT.",
        f"// Generator version: {GENERATOR_VERSION}",
        "// Source: Antelope-Ctl hardware profiles/*.json (non-hardware model data excluded).",
        "#![allow(clippy::too_many_arguments)]",
        "",
        "use super::definition::{",
        "    AddressSpaceDefinition, AddressSpaceKind, AddressingMode, ConstraintDefinition,",
        "    InputCapabilityDefinition, InputControlKind,",
        "    DecoderDefinition, DeviceDefinition, DeviceEntry, DeviceIdentity, FrameDefinition,",
        "    FrameEndianDefinition, FrameFieldDefinition, FrameKind, FrameOperationDefinition, HazardDefinition, InputDefinition, LinkDomainDefinition, LinkDomainKind, MixerDefinition,",
        "    OutputDefinition, ParamDefinition, ParamOffsetDefinition, ParamRangeDefinition, ParamReference, RoutingGroupDefinition, RoutingSourceDomainDefinition,",
        "    ParamValueDefinition, ParamValueType, Provenance, ReadbackCategoryDefinition, ReadbackDefinition,",
        "    SafeQueryDefinition, MixerReadbackLayoutDefinition, StateReportDefinition,",
        "    CandidatePreampMeterDefinition, FaderDirectionDefinition, FaderSemanticsDefinition,",
        "    Readiness, StartupQueryDefinition, Status, SupportLevel, TransportDefinition, TransportKind,",
        "};",
        "",
    ]

    rendered: list[dict[str, Any]] = []
    for profile in profiles:
        readiness = classify_readiness(profile)
        slug = _ident(profile.path.stem)
        spaces = _build_address_spaces(profile)
        input_capabilities = _build_input_capabilities(profile)
        inputs = _build_inputs(profile)
        outputs = _build_outputs(profile)
        mixers = _build_mixers(profile)
        link_domains = _build_link_domains(profile)
        routing_groups = _build_routing_groups(profile)
        frames = _build_frames(profile)
        decoders = _build_decoders(frames)
        params = _build_params(profile)
        constraints = _build_constraints(profile)
        hazards = _build_hazards(profile)
        runtime_record = _normalized_profile_record(profile)
        runtime_frames = {frame["id"]: frame for frame in runtime_record["frames"]}

        # Helper static slices keep DeviceDefinition literals readable and make
        # every generated section independently inspectable from Rust.
        capability_helpers: dict[str, str] = {}
        for space_index, item in enumerate(spaces):
            helper = f"{slug}_ADDRESS_SPACE_{space_index}_INPUT_CAPABILITIES"
            capability_helpers[item["id"]] = helper
            lines.append(f"static {helper}: &[InputCapabilityDefinition] = &[")
            for capability in input_capabilities.get(item["id"], []):
                lines.append(
                    "    InputCapabilityDefinition { "
                    f"kind: InputControlKind::{capability['kind'].title()}, "
                    f"parameter: {_rust_string(capability['parameter'])}, "
                    f"parameter_id: {_rust_option(capability['parameter_id'], _rust_u16)}, "
                    f"label: {_rust_string(capability['label'])} }},"
                )
            lines.append("];\n")
        lines.append(f"static {slug}_ADDRESS_SPACES: &[AddressSpaceDefinition] = &[")
        for item in spaces:
            count = _rust_option(item["count"], _rust_u16)
            lines.append(
                "    AddressSpaceDefinition { "
                f"id: {_rust_string(item['id'])}, name: {_rust_string(item['name'])}, "
                f"kind: AddressSpaceKind::{item['kind']}, count: {count}, "
                f"addressing: AddressingMode::{item['addressing']}, status: Status::{_status_variant(item['status'])}, "
                f"status_text: {_rust_string(item['status_text'])}, notes: {_rust_string(item['notes'])}, "
                f"metadata: {_rust_string(item['metadata'])}, input_capabilities: {capability_helpers[item['id']]} }},"
            )
        lines.append("];\n")

        lines.append(f"static {slug}_INPUTS: &[InputDefinition] = &[")
        for item in inputs:
            lines.append(
                "    InputDefinition { "
                f"id: {_rust_string(item['id'])}, space: {_rust_string(item['space'])}, "
                f"index: {_rust_u16(item['index'])}, name: {_rust_string(item['name'])}, "
                f"hiz_capable: {str(item['hiz']).lower()}, status: Status::{_status_variant(item['status'])}, "
                f"metadata: {_rust_string(item['metadata'])} }},"
            )
        lines.append("];\n")

        # Alias arrays need stable helpers because nested array expressions in a
        # static are not accepted in all Rust const contexts.
        output_alias_helpers: list[str] = []
        for index, item in enumerate(outputs):
            helper = f"{slug}_OUTPUT_{index}_ALIASES"
            output_alias_helpers.append(helper)
            lines.append(f"static {helper}: &[&str] = {_render_aliases(item['aliases'])};")
        if outputs:
            lines.append("")
        lines.append(f"static {slug}_OUTPUTS: &[OutputDefinition] = &[")
        for index, item in enumerate(outputs):
            lines.append(
                "    OutputDefinition { "
                f"id: {_rust_u16(item['id'])}, name: {_rust_string(item['name'])}, "
                f"aliases: {output_alias_helpers[index]}, verified: {str(item['verified']).lower()}, "
                f"status: Status::{_status_variant(item['status'])}, metadata: {_rust_string(item['metadata'])} }},"
            )
        lines.append("];\n")

        lines.append(f"static {slug}_MIXERS: &[MixerDefinition] = &[")
        for item in mixers:
            lines.append(
                "    MixerDefinition { "
                f"id: {_rust_string(item['id'])}, name: {_rust_string(item['name'])}, "
                f"mix_index: {_rust_u8(item['mix_index'])}, strip_count: {_rust_u16(item['strip_count'])}, has_master: {str(item['has_master']).lower()}, "
                f"fader_range: {_render_range(item['fader_range'])}, fader: {_render_fader(item['fader'])}, "
                f"pan_range: {_render_range(item['pan_range'])}, "
                f"pan_center: {_rust_option(item['pan_center'], _rust_i32)}, send_range: {_render_range(item['send_range'])}, "
                f"status: Status::{_status_variant(item['status'])}, status_text: {_rust_string(item['status_text'])}, "
                f"notes: {_rust_string(item['notes'])}, metadata: {_rust_string(item['metadata'])} }},"
            )
        lines.append("];\n")

        lines.append(f"static {slug}_LINK_DOMAINS: &[LinkDomainDefinition] = &[")
        for domain in link_domains:
            lines.append(
                "    LinkDomainDefinition { "
                f"protocol_space: {_rust_u8(domain['protocol_space'])}, kind: LinkDomainKind::Mixer, "
                f"pair_count: {_rust_u16(domain['pair_count'])}, status: Status::{_status_variant(domain['status'])}, "
                f"evidence: {_rust_string(domain['evidence'])} }},"
            )
        lines.append("];\n")

        routing_source_helpers: list[str] = []
        for index, group in enumerate(routing_groups):
            helper = f"{slug}_ROUTING_GROUP_{index}_SOURCE_DOMAINS"
            routing_source_helpers.append(helper)
            lines.append(f"static {helper}: &[RoutingSourceDomainDefinition] = &[")
            for domain in group["source_domains"]:
                lines.append(
                    "    RoutingSourceDomainDefinition { "
                    f"bank: {_rust_u8(domain['bank'])}, index_count: {_rust_u16(domain['index_count'])}, "
                    f"status: Status::{_status_variant(domain['status'])}, evidence: {_rust_string(domain['evidence'])} }},"
                )
            lines.append("];\n")
        lines.append(f"static {slug}_ROUTING_GROUPS: &[RoutingGroupDefinition] = &[")
        for index, group in enumerate(routing_groups):
            lines.append(
                "    RoutingGroupDefinition { "
                f"destination: {_rust_u16(group['destination'])}, name: {_rust_string(group['name'])}, "
                f"channel_count: {_rust_u16(group['channel_count'])}, source_domains: {routing_source_helpers[index]} }},"
            )
        lines.append("];\n")

        frame_field_helpers: list[str] = []
        frame_operation_helpers: list[str] = []
        for index, frame in enumerate(frames):
            helper = f"{slug}_FRAME_{index}_FIELDS"
            frame_field_helpers.append(_render_frame_fields(frame, helper, lines))
            frame_operation_helpers.append(
                _render_frame_operations(
                    runtime_frames[frame["id"]]["operations"],
                    f"{slug}_FRAME_{index}_OPERATIONS",
                    lines,
                )
            )
        lines.append(f"static {slug}_FRAMES: &[FrameDefinition] = &[")
        for index, frame in enumerate(frames):
            lines.append(
                "    FrameDefinition { "
                f"id: {_rust_string(frame['id'])}, kind: FrameKind::{frame['kind']}, "
                f"status: Status::{_status_variant(frame['status'])}, status_text: {_rust_string(frame['status_text'])}, "
                f"magic_offset: {_rust_option(frame['magic_offset'], _rust_u16)}, magic: {_rust_option(frame['magic'], _rust_u8)}, "
                f"opcode_offset: {_rust_option(frame['opcode_offset'], _rust_u16)}, opcode: {_rust_option(frame['opcode'], _rust_u8)}, "
                f"opcode_name: {_rust_string(frame['opcode_name'])}, fields: {frame_field_helpers[index]}, "
                f"operations: {frame_operation_helpers[index]}, metadata: {_rust_string(frame['metadata'])} }},"
            )
        lines.append("];\n")

        lines.append(f"static {slug}_DECODERS: &[DecoderDefinition] = &[")
        for decoder in decoders:
            lines.append(
                "    DecoderDefinition { "
                f"id: {_rust_string(decoder['id'])}, frame_id: {_rust_string(decoder['frame_id'])}, "
                f"kind: FrameKind::{decoder['kind']}, status: Status::{_status_variant(decoder['status'])}, "
                f"metadata: {_rust_string(decoder['metadata'])} }},"
            )
        lines.append("];\n")

        param_value_helpers: list[str] = []
        param_range_helpers: list[str] = []
        param_range_form_helpers: list[str] = []
        param_frame_helpers: list[str] = []
        param_readback_helpers: list[str] = []
        for index, param in enumerate(params):
            value_helper = f"{slug}_PARAM_{index}_VALUES"
            param_value_helpers.append(_render_values(param["values"], value_helper, lines))
            mode_ranges = [
                {"name": mode, "range": range_value if isinstance(range_value, tuple) else None,
                 "text": "" if isinstance(range_value, tuple) else str(range_value)}
                for mode, range_value in param["range_by_mode"].items()
            ]
            param_range_helpers.append(
                _render_param_ranges(mode_ranges, f"{slug}_PARAM_{index}_RANGES", lines)
            )
            param_range_form_helpers.append(
                _render_param_ranges(param["range_forms"], f"{slug}_PARAM_{index}_RANGE_FORMS", lines)
            )
            param_frame_helpers.append(
                _render_parameter_reference(param["frame_reference"], f"{slug}_PARAM_{index}_FRAME", lines)
            )
            param_readback_helpers.append(
                _render_parameter_reference(
                    param["readback_reference"], f"{slug}_PARAM_{index}_READBACK", lines
                )
            )
        lines.append(f"static {slug}_PARAMS: &[ParamDefinition] = &[")
        for index, param in enumerate(params):
            lines.append(
                "    ParamDefinition { "
                f"name: {_rust_string(param['name'])}, id: {_rust_option(param['id'], _rust_u16)}, "
                f"value_type: ParamValueType::{param['value_type']}, status: Status::{_status_variant(param['status'])}, "
                f"status_text: {_rust_string(param['status_text'])}, applies_to: {_rust_string(param['applies_to'])}, "
                f"range: {_render_range(param['range'])}, range_by_mode: {param_range_helpers[index]}, "
                f"range_forms: {param_range_form_helpers[index]}, values: {param_value_helpers[index]}, "
                f"frame: {param_frame_helpers[index]}, readback: {param_readback_helpers[index]}, "
                f"encoding: {_rust_string(param['encoding'])}, metadata: {_rust_string(param['metadata'])} }},"
            )
        lines.append("];\n")

        constraint_value_helpers: list[str] = []
        for index, constraint in enumerate(constraints):
            helper = f"{slug}_CONSTRAINT_{index}_VALUES"
            values = constraint["values"]
            if values:
                lines.append(f"static {helper}: &[i32] = &[{', '.join(str(value) for value in values)}];")
                constraint_value_helpers.append(helper)
            else:
                constraint_value_helpers.append("&[]")
        if constraints:
            lines.append("")
        lines.append(f"static {slug}_CONSTRAINTS: &[ConstraintDefinition] = &[")
        for index, constraint in enumerate(constraints):
            range_value = constraint["range"]
            # Flattened min/max constraints can have one side absent.
            if isinstance(range_value, tuple) and all(item is not None for item in range_value):
                rendered_range = f"Some(({range_value[0]}, {range_value[1]}))"
            else:
                rendered_range = "None"
            lines.append(
                "    ConstraintDefinition { "
                f"name: {_rust_string(constraint['name'])}, status: Status::{_status_variant(constraint['status'])}, "
                f"range: {rendered_range}, scalar: {_rust_option(constraint['scalar'], _rust_i32)}, "
                f"values: {constraint_value_helpers[index]}, "
                f"text: {_rust_string(constraint['text'])}, metadata: {_rust_string(constraint['metadata'])} }},"
            )
        lines.append("];\n")

        hazard_opcode_helpers: list[str] = []
        for index, hazard in enumerate(hazards):
            helper = f"{slug}_HAZARD_{index}_OPCODES"
            hazard_opcode_helpers.append(_render_opcodes(hazard["opcodes"], helper, lines))
        lines.append(f"static {slug}_HAZARDS: &[HazardDefinition] = &[")
        for index, hazard in enumerate(hazards):
            lines.append(
                "    HazardDefinition { "
                f"name: {_rust_string(hazard['name'])}, status: Status::{_status_variant(hazard['status'])}, "
                f"rule: {_rust_string(hazard['rule'])}, effect: {_rust_string(hazard['effect'])}, "
                f"notes: {_rust_string(hazard['notes'])}, opcodes: {hazard_opcode_helpers[index]}, "
                f"metadata: {_rust_string(hazard['metadata'])} }},"
            )
        lines.append("];\n")

        lines.append(f"static {slug}_STARTUP_QUERIES: &[StartupQueryDefinition] = &[")
        for query in runtime_record["startup_queries"]:
            lines.append(
                "    StartupQueryDefinition { "
                f"query_id: {query['query_id']}u8, sub_id: {query['sub_id']}u8 }},"
            )
        lines.append("];\n")

        state_report = runtime_record.get("state_report")
        if state_report is None:
            state_report_name = "None"
        else:
            lines.append(f"static {slug}_CANDIDATE_PREAMP_METERS: &[CandidatePreampMeterDefinition] = &[")
            for meter in state_report["candidate_preamp_meters"]:
                lines.append(
                    "    CandidatePreampMeterDefinition { "
                    f"input_index: {meter['input_index']}u16, offset: {meter['offset']}usize }},"
                )
            lines.append("];\n")
            lines.append(
                f"static {slug}_STATE_REPORT: StateReportDefinition = StateReportDefinition {{ "
                f"candidate_preamp_meters: {slug}_CANDIDATE_PREAMP_METERS }};"
            )
            state_report_name = f"Some({slug}_STATE_REPORT)"

        readback = runtime_record["readback"]
        if readback is None:
            readback_name = "None"
        else:
            lines.append(f"static {slug}_READBACK_CATEGORIES: &[ReadbackCategoryDefinition] = &[")
            for category in readback["category_counts"]:
                lines.append(
                    "    ReadbackCategoryDefinition { "
                    f"category: {category['category']}u8, count: {category['count']}u16 }},"
                )
            lines.append("];\n")
            lines.append(f"static {slug}_READBACK_SAFE_QUERIES: &[SafeQueryDefinition] = &[")
            for query in readback.get("safe_queries", []):
                lines.append(
                    "    SafeQueryDefinition { "
                    f"category: {query['category']}u8, index: {query['index']}u8 }},"
                )
            lines.append("];\n")
            layout_helpers: list[str] = []
            for layout_index, layout in enumerate(readback.get("layouts", [])):
                helper = f"{slug}_READBACK_LAYOUT_{layout_index}_FIELDS"
                layout_helpers.append(helper)
                fields = layout.get("supported_fields", [])
                lines.append(f"static {helper}: &[&str] = &[{', '.join(_rust_string(field) for field in fields)}];")
            lines.append(f"static {slug}_READBACK_LAYOUTS: &[MixerReadbackLayoutDefinition] = &[")
            for layout_index, layout in enumerate(readback.get("layouts", [])):
                lines.append(
                    "    MixerReadbackLayoutDefinition { "
                    f"category: {layout['category']}u8, index: {layout['index']}u8, "
                    f"body_size: {layout['body_size']}usize, record_count: {layout['record_count']}usize, "
                    f"record_stride: {layout['record_stride']}usize, level_offset: {layout['level_offset']}usize, "
                    f"state_offset: {layout['state_offset']}usize, "
                    f"surface: {_rust_option(layout.get('surface'), _rust_u8)}, "
                    f"surface_stride: {_rust_option(layout.get('surface_stride'), str)}, "
                    f"supported_fields: {layout_helpers[layout_index]} }},"
                )
            lines.append("];\n")
            lines.append(
                f"static {slug}_READBACK: ReadbackDefinition = ReadbackDefinition {{ "
                f"request_magic: {readback['request_magic']}u8, request_subcommand: {readback['request_subcommand']}u32, "
                f"response_magic: {readback['response_magic']}u8, "
                f"response_discriminator_offset: {readback['response_discriminator_offset']}u16, "
                f"response_discriminator: {readback['response_discriminator']}u8, "
                f"category_offset: {readback['category_offset']}u16, index_offset: {readback['index_offset']}u16, "
                f"data_offset: {readback['data_offset']}u16, category_counts: {slug}_READBACK_CATEGORIES, "
                f"safe_queries: {slug}_READBACK_SAFE_QUERIES, layouts: {slug}_READBACK_LAYOUTS }};"
            )
            readback_name = f"Some({slug}_READBACK)"

        raw_profile = profile.raw_text
        lines.append(f"static {slug}_RAW_PROFILE: &str = {_rust_string(raw_profile)};")
        lines.append("")
        rendered.append(
            {
                "slug": slug,
                "profile": profile,
                "readiness": readiness,
                "spaces": f"{slug}_ADDRESS_SPACES",
                "inputs": f"{slug}_INPUTS",
                "outputs": f"{slug}_OUTPUTS",
                "mixers": f"{slug}_MIXERS",
                "link_domains": f"{slug}_LINK_DOMAINS",
                "routing_groups": f"{slug}_ROUTING_GROUPS",
                "frames": f"{slug}_FRAMES",
                "decoders": f"{slug}_DECODERS",
                "params": f"{slug}_PARAMS",
                "constraints": f"{slug}_CONSTRAINTS",
                "hazards": f"{slug}_HAZARDS",
                "startup_queries": f"{slug}_STARTUP_QUERIES",
                "state_report": state_report_name,
                "readback": readback_name,
                "raw": f"{slug}_RAW_PROFILE",
            }
        )

    lines.append("/// All canonical Antelope hardware profiles; non-hardware model data is excluded.")
    lines.append("pub static DEVICE_CATALOG: &[DeviceEntry] = &[")
    for item in rendered:
        profile = item["profile"]
        readiness: Readiness = item["readiness"]
        status = _profile_status(profile)
        lines.append("    DeviceEntry {")
        lines.append("        definition: DeviceDefinition {")
        lines.append("            identity: DeviceIdentity {")
        lines.append(
            f"                name: {_rust_string(profile.identity.name)}, vid: {_rust_u16(profile.identity.vid)}, "
            )
        lines.append(
            f"                pid: {_rust_u16(profile.identity.pid)}, bcd_device: {_rust_option(profile.identity.bcd_device, _rust_string)}, "
        )
        lines.append(
            f"                status: Status::{_status_variant(profile.identity.status)}, status_text: {_rust_string(profile.identity.status)}, "
        )
        lines.append(
            f"                notes: {_rust_string(profile.identity.notes)}, evidence: {_rust_string(profile.identity.evidence)},"
        )
        lines.append("            },")
        lines.append("            transport: TransportDefinition {")
        lines.append(
            f"                kind: TransportKind::{_kind_variant(profile.transport.kind)}, report_size: {_rust_option(profile.transport.report_size, _rust_u16)}, "
        )
        lines.append(
            f"                out_endpoint: {_rust_option(profile.transport.out_endpoint, _rust_u8)}, in_endpoint: {_rust_option(profile.transport.in_endpoint, _rust_u8)}, "
        )
        lines.append(
            f"                poll_interval_ms: {_rust_option(profile.transport.poll_interval_ms, _rust_u16)}, uses_numbered_reports: {_rust_option(_effective_framing(profile), lambda value: str(value).lower())}, "
        )
        lines.append(
            f"                expected_interface_number: {_rust_option(profile.transport.expected_interface_number)}, expected_usage_page: {_rust_option(profile.transport.expected_usage_page, _rust_u16)}, expected_usage: {_rust_option(profile.transport.expected_usage, _rust_u16)}, "
        )
        lines.append(
            f"                status: Status::{_status_variant(profile.transport.status)}, status_text: {_rust_string(profile.transport.status)}, "
        )
        lines.append(
            f"                notes: {_rust_string(profile.transport.notes)}, evidence: {_rust_string(profile.transport.evidence)},"
        )
        lines.append("            },")
        lines.append(
            f"            address_spaces: {item['spaces']}, inputs: {item['inputs']}, outputs: {item['outputs']}, mixers: {item['mixers']}, link_domains: {item['link_domains']}, routing_groups: {item['routing_groups']},"
        )
        lines.append(
            f"            frames: {item['frames']}, decoders: {item['decoders']}, params: {item['params']}, constraints: {item['constraints']}, hazards: {item['hazards']},"
        )
        lines.append(
            f"            startup_queries: {item['startup_queries']}, state_report: {item['state_report']}, readback: {item['readback']},"
        )
        lines.append(
            f"            status: Status::{_status_variant(status)}, status_text: {_rust_string(status)}, "
            f"support_level: SupportLevel::{_support_level(readiness)}, readiness: Readiness::{readiness.name.title()},"
        )
        lines.append(
            f"            provenance: Provenance {{ source_path: {_rust_string(profile.provenance.source_path)}, "
            f"source_sha256: {_rust_string(profile.provenance.source_sha256)}, generator_version: {_rust_string(profile.provenance.generator_version)} }},"
        )
        lines.append(f"            raw_profile: {item['raw']},")
        lines.append("        },")
        lines.append(
            f"        support_level: SupportLevel::{_support_level(readiness)}, readiness: Readiness::{readiness.name.title()},"
        )
        lines.append("    },")
    lines.append("];\n")
    return "\n".join(line.rstrip() for line in lines).rstrip() + "\n"


def _load_profiles(profiles_dir: Path | str) -> list[NormalizedProfile]:
    sources = discover_profiles(profiles_dir)
    if not sources:
        raise ProfileError(f"no hardware profiles found in {profiles_dir}")
    return [load_profile(source.path, profiles_dir) for source in sources]


def generate_catalog(profiles_dir: Path | str) -> str:
    return render_catalog(_load_profiles(profiles_dir))


def generate_profile_pack(profiles_dir: Path | str) -> str:
    return render_profile_pack(_load_profiles(profiles_dir))


def write_catalog(profiles_dir: Path | str, output: Path | str) -> None:
    output_path = Path(output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(generate_catalog(profiles_dir), encoding="utf-8", newline="\n")


def write_profile_pack(profiles_dir: Path | str, output: Path | str) -> None:
    output_path = Path(output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(generate_profile_pack(profiles_dir), encoding="utf-8", newline="\n")


def _normalized_text(path: Path | str) -> str | None:
    try:
        return Path(path).read_text(encoding="utf-8").replace("\r\n", "\n").replace("\r", "\n")
    except OSError:
        return None


def check_catalog(profiles_dir: Path | str, generated: Path | str) -> bool:
    return _normalized_text(generated) == generate_catalog(profiles_dir)


def check_profile_pack(profiles_dir: Path | str, generated: Path | str) -> bool:
    return _normalized_text(generated) == generate_profile_pack(profiles_dir)


def check_generated_artifacts(
    profiles_dir: Path | str, generated: Path | str, pack_generated: Path | str
) -> bool:
    # Evaluate both checks so callers get complete drift coverage.
    rust_matches = check_catalog(profiles_dir, generated)
    pack_matches = check_profile_pack(profiles_dir, pack_generated)
    return rust_matches and pack_matches


def _parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profiles-dir", type=Path, help="Antelope-Ctl profiles directory")
    parser.add_argument("--output", type=Path, help="generated Rust output path")
    parser.add_argument("--pack-output", type=Path, help="normalized JSON profile-pack output path")
    parser.add_argument("--check", type=Path, metavar="PROFILES_DIR", help="check generated output against profiles")
    parser.add_argument("--generated", type=Path, help="generated Rust path used with --check")
    parser.add_argument("--pack-generated", type=Path, help="generated JSON profile-pack path used with --check")
    args = parser.parse_args(argv)
    generate_mode = any(value is not None for value in (args.profiles_dir, args.output, args.pack_output))
    check_mode = any(value is not None for value in (args.check, args.generated, args.pack_generated))
    if generate_mode and check_mode:
        parser.error("choose --profiles-dir/--output or --check/--generated")
    if generate_mode:
        if args.output is None or args.pack_output is None:
            parser.error("--output and --pack-output are required together")
        args.profiles_dir = args.profiles_dir or DEFAULT_PROFILES_DIR
    elif check_mode:
        if args.check is None or args.generated is None or args.pack_generated is None:
            parser.error("--check, --generated, and --pack-generated are required together")
    else:
        parser.error("one generation or check mode is required")
    return args


def main(argv: Sequence[str] | None = None) -> int:
    args = _parse_args(sys.argv[1:] if argv is None else argv)
    try:
        if args.profiles_dir is not None:
            write_catalog(args.profiles_dir, args.output)
            write_profile_pack(args.profiles_dir, args.pack_output)
            return 0
        rust_matches = check_catalog(args.check, args.generated)
        pack_matches = check_profile_pack(args.check, args.pack_generated)
        if not rust_matches:
            print(f"generated catalog is stale or missing: {args.generated}", file=sys.stderr)
        if not pack_matches:
            print(f"generated profile pack is stale or missing: {args.pack_generated}", file=sys.stderr)
        if not rust_matches or not pack_matches:
            return 1
        return 0
    except ProfileError as exc:
        print(f"profile error: {exc}", file=sys.stderr)
        return 2
    except OSError as exc:
        print(f"I/O error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
