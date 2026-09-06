use antelope_protocol::{
    Action, DeviceDriver, GlobalControl, InputAddress, InputControl, MixerAddress, OutputAddress,
    OutputControl,
};

use crate::transport::Transport;
use anyhow::Result;

const MAX_QUEUE_SIZE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoalesceKey {
    Input(InputAddress, InputControl),
    Output(OutputAddress, OutputControl),
    MixerStrip(MixerAddress),
    Link { surface: u8, pair: u16 },
    Global(GlobalControl),
}

fn coalesce_key_for_command(command: &Action) -> Option<CoalesceKey> {
    match command {
        Action::SetInput {
            address, control, ..
        } => Some(CoalesceKey::Input(*address, *control)),
        Action::SetOutput {
            address, control, ..
        } => Some(CoalesceKey::Output(*address, *control)),
        Action::SetMixerStripState { address, .. } => Some(CoalesceKey::MixerStrip(*address)),
        Action::SetLink { surface, pair, .. } => Some(CoalesceKey::Link {
            surface: *surface,
            pair: *pair,
        }),
        Action::SetGlobal { control, .. } => Some(CoalesceKey::Global(*control)),
        Action::SetMixer { .. }
        | Action::SetRouting { .. }
        | Action::SetRoutingGroup { .. }
        | Action::SetWholeState { .. }
        | Action::Query(_) => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QueueEntryId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueueEntryOutcome {
    /// Every frame belonging to the queued entry was written successfully.
    Sent,
    /// A transport operation failed while delivery of the current entry was uncertain.
    Failed,
    /// No frame belonging to the queued entry was attempted.
    Unsent,
}

#[derive(Debug, Clone)]
pub struct QueuedCommand {
    pub command: Action,
}

#[derive(Debug, Clone)]
struct QueueEntry {
    command: Action,
    id: QueueEntryId,
}

#[derive(Debug, Default)]
pub struct CommandQueue {
    entries: Vec<QueueEntry>,
    next_id: u64,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(&mut self, command: Action) -> bool {
        self.enqueue_with_id(command).is_some()
    }

    pub(crate) fn enqueue_with_id(&mut self, command: Action) -> Option<QueueEntryId> {
        if let Some(key) = coalesce_key_for_command(&command) {
            for entry in &mut self.entries {
                if coalesce_key_for_command(&entry.command) == Some(key) {
                    entry.command = command;
                    return Some(entry.id);
                }
            }
        }
        if self.entries.len() >= MAX_QUEUE_SIZE {
            return None;
        }
        let id = QueueEntryId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.entries.push(QueueEntry { command, id });
        Some(id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn flush(&mut self, transport: &dyn Transport, driver: &dyn DeviceDriver) -> Result<usize> {
        self.flush_with(transport, driver, |_, _| {})
    }

    pub(crate) fn flush_with<F>(
        &mut self,
        transport: &dyn Transport,
        driver: &dyn DeviceDriver,
        mut on_outcome: F,
    ) -> Result<usize>
    where
        F: FnMut(QueueEntryId, QueueEntryOutcome),
    {
        let mut count = 0;
        let mut entries = self.entries.drain(..);
        while let Some(entry) = entries.next() {
            let id = entry.id;
            let batch = match driver.encode(entry.command) {
                Ok(batch) => batch,
                Err(error) => {
                    on_outcome(id, QueueEntryOutcome::Unsent);
                    for entry in entries {
                        on_outcome(entry.id, QueueEntryOutcome::Unsent);
                    }
                    return Err(error.into());
                }
            };
            let mut wrote_frame = false;
            for frame in batch.frames {
                if let Err(error) = transport.write(&frame) {
                    // A transport error does not prove that the current frame
                    // was not delivered. Keep the current entry uncertain,
                    // including when this was its first write.
                    on_outcome(id, QueueEntryOutcome::Failed);
                    for entry in entries {
                        on_outcome(entry.id, QueueEntryOutcome::Unsent);
                    }
                    return Err(error);
                }
                wrote_frame = true;
            }
            for request in batch.refresh_requests {
                let refresh = match driver.encode(Action::Query(request)) {
                    Ok(refresh) => refresh,
                    Err(error) => {
                        on_outcome(
                            id,
                            if wrote_frame {
                                QueueEntryOutcome::Failed
                            } else {
                                QueueEntryOutcome::Unsent
                            },
                        );
                        for entry in entries {
                            on_outcome(entry.id, QueueEntryOutcome::Unsent);
                        }
                        return Err(error.into());
                    }
                };
                for frame in refresh.frames {
                    match transport.write(&frame) {
                        Ok(()) => wrote_frame = true,
                        Err(error) => {
                            on_outcome(id, QueueEntryOutcome::Failed);
                            for entry in entries {
                                on_outcome(entry.id, QueueEntryOutcome::Unsent);
                            }
                            return Err(error);
                        }
                    }
                }
            }
            count += 1;
            on_outcome(id, QueueEntryOutcome::Sent);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;
    use antelope_protocol::{
        Action, ClockSource, ControlValue, GlobalControl, InputAddress, InputControl, MixerAddress,
        OutputAddress, OutputControl, RoutingSource, SampleRate,
    };

    fn mixer(fader: i32, strip: u16) -> Action {
        Action::SetMixerStripState {
            address: MixerAddress { surface: 0, strip },
            fader,
            pan: 0,
            muted: false,
            soloed: false,
            send: None,
        }
    }

    #[test]
    fn coalesces_latest_complete_mixer_state_by_numeric_address() {
        let mut queue = CommandQueue::new();
        assert!(queue.enqueue(mixer(0x10, 1)));
        assert!(queue.enqueue(mixer(0x20, 1)));
        assert!(queue.enqueue(mixer(0x30, 2)));
        assert_eq!(queue.len(), 2);
        let transport = MockTransport::default();
        queue
            .flush(
                &transport,
                &crate::device::builtin_zen_go_driver().expect("Zen Go driver"),
            )
            .expect("flush");
        let writes = transport.take_writes();
        assert_eq!(
            &writes[0][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x01, 0x20, 0x20]
        );
        assert_eq!(
            &writes[1][0x10..0x16],
            &[0xd4, 0x04, 0x00, 0x02, 0x30, 0x20]
        );
    }

    #[test]
    fn numeric_input_output_and_global_controls_coalesce_separately() {
        let mut queue = CommandQueue::new();
        for raw in [1, 2] {
            queue.enqueue(Action::SetInput {
                address: InputAddress { space: 0, index: 0 },
                control: InputControl::Gain,
                value: ControlValue::Int(raw),
            });
            queue.enqueue(Action::SetOutput {
                address: OutputAddress { id: 0 },
                control: OutputControl::Level,
                value: ControlValue::Int(raw),
            });
            queue.enqueue(Action::SetGlobal {
                control: GlobalControl::SampleRate,
                value: ControlValue::Enum(i32::from(SampleRate::Hz48000.code())),
            });
            queue.enqueue(Action::SetGlobal {
                control: GlobalControl::ClockSource,
                value: ControlValue::Enum(i32::from(ClockSource::Internal.code())),
            });
        }
        assert_eq!(queue.len(), 4);
    }

    #[test]
    fn partial_mixer_actions_never_coalesce() {
        let mut queue = CommandQueue::new();
        let action = Action::SetMixer {
            address: MixerAddress {
                surface: 0,
                strip: 1,
            },
            control: antelope_protocol::MixerControl::Fader,
            value: ControlValue::Int(1),
        };
        queue.enqueue(action.clone());
        queue.enqueue(action);
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn complete_routing_and_whole_state_actions_never_coalesce() {
        let mut queue = CommandQueue::new();
        let sources = vec![
            RoutingSource {
                bank: 0x08,
                index: 0
            };
            16
        ];
        queue.enqueue(Action::SetRoutingGroup {
            destination: 0,
            changed_channel: None,
            sources: sources.clone(),
        });
        queue.enqueue(Action::SetRoutingGroup {
            destination: 0,
            changed_channel: None,
            sources,
        });
        let whole_state = Action::SetWholeState {
            operation: 0xda,
            target: 0,
            enabled: true,
            fields: vec![],
        };
        queue.enqueue(whole_state.clone());
        queue.enqueue(whole_state);
        assert_eq!(queue.len(), 4);
    }

    #[test]
    fn bounded_queue_rejects_only_new_keys() {
        let mut queue = CommandQueue::new();
        for strip in 1..=MAX_QUEUE_SIZE as u16 {
            assert!(queue.enqueue(mixer(0x20, strip)));
        }
        assert!(!queue.enqueue(Action::Query(antelope_protocol::QueryRequest::new(0, 0))));
        assert!(queue.enqueue(mixer(0x30, 1)));
        assert_eq!(queue.len(), MAX_QUEUE_SIZE);
    }

    #[test]
    fn empty_flush_is_zero() {
        let transport = MockTransport::default();
        let mut queue = CommandQueue::new();
        assert_eq!(
            queue
                .flush(
                    &transport,
                    &crate::device::builtin_zen_go_driver().expect("Zen Go driver"),
                )
                .expect("flush"),
            0
        );
        assert!(queue.is_empty());
    }
}
