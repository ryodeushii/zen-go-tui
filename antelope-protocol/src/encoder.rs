//! Command encoding: builds outgoing HID frames from typed Command variants.

use crate::mixer::{MixerAssignment, MixerStrip};
use crate::query::QueryRequest;
use crate::types::{
    ClockSource, OutputTarget, PanState, PreampMode, SampleRate, Surface, HID_REPORT_SIZE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SetSampleRate(SampleRate),
    SetClockSource(ClockSource),
    SelectSurface(Surface),
    SetPreampMode {
        input: u8,
        mode: PreampMode,
    },
    SetPreampGain {
        input: u8,
        raw: u8,
    },
    SetPreampPhantom {
        input: u8,
        enabled: bool,
    },
    SetPreampPhase {
        input: u8,
        enabled: bool,
    },
    SetOutputVolume {
        target: OutputTarget,
        step: u8,
    },
    SetOutputMute {
        target: OutputTarget,
        enabled: bool,
    },
    SetOutputDim {
        target: OutputTarget,
        enabled: bool,
    },
    SetMixerLevel {
        mixer: crate::mixer::MixerSurface,
        channel: u8,
        level: u8,
        pan_state: PanState,
        muted: bool,
        soloed: bool,
    },
    SetMixerMute {
        mixer: crate::mixer::MixerSurface,
        channel: u8,
        muted: bool,
        pan_state: PanState,
        soloed: bool,
    },
    SetMixerSolo {
        mixer: crate::mixer::MixerSurface,
        channel: u8,
        soloed: bool,
        muted: bool,
        pan_state: PanState,
    },
    SetMixerPan {
        mixer: crate::mixer::MixerSurface,
        channel: u8,
        pan: PanState,
        muted: bool,
        soloed: bool,
    },
    SetMixerAssignment {
        strip: u8,
        assignment: MixerAssignment,
    },
    SetLinkState {
        selector: u8,
        enabled: bool,
        companion_bank: Option<u8>,
    },
}

pub fn encode_query(query: QueryRequest) -> Vec<u8> {
    let mut frame = vec![0_u8; HID_REPORT_SIZE];
    frame[0..4].copy_from_slice(&0x74_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x10_u32.to_le_bytes());
    frame[0x08] = query.query_id;
    frame[0x0c] = query.sub_id;
    frame
}

pub fn encode_command(command: Command) -> Vec<u8> {
    match command {
        Command::SetSampleRate(rate) => host_frame(0x12, &[0x03, rate.code()]),
        Command::SetClockSource(source) => host_frame(0x12, &[0x04, source.code()]),
        Command::SelectSurface(surface) => host_frame(0x13, &[0x49, 0x00, surface.code()]),
        Command::SetPreampMode { input, mode } => {
            host_frame(0x13, &[0x4f, input.min(1), mode.code()])
        }
        Command::SetPreampGain { input, raw } => host_frame(0x13, &[0x50, input.min(1), raw]),
        Command::SetPreampPhantom { input, enabled } => {
            host_frame(0x13, &[0x51, input.min(1), u8::from(enabled)])
        }
        Command::SetPreampPhase { input, enabled } => {
            host_frame(0x13, &[0x52, input.min(1), u8::from(enabled)])
        }
        Command::SetOutputVolume { target, step } => {
            host_frame(0x13, &[0x47, target.index(), step])
        }
        Command::SetOutputMute { target, enabled } => {
            host_frame(0x13, &[0x48, target.index(), u8::from(enabled)])
        }
        Command::SetOutputDim { target, enabled } => {
            host_frame(0x13, &[0x66, target.index(), u8::from(enabled)])
        }
        Command::SetMixerLevel {
            mixer,
            channel,
            level,
            pan_state,
            muted,
            soloed,
        } => host_frame(
            0x16,
            &[
                0xd4,
                0x04,
                mixer.code(),
                channel,
                level,
                pan_state.state_code(muted, soloed),
            ],
        ),
        Command::SetMixerMute {
            mixer,
            channel,
            muted,
            pan_state,
            soloed,
        } => host_frame(
            0x16,
            &[
                0xd4,
                0x04,
                mixer.code(),
                channel,
                0x00,
                pan_state.state_code(muted, soloed),
            ],
        ),
        Command::SetMixerSolo {
            mixer,
            channel,
            soloed,
            muted,
            pan_state,
        } => host_frame(
            0x16,
            &[
                0xd4,
                0x04,
                mixer.code(),
                channel,
                0x00,
                pan_state.state_code(muted, soloed),
            ],
        ),
        Command::SetMixerPan {
            mixer,
            channel,
            pan,
            muted,
            soloed,
        } => host_frame(
            0x16,
            &[
                0xd4,
                0x04,
                mixer.code(),
                channel,
                0x00,
                pan.state_code(muted, soloed),
            ],
        ),
        Command::SetMixerAssignment { strip, assignment } => {
            encode_mixer_assignment(strip, assignment)
        }
        Command::SetLinkState {
            selector,
            enabled,
            companion_bank: _,
        } => host_frame(0x14, &[0xa2, 0x03, selector, u8::from(enabled)]),
    }
}

