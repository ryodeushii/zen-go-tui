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


GENERATOR_VERSION = "1.3.0"
EXCLUDED_PROFILE_NAMES = frozenset({"mic_models.json"})


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
    if any(
        value is None
        for value in (
            transport.report_size,
            transport.out_endpoint,
            transport.in_endpoint,
            transport.poll_interval_ms,
        )
    ):
        return False
    if (
        transport.report_size <= 0
        or transport.out_endpoint <= 0
        or transport.in_endpoint <= 0
        or transport.poll_interval_ms <= 0
    ):
        return False
    if _status_variant(transport.status) != "Confirmed":
        return False

    report_size = transport.report_size
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

    device = profile.get("device") if isinstance(profile.get("device"), Mapping) else {}
    transport = profile.get("transport") if isinstance(profile.get("transport"), Mapping) else {}
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
    _build_inputs(normalized)
    _build_outputs(normalized)
    _build_mixers(normalized)
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


def classify_readiness(profile: NormalizedProfile) -> Readiness:
    """Classify runtime readiness without conflating it with source status."""

    known = _KNOWN_READINESS.get((profile.identity.vid, profile.identity.pid))
    if known is not None:
        if known is Readiness.SUPPORTED and not _profile_has_confirmed_runtime_shape(profile):
            return Readiness.DISABLED
        return known
    if "unconfirm" in profile.identity.status.lower():
        return Readiness.UNVERIFIED
    return Readiness.DISABLED


def _status_variant(text: str | None) -> str:
    normalized = (text or "").strip().lower()
    if normalized.startswith("confirm"):
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
    """Use explicit status, or a status stated in canonical evidence/notes."""

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


def _build_inputs(profile: NormalizedProfile) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    physical_count = _count(profile.channels, ("count", "count_confirmed", "count_assumed_total"), "channels")
    names = profile.channels.get("names", [])
    if names is not None and not isinstance(names, list):
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


def _mixer_geometry(
    profile: NormalizedProfile,
) -> tuple[int, int, Mapping[str, Any], bool]:
    mixer = profile.mixer
    frame_mixer = profile.frame.get("mix_command", {})
    if not isinstance(frame_mixer, Mapping):
        frame_mixer = {}
    mix_count = _count(mixer, ("mixes", "count"), "mixer") if mixer else None
    strip_count = _count(mixer, ("channels_per_mix", "strip_count", "strips"), "mixer") if mixer else None
    inferred_mix_count = False
    inferred_strip_count = False
    # Some canonical profiles put mixer dimensions in frame notes because no
    # separate typed mixer section was captured.  Parse those notes only when
    # they positively identify dimensions; never use PID defaults.  Text-derived
    # dimensions remain observed rather than becoming confirmed geometry.
    text = _json(frame_mixer)
    frame_status = _status_variant(_section_status(frame_mixer, profile.profile_status or "unknown"))
    if frame_status == "Confirmed" and mix_count is None:
        match = re.search(r"mix(?:es)?\s*(?:1\s*[-–]\s*)?(\d+)\b", text, re.IGNORECASE)
        if match:
            mix_count = _checked_u16(match.group(1), "frame.mix_command.mix_count")
            inferred_mix_count = True
    if frame_status == "Confirmed" and strip_count is None:
        match = re.search(
            r"(?:each mix has|input strips|channel(?:s)?\s*=\s*0-)\s*(\d+)",
            text,
            re.IGNORECASE,
        )
        if match:
            strip_count = _checked_u16(match.group(1), "frame.mix_command.strip_count")
            inferred_strip_count = True
    return (
        mix_count or 0,
        strip_count or 0,
        frame_mixer,
        inferred_mix_count or inferred_strip_count,
    )


def _profile_param_range(profile: NormalizedProfile, names: Sequence[str]) -> tuple[int, int] | None:
    for name in names:
        value = profile.params.get(name)
        if isinstance(value, Mapping) and "range" in value:
            return _range(value.get("range"), f"params.{name}.range")
    return None


