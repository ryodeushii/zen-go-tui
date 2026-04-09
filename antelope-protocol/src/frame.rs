//! Frame parsing: HID report decoding into typed Frame variants.

use crate::mixer::decode_passive_mixer_state;
use crate::query::QueryResponse;
use crate::types::{
    ClockSource, DeviceStateSnapshot, OutputMode, OutputState, OutputTarget, PreampState,
    ProtocolError, SampleRate, Surface,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceNotification {
    pub bytes: [u8; 6],
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Snapshot {
        snapshot: DeviceStateSnapshot,
        raw: Vec<u8>,
    },
    QueryReply {
        reply: QueryResponse,
        raw: Vec<u8>,
    },
    Auxiliary {
        bytes: Vec<u8>,
        raw: Vec<u8>,
    },
    Notification {
        notification: DeviceNotification,
        raw: Vec<u8>,
    },
}

impl Frame {
    pub fn parse(bytes: &[u8]) -> Result<Self, ProtocolError> {
        Self::parse_owned(bytes.to_vec())
    }

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
            0x73 => Ok(Self::Snapshot {
                snapshot: parse_snapshot73(&bytes)?,
                raw: bytes,
            }),
            0x75 => Ok(Self::QueryReply {
                reply: QueryResponse {
                    query_id: bytes[0x08],
                    sub_id: bytes[0x0c],
                    body: bytes[0x10..].to_vec(),
                },
                raw: bytes,
            }),
            0x83 => Ok(Self::Auxiliary {
                bytes: bytes[0x10..].to_vec(),
                raw: bytes,
            }),
            other => Err(ProtocolError::UnsupportedFrame(other)),
        }
    }

    pub fn as_snapshot(&self) -> Option<&DeviceStateSnapshot> {
        match self {
            Self::Snapshot { snapshot, .. } => Some(snapshot),
            _ => None,
        }
    }

    pub fn as_query_reply(&self) -> Option<&QueryResponse> {
        match self {
            Self::QueryReply { reply, .. } => Some(reply),
            _ => None,
        }
    }

    pub fn raw_bytes(&self) -> &[u8] {
        match self {
            Self::Snapshot { raw, .. } => raw,
            Self::QueryReply { raw, .. } => raw,
            Self::Auxiliary { raw, .. } => raw,
            Self::Notification { raw, .. } => raw,
        }
    }

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

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSnapshot {
    Snapshot(DeviceStateSnapshot),
    Auxiliary(Vec<u8>),
    QueryReply(QueryResponse),
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
    if bytes.len() < 0x10 + 0xe6 {
        return Err(ProtocolError::FrameTooShort(bytes.len()));
    }

    let payload = &bytes[0x10..];
    Ok(DeviceStateSnapshot {
        sample_rate: SampleRate::from_code(payload[0x02]),
        clock_source: ClockSource::from_code(payload[0x03]),
        sample_rate_hz: u32::from_be_bytes(payload[0x04..0x08].try_into().expect("sample rate")),
        status_flags: [payload[0x00], payload[0x01]],
        front_panel_bytes: [payload[0x08], payload[0x09], payload[0x0a]],
        outputs: [
            OutputState::new(
                OutputTarget::Monitor,
                payload[0x0c],
                OutputMode::from_code(payload[0x0d]),
            ),
            OutputState::new(
                OutputTarget::Hp1,
                payload[0x0e],
                OutputMode::from_code(payload[0x0f]),
            ),
            OutputState::new(
                OutputTarget::Hp2,
                payload[0x10],
                OutputMode::from_code(payload[0x11]),
            ),
        ],
        dsp_cluster: [payload[0x18], payload[0x19], payload[0x1a], payload[0x1b]],
        preamp: PreampState::from_cluster([
            payload[0x18],
            payload[0x19],
            payload[0x1a],
            payload[0x1b],
        ]),
        surface: Surface::from_code(payload[0x6a]),
        mixer_decode: decode_passive_mixer_state(payload),
        late_shadow: [
            payload[0xda],
            payload[0xdb],
            payload[0xdc],
            payload[0xdd],
            payload[0xde],
            payload[0xdf],
            payload[0xe0],
            payload[0xe1],
            payload[0xe2],
            payload[0xe3],
            payload[0xe4],
            payload[0xe5],
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixer::MixerSurface;
    use crate::types::{PanState, PreampInputState, PreampMode, PreampState};

    fn empty_snapshot_frame() -> Vec<u8> {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame
    }

    #[test]
    fn decodes_snapshot_global_fields_and_outputs() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x20_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x00] = 0x08;
        payload[0x02] = 0x02;
        payload[0x03] = 0x01;
        payload[0x04..0x08].copy_from_slice(&48_000_u32.to_be_bytes());
        payload[0x0c] = 0x40;
        payload[0x0d] = 0x02;
        payload[0x0e] = 0x30;
        payload[0x0f] = 0x01;
        payload[0x10] = 0x20;
        payload[0x11] = 0x00;
        payload[0x6a] = 0x0c;
        payload[0x18..0x1c].copy_from_slice(&[0x2f, 0x34, 0x50, 0x10]);

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
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x8e] = 0x5a;
        frame[0x10 + 0xcf] = 0x4c;
        frame[0x10 + 0xde] = 0x11;

        let parsed = Frame::parse(&frame).expect("frame should parse");
        assert_eq!(parsed.raw_bytes()[0x10 + 0x8e], 0x5a);
        assert_eq!(parsed.raw_bytes()[0x10 + 0xcf], 0x4c);
        assert_eq!(parsed.raw_bytes()[0x10 + 0xde], 0x11);
    }

    #[test]
    fn snapshot_frame_parse_owned_preserves_raw_bytes() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        frame[0x10 + 0x8e] = 0x5a;
        frame[0x10 + 0xcf] = 0x4c;
        frame[0x10 + 0xde] = 0x11;

        let parsed = Frame::parse_owned(frame).expect("frame should parse");
        assert_eq!(parsed.raw_bytes()[0x10 + 0x8e], 0x5a);
        assert_eq!(parsed.raw_bytes()[0x10 + 0xcf], 0x4c);
        assert_eq!(parsed.raw_bytes()[0x10 + 0xde], 0x11);
    }

    #[test]
    fn does_not_decode_observed_preamp1_meter_from_untrusted_lane() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
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
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xce] = 0x54;
        payload[0xcf] = 0x4e;

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
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xce] = 0x38;

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
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xcf] = 0x49;

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
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xce] = 0x2a;
        payload[0x8e] = 0x12;

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
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xcf] = 0x22;
        payload[0x8e] = 0x12;

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
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0x8e] = 0x12;

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
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
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
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0x8f] = 0x51;
        payload[0xcf] = 0x51;
        payload[0xda] = 0x51;
        payload[0xdb] = 0x51;
        payload[0xdc] = 0x51;
        payload[0xdd] = 0x51;
        payload[0xde] = 0x4e;
        payload[0xdf] = 0x4e;

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
        frame[0..4].copy_from_slice(&0x73_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x140_u32.to_le_bytes());
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0x8f] = 0x51;
        payload[0xcf] = 0x51;
        payload[0xda] = 0x4e;
        payload[0xdb] = 0x4e;
        payload[0xdc] = 0x4e;
        payload[0xdd] = 0x4e;
        payload[0xde] = 0x4e;
        payload[0xdf] = 0x4e;

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
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0f;
        payload[0xda] = 0x0a;
        payload[0xdb] = 0x05;
        payload[0xdc] = 0x0a;
        payload[0xdd] = 0x05;
        payload[0xe0] = 0x60;
        payload[0xe1] = 0x60;

        assert_eq!(
            experimental_surface_pair_lanes(&frame[0x10..]),
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
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0c;
        payload[0xde] = 0x00;
        payload[0xdf] = 0x06;
        payload[0xe0] = 0x60;
        payload[0xe1] = 0x60;

        assert_eq!(
            experimental_surface_pair_lanes(&frame[0x10..]),
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
        let payload = &mut frame[0x10..];
        payload[0x6a] = 0x0c;
        payload[0xde] = 0x60;
        payload[0xdf] = 0x60;

        assert_eq!(
            experimental_surface_pair_lanes(&frame[0x10..]),
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
        let mixer = MixerSurface::from_surface(Surface::from_code(*payload.get(0x6a)?));
        match mixer {
            MixerSurface::Mix1 => {
                let lane_a = *payload.get(0xda)?;
                let lane_b = *payload.get(0xdb)?;
                Some(ExperimentalSurfacePairLanes {
                    mixer,
                    lane_a,
                    lane_b,
                    mirrored: payload.get(0xdc) == Some(&lane_a)
                        && payload.get(0xdd) == Some(&lane_b),
                })
            }
            MixerSurface::Mix2 => Some(ExperimentalSurfacePairLanes {
                mixer,
                lane_a: *payload.get(0xde)?,
                lane_b: *payload.get(0xdf)?,
                mirrored: false,
            }),
        }
    }
}