pub fn encode_link_companion(bank: u8, enabled: bool) -> Vec<u8> {
    host_frame(0x14, &[0xa2, 0x04, bank, u8::from(enabled)])
}

fn encode_mixer_assignment(strip: u8, assignment: MixerAssignment) -> Vec<u8> {
    encode_mixer_assignment_frames(strip, assignment)
        .into_iter()
        .next()
        .expect("assignment write must emit at least one frame")
}

pub fn encode_mixer_assignment_frames(strip: u8, assignment: MixerAssignment) -> Vec<Vec<u8>> {
    let strip = MixerStrip::new(strip).expect("assignment write requires grounded strip mapping");
    let entry_index = strip.assignment_entry_index();
    let assignment_bytes = assignment.ordinary_strip_bytes();

    strip
        .assignment_write_banks()
        .iter()
        .copied()
        .map(|bank| {
            let mut frame = assignment_frame(bank);
            write_assignment_entry(&mut frame, entry_index, assignment_bytes);
            frame
        })
        .collect()
}

pub fn encode_mixer_assignment_frames_with_table(
    strip: u8,
    assignment: MixerAssignment,
    assignments: &[MixerAssignment; 16],
) -> Vec<Vec<u8>> {
    let strip = MixerStrip::new(strip).expect("assignment write requires grounded strip mapping");
    let mut full_assignments = *assignments;
    full_assignments[strip.assignment_entry_index()] = assignment;

    strip
        .assignment_write_banks()
        .iter()
        .copied()
        .map(|bank| {
            let mut frame = assignment_frame(bank);

            for entry_index in assignment_entries_for_bank(bank) {
                write_assignment_entry(
                    &mut frame,
                    entry_index,
                    assignment_entry_bytes(bank, entry_index, &full_assignments),
                );
            }

            frame
        })
        .collect()
}

fn assignment_frame(bank: u8) -> Vec<u8> {
    let mut frame = vec![0_u8; HID_REPORT_SIZE];
    frame[0..4].copy_from_slice(&0x70_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&0x53_u32.to_le_bytes());
    frame[0x10..0x13].copy_from_slice(&[0xd3, 0x41, bank]);
    frame
}

fn write_assignment_entry(frame: &mut [u8], entry_index: usize, assignment: [u8; 2]) {
    let tuple_offset = 0x03 + entry_index * 2;
    frame[0x10 + tuple_offset] = assignment[0];
    frame[0x10 + tuple_offset + 1] = assignment[1];
}

fn assignment_entries_for_bank(bank: u8) -> std::ops::Range<usize> {
    match bank {
        0x05 => 0..4,
        0x03 => 0..8,
        0x06..=0x09 => 0..16,
        _ => 0..0,
    }
}

fn assignment_entry_bytes(
    bank: u8,
    entry_index: usize,
    assignments: &[MixerAssignment; 16],
) -> [u8; 2] {
    match (bank, entry_index) {
        (0x03 | 0x06 | 0x07 | 0x08 | 0x09, 0..=3) => [0x03, entry_index as u8],
        _ => assignments[entry_index].ordinary_strip_bytes(),
    }
}

