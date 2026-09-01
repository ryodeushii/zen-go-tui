use crate::mixer::{MixerAssignment, MixerChannelState, MixerSurface};
use crate::query::QueryRequest;
use crate::types::{
    ClockSource, OutputState, OutputTarget, PanState, PreampInputState, PreampMode, ProtocolError,
    SampleRate, Surface,
};

/// Static identity and protocol facts exposed by a device driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceDefinition {
    /// Stable driver identifier.
    pub id: &'static str,
    /// Human-readable device name.
    pub name: &'static str,
    /// USB vendor identifier.
    pub vid: u16,
    /// USB product identifier.
    pub pid: u16,
    /// Whether this driver is safe for normal control.
    pub supported: bool,
}

/// Frames and follow-up requests produced by one normalized action.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandBatch {
    /// HID report frames to write in order.
    pub frames: Vec<Vec<u8>>,
    /// Queries to issue after the frames are written.
    pub refresh_requests: Vec<QueryRequest>,
}

/// Driver-neutral event decoded from one device report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    /// Full state update with the original report bytes.
    Snapshot {
        /// Normalized state values.
        state: DynamicDeviceState,
        /// Original report bytes.
        raw: Vec<u8>,
    },
    /// Query response with its unmodified body and original report bytes.
    QueryReply {
        /// Query identifier from the report.
        query_id: u8,
        /// Query sub-identifier from the report.
        sub_id: u8,
        /// Response body after the protocol header.
        body: Vec<u8>,
        /// Original report bytes.
        raw: Vec<u8>,
    },
    /// Auxiliary report that does not update normalized state.
    Auxiliary {
        /// Auxiliary payload bytes.
        bytes: Vec<u8>,
        /// Original report bytes.
        raw: Vec<u8>,
    },
    /// Short device notification.
    Notification {
        /// Notification payload bytes.
        bytes: Vec<u8>,
        /// Original report bytes.
        raw: Vec<u8>,
    },
}

/// Vector-backed state shared by device drivers and application code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicDeviceState {
    /// Current sample-rate setting.
    pub sample_rate: SampleRate,
    /// Current clock source.
    pub clock_source: ClockSource,
    /// Sample rate reported directly in hertz.
    pub sample_rate_hz: u32,
    /// Raw status flags.
    pub status_flags: Vec<u8>,
    /// Raw front-panel state bytes.
    pub front_panel_bytes: Vec<u8>,
    /// Physical output states.
    pub outputs: Vec<OutputState>,
    /// Preamp input states.
    pub preamps: Vec<PreampInputState>,
    /// Raw DSP/preamp bytes.
    pub dsp_cluster: Vec<u8>,
    /// Selected front-panel surface.
    pub surface: Surface,
    /// Mixer state grouped by surface.
    pub mixer_surfaces: Vec<DynamicMixerSurface>,
    /// Late protocol shadow bytes.
    pub late_shadow: Vec<u8>,
}

/// State for one normalized mixer surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicMixerSurface {
    /// Logical mixer surface.
    pub mixer: MixerSurface,
    /// Channels present on this surface.
    pub strips: Vec<MixerChannelState>,
}

/// Driver-neutral application action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Set sample rate.
    SetSampleRate(SampleRate),
    /// Set clock source.
    SetClockSource(ClockSource),
    /// Select front-panel surface.
    SelectSurface(Surface),
    /// Set preamp input mode.
    SetPreampMode {
        /// Zero-based input index.
        input: u8,
        /// Desired mode.
        mode: PreampMode,
    },
    /// Set raw preamp gain.
    SetPreampGain {
        /// Zero-based input index.
        input: u8,
        /// Raw gain value.
        raw: u8,
    },
    /// Set phantom power.
    SetPreampPhantom {
        /// Zero-based input index.
        input: u8,
        /// Whether phantom power is enabled.
        enabled: bool,
    },
    /// Set phase inversion.
    SetPreampPhase {
        /// Zero-based input index.
        input: u8,
        /// Whether phase inversion is enabled.
        enabled: bool,
    },
    /// Set physical output volume.
    SetOutputVolume {
        /// Output destination.
        target: OutputTarget,
        /// Device attenuation step.
        step: u8,
    },
    /// Set physical output mute.
    SetOutputMute {
        /// Output destination.
        target: OutputTarget,
        /// Whether mute is enabled.
        enabled: bool,
    },
    /// Set physical output dim mode.
    SetOutputDim {
        /// Output destination.
        target: OutputTarget,
        /// Whether dim is enabled.
        enabled: bool,
    },
    /// Set complete mixer strip state.
    SetMixerLevel {
        /// Mixer surface.
        mixer: MixerSurface,
        /// One-based channel number.
        channel: u8,
        /// Raw level.
        level: u8,
        /// Pan position.
        pan_state: PanState,
        /// Whether mute is enabled.
        muted: bool,
        /// Whether solo is enabled.
        soloed: bool,
    },
    /// Set mixer strip mute state.
    SetMixerMute {
        /// Mixer surface.
        mixer: MixerSurface,
        /// One-based channel number.
        channel: u8,
        /// Whether mute is enabled.
        muted: bool,
        /// Current pan position.
        pan_state: PanState,
        /// Current solo state.
        soloed: bool,
    },
    /// Set mixer strip solo state.
    SetMixerSolo {
        /// Mixer surface.
        mixer: MixerSurface,
        /// One-based channel number.
        channel: u8,
        /// Whether solo is enabled.
        soloed: bool,
        /// Current mute state.
        muted: bool,
        /// Current pan position.
        pan_state: PanState,
    },
    /// Set mixer strip pan state.
    SetMixerPan {
        /// Mixer surface.
        mixer: MixerSurface,
        /// One-based channel number.
        channel: u8,
        /// New pan position.
        pan: PanState,
        /// Current mute state.
        muted: bool,
        /// Current solo state.
        soloed: bool,
    },
    /// Set one mixer assignment using the current full assignment table.
    SetMixerAssignment {
        /// One-based strip number.
        strip: u8,
        /// New assignment.
        assignment: MixerAssignment,
        /// Current assignments for all strips.
        assignments: [MixerAssignment; 16],
    },
    /// Set a stereo-link selector and optional companion bank.
    SetLinkState {
        /// Raw link selector.
        selector: u8,
        /// Whether linking is enabled.
        enabled: bool,
        /// Companion bank required by some selectors.
        companion_bank: Option<u8>,
    },
    /// Encode one startup or refresh query.
    Query(QueryRequest),
}

