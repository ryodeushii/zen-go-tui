# Orion and Zen Go capture-family inventory

Status: confirmed historical snapshot plus bounded Mix 1 and active-mix switch file increments.  
Todo: 51, durable protocol-analysis record.

## Historical complete snapshot

The complete scan covered every PCAPNG in these paths, relative to the capture checkout root:

- `antelope_pcap/orion 3/`
- `antelope_pcap/zen go sc/`

The scanner hashed and scanned all 204 files in that snapshot. It found no malformed TShark rows or capture truncation.

| Device | Files | Captured bytes | Application-shaped report families |
|---|---:|---:|---|
| Orion Studio III | 77 | 1,829,955,908 | IN `0x73`: 644,519<br>IN `0x75/0x00`: 1,159<br>IN `0x75/0x1f`: 644,510<br>OUT `0x70`: 5,673<br>OUT `0x74`: 1,159 |
| Zen Go SC | 127 | 1,917,965,288 | IN `0x73`: 317,953<br>IN `0x75/0x00`: 1,109<br>IN `0x83`: 317,957<br>OUT `0x70`: 4,035<br>OUT `0x74`: 1,109 |
| **Historical total** | **204** | **3,747,921,196** | **All snapshot files completed without a scan failure** |

Orion has no byte-0 `0x83` report in these 77 historical captures. This is a snapshot-bounded absence claim, not a device-wide claim.

Zen `0x83` is a 320-byte report on endpoint `0x82`. The value `0x83` is report byte 0, not an endpoint address.

Zen endpoint `0x81` has 262,763 six-byte payloads. Their byte-0 counts are `0x00`: 252,708, `0x01`: 10,054, and `0x02`: 1.
The historical scan retained 208,785 distinct six-byte values in the external aggregate.

The exact Zen query total is 1,109 requests and 1,109 replies. This corrects the prior 1,063 aggregate.
The omitted 46 pairs are in `antelope_pcap/zen go sc/channel_assignments/control panel open.pcapng`.

## Bounded Mix 1 file increment

One Orion capture was added after the complete 204-file snapshot:

`antelope_pcap/orion 3/captures 2026-09-07/antelope-orion-mix1-ch1-32-oscillator1khz-4secpauses.pcapng`

- size: 34,126,460 bytes
- SHA-256: `3df29fd4a67c4b93ed38bbe96a5ef1ed828d1d7f59a2a93a32e42dc4a83d009d`
- IN `0x73`: 38,746
- IN strict `0x75/0x1f`: 38,747
- OUT `0x70`: 64
- IN `0x75/0x00` and OUT `0x74`: 0

This first incremental accounting state has 78 Orion files and 127 Zen files. It has 205 files and 3,782,047,656 bytes.

| Orion inventory | Files | Captured bytes | `0x73` | `0x75/0x00` | `0x75/0x1f` | `0x70` | `0x74` |
|---|---:|---:|---:|---:|---:|---:|---:|
| Historical complete snapshot | 77 | 1,829,955,908 | 644,519 | 1,159 | 644,510 | 5,673 | 1,159 |
| Bounded Mix 1 increment | 78 | 1,864,082,368 | 683,265 | 1,159 | 683,257 | 5,737 | 1,159 |

This update is not a complete 205-file rescan. The analyzer hashed and scanned only the Mix 1 file.
A path-and-size comparison found one addition, no removals, and no size changes among the prior 77 Orion paths.
The comparison did not rehash those 77 files. It cannot exclude same-size content changes in them.

## Bounded active-mix switch file increment

One more Orion capture was added after the bounded Mix 1 increment:

`antelope_pcap/orion 3/captures 2026-09-07/mixer-selection-mix2-mix3-mix4-mix1.pcapng`

- size: 1,793,940 bytes
- SHA-256: `0b043b5116af7cac3b460ea497c94050640cf56d7fe358ad024eb80c105e3666`
- IN `0x73`: 2,035
- IN strict `0x75/0x1f`: 2,035
- OUT `0x70`: 4
- IN `0x75/0x00` and OUT `0x74`: 0

The current bounded accounting state has 79 Orion files and 127 Zen files. It has 206 files and 3,783,841,596 bytes.

| Orion inventory | Files | Captured bytes | `0x73` | `0x75/0x00` | `0x75/0x1f` | `0x70` | `0x74` |
|---|---:|---:|---:|---:|---:|---:|---:|
| Historical complete snapshot | 77 | 1,829,955,908 | 644,519 | 1,159 | 644,510 | 5,673 | 1,159 |
| Bounded Mix 1 increment | 78 | 1,864,082,368 | 683,265 | 1,159 | 683,257 | 5,737 | 1,159 |
| Bounded switch increment | 79 | 1,865,876,308 | 685,300 | 1,159 | 685,292 | 5,741 | 1,159 |

