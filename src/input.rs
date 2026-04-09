//! Input thread for collecting terminal input events.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use anyhow::Result;

use zen_go_tui::terminal::AppInputEvent;

/// Message sent from the input reader thread to the main loop.
#[derive(Debug)]
pub(crate) enum InputThreadMessage {
    Event(AppInputEvent),
    Error(String),
}

/// Spawn a background thread that reads input events and sends them through a channel.
pub(crate) fn spawn_input_reader() -> Receiver<InputThreadMessage> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || loop {
        match zen_go_tui::terminal::read_input_event() {
            Ok(Some(event)) => {
                if sender.send(InputThreadMessage::Event(event)).is_err() {
                    break;
                }
            }
            Ok(None) => {}
            Err(error) => {
                let _ = sender.send(InputThreadMessage::Error(error.to_string()));
                break;
            }
        }
    });
    receiver
}

/// Drain all pending input events from the channel, returning an error if the reader failed.
pub(crate) fn collect_pending_input(
    receiver: &Receiver<InputThreadMessage>,
) -> Result<Vec<AppInputEvent>> {
    let mut events = Vec::new();
    loop {
        match receiver.try_recv() {
            Ok(InputThreadMessage::Event(event)) => events.push(event),
            Ok(InputThreadMessage::Error(message)) => return Err(anyhow::anyhow!(message)),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return Ok(events),
        }
    }
}
