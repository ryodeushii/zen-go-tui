use antelope_protocol::{Action, DeviceDriver};

use crate::transport::Transport;
use anyhow::Result;

/// Maximum number of pending commands in the queue.
const MAX_QUEUE_SIZE: usize = 64;

/// Identifies the coalescing group for a command.
/// Commands with the same key are coalesced — only the latest is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoalesceKey {
    MixerLevel { mixer_code: u8, channel: u8 },
    MixerPan { mixer_code: u8, channel: u8 },
    MixerMute { mixer_code: u8, channel: u8 },
    MixerSolo { mixer_code: u8, channel: u8 },
    OutputVolume { target_code: u8 },
    OutputMute { target_code: u8 },
    OutputDim { target_code: u8 },
    PreampGain { input: u8 },
    PreampMode { input: u8 },
    PreampPhantom { input: u8 },
    PreampPhase { input: u8 },
    MixerAssignment { strip: u8 },
    LinkState { selector: u8 },
    SelectSurface,
    SampleRate,
    ClockSource,
}

/// Returns the coalescing key for a command, or `None` if it should not be coalesced.
fn coalesce_key_for_command(cmd: &Action) -> Option<CoalesceKey> {
    match cmd {
        Action::SetMixerLevel { mixer, channel, .. } => Some(CoalesceKey::MixerLevel {
            mixer_code: mixer.code(),
            channel: *channel,
        }),
        Action::SetMixerPan { mixer, channel, .. } => Some(CoalesceKey::MixerPan {
            mixer_code: mixer.code(),
            channel: *channel,
        }),
        Action::SetMixerMute { mixer, channel, .. } => Some(CoalesceKey::MixerMute {
            mixer_code: mixer.code(),
            channel: *channel,
        }),
        Action::SetMixerSolo { mixer, channel, .. } => Some(CoalesceKey::MixerSolo {
            mixer_code: mixer.code(),
            channel: *channel,
        }),
        Action::SetOutputVolume { target, .. } => Some(CoalesceKey::OutputVolume {
            target_code: (*target).index(),
        }),
        Action::SetOutputMute { target, .. } => Some(CoalesceKey::OutputMute {
            target_code: (*target).index(),
        }),
        Action::SetOutputDim { target, .. } => Some(CoalesceKey::OutputDim {
            target_code: (*target).index(),
        }),
        Action::SetPreampGain { input, .. } => Some(CoalesceKey::PreampGain { input: *input }),
        Action::SetPreampMode { input, .. } => Some(CoalesceKey::PreampMode { input: *input }),
        Action::SetPreampPhantom { input, .. } => {
            Some(CoalesceKey::PreampPhantom { input: *input })
        }
        Action::SetPreampPhase { input, .. } => Some(CoalesceKey::PreampPhase { input: *input }),
        Action::SetMixerAssignment { strip, .. } => {
            Some(CoalesceKey::MixerAssignment { strip: *strip })
        }
        Action::SetLinkState { selector, .. } => Some(CoalesceKey::LinkState {
            selector: *selector,
        }),
        Action::SelectSurface(_) => Some(CoalesceKey::SelectSurface),
        Action::SetSampleRate(_) => Some(CoalesceKey::SampleRate),
        Action::SetClockSource(_) => Some(CoalesceKey::ClockSource),
        Action::Query(_) => None,
    }
}

/// A queued command awaiting dispatch to the transport.
#[derive(Debug, Clone)]
pub struct QueuedCommand {
    pub command: Action,
}

/// A bounded command queue that coalesces duplicate commands.
///
/// When a command is enqueued, if a pending command with the same coalescing key
/// already exists, it is replaced with the newer one. This prevents redundant
/// HID writes when a user rapidly adjusts a slider or gain control.
#[derive(Debug, Default)]
pub struct CommandQueue {
    entries: Vec<QueuedCommand>,
}

