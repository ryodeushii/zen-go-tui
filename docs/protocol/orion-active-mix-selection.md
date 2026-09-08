# Orion active mix selection

Status: capture-confirmed wire contract. Runtime and profile support remain unimplemented at root revision `dbf1314`.

## Command and state contract

Each active-mix click sends one 320-byte OUT report to endpoint `0x01`:

| Full-report offset | Value | Meaning |
|---:|---:|---|
| @0 | `0x70` | Command report |
| @4 | `0x13` | `SET_PARAM` |
| @16 | `0x49` | Active-mix parameter |
| @17 | `0x01` | Fixed target |
| @18 | `0..3` | Mix 1 through Mix 4 |
| all other offsets | `0x00` | Required zero fill |

The encoder must allocate a new zeroed 320-byte report. It must reject values outside `0..3` before transport.

Complete 320-byte IN endpoint `0x82` family `0x73` reports provide passive state:

```text
full-report @122 = active mix ID 0..3
payload-relative offset = 0x6a
```

The four command values have this finite meaning:

| UI selection | @18 command value | Matching `0x73` @122 |
|---|---:|---:|
| Mix 1 | 0 | 0 |
| Mix 2 | 1 | 1 |
| Mix 3 | 2 | 2 |
| Mix 4 | 3 | 3 |

The observed click order was Mix 2, Mix 3, Mix 4, then Mix 1. The state sequence was `0 -> 1 -> 2 -> 3 -> 0`.

## Separate selector domains

Full-report @121 stayed 18 in all 2,035 `0x73` reports. It did not follow the active UI selection.

The active mix uses `SET_PARAM(0x49, target=1, value=0..3)` and reads at @122. The meter context uses a separate target and state byte.

Earlier evidence connects `SET_PARAM(0x49, target=0, value=22)` to meter-context state @121. Mix 1 meter evidence has only an @121 read gate of 21.

This capture provides no target-0 selector-21 write. It also provides no evidence for target-0 values 23 or 24.

Routing remains a separate whole-table command. Active mix IDs do not replace Mix 1 through Mix 4 routing destinations 10 through 13.

## Command and readback alignment

Times use TShark `frame.time_relative`. Old reports count fresh `0x73` reports that retained the previous @122 value.

| Click | OUT frame / time | Command bytes `49 01 value` | First matching state frame / time | Old reports | Delay |
|---|---|---|---|---:|---:|
| Mix 2 | 1331 / 2.635770 s | `49 01 01` | 1367 / 2.707066 s | 8 | 71.296 ms |
| Mix 3 | 3047 / 6.064271 s | `49 01 02` | 3077 / 6.123037 s | 7 | 58.766 ms |
| Mix 4 | 4745 / 9.498507 s | `49 01 03` | 4759 / 9.522949 s | 3 | 24.442 ms |
| Mix 1 | 6263 / 12.528379 s | `49 01 00` | 6277 / 12.554931 s | 3 | 26.552 ms |

Each clean last-before and first-matching pair differs only at @122. The file contains exactly four OUT reports and no duplicate writes.

The cyclic state report confirms state, not command execution identity. A matching same-value report cannot serve as a unique command acknowledgment.

## Meter boundary

Every mixer-meter byte at full-report @125..188 stayed 96. The capture had no active mixer signal.

The capture confirms @122 as the active UI mix ID. It does not prove that @122 selects a physical meter bank.

Do not infer target-0 meter selectors 23 or 24 from active mix IDs 2 and 3. Do not infer Mix 3 or Mix 4 meter offsets.

Keep the confirmed Mix 1 meter gate narrow. Mix 1 remains valid only when the same report has @121 equal to 21.

The six provisional output offsets overlap Mix 1 mirrors under @121 equal to 21. This capture adds no physical-output ownership evidence.

## Capture inventory and integrity

The capture path is relative to the capture checkout root:

