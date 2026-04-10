//! Frame parsing: HID report decoding into typed Frame variants.

use crate::mixer::decode_passive_mixer_state;
use crate::query::QueryResponse;
use crate::types::{
    ClockSource, DeviceStateSnapshot, OutputMode, OutputState, OutputTarget, PreampState,
    ProtocolError, SampleRate, Surface, FRAME_TYPE_AUXILIARY, FRAME_TYPE_QUERY_REPLY,
    FRAME_TYPE_SNAPSHOT, MIN_SNAPSHOT_FRAME_LEN, OFFSET_CLOCK_SOURCE,
    OFFSET_FRONT_PANEL_BYTES_START, OFFSET_HP1_MODE, OFFSET_HP1_VOLUME, OFFSET_HP2_MODE,
    OFFSET_HP2_VOLUME, OFFSET_LATE_SHADOW_END, OFFSET_LATE_SHADOW_START, OFFSET_MONITOR_MODE,
    OFFSET_MONITOR_VOLUME, OFFSET_PREAMP1_GAIN, OFFSET_PREAMP1_MODE, OFFSET_PREAMP2_GAIN,
    OFFSET_PREAMP2_MODE, OFFSET_SAMPLE_RATE_CODE, OFFSET_SAMPLE_RATE_HZ_END,
    OFFSET_SAMPLE_RATE_HZ_START, OFFSET_STATUS_FLAGS_0, OFFSET_STATUS_FLAGS_1,
    OFFSET_SURFACE_SELECTOR, SNAPSHOT_PAYLOAD_OFFSET,
};

/// A short-form device notification (6-byte frame).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceNotification {
    /// Raw notification bytes.
    pub bytes: [u8; 6],
}

/// A parsed HID report frame, classified by its type identifier.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Full device state snapshot (frame type 0x73).
    Snapshot {
        /// Decoded snapshot data.
        snapshot: DeviceStateSnapshot,
        /// Raw frame bytes.
        raw: Vec<u8>,
    },
    /// Response to a query request (frame type 0x75).
    QueryReply {
        /// Decoded query response.
        reply: QueryResponse,
        /// Raw frame bytes.
        raw: Vec<u8>,
    },
    /// Auxiliary data frame (type 0x83), purpose not fully decoded.
    Auxiliary {
        /// Payload bytes after the header.
        bytes: Vec<u8>,
        /// Raw frame bytes.
        raw: Vec<u8>,
    },
    /// Short-form device notification (6-byte frame).
    Notification {
        /// Decoded notification data.
        notification: DeviceNotification,
        /// Raw frame bytes.
        raw: Vec<u8>,
    },
}

impl Frame {
    /// Parses a frame from a byte slice, copying the data.
    pub fn parse(bytes: &[u8]) -> Result<Self, ProtocolError> {
        Self::parse_owned(bytes.to_vec())
    }

    /// Parses a frame from an owned byte vector, preserving the original bytes as `raw`.
    pub fn parse_owned(bytes: Vec<u8>) -> Result<Self, ProtocolError> {
        if bytes.len() < 6 {
            return Err(ProtocolError::FrameTooShort(bytes.len()));
        }

        if bytes.len() == 6 {
            let mut raw = [0_u8; 6];
            raw.copy_from_slice(&bytes);
            return Ok(Self::Notification {
                notification: DeviceNotification { bytes: raw },
                raw: bytes,
            });
        }

        if bytes.len() < 0x12 {
            return Err(ProtocolError::FrameTooShort(bytes.len()));
        }

        let frame_type = u32::from_le_bytes(bytes[0..4].try_into().expect("type header"));
        match frame_type {
            FRAME_TYPE_SNAPSHOT => Ok(Self::Snapshot {
                snapshot: parse_snapshot73(&bytes)?,
                raw: bytes,
            }),
            FRAME_TYPE_QUERY_REPLY => Ok(Self::QueryReply {
                reply: QueryResponse {
                    query_id: bytes[0x08],
                    sub_id: bytes[0x0c],
                    body: bytes[SNAPSHOT_PAYLOAD_OFFSET..].to_vec(),
                },
                raw: bytes,
            }),
            FRAME_TYPE_AUXILIARY => Ok(Self::Auxiliary {
                bytes: bytes[SNAPSHOT_PAYLOAD_OFFSET..].to_vec(),
                raw: bytes,
            }),
            other => Err(ProtocolError::UnsupportedFrame(other)),
        }
    }