fn host_frame(length: u32, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0_u8; HID_REPORT_SIZE];
    frame[0..4].copy_from_slice(&0x70_u32.to_le_bytes());
    frame[4..8].copy_from_slice(&length.to_le_bytes());
    frame[0x10..0x10 + payload.len()].copy_from_slice(payload);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixer::MixerAssignment;
    use crate::types::PanState;

    #[test]
    fn encodes_ordinary_strip_assignment_write_sequence_for_strip_11() {
        let frames = encode_mixer_assignment_frames(11, MixerAssignment::EmuMic(2));

        assert_eq!(frames.len(), 4);
        for (frame, bank) in frames.iter().zip([0x06_u8, 0x07, 0x08, 0x09]) {
            assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
            assert_eq!(&frame[4..8], &0x53_u32.to_le_bytes());
            assert_eq!(&frame[0x10..0x13], &[0xd3, 0x41, bank]);
            assert_eq!(&frame[0x10 + 0x17..0x10 + 0x19], &[0x0a, 0x01]);
        }
    }

    #[test]
    fn encodes_ordinary_assignment_write_sequence_for_strip_5_with_bank_03() {
        let frames = encode_mixer_assignment_frames(5, MixerAssignment::Oscillator(1));

        assert_eq!(frames.len(), 5);
        for (frame, bank) in frames.iter().zip([0x03_u8, 0x06, 0x07, 0x08, 0x09]) {
            assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
            assert_eq!(&frame[4..8], &0x53_u32.to_le_bytes());
            assert_eq!(&frame[0x10..0x13], &[0xd3, 0x41, bank]);
            assert_eq!(&frame[0x10 + 0x0b..0x10 + 0x0d], &[0x09, 0x00]);
        }
    }

    #[test]
    fn encodes_early_assignment_write_sequence_for_strip_1_with_bank_05() {
        let frames = encode_mixer_assignment_frames(1, MixerAssignment::Oscillator(1));

        assert_eq!(frames.len(), 1);
        let frame = &frames[0];
        assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
        assert_eq!(&frame[4..8], &0x53_u32.to_le_bytes());
        assert_eq!(&frame[0x10..0x13], &[0xd3, 0x41, 0x05]);
        assert_eq!(&frame[0x10 + 0x03..0x10 + 0x05], &[0x09, 0x00]);
    }

    #[test]
    fn encodes_grounded_link_selector_write() {
        let frame = encode_command(Command::SetLinkState {
            selector: 0x00,
            enabled: true,
            companion_bank: Some(0x00),
        });

        assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
        assert_eq!(&frame[4..8], &0x14_u32.to_le_bytes());
        assert_eq!(&frame[0x10..0x14], &[0xa2, 0x03, 0x00, 0x01]);
    }

    #[test]
    fn encodes_grounded_link_companion_write() {
        let frame = encode_link_companion(0x01, true);

        assert_eq!(&frame[0..4], &0x70_u32.to_le_bytes());
        assert_eq!(&frame[4..8], &0x14_u32.to_le_bytes());
        assert_eq!(&frame[0x10..0x14], &[0xa2, 0x04, 0x01, 0x01]);
    }

    #[test]
    fn encodes_confirmed_commands() {
        let sample = encode_command(Command::SetSampleRate(SampleRate::Hz44100));
        assert_eq!(&sample[0..4], &0x70_u32.to_le_bytes());
        assert_eq!(sample[4], 0x12);
        assert_eq!(&sample[0x10..0x12], &[0x03, 0x01]);

        let output = encode_command(Command::SetOutputVolume {
            target: OutputTarget::Hp2,
            step: 0x33,
        });
        assert_eq!(output[4], 0x13);
        assert_eq!(&output[0x10..0x13], &[0x47, 0x02, 0x33]);

        let mixer = encode_command(Command::SetMixerLevel {
            mixer: crate::mixer::MixerSurface::Mix2,
            channel: 4,
            level: 0x28,
            pan_state: PanState::right(),
            muted: false,
            soloed: false,
        });
        assert_eq!(mixer[4], 0x16);
        assert_eq!(&mixer[0x10..0x16], &[0xd4, 0x04, 0x01, 0x04, 0x28, 0x3e]);

        let preamp_mode = encode_command(Command::SetPreampMode {
            input: 1,
            mode: PreampMode::HiZ,
        });
        assert_eq!(preamp_mode[4], 0x13);
        assert_eq!(&preamp_mode[0x10..0x13], &[0x4f, 0x01, 0x02]);

        let preamp_gain = encode_command(Command::SetPreampGain {
            input: 0,
            raw: 0x2d,
        });
        assert_eq!(preamp_gain[4], 0x13);
        assert_eq!(&preamp_gain[0x10..0x13], &[0x50, 0x00, 0x2d]);

        let preamp_phantom = encode_command(Command::SetPreampPhantom {
            input: 1,
            enabled: true,
        });
        assert_eq!(preamp_phantom[4], 0x13);
        assert_eq!(&preamp_phantom[0x10..0x13], &[0x51, 0x01, 0x01]);

        let preamp_phase = encode_command(Command::SetPreampPhase {
            input: 0,
            enabled: false,
        });
        assert_eq!(preamp_phase[4], 0x13);
        assert_eq!(&preamp_phase[0x10..0x13], &[0x52, 0x00, 0x00]);
    }
}