impl CommandQueue {
    /// Creates a new empty command queue.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Enqueues a command, coalescing with any existing command that shares the same key.
    ///
    /// If the queue is at capacity and no coalescing occurs, the command is dropped
    /// and `false` is returned. Otherwise returns `true`.
    pub fn enqueue(&mut self, command: Action) -> bool {
        if let Some(key) = coalesce_key_for_command(&command) {
            // Try to find and replace an existing command with the same key
            for entry in &mut self.entries {
                if let Some(existing_key) = coalesce_key_for_command(&entry.command) {
                    if existing_key == key {
                        entry.command = command;
                        return true;
                    }
                }
            }
        }

        // No coalescing possible — append if space
        if self.entries.len() >= MAX_QUEUE_SIZE {
            return false;
        }

        self.entries.push(QueuedCommand { command });
        true
    }

    /// Returns the number of pending commands.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drains the queue and sends all commands through the transport.
    ///
    /// Commands are sent in queue order (oldest first), with coalesced
    /// commands already replaced by their latest version.
    ///
    /// Returns the number of commands sent, or an error if a write fails.
    pub fn flush(&mut self, transport: &dyn Transport, driver: &dyn DeviceDriver) -> Result<usize> {
        let count = self.entries.len();
        for entry in self.entries.drain(..) {
            let batch = driver.encode(entry.command)?;
            for frame in batch.frames {
                transport.write(&frame)?;
            }
            for request in batch.refresh_requests {
                let refresh = driver.encode(Action::Query(request))?;
                for frame in refresh.frames {
                    transport.write(&frame)?;
                }
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use antelope_protocol::{
        Action, ClockSource, MixerAssignment, MixerSurface, OutputTarget, PanState, PreampMode,
        SampleRate, Surface, ZenGoDriver,
    };

    use crate::transport::MockTransport;

    use super::*;

    #[test]
    fn enqueue_coalesces_mixer_level_changes() {
        let mut queue = CommandQueue::new();

        queue.enqueue(Action::SetMixerLevel {
            mixer: MixerSurface::Mix1,
            channel: 1,
            level: 0x10,
            pan_state: PanState::center(),
            muted: false,
            soloed: false,
        });
        assert_eq!(queue.len(), 1);

        // Same mixer+channel — should coalesce
        queue.enqueue(Action::SetMixerLevel {
            mixer: MixerSurface::Mix1,
            channel: 1,
            level: 0x20,
            pan_state: PanState::center(),
            muted: false,
            soloed: false,
        });
        assert_eq!(queue.len(), 1);

        // Different channel — should not coalesce
        queue.enqueue(Action::SetMixerLevel {
            mixer: MixerSurface::Mix1,
            channel: 2,
            level: 0x30,
            pan_state: PanState::center(),
            muted: false,
            soloed: false,
        });
        assert_eq!(queue.len(), 2);

        // Different mixer — should not coalesce
        queue.enqueue(Action::SetMixerLevel {
            mixer: MixerSurface::Mix2,
            channel: 1,
            level: 0x40,
            pan_state: PanState::center(),
            muted: false,
            soloed: false,
        });
        assert_eq!(queue.len(), 3);
    }

    #[test]
    fn flush_sends_coalesced_latest_value() {
        let transport = MockTransport::default();
        let mut queue = CommandQueue::new();

        // Rapid level changes on same channel
        for level in 0x10..=0x15 {
            queue.enqueue(Action::SetMixerLevel {
                mixer: MixerSurface::Mix1,
                channel: 3,
                level,
                pan_state: PanState::center(),
                muted: false,
                soloed: false,
            });
        }
        assert_eq!(queue.len(), 1);

        let count = queue.flush(&transport, &ZenGoDriver::new()).expect("flush");
        assert_eq!(count, 1);

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        // Should have the latest level (0x15)
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x03, 0x15, 0x20]
        );
    }

    #[test]
    fn coalescing_works_for_all_coalescable_types() {
        let mut queue = CommandQueue::new();

        // MixerPan
        queue.enqueue(Action::SetMixerPan {
            mixer: MixerSurface::Mix1,
            channel: 1,
            pan: PanState::left(),
            muted: false,
            soloed: false,
        });
        queue.enqueue(Action::SetMixerPan {
            mixer: MixerSurface::Mix1,
            channel: 1,
            pan: PanState::right(),
            muted: false,
            soloed: false,
        });
        assert_eq!(queue.len(), 1);

        // MixerMute
        queue.enqueue(Action::SetMixerMute {
            mixer: MixerSurface::Mix1,
            channel: 1,
            muted: false,
            pan_state: PanState::center(),
            soloed: false,
        });
        queue.enqueue(Action::SetMixerMute {
            mixer: MixerSurface::Mix1,
            channel: 1,
            muted: true,
            pan_state: PanState::center(),
            soloed: false,
        });
        assert_eq!(queue.len(), 2);

        // MixerSolo
        queue.enqueue(Action::SetMixerSolo {
            mixer: MixerSurface::Mix1,
            channel: 1,
            soloed: false,
            muted: false,
            pan_state: PanState::center(),
        });
        queue.enqueue(Action::SetMixerSolo {
            mixer: MixerSurface::Mix1,
            channel: 1,
            soloed: true,
            muted: false,
            pan_state: PanState::center(),
        });
        assert_eq!(queue.len(), 3);

        // OutputVolume
        queue.enqueue(Action::SetOutputVolume {
            target: OutputTarget::Monitor,
            step: 0x10,
        });
        queue.enqueue(Action::SetOutputVolume {
            target: OutputTarget::Monitor,
            step: 0x20,
        });
        assert_eq!(queue.len(), 4);

        // OutputMute
        queue.enqueue(Action::SetOutputMute {
            target: OutputTarget::Hp1,
            enabled: false,
        });
        queue.enqueue(Action::SetOutputMute {
            target: OutputTarget::Hp1,
            enabled: true,
        });
        assert_eq!(queue.len(), 5);

        // OutputDim
        queue.enqueue(Action::SetOutputDim {
            target: OutputTarget::Hp2,
            enabled: false,
        });
        queue.enqueue(Action::SetOutputDim {
            target: OutputTarget::Hp2,
            enabled: true,
        });
        assert_eq!(queue.len(), 6);

        // PreampGain
        queue.enqueue(Action::SetPreampGain {
            input: 0,
            raw: 0x20,
        });
        queue.enqueue(Action::SetPreampGain {
            input: 0,
            raw: 0x30,
        });
        assert_eq!(queue.len(), 7);

        // PreampMode
        queue.enqueue(Action::SetPreampMode {
            input: 0,
            mode: PreampMode::Mic,
        });
        queue.enqueue(Action::SetPreampMode {
            input: 0,
            mode: PreampMode::Line,
        });
        assert_eq!(queue.len(), 8);

        // PreampPhantom
        queue.enqueue(Action::SetPreampPhantom {
            input: 0,
            enabled: false,
        });
        queue.enqueue(Action::SetPreampPhantom {
            input: 0,
            enabled: true,
        });
        assert_eq!(queue.len(), 9);

        // PreampPhase
        queue.enqueue(Action::SetPreampPhase {
            input: 0,
            enabled: false,
        });
        queue.enqueue(Action::SetPreampPhase {
            input: 0,
            enabled: true,
        });
        assert_eq!(queue.len(), 10);

        // MixerAssignment
        queue.enqueue(Action::SetMixerAssignment {
            strip: 1,
            assignment: MixerAssignment::Mute,
            assignments: [MixerAssignment::Mute; 16],
        });
        queue.enqueue(Action::SetMixerAssignment {
            strip: 1,
            assignment: MixerAssignment::Preamp(1),
            assignments: [MixerAssignment::Mute; 16],
        });
        assert_eq!(queue.len(), 11);

        // SetLinkState
        queue.enqueue(Action::SetLinkState {
            selector: 0x00,
            enabled: false,
            companion_bank: None,
        });
        queue.enqueue(Action::SetLinkState {
            selector: 0x00,
            enabled: true,
            companion_bank: None,
        });
        assert_eq!(queue.len(), 12);

        // SelectSurface
        queue.enqueue(Action::SelectSurface(Surface::MonitorHp1));
        queue.enqueue(Action::SelectSurface(Surface::Hp2));
        assert_eq!(queue.len(), 13);

        // SetSampleRate
        queue.enqueue(Action::SetSampleRate(SampleRate::Hz48000));
        queue.enqueue(Action::SetSampleRate(SampleRate::Hz96000));
        assert_eq!(queue.len(), 14);

        // SetClockSource
        queue.enqueue(Action::SetClockSource(ClockSource::Internal));
        queue.enqueue(Action::SetClockSource(ClockSource::Usb));
        assert_eq!(queue.len(), 15);
    }

    #[test]
    fn non_coalescable_commands_all_queue() {
        let mut queue = CommandQueue::new();

        // These commands have different coalesce keys and should all be queued
        queue.enqueue(Action::SetMixerLevel {
            mixer: MixerSurface::Mix1,
            channel: 1,
            level: 0x10,
            pan_state: PanState::center(),
            muted: false,
            soloed: false,
        });
        queue.enqueue(Action::SetMixerMute {
            mixer: MixerSurface::Mix1,
            channel: 1,
            muted: true,
            pan_state: PanState::center(),
            soloed: false,
        });
        queue.enqueue(Action::SetOutputVolume {
            target: OutputTarget::Monitor,
            step: 0x20,
        });
        queue.enqueue(Action::SetOutputMute {
            target: OutputTarget::Monitor,
            enabled: true,
        });

        // All different keys — no coalescing
        assert_eq!(queue.len(), 4);
    }

    #[test]
    fn bounded_queue_rejects_when_full() {
        let mut queue = CommandQueue::new();

        // Fill the queue with commands on different keys
        for channel in 1..=(MAX_QUEUE_SIZE as u8) {
            let ok = queue.enqueue(Action::SetMixerLevel {
                mixer: MixerSurface::Mix1,
                channel,
                level: 0x20,
                pan_state: PanState::center(),
                muted: false,
                soloed: false,
            });
            if channel as usize <= MAX_QUEUE_SIZE {
                assert!(ok, "should accept up to MAX_QUEUE_SIZE");
            }
        }
        assert_eq!(queue.len(), MAX_QUEUE_SIZE);

        // Next command with a new key should be rejected
        let ok = queue.enqueue(Action::SetMixerLevel {
            mixer: MixerSurface::Mix2,
            channel: 1,
            level: 0x30,
            pan_state: PanState::center(),
            muted: false,
            soloed: false,
        });
        assert!(!ok, "should reject when queue is full and no coalescing");
        assert_eq!(queue.len(), MAX_QUEUE_SIZE);
    }

    #[test]
    fn bounded_queue_accepts_coalesced_command_when_full() {
        let mut queue = CommandQueue::new();

        // Fill with unique keys
        for channel in 1..=(MAX_QUEUE_SIZE as u8) {
            queue.enqueue(Action::SetMixerLevel {
                mixer: MixerSurface::Mix1,
                channel,
                level: 0x20,
                pan_state: PanState::center(),
                muted: false,
                soloed: false,
            });
        }
        assert_eq!(queue.len(), MAX_QUEUE_SIZE);

        // Coalescing should still work even when full
        let ok = queue.enqueue(Action::SetMixerLevel {
            mixer: MixerSurface::Mix1,
            channel: 1,
            level: 0x30,
            pan_state: PanState::center(),
            muted: false,
            soloed: false,
        });
        assert!(ok, "should accept when coalescing");
        assert_eq!(queue.len(), MAX_QUEUE_SIZE);
    }

    #[test]
    fn flush_sends_commands_in_order() {
        let transport = MockTransport::default();
        let mut queue = CommandQueue::new();

        // Add commands for different targets
        queue.enqueue(Action::SetOutputVolume {
            target: OutputTarget::Monitor,
            step: 0x10,
        });
        queue.enqueue(Action::SetOutputVolume {
            target: OutputTarget::Hp1,
            step: 0x20,
        });
        queue.enqueue(Action::SetOutputVolume {
            target: OutputTarget::Hp2,
            step: 0x30,
        });

        // Update Monitor (should replace first entry, keeping position)
        queue.enqueue(Action::SetOutputVolume {
            target: OutputTarget::Monitor,
            step: 0x15,
        });

        assert_eq!(queue.len(), 3);

        let count = queue.flush(&transport, &ZenGoDriver::new()).expect("flush");
        assert_eq!(count, 3);

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 3);

        // Order should be: Monitor (updated), Hp1, Hp2
        // Monitor at position 0 was updated to 0x15
        assert_eq!(&writes[0][0x10..0x13], &[0x47, 0x00, 0x15]);
        assert_eq!(&writes[1][0x10..0x13], &[0x47, 0x01, 0x20]);
        assert_eq!(&writes[2][0x10..0x13], &[0x47, 0x02, 0x30]);
    }