    /// Returns a reference to the inner snapshot if this frame is a snapshot variant.
    pub fn as_snapshot(&self) -> Option<&DeviceStateSnapshot> {
        match self {
            Self::Snapshot { snapshot, .. } => Some(snapshot),
            _ => None,
        }
    }

    /// Returns a reference to the inner query reply if this frame is a query reply variant.
    pub fn as_query_reply(&self) -> Option<&QueryResponse> {
        match self {
            Self::QueryReply { reply, .. } => Some(reply),
            _ => None,
        }
    }

    /// Returns the raw bytes of the original HID report frame.
    pub fn raw_bytes(&self) -> &[u8] {
        match self {
            Self::Snapshot { raw, .. } => raw,
            Self::QueryReply { raw, .. } => raw,
            Self::Auxiliary { raw, .. } => raw,
            Self::Notification { raw, .. } => raw,
        }
    }

    /// Consumes the frame and returns a [`DeviceSnapshot`] alongside the raw bytes.
    pub fn into_snapshot_and_raw(self) -> (DeviceSnapshot, Vec<u8>) {
        match self {
            Self::Snapshot { snapshot, raw } => (DeviceSnapshot::Snapshot(snapshot), raw),
            Self::QueryReply { reply, raw } => (DeviceSnapshot::QueryReply(reply), raw),
            Self::Auxiliary { bytes, raw } => (DeviceSnapshot::Auxiliary(bytes), raw),
            Self::Notification { notification, raw } => {
                (DeviceSnapshot::Notification(notification), raw)
            }
        }
    }
}

/// Unified representation of any decoded device response.
///
/// This is the owned, non-raw counterpart of [`Frame`], suitable for
/// storage and further processing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSnapshot {
    /// Full device state snapshot.
    Snapshot(DeviceStateSnapshot),
    /// Auxiliary data payload.
    Auxiliary(Vec<u8>),
    /// Response to a query request.
    QueryReply(QueryResponse),
    /// Short-form device notification.
    Notification(DeviceNotification),
}

impl From<Frame> for DeviceSnapshot {
    fn from(frame: Frame) -> Self {
        match frame {
            Frame::Snapshot { snapshot, .. } => Self::Snapshot(snapshot),
            Frame::Auxiliary { bytes, .. } => Self::Auxiliary(bytes),
            Frame::QueryReply { reply, .. } => Self::QueryReply(reply),
            Frame::Notification { notification, .. } => Self::Notification(notification),
        }
    }
}