def _build_mixers(profile: NormalizedProfile) -> list[dict[str, Any]]:
    mix_count, strip_count, frame_mixer, inferred_geometry = _mixer_geometry(profile)
    if not mix_count or not strip_count:
        return []
    fader = profile.mixer.get("fader", {}) if isinstance(profile.mixer.get("fader", {}), Mapping) else {}
    pan = profile.mixer.get("pan", {}) if isinstance(profile.mixer.get("pan", {}), Mapping) else {}
    fader_range = _range(fader.get("range"), "mixer.fader.range")
    fader_range = fader_range or _profile_param_range(profile, ("mix_fader",))
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
                "fader_range": fader_range,
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
            continue
        status = _section_status(value, profile.profile_status or "unknown")
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
                "kind": _frame_kind(frame_name),
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


def _build_params(profile: NormalizedProfile) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    for name, value in profile.params.items():
        if name.startswith("_"):
            continue
        if not isinstance(value, Mapping):
            raise ProfileError(f"params.{name} must be an object")
        status = _section_status(value, profile.profile_status or "unknown")
        parameter_range = _range(value.get("range"), f"params.{name}.range")
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
        if isinstance(enum_values, Mapping):
            for raw_key, raw_value in enum_values.items():
                number = _checked_i32(raw_key, f"params.{name}.values.key")
                label = str(raw_value)
                values.append({"value": number, "name": label})
            values.sort(key=lambda item: item["value"])
        elif enum_values is not None:
            raise ProfileError(f"params.{name}.values must be an object")
        frame_value = value.get("frame")
        readback_value = value.get("readback")
        if readback_value is None and "state_report_offset_formula" in value:
            readback_value = value.get("state_report_offset_formula")
        result.append(
            {
                "name": name,
                "id": _checked_u16(value["id"], f"params.{name}.id") if value.get("id") is not None else None,
                "value_type": _param_type(value.get("type")),
                "status": status,
                "status_text": status,
                "applies_to": str(value.get("applies_to", "")),
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
        "    DecoderDefinition, DeviceDefinition, DeviceEntry, DeviceIdentity, FrameDefinition,",
        "    FrameFieldDefinition, FrameKind, HazardDefinition, InputDefinition, MixerDefinition,",
        "    OutputDefinition, ParamDefinition, ParamOffsetDefinition, ParamRangeDefinition, ParamReference,",
        "    ParamValueDefinition, ParamValueType, Provenance,",
        "    Readiness, Status, SupportLevel, TransportDefinition, TransportKind,",
        "};",
        "",
    ]

    rendered: list[dict[str, Any]] = []
    for profile in profiles:
        readiness = classify_readiness(profile)
        slug = _ident(profile.path.stem)
        spaces = _build_address_spaces(profile)
        inputs = _build_inputs(profile)
        outputs = _build_outputs(profile)
        mixers = _build_mixers(profile)
        frames = _build_frames(profile)
        decoders = _build_decoders(frames)
        params = _build_params(profile)
        constraints = _build_constraints(profile)
        hazards = _build_hazards(profile)

        # Helper static slices keep DeviceDefinition literals readable and make
        # every generated section independently inspectable from Rust.
        lines.append(f"static {slug}_ADDRESS_SPACES: &[AddressSpaceDefinition] = &[")
        for item in spaces:
            count = _rust_option(item["count"], _rust_u16)
            lines.append(
                "    AddressSpaceDefinition { "
                f"id: {_rust_string(item['id'])}, name: {_rust_string(item['name'])}, "
                f"kind: AddressSpaceKind::{item['kind']}, count: {count}, "
                f"addressing: AddressingMode::{item['addressing']}, status: Status::{_status_variant(item['status'])}, "
                f"status_text: {_rust_string(item['status_text'])}, notes: {_rust_string(item['notes'])}, "
                f"metadata: {_rust_string(item['metadata'])} }},"
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
                f"mix_index: {_rust_u8(item['mix_index'])}, strip_count: {_rust_u16(item['strip_count'])}, "
                f"fader_range: {_render_range(item['fader_range'])}, pan_range: {_render_range(item['pan_range'])}, "
                f"pan_center: {_rust_option(item['pan_center'], _rust_i32)}, send_range: {_render_range(item['send_range'])}, "
                f"status: Status::{_status_variant(item['status'])}, status_text: {_rust_string(item['status_text'])}, "
                f"notes: {_rust_string(item['notes'])}, metadata: {_rust_string(item['metadata'])} }},"
            )
        lines.append("];\n")

        frame_field_helpers: list[str] = []
        for index, frame in enumerate(frames):
            helper = f"{slug}_FRAME_{index}_FIELDS"
            frame_field_helpers.append(_render_frame_fields(frame, helper, lines))
        lines.append(f"static {slug}_FRAMES: &[FrameDefinition] = &[")
        for index, frame in enumerate(frames):
            lines.append(
                "    FrameDefinition { "
                f"id: {_rust_string(frame['id'])}, kind: FrameKind::{frame['kind']}, "
                f"status: Status::{_status_variant(frame['status'])}, status_text: {_rust_string(frame['status_text'])}, "
                f"magic_offset: {_rust_option(frame['magic_offset'], _rust_u16)}, magic: {_rust_option(frame['magic'], _rust_u8)}, "
                f"opcode_offset: {_rust_option(frame['opcode_offset'], _rust_u16)}, opcode: {_rust_option(frame['opcode'], _rust_u8)}, "
                f"opcode_name: {_rust_string(frame['opcode_name'])}, fields: {frame_field_helpers[index]}, "
                f"metadata: {_rust_string(frame['metadata'])} }},"
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
                "frames": f"{slug}_FRAMES",
                "decoders": f"{slug}_DECODERS",
                "params": f"{slug}_PARAMS",
                "constraints": f"{slug}_CONSTRAINTS",
                "hazards": f"{slug}_HAZARDS",
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
            f"                poll_interval_ms: {_rust_option(profile.transport.poll_interval_ms, _rust_u16)}, uses_numbered_reports: {_rust_option(profile.transport.uses_numbered_reports, lambda value: str(value).lower())}, "
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
            f"            address_spaces: {item['spaces']}, inputs: {item['inputs']}, outputs: {item['outputs']}, mixers: {item['mixers']},"
        )
        lines.append(
            f"            frames: {item['frames']}, decoders: {item['decoders']}, params: {item['params']}, constraints: {item['constraints']}, hazards: {item['hazards']},"
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


def generate_catalog(profiles_dir: Path | str) -> str:
    sources = discover_profiles(profiles_dir)
    if not sources:
        raise ProfileError(f"no hardware profiles found in {profiles_dir}")
    profiles = [load_profile(source.path, profiles_dir) for source in sources]
    return render_catalog(profiles)


def write_catalog(profiles_dir: Path | str, output: Path | str) -> None:
    output_path = Path(output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(generate_catalog(profiles_dir), encoding="utf-8")


def check_catalog(profiles_dir: Path | str, generated: Path | str) -> bool:
    expected = generate_catalog(profiles_dir)
    generated_path = Path(generated)
    try:
        actual = generated_path.read_text(encoding="utf-8")
    except OSError:
        return False
    return actual == expected


def _parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profiles-dir", type=Path, help="Antelope-Ctl profiles directory")
    parser.add_argument("--output", type=Path, help="generated Rust output path")
    parser.add_argument("--check", type=Path, metavar="PROFILES_DIR", help="check generated output against profiles")
    parser.add_argument("--generated", type=Path, help="generated Rust path used with --check")
    args = parser.parse_args(argv)
    generate_mode = args.profiles_dir is not None or args.output is not None
    check_mode = args.check is not None or args.generated is not None
    if generate_mode and check_mode:
        parser.error("choose --profiles-dir/--output or --check/--generated")
    if generate_mode:
        if args.profiles_dir is None or args.output is None:
            parser.error("--profiles-dir and --output are required together")
    elif check_mode:
        if args.check is None or args.generated is None:
            parser.error("--check and --generated are required together")
    else:
        parser.error("one generation or check mode is required")
    return args


def main(argv: Sequence[str] | None = None) -> int:
    args = _parse_args(sys.argv[1:] if argv is None else argv)
    try:
        if args.profiles_dir is not None:
            write_catalog(args.profiles_dir, args.output)
            return 0
        if not check_catalog(args.check, args.generated):
            print(f"generated catalog is stale or missing: {args.generated}", file=sys.stderr)
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