/// Errors produced by a device driver.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// Underlying packet parsing failed.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// Action has no implementation for this driver.
    #[error("unsupported driver action: {0}")]
    UnsupportedAction(&'static str),
    /// Action arguments are inconsistent or unsafe.
    #[error("invalid driver action: {0}")]
    InvalidAction(String),
}

/// Converts normalized application actions and raw reports for one device.
pub trait DeviceDriver: Send {
    /// Return static identity and support information.
    fn definition(&self) -> &DeviceDefinition;

    /// Return requests issued during device startup.
    fn startup_requests(&self) -> &[QueryRequest];

    /// Encode one normalized action into ordered frames and follow-up queries.
    fn encode(&self, action: Action) -> Result<CommandBatch, DriverError>;

    /// Decode one raw HID report into a normalized event.
    fn decode(&self, bytes: &[u8]) -> Result<Option<DeviceEvent>, DriverError>;
}

#[cfg(test)]
mod tests {
    use super::{Action, DeviceDriver, DeviceEvent};
    use crate::mixer::MixerSurface;
    use crate::types::{OutputTarget, PanState, PreampMode, SampleRate};
    use crate::zen_go::ZenGoDriver;

    #[test]
    fn zen_go_driver_reports_canonical_identity() {
        let driver = ZenGoDriver::new();
        let definition = driver.definition();

        assert_eq!(definition.name, "Antelope Zen Go Synergy Core");
        assert_eq!(definition.vid, 0x23e5);
        assert_eq!(definition.pid, 0xa015);
        assert!(definition.supported);
    }

    #[test]
    fn zen_go_driver_exposes_startup_requests_and_query_bytes() {
        let driver = ZenGoDriver::new();

        assert_eq!(driver.startup_requests().len(), 47);
        let query = driver.startup_requests()[0];
        let batch = driver
            .encode(Action::Query(query))
            .expect("startup query must encode");

        assert_eq!(batch.frames.len(), 1);
        assert!(batch.refresh_requests.is_empty());
        assert_eq!(&batch.frames[0][0..4], &0x74_u32.to_le_bytes());
        assert_eq!(&batch.frames[0][4..8], &0x10_u32.to_le_bytes());
        assert_eq!(batch.frames[0][0x08], query.query_id);
        assert_eq!(batch.frames[0][0x0c], query.sub_id);
    }

    #[test]
    fn zen_go_driver_preserves_representative_command_bytes() {
        let driver = ZenGoDriver::new();

        let output = driver
            .encode(Action::SetOutputVolume {
                target: OutputTarget::Monitor,
                step: 0x12,
            })
            .expect("output action must encode");
        assert_eq!(&output.frames[0][0x10..0x13], &[0x47, 0x00, 0x12]);

        let preamp = driver
            .encode(Action::SetPreampMode {
                input: 1,
                mode: PreampMode::Line,
            })
            .expect("preamp action must encode");
        assert_eq!(&preamp.frames[0][0x10..0x13], &[0x4f, 0x01, 0x01]);

        let mixer = driver
            .encode(Action::SetMixerLevel {
                mixer: MixerSurface::Mix2,
                channel: 7,
                level: 0x22,
                pan_state: PanState::right(),
                muted: true,
                soloed: false,
            })
            .expect("mixer action must encode");
        assert_eq!(
            &mixer.frames[0][0x10..0x16],
            &[0xd4, 0x04, 0x01, 0x07, 0x22, 0x7e]
        );
    }

    #[test]
    fn zen_go_driver_decodes_snapshot_and_preserves_raw_bytes() {
        let driver = ZenGoDriver::new();
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0xe6_u32.to_le_bytes());
        frame[0x10 + 0x02] = SampleRate::Hz48000.code();
        frame[0x10 + 0x03] = 0x00;
        frame[0x10 + 0x04..0x10 + 0x08].copy_from_slice(&48_000_u32.to_be_bytes());

        let event = driver
            .decode(&frame)
            .expect("snapshot must decode")
            .expect("snapshot must produce event");
        let DeviceEvent::Snapshot { state, raw } = event else {
            panic!("expected snapshot event");
        };

        assert_eq!(state.sample_rate, SampleRate::Hz48000);
        assert_eq!(state.outputs.len(), 3);
        assert_eq!(raw, frame);
    }
}