fn parse_snapshot73(bytes: &[u8]) -> Result<DeviceStateSnapshot, ProtocolError> {
    if bytes.len() < MIN_SNAPSHOT_FRAME_LEN {
        return Err(ProtocolError::FrameTooShort(bytes.len()));
    }

    let payload = &bytes[SNAPSHOT_PAYLOAD_OFFSET..];
    Ok(DeviceStateSnapshot {
        sample_rate: SampleRate::from_code(payload[OFFSET_SAMPLE_RATE_CODE]),
        clock_source: ClockSource::from_code(payload[OFFSET_CLOCK_SOURCE]),
        sample_rate_hz: u32::from_be_bytes(
            payload[OFFSET_SAMPLE_RATE_HZ_START..OFFSET_SAMPLE_RATE_HZ_END]
                .try_into()
                .expect("sample rate"),
        ),
        status_flags: [
            payload[OFFSET_STATUS_FLAGS_0],
            payload[OFFSET_STATUS_FLAGS_1],
        ],
        front_panel_bytes: [
            payload[OFFSET_FRONT_PANEL_BYTES_START],
            payload[OFFSET_FRONT_PANEL_BYTES_START + 1],
            payload[OFFSET_FRONT_PANEL_BYTES_START + 2],
        ],
        outputs: [
            OutputState::new(
                OutputTarget::Monitor,
                payload[OFFSET_MONITOR_VOLUME],
                OutputMode::from_code(payload[OFFSET_MONITOR_MODE]),
            ),
            OutputState::new(
                OutputTarget::Hp1,
                payload[OFFSET_HP1_VOLUME],
                OutputMode::from_code(payload[OFFSET_HP1_MODE]),
            ),
            OutputState::new(
                OutputTarget::Hp2,
                payload[OFFSET_HP2_VOLUME],
                OutputMode::from_code(payload[OFFSET_HP2_MODE]),
            ),
        ],
        dsp_cluster: [
            payload[OFFSET_PREAMP1_GAIN],
            payload[OFFSET_PREAMP2_GAIN],
            payload[OFFSET_PREAMP1_MODE],
            payload[OFFSET_PREAMP2_MODE],
        ],
        preamp: PreampState::from_cluster([
            payload[OFFSET_PREAMP1_GAIN],
            payload[OFFSET_PREAMP2_GAIN],
            payload[OFFSET_PREAMP1_MODE],
            payload[OFFSET_PREAMP2_MODE],
        ]),
        surface: Surface::from_code(payload[OFFSET_SURFACE_SELECTOR]),
        mixer_decode: decode_passive_mixer_state(payload),
        late_shadow: {
            let mut shadow = [0u8; 12];
            shadow.copy_from_slice(&payload[OFFSET_LATE_SHADOW_START..=OFFSET_LATE_SHADOW_END]);
            shadow
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixer::MixerSurface;
    use crate::types::{
        PanState, PreampInputState, PreampMode, PreampState, OFFSET_DSP_CLUSTER_END,
        OFFSET_DSP_CLUSTER_START, OFFSET_METER_LANES_START, OFFSET_MIX1_LANE_A, OFFSET_MIX1_LANE_B,
        OFFSET_MIX1_MIRROR_A, OFFSET_MIX1_MIRROR_B, OFFSET_MIX1_PRIMARY, OFFSET_MIX2_LANE_A,
        OFFSET_MIX2_LANE_B, OFFSET_MIX2_PRIMARY, OFFSET_PREAMP1_METER, OFFSET_PREAMP2_METER,
        OFFSET_SHARED_SHADOW_0, OFFSET_SHARED_SHADOW_1, OFFSET_SURFACE_SELECTOR,
    };

    fn empty_snapshot_frame() -> Vec<u8> {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame
    }

    #[test]
    fn decodes_snapshot_global_fields_and_outputs() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x20_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_STATUS_FLAGS_0] = 0x08;
        payload[OFFSET_SAMPLE_RATE_CODE] = 0x02;
        payload[OFFSET_CLOCK_SOURCE] = 0x01;
        payload[OFFSET_SAMPLE_RATE_HZ_START..OFFSET_SAMPLE_RATE_HZ_END]
            .copy_from_slice(&48_000_u32.to_be_bytes());
        payload[OFFSET_MONITOR_VOLUME] = 0x40;
        payload[OFFSET_MONITOR_MODE] = 0x02;
        payload[OFFSET_HP1_VOLUME] = 0x30;
        payload[OFFSET_HP1_MODE] = 0x01;
        payload[OFFSET_HP2_VOLUME] = 0x20;
        payload[OFFSET_HP2_MODE] = 0x00;
        payload[OFFSET_SURFACE_SELECTOR] = 0x0c;
        payload[OFFSET_DSP_CLUSTER_START..OFFSET_DSP_CLUSTER_END]
            .copy_from_slice(&[0x2f, 0x34, 0x50, 0x10]);

        let parsed = Frame::parse(&frame).expect("frame should parse");
        let snapshot = parsed.as_snapshot().expect("snapshot");

        assert_eq!(snapshot.sample_rate, SampleRate::Hz48000);
        assert_eq!(snapshot.clock_source, ClockSource::Spdif);
        assert_eq!(snapshot.sample_rate_hz, 48_000);
        assert_eq!(snapshot.surface, Surface::Hp2);
        assert_eq!(snapshot.output(OutputTarget::Monitor).volume, 0x40);
        assert_eq!(snapshot.output(OutputTarget::Monitor).mode, OutputMode::Dim);
        assert_eq!(snapshot.output(OutputTarget::Hp1).mode, OutputMode::Mute);
        assert_eq!(snapshot.output(OutputTarget::Hp2).mode, OutputMode::Normal);
        assert_eq!(snapshot.dsp_cluster, [0x2f, 0x34, 0x50, 0x10]);
        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix2, 1)
                .unwrap()
                .meter,
            None
        );
    }

    #[test]
    fn snapshot_frame_preserves_raw_bytes() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX1_PRIMARY] = 0x5a;
        frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX2_PRIMARY] = 0x4c;
        frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX2_LANE_A] = 0x11;

        let parsed = Frame::parse(&frame).expect("frame should parse");
        assert_eq!(
            parsed.raw_bytes()[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX1_PRIMARY],
            0x5a
        );
        assert_eq!(
            parsed.raw_bytes()[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX2_PRIMARY],
            0x4c
        );
        assert_eq!(
            parsed.raw_bytes()[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX2_LANE_A],
            0x11
        );
    }

    #[test]
    fn snapshot_frame_parse_owned_preserves_raw_bytes() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX1_PRIMARY] = 0x5a;
        frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX2_PRIMARY] = 0x4c;
        frame[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX2_LANE_A] = 0x11;

        let parsed = Frame::parse_owned(frame).expect("frame should parse");
        assert_eq!(
            parsed.raw_bytes()[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX1_PRIMARY],
            0x5a
        );
        assert_eq!(
            parsed.raw_bytes()[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX2_PRIMARY],
            0x4c
        );
        assert_eq!(
            parsed.raw_bytes()[SNAPSHOT_PAYLOAD_OFFSET + OFFSET_MIX2_LANE_A],
            0x11
        );
    }

    #[test]
    fn does_not_decode_observed_preamp1_meter_from_untrusted_lane() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[0x7e] = 0x18;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, None);
    }

    #[test]
    fn does_not_decode_observed_preamp_meters_from_row_status_values() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[OFFSET_PREAMP1_METER] = 0x54;
        payload[OFFSET_PREAMP2_METER] = 0x4e;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, None);
    }

    #[test]
    fn decodes_observed_preamp1_meter_from_direct_lane() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[OFFSET_PREAMP1_METER] = 0x38;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, Some(0x38));
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, None);
    }

    #[test]
    fn decodes_observed_preamp2_meter_from_direct_lane() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[OFFSET_PREAMP2_METER] = 0x49;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, Some(0x49));
    }

    #[test]
    fn observed_preamp1_meter_can_coexist_with_strip1_meter() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[OFFSET_PREAMP1_METER] = 0x2a;
        payload[OFFSET_METER_LANES_START] = 0x12;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 1)
                .expect("mix1 strip 1")
                .meter,
            Some(0x12)
        );
        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, Some(0x2a));
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, None);
    }

    #[test]
    fn observed_preamp2_meter_can_coexist_with_strip_meter() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[OFFSET_PREAMP2_METER] = 0x22;
        payload[OFFSET_METER_LANES_START] = 0x12;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 1)
                .expect("mix1 strip 1")
                .meter,
            Some(0x12)
        );
        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, Some(0x22));
    }

    #[test]
    fn decodes_passive_shared_strip1_meter_from_slot_byte() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[OFFSET_METER_LANES_START] = 0x12;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 1)
                .expect("mix1 strip 1")
                .meter,
            Some(0x12)
        );
        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix2, 1)
                .expect("mix2 strip 1")
                .meter,
            Some(0x12)
        );
        assert_eq!(snapshot.mixer_decode.observed_preamp1_meter, None);
        assert_eq!(snapshot.mixer_decode.observed_preamp2_meter, None);
    }

    #[test]
    fn decodes_passive_shared_strip11_meter_from_slot_byte() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[0x98] = 0x05;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 11)
                .expect("mix1 strip 11")
                .meter,
            Some(0x05)
        );
        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix2, 11)
                .expect("mix2 strip 11")
                .meter,
            Some(0x05)
        );
    }

    #[test]
    fn decodes_passive_mix1_strip1_mute_from_late_row_cluster() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[OFFSET_MIX1_PRIMARY] = 0x51;
        payload[OFFSET_MIX2_PRIMARY] = 0x51;
        payload[OFFSET_MIX1_LANE_A] = 0x51;
        payload[OFFSET_MIX1_LANE_B] = 0x51;
        payload[OFFSET_MIX1_MIRROR_A] = 0x51;
        payload[OFFSET_MIX1_MIRROR_B] = 0x51;
        payload[OFFSET_MIX2_LANE_A] = 0x4e;
        payload[OFFSET_MIX2_LANE_B] = 0x4e;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        let strip = snapshot
            .mixer_decode
            .strip(MixerSurface::Mix1, 1)
            .expect("mix1 strip 1");
        assert_eq!(strip.muted, Some(true));
    }

    #[test]
    fn decodes_passive_mix1_link_pair_from_late_row_cluster() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&FRAME_TYPE_SNAPSHOT.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[OFFSET_MIX1_PRIMARY] = 0x51;
        payload[OFFSET_MIX2_PRIMARY] = 0x51;
        payload[OFFSET_MIX1_LANE_A] = 0x4e;
        payload[OFFSET_MIX1_LANE_B] = 0x4e;
        payload[OFFSET_MIX1_MIRROR_A] = 0x4e;
        payload[OFFSET_MIX1_MIRROR_B] = 0x4e;
        payload[OFFSET_MIX2_LANE_A] = 0x4e;
        payload[OFFSET_MIX2_LANE_B] = 0x4e;

        let snapshot = Frame::parse(&frame)
            .expect("frame should parse")
            .as_snapshot()
            .expect("snapshot")
            .clone();

        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 1)
                .unwrap()
                .linked,
            Some(true)
        );
        assert_eq!(
            snapshot
                .mixer_decode
                .strip(MixerSurface::Mix1, 2)
                .unwrap()
                .linked,
            Some(true)
        );
    }

    #[test]
    fn experimental_pair_state_lanes_extract_mix1_mirrored_codebook() {
        let mut frame = empty_snapshot_frame();
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0f;
        payload[OFFSET_MIX1_LANE_A] = 0x0a;
        payload[OFFSET_MIX1_LANE_B] = 0x05;
        payload[OFFSET_MIX1_MIRROR_A] = 0x0a;
        payload[OFFSET_MIX1_MIRROR_B] = 0x05;
        payload[OFFSET_SHARED_SHADOW_0] = 0x60;
        payload[OFFSET_SHARED_SHADOW_1] = 0x60;

        assert_eq!(
            experimental_surface_pair_lanes(&frame[SNAPSHOT_PAYLOAD_OFFSET..]),
            Some(ExperimentalSurfacePairLanes {
                mixer: MixerSurface::Mix1,
                lane_a: 0x0a,
                lane_b: 0x05,
                mirrored: true,
            })
        );
    }

    #[test]
    fn experimental_pair_state_lanes_extract_mix2_compact_codebook() {
        let mut frame = empty_snapshot_frame();
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0c;
        payload[OFFSET_MIX2_LANE_A] = 0x00;
        payload[OFFSET_MIX2_LANE_B] = 0x06;
        payload[OFFSET_SHARED_SHADOW_0] = 0x60;
        payload[OFFSET_SHARED_SHADOW_1] = 0x60;

        assert_eq!(
            experimental_surface_pair_lanes(&frame[SNAPSHOT_PAYLOAD_OFFSET..]),
            Some(ExperimentalSurfacePairLanes {
                mixer: MixerSurface::Mix2,
                lane_a: 0x00,
                lane_b: 0x06,
                mirrored: false,
            })
        );
    }

    #[test]
    fn experimental_pair_state_lanes_preserve_both_mute_idle_form() {
        let mut frame = empty_snapshot_frame();
        let payload = &mut frame[SNAPSHOT_PAYLOAD_OFFSET..];
        payload[OFFSET_SURFACE_SELECTOR] = 0x0c;
        payload[OFFSET_MIX2_LANE_A] = 0x60;
        payload[OFFSET_MIX2_LANE_B] = 0x60;

        assert_eq!(
            experimental_surface_pair_lanes(&frame[SNAPSHOT_PAYLOAD_OFFSET..]),
            Some(ExperimentalSurfacePairLanes {
                mixer: MixerSurface::Mix2,
                lane_a: 0x60,
                lane_b: 0x60,
                mirrored: false,
            })
        );
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct ExperimentalSurfacePairLanes {
        mixer: MixerSurface,
        lane_a: u8,
        lane_b: u8,
        mirrored: bool,
    }

    fn experimental_surface_pair_lanes(payload: &[u8]) -> Option<ExperimentalSurfacePairLanes> {
        let mixer =
            MixerSurface::from_surface(Surface::from_code(*payload.get(OFFSET_SURFACE_SELECTOR)?));
        match mixer {
            MixerSurface::Mix1 => {
                let lane_a = *payload.get(OFFSET_MIX1_LANE_A)?;
                let lane_b = *payload.get(OFFSET_MIX1_LANE_B)?;
                Some(ExperimentalSurfacePairLanes {
                    mixer,
                    lane_a,
                    lane_b,
                    mirrored: payload.get(OFFSET_MIX1_MIRROR_A) == Some(&lane_a)
                        && payload.get(OFFSET_MIX1_MIRROR_B) == Some(&lane_b),
                })
            }
            MixerSurface::Mix2 => Some(ExperimentalSurfacePairLanes {
                mixer,
                lane_a: *payload.get(OFFSET_MIX2_LANE_A)?,
                lane_b: *payload.get(OFFSET_MIX2_LANE_B)?,
                mirrored: false,
            }),
        }
    }
}
