# Zen Go Synergy Core Control Panel — Protocol Reference

Source: Official "EN - Zen Go SC Manual.pdf" (pages 36-70)

## Architecture Overview

### Two Virtual Mixers
- **MIX 1 (Monitor/HP1)** — stereo mix routed to both Monitor outputs (rear TS/TRS + RCA) AND Headphone 1 (front HP1 jack). Same mix sends, independent volume/dim/mute per physical output.
- **MIX 2 (Headphone 2)** — stereo mix routed to Headphone 2 (front HP2 jack) only.

Each mixer has **15 channels** (8 visible by default, expandable via "Mixer 8↔15" toggle).

### Channel Input Sources (per mixer channel)
| Input | Description |
|-------|-------------|
| PREAMP 1-2 | Analog inputs A1-A2 (rear XLR/TRS) |
| EMU MIC 1-4 | Edge/Verge mic emulation virtual outputs |
| COMPUTER PLAY 1-8 | DAW playback channels 1-8 |
| S/PDIF IN 1-2 | Digital stereo input |
| MUTE | No audio (silence) |
| OSCILLATOR 1-2 | Test tones (440Hz/1kHz) |

### Per-Channel Controls (per mixer)
- **Pan** — -90 (hard left) to 0 (center) to +90 (hard right)
- **Volume fader** — send level to the mix bus
- **Solo (S)** — solo this channel (mutes all non-soloed)
- **Mute (M)** — mute this channel's send
- **Link (L)** — stereo-pair adjacent channels (1-2, 3-4, etc.)

### AFX Strips (Channels 1-4 only)
- Synergy Core FX chains with bypass (BP) per effect
- BP ALL to bypass entire chain
- SAVE/LOAD FX chain presets
- DEL ALL to clear (Ctrl/Cmd+Click protected)

### Physical Outputs
| Output | Mix Source | Controls |
|--------|-----------|----------|
| MONITOR | MIX 1 | Volume, DIM, MUTE |
| HEADPHONE 1 | MIX 1 | Volume, DIM, MUTE |
| HEADPHONE 2 | MIX 2 | Volume, DIM, MUTE |

Each has independent volume knob, DIM button, MUTE button, and stereo peak meters.

## Control Panel Views

### Monitors & Headphones (Main View)
- Two tabs: "Monitor/HP1" and "Headphones 2"
- Input selectors + AFX + mixer faders + output controls
- Peak meters for each mix's stereo output

### Digital Outs
- S/PDIF 1-2 input selectors
- Peak meters (no volume control — digital output is fixed level)

### DAW I/O
- Mirrors Monitors & Headphones input selectors + AFX
- **Record 1-8** → routed to DAW inputs (pre-mixer, dry signal)
- **Play 1-8** ← from DAW outputs (appear as COMPUTER PLAY in mixers)
- **MON/HP1 MIX 1-2** — loopback of MIX 1 stereo output into DAW
- **HP2 MIX 1-2** — loopback of MIX 2 stereo output into DAW

Key insight: DAW recording happens **before** the virtual mixer. Mixer pan/volume don't affect what gets recorded — only monitoring.

## Function Strips

### Strip 1 (Top Bar)
- On/Standby
- Settings: Monitor Out Trim (14-20dBu), Panning Law (0 to -4.5dB), Oscillator 1/2 (freq + level), ASIO buffer, Brightness (USB only)

### Strip 2
- Clock Source: Internal / S/PDIF / USB
- Sample Rate: 32kHz - 192kHz
- Lock indicator (S/PDIF)
- Sessions: save/load *.as preset files (full Control Panel snapshot)

### Strip 3
- View selector (Monitors/HP, Digital Outs, DAW I/O)
- AuraVerb button (send-effect reverb — ignored for now)
- Mixer 8↔15 toggle

## Preamps Strip
- Signal type per channel: Mic / Line / Hi-Z
- Link button (stereo-pair A1/A2 controls)
- Gain: Mic 0-65dB, Line -6 to 20dB, Hi-Z 0-60dB
- 48V phantom power (Ctrl+Click protected)
- Phase flip
- Mic emulations (Edge/Verge modeling mic selector)
- Custom text labels (double-click to rename)

## Mouse/Keyboard Shortcuts
- Double-click knob/fader → reset to default
- Ctrl/Cmd + drag → fine adjustment (1dB increments)
- Click peak meter → clear it
- Drag window edges → resize
