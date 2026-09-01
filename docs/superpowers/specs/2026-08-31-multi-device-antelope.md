# Multi-device Antelope support specification

## Goal

Make `zen-go-tui` enumerate every hardware profile in Antelope-Ctl, select a safe compatible HID interface, and provide full Orion Studio III support while preserving Zen Go behavior.

## Source of truth

Antelope-Ctl `profiles/*.json` is canonical. `mic_models.json` is not hardware and is excluded. The TUI consumes generated Rust data, not JSON at runtime. Generated data records each source profile SHA-256 and generator version. A generator check reports drift when run against a profile directory.

Ordinary Cargo builds must not require an Antelope-Ctl checkout.

## Catalog and readiness

Every hardware profile appears in generated catalog. Catalog identity is VID/PID plus optional revision/name metadata. Runtime readiness is separate from profile existence:

- Zen Go `0x23e5:0xa015`: selectable with existing protocol behavior.
- Orion Studio III `0x23e5:0xa221`: selectable after full Orion driver and UI mapping.
- Discrete 8 Pro `0x23e5:0xa2b5`: visible and disabled because profile has no readable state report.
- Discrete 4 `0x23e5:0xa2be`: visible and disabled because transport/frame data is incomplete.
- Discrete 4 Pro `0x23e5:0xa2bf`: visible and disabled because transport/frame data is incomplete.

Unsupported entries must never instantiate a driver or send a command.

## Generated Rust data

Generated definitions include typed data for:

- device identity and readiness;
- HID report size, endpoints, polling, and numbered-report behavior;
- input/preamp spaces and modes;
- ADAT and S/PDIF spaces when present;
- output buses and names;
- mixer surfaces, strip counts, and capabilities;
- command frame layouts and incoming state/meter/name/init/error decoders;
- parameters, ranges, enum values, and readback locations;
- constraints and hazards;
- source profile path, source hash, and generation metadata.

Unconfirmed profile fields remain represented with status but are not usable by normal commands.

## HID discovery

Discovery filters `HidApi::device_list()` by Antelope VID `0x23e5`, returns candidate metadata, classifies candidates against generated catalog, groups/deduplicates interfaces, and sorts selectable candidates before disabled candidates. Selection opens the exact HID path. It must not open the first matching VID/PID blindly.

Candidate metadata includes path, VID, PID, serial, product, usage page, usage, and interface number. Control-interface matching is profile-driven where profile metadata exists; ambiguous candidates remain visible with a diagnostic.

## Protocol boundary

Application code depends on a driver interface and normalized actions/events, not Zen Go-specific `Command`, `Frame`, or fixed arrays. Each driver owns startup requests, command encoding, packet decoding, and capability mapping. Generic profile-driven command encoding may be shared. Zen Go and Orion retain separate decoder implementations where report layouts differ.

## Runtime model

Controller construction receives selected transport and driver. Runtime state uses dynamic collections for inputs, outputs, mixer surfaces, and strips. UI renders capabilities and names from the selected definition.

Minimum full Orion mapping:

- 12 physical inputs;
- 16 ADAT inputs;
- 2 S/PDIF inputs;
- 6 output buses;
- Mix 1-4;
- 32 strips per mix;
- profile-defined routing and supported profile commands;
- state, meter, startup, and write/readback paths supported by captures/profile.

Current saved-state TOML profile types remain separate from generated protocol device definitions.

## Safety and failures

Handle no devices, permissions, unsupported profiles, malformed/generated definitions, duplicate interfaces, multiple identical units, unplug/replug, path changes, wrong report length, unknown frames, and unconfirmed capabilities. Discovery is read-only. Unsupported or ambiguous candidates cannot be controlled.

## Verification

Use unit tests for profile generation/validation, catalog classification and sorting, exact-path selection, driver action/event translation, dynamic state dimensions, and picker behavior. Use golden captured packets for Zen Go and Orion encoding/decoding. Run hardware validation separately per supported device. Documentation must state generated-data workflow and readiness status.
