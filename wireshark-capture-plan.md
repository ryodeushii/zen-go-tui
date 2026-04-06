# Wireshark Capture Plan — Zen Go Synergy Core

> Note
> This file preserves the original planned sessions for `capture_01` through `capture_09`.
> Later ad hoc captures `capture_10`, `capture_10_2`, and `capture_11` were not described here originally.
> Use `docs/protocol/pcap-analysis.md` for the canonical all-captures catalog and the normalized intent/source mapping.

## Prerequisites

1. **Windows machine** with Zen Go connected via USB
2. **Wireshark** installed with USBPcap driver
3. **Antelope USB Audio Driver** installed and working
4. **Antelope Launcher** or **Zen Go Control Panel** accessible

## Setup

### 1. Select the Correct USBPcap Interface
- Open Wireshark
- Find the USBPcap interface that corresponds to the Zen Go
- Verify by checking for device `23e5:a015` in the device list
- Note the interface number (e.g., `\\.\USBPcap1`)

### 2. Apply Capture Filter (optional but recommended)
```
usb.device_address == <address>
```
Or filter after capture with display filter:
```
usb.src == "host" && usb.capdata || usb.dst == "host" && usb.capdata
```

### 3. Before Starting Capture
- Close Antelope Launcher / Control Panel
- Stop any audio playback
- Clear Wireshark display

## Capture Sessions

### Session 1: Device Enumeration (Connect)
**Goal:** See what happens when the device is first connected — descriptor requests, initial configuration, any vendor-specific setup commands.

1. Start Wireshark capture
2. **Unplug** Zen Go USB cable
3. Wait 3 seconds
4. **Plug in** Zen Go USB cable
5. Wait for Windows to recognize the device (~10 seconds)
6. Wait for driver to load (device appears in Sound settings)
7. Stop capture
8. **Save as:** `capture_01_enumeration.pcapng`

### Session 2: Volume DOWN
**Goal:** Capture the exact interrupt packets sent when lowering volume.

1. Start Wireshark capture
2. Open Antelope Launcher / Control Panel
3. Slowly drag **one output channel volume** from 0 dB down to -∞ (minimum)
4. Wait 2 seconds
5. Stop capture
6. **Save as:** `capture_02_volume_down.pcapng`

### Session 3: Volume UP
**Goal:** Capture the exact interrupt packets sent when raising volume.

1. Start Wireshark capture
2. Slowly drag **the same output channel volume** from minimum back up to 0 dB
3. Wait 2 seconds
4. Stop capture
5. **Save as:** `capture_03_volume_up.pcapng`

### Session 4: Mute / Unmute Toggle
**Goal:** Capture the difference between mute and unmute commands.

1. Start Wireshark capture
2. Click **Mute** on one channel
3. Wait 1 second
4. Click **Unmute** on the same channel
5. Wait 1 second
6. Click **Mute** again
7. Wait 1 second
8. Click **Unmute** again
9. Stop capture
10. **Save as:** `capture_04_mute_toggle.pcapng`

### Session 5: Sample Rate Changes
**Goal:** Identify packets sent when changing sample rate.

1. Start Wireshark capture
2. Change sample rate to **44100 Hz**
3. Wait 2 seconds
4. Change to **48000 Hz**
5. Wait 2 seconds
6. Change to **88200 Hz**
7. Wait 2 seconds
8. Change to **96000 Hz**
9. Wait 2 seconds
10. Change to **176400 Hz**
11. Wait 2 seconds
12. Change to **192000 Hz**
13. Wait 2 seconds
14. Change back to **48000 Hz** (default)
15. Stop capture
16. **Save as:** `capture_05_sample_rate.pcapng`

### Session 6: Clock Source Changes
**Goal:** Identify packets for clock source selection.

1. Start Wireshark capture
2. Change clock source to **Internal**
3. Wait 2 seconds
4. Change to **S/PDIF** (if available)
5. Wait 2 seconds
6. Change to **ADAT** (if available)
7. Wait 2 seconds
8. Change to **Word Clock** (if available)
9. Wait 2 seconds
10. Change back to **Internal**
11. Stop capture
12. **Save as:** `capture_06_clock_source.pcapng`

### Session 7: DSP Properties
**Goal:** Capture reads and writes of DSP settings (phantom power, Hi-Z, etc.).

1. Start Wireshark capture
2. Toggle **Phantom Power (+48V)** ON for one input
3. Wait 1 second
4. Toggle **Phantom Power** OFF
5. Wait 1 second
6. Toggle **Hi-Z** impedance ON for one input
7. Wait 1 second
8. Toggle **Hi-Z** OFF
9. Wait 1 second
10. Change any other DSP setting available (dim, mono, phase invert, etc.)
11. Stop capture
12. **Save as:** `capture_07_dsp_properties.pcapng`

### Session 8: Routing / Matrix
**Goal:** Capture routing changes if the control panel supports them.

