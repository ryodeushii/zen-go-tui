//! Protocol definitions and encoding/decoding for Antelope Audio Zen Go Synergy Core.
//!
//! This crate provides types and functions for communicating with the Zen Go Synergy Core
//! audio interface over USB HID. It covers:
//!
//! - **Frame parsing**: Decode incoming HID reports into typed [`Frame`] variants
//! - **Command encoding**: Build outgoing HID frames via [`encode_command`]
//! - **State types**: Strongly-typed representations of device state (sample rate, clock source,
//!   preamp settings, mixer strips, etc.)
//! - **Startup queries**: The sequence of queries sent during device initialization via
//!   [`control_panel_startup_queries`]
//!
//! # Example
//!
//! ```no_run
//! use antelope_protocol::{Frame, Command, encode_command, SampleRate};
//!
//! // Parse an incoming frame
//! let raw = vec![0u8; 320];
//! let frame = Frame::parse(&raw).unwrap();
//!
//! // Encode a command
//! let cmd = Command::SetSampleRate(SampleRate::Hz48000);
//! let encoded = encode_command(cmd);
//! ```

mod encoder;
mod frame;
mod mixer;
mod query;
mod types;

// Re-export everything for backward compatibility
pub use encoder::*;
pub use frame::*;
pub use mixer::*;
pub use query::*;
pub use types::*;