The switch analysis hashed and scanned only the new file. It did not rehash or rescan the other 205 files.

These totals are additive accounting, not a new complete 206-file manifest. The [active mix selection contract](orion-active-mix-selection.md) records the bounded command and state findings.

## Method for the historical snapshot

The scanner used TShark 4.7.3 without a display filter. It selected one payload per packet in this order:

1. `usbhid.data`
2. `usb.capdata`
3. `usb.data_fragment`

No packet populated more than one selected extraction field. Equal cyclic reports were retained as separate transfers.
Submission and completion uniqueness relies on one-sided extracted bytes and representative count checks.
The captures do not expose URB IDs, so this is not a full URB-pair proof.

USBPcap records provide endpoint direction. USB-Darwin records do not provide the direction bit or transfer type.
For USB-Darwin, endpoint numbers 1, 2, 4, and 5 use directions corroborated by USBPcap from the same device.
Endpoint number 3 remains direction-unknown. The aggregate records this provenance for each normalized row.

Control, descriptor, short, oversized, and audio traffic remains outside the application-shaped table.
A missing extracted payload does not prove an empty transfer.

## Runtime boundary

The current RAW journal observes data after HID report normalization. It can record the 320-byte selected-interface families in the table.
It cannot see Zen endpoint `0x81` through the existing 320-byte HID handle.
It also cannot see other interfaces, descriptors, control traffic, or audio traffic without new USB I/O.
A wrong-length selected-handle read becomes an error before raw bytes reach the journal.

These limits describe the journal's post-normalization boundary. They do not justify a decoder, endpoint, or transport change.

## Artifact provenance

The historical source bundle is artifact set `7a2dc372-957e-42bc-b630-1bb314d64972` in the agent session output store.
Use the artifact set ID and file name together. The repository does not contain the large JSON corpus.

| Historical artifact | Purpose | SHA-256 |
|---|---|---|
| `complete-raw-family-inventory.md` | Detailed bounded report | `94db9af149c46cdee0fbe4db87e15740c5a857270992664d41f0c109bab03384` |
| `complete-raw-family-inventory.json` | Authoritative 204-file hashes and aggregate | `d8ff1c27d19d185a42ef92fd19b416ad4d3fffa65a133124aa63eb14986dde45` |
| `orion-all-77-per-file.json` | Historical Orion per-file scan | `3b6611ca365b384ec083791ad7bc56008c0417458b7a48781469f3100bd0830b` |
| `zen-all-127-per-file.json` | Zen per-file scan | `675041d4088e142ccf8cda646688fc7fea115cc5ea75f24715e23ea4971ec3f6` |
| `scan_capture_families.py` | Bounded scanner | `af66de2cfbb88a5936e177684585974167a68b5ab5064adb516e0a608aacffd1` |

The historical JSON aggregate is authoritative for its 204 capture paths, sizes, and hashes.
This note does not duplicate its hash rows or the six-byte signature corpus.

The Mix 1 incremental source bundle is artifact set `e901b159-1ee1-4f99-87f9-0d37f21a5eb8`.

| Incremental artifact | Purpose | SHA-256 |
|---|---|---|
| `new-mix1-meter-capture-analysis.md` | Authoritative Mix 1 analysis report | `7c7c633c4f52a4f1079a5b86261d12ca4d2bf9f1bea583987292948a24b2e0bd` |
| `new-mix1-partial-inventory.json` | New-file packet and family inventory | `8e651424e8fcb9a6910466136c4b09eee8ccdb9bcfadf65c507ad23d54118079` |
| `orion-77-to-78-comparison.json` | Path, size, and new-file hash comparison | `5806ecd4ededca07402fcdc75323d4c5b6694a1fd17946fb2b425d9700500195` |
| `new-mix1-window-stats.json` | Exact distributions, windows, controls, and fixtures | `3773d2c94a9efb86ef71b5c7dc8056f03cf97dde06293bf246cbc0d4337fbad9` |
| `analyze_new_mix1.py` | Reproducible read-only analyzer | `19194a1a2c0dea7727cadcdc2765091fa4914a0cd8c7567439346f1586cb17d4` |

The switch source bundle is artifact set `7cee3c5e-37d8-4f08-8fa9-cb25a3c6998d`.

| Switch artifact | Purpose | SHA-256 |
|---|---|---|
| `orion-active-mix-switch-contract.md` | Authoritative bounded analysis and exact fixtures | `d7a2728a9d967a3af8302ad5dad9ae97f7ed631532a3fdde5671f1d51a9a61a5` |