1. Start Wireshark capture
2. Change any **input→output routing** setting
3. Wait 2 seconds
4. Change another routing setting
5. Wait 2 seconds
6. Adjust any **mixer fader** if available
7. Wait 2 seconds
8. Stop capture
9. **Save as:** `capture_08_routing.pcapng`

### Session 9: Device Notification Polling
**Goal:** Understand periodic polling/notification traffic.

1. Start Wireshark capture
2. **Do nothing** — let the device sit idle
3. Capture for **30 seconds**
4. Stop capture
5. **Save as:** `capture_09_idle_polling.pcapng`

### Session 10: Mixer Single-Channel Changes One at a Time
**Goal:** Capture mixer-channel behavior one control at a time on individual channels, including fades and mute/unmute, first while channels are linked and then after unlinking specific channels.

> Reconstructed later from filename, packet review, and user clarification.
> Treat this as normalized intent, not original contemporaneous notes.

1. Start Wireshark capture
2. Open the mixer page in the Antelope control panel
3. Focus on **PREA 1** and **PREA 2** first
4. Perform one change at a time per channel with pauses between actions:
   - fade to minimum
   - fade to a middle position
   - fade to maximum / 0 dB region
   - mute
   - unmute
5. Repeat the same one-at-a-time actions for the corresponding path on **MIX 1**
6. Repeat the same one-at-a-time actions for the corresponding path on **MIX 2**
7. Unlink **COMP 1** and **COMP 2** if they were linked
8. Repeat the same single-channel action sequence for **COMP 1** individually:
   - fade down
   - fade mid
   - fade up
   - mute
   - unmute
9. Repeat the same single-channel action sequence for **COMP 2** individually
10. Stop capture
11. **Save as:** `capture_10_mixer_single_channel_one_change_at_a_time_then_unlink_comp12_and_go_individually_onc1andc2.pcapng`

### Session 10_2: Linked COMP1 + COMP2 Mixer Actions
**Goal:** Capture the linked-channel behavior for `COMP 1` and `COMP 2`, especially how fades and mute/unmute propagate while the pair remains linked.

> Reconstructed later from filename, packet review, and user clarification.
> Treat this as normalized intent, not original contemporaneous notes.

1. Start Wireshark capture
2. Open the mixer page in the Antelope control panel
3. Ensure **COMP 1** and **COMP 2** are **linked**
4. Perform the following actions while they remain linked:
   - fade down
   - fade to a middle position
   - fade up to maximum / 0 dB region
   - mute
   - unmute
5. If relevant, repeat the linked actions on both **MIX 1** and **MIX 2** surfaces
6. Stop capture
7. **Save as:** `capture_10_2_mixers_linked_comp12_fades_mute_unmute.pcapng`

### Session 11: Output Controls One Change at a Time
**Goal:** Isolate output-page behavior for monitor and headphone outputs by moving one target at a time through a small set of well-separated volume positions.

> Reconstructed later from filename, packet review, and user clarification.
> Treat this as normalized intent, not original contemporaneous notes.

1. Start Wireshark capture
2. Open the **Monitors & Headphones** / output page in the Antelope control panel
3. Select the **Monitor** output and perform one change at a time:
   - set volume to minimum / none
   - set volume to a middle position
   - set volume to maximum / 0 dB region
4. Wait briefly between each step
5. Repeat the same sequence for **HP1**:
   - minimum
   - middle
   - maximum
6. Repeat the same sequence for **HP2**:
   - minimum
   - middle
   - maximum
7. Avoid mixing in unrelated DSP or routing changes during this capture
8. Stop capture
9. **Save as:** `capture_11_output_controls_once_change_at_a_time.pcapng`

## Analysis Guide

After capturing, look for:

### URB_INTERRUPT Transfers
- Filter: `usb.transfer_type == 0x01`
- These are the control packets (128 bytes based on old capture)
- Look for the magic bytes `70 00 00 00` at offset 0

### URB_CONTROL Transfers
- Filter: `usb.transfer_type == 0x02`
- Standard UAC2 requests (volume, mute, sample rate via USB Audio Class)
- May coexist with interrupt transfers

### URB_BULK Transfers
- Filter: `usb.transfer_type == 0x03`
- Audio streaming data (isochronous usually, but bulk possible)

### Key Fields to Extract
For each interrupt packet:
- Bytes 0-3: Magic (should be `70 00 00 00`)
- Bytes 4-7: Payload size
- Bytes 8-15: Reserved/zeros
- Bytes 16-19: **Command ID** (changes per command type)
- Byte 20: Sequence counter
- Byte 21: **Flags** (direction, state)
- Bytes 22+: **Data payload** (volume value, sample rate, etc.)

## Tips

- Perform actions **slowly and deliberately** — leave gaps between actions
- Note the **exact time** you perform each action (helps correlate with packets)
- If the control panel has a "apply" button, click it and note the timing
- Capture each session **separately** for cleaner analysis
- If a session produces too much noise, redo it with fewer background processes