`antelope_pcap/orion 3/captures 2026-09-07/mixer-selection-mix2-mix3-mix4-mix1.pcapng`

- Size: 1,793,940 bytes.
- SHA-256: `0b043b5116af7cac3b460ea497c94050640cf56d7fe358ad024eb80c105e3666`.
- Packets: 8,160.
- Extracted 320-byte HID payloads: 4,074.
- IN `0x73`: 2,035.
- IN strict `0x75/0x1f`: 2,035.
- OUT `0x70`: 4.
- IN `0x75/0x00` and OUT `0x74`: 0.

All strict `0x75/0x1f` reports are identical. Their @8 and @20 template values are both 48.

This bounded capture contains no queries. It also provides no evidence for hidden startup commands, automatic retries, or duplicate delivery handling.

The nested profile records an earlier startup command with `0x49`, target 1, and value 0. This contract identifies that command as Mix 1 selection.

This identification does not authorize automatic startup emission. Preserve the existing no-auto-command behavior.

## Controller handoff for task 54

The current controller changes `mixer.surface_index` locally and emits no transport write. Task 54 can add this contract behind an Orion profile capability.

A write success does not confirm the selected mix. Confirmation requires a complete, fresh `0x73` report whose @122 matches the requested value.

The controller must tolerate old fresh reports after a write. This capture contains three through eight such reports per transition.

A 250 ms timeout is a conservative controller suggestion. It is not a protocol constant or device guarantee.

On timeout or uncertain delivery, the controller must not retry blindly. It must keep the last confirmed state and reconcile with later state reports.

Rapid selections must use request generations. Late reports for an older request must not replace the latest desired selection.

## Exact fixture hashes

Each fixture is a complete 320-byte `usbhid.data` payload. The analysis report contains the full fixture hex.

| Selection | Command frame and SHA-256 | Last-before state frame and SHA-256 | First-matching state frame and SHA-256 |
|---|---|---|---|
| Mix 2 | 1331, `0910e163bba045c48721a72a5c114fdc066a8e78224f6a01c2efb88baedad1a5` | 1329, `e194a7d543ddfd40289a64b5b24923daa27354ded675745eb0699d7fced336e1` | 1367, `f752bb67a1e6b866cdbebdcfb34c3cc0c9cba282b9a4eee4ca42dae7e8d4d956` |
| Mix 3 | 3047, `c9a30c7ac58581d0e84be0ec525212115ae218cfdaab23706ac461320882a64a` | 3043, `f752bb67a1e6b866cdbebdcfb34c3cc0c9cba282b9a4eee4ca42dae7e8d4d956` | 3077, `148ec3da90730c6c78c49d933b8a7efab9919d33e6d196eeea251b9b6049b7bf` |
| Mix 4 | 4745, `d6b3282a59390830d200e140378b71fa725ef2b8be4d730845667302376a9b86` | 4741, `148ec3da90730c6c78c49d933b8a7efab9919d33e6d196eeea251b9b6049b7bf` | 4759, `11c4e73024389e62ace8fda1b3b08d961a5a8194f23043c577e27a5cf7ade205` |
| Mix 1 | 6263, `cb09b37d02d3c17ab59c1fac923d4a00d650d31d781f24f66d712adcd2b1241f` | 6259, `11c4e73024389e62ace8fda1b3b08d961a5a8194f23043c577e27a5cf7ade205` | 6277, `e194a7d543ddfd40289a64b5b24923daa27354ded675745eb0699d7fced336e1` |

## Analysis provenance

Artifact set `7cee3c5e-37d8-4f08-8fa9-cb25a3c6998d` contains the authoritative analysis report.

| Artifact | SHA-256 |
|---|---|
| `orion-active-mix-switch-contract.md` | `d7a2728a9d967a3af8302ad5dad9ae97f7ed631532a3fdde5671f1d51a9a61a5` |

The report fixture hex was decoded again during integration. All eight payloads were 320 bytes and matched their documented SHA-256 values.