    #[test]
    fn flush_clears_the_queue() {
        let transport = MockTransport::default();
        let mut queue = CommandQueue::new();

        queue.enqueue(Action::SetOutputVolume {
            target: OutputTarget::Monitor,
            step: 0x10,
        });
        queue.enqueue(Action::SetOutputVolume {
            target: OutputTarget::Hp1,
            step: 0x20,
        });

        queue.flush(&transport, &ZenGoDriver::new()).expect("flush");
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn flush_on_empty_queue_returns_zero() {
        let transport = MockTransport::default();
        let mut queue = CommandQueue::new();

        let count = queue.flush(&transport, &ZenGoDriver::new()).expect("flush");
        assert_eq!(count, 0);
    }

    #[test]
    fn rapid_slider_simulation() {
        // Simulate a user rapidly dragging a slider through 97 positions
        let transport = MockTransport::default();
        let mut queue = CommandQueue::new();

        for level in 0x00..=0x60 {
            queue.enqueue(Action::SetMixerLevel {
                mixer: MixerSurface::Mix1,
                channel: 5,
                level,
                pan_state: PanState::center(),
                muted: false,
                soloed: false,
            });
        }

        // All coalesced into one
        assert_eq!(queue.len(), 1);

        let count = queue.flush(&transport, &ZenGoDriver::new()).expect("flush");
        assert_eq!(count, 1);

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 1);
        // Latest value should be 0x60
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x05, 0x60, 0x20]
        );
    }

    #[test]
    fn mixed_coalescable_and_non_coalescable() {
        let transport = MockTransport::default();
        let mut queue = CommandQueue::new();

        // Coalescable: multiple level changes on same channel
        queue.enqueue(Action::SetMixerLevel {
            mixer: MixerSurface::Mix1,
            channel: 1,
            level: 0x10,
            pan_state: PanState::center(),
            muted: false,
            soloed: false,
        });
        queue.enqueue(Action::SetMixerLevel {
            mixer: MixerSurface::Mix1,
            channel: 1,
            level: 0x20,
            pan_state: PanState::center(),
            muted: false,
            soloed: false,
        });

        // Different key: different channel
        queue.enqueue(Action::SetMixerLevel {
            mixer: MixerSurface::Mix1,
            channel: 2,
            level: 0x30,
            pan_state: PanState::center(),
            muted: false,
            soloed: false,
        });

        // Different command type on same channel
        queue.enqueue(Action::SetMixerMute {
            mixer: MixerSurface::Mix1,
            channel: 1,
            muted: true,
            pan_state: PanState::center(),
            soloed: false,
        });

        assert_eq!(queue.len(), 3);

        let count = queue.flush(&transport, &ZenGoDriver::new()).expect("flush");
        assert_eq!(count, 3);

        let writes = transport.take_writes();
        assert_eq!(writes.len(), 3);
        // First: level for ch1 (coalesced to 0x20)
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x20, 0x20]
        );
        // Second: level for ch2
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x02, 0x30, 0x20]
        );
        // Third: mute for ch1
        assert_eq!(
            &writes[2][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x00, 0x60]
        );
    }
}
