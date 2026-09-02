//! Query request/response types and startup query sequences.

use crate::mixer::{MixerAssignment, MixerSurface};
use crate::types::PanState;

/// Categories of query replies observed during device startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupQueryKind {
    /// Device metadata (product name, serial, hardware version).
    Metadata,
    /// Capability and default configuration block.
    CapabilityDefaults,
    /// Status or capability value.
    StatusValue,
    /// An unrecognized query reply ID.
    Unknown(u8),
}

impl StartupQueryKind {
    /// Classifies a query reply by its `query_id` byte.
    pub fn from_query_id(query_id: u8) -> Self {
        match query_id {
            0x01 => Self::Metadata,
            0x00 => Self::CapabilityDefaults,
            0x11 => Self::StatusValue,
            value => Self::Unknown(value),
        }
    }

    /// Returns a human-readable label for this query kind.
    pub fn label(self) -> &'static str {
        match self {
            Self::Metadata => "Metadata",
            Self::CapabilityDefaults => "Capability/default block",
            Self::StatusValue => "Status/capability value",
            Self::Unknown(_) => "Unknown query reply",
        }
    }
}

/// A query request sent to the device to retrieve state or capability information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryRequest {
    /// Primary query identifier.
    pub query_id: u8,
    /// Sub-query identifier for parameterized queries.
    pub sub_id: u8,
}

impl QueryRequest {
    /// Creates a new `QueryRequest`.
    pub const fn new(query_id: u8, sub_id: u8) -> Self {
        Self { query_id, sub_id }
    }
}

/// Device identity information extracted from a metadata query reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceMetadata {
    /// Product name (e.g. `"Zen Go Synergy Core"`).
    pub product_name: String,
    /// Device serial number.
    pub serial: String,
    /// Hardware/firmware version (e.g. `"6.6"`).
    pub hardware_version: String,
}

/// Raw response body from a device query.
///
/// Use the decoding methods on this type (e.g. [`metadata`](Self::metadata),
/// [`assignment_readback`](Self::assignment_readback)) to interpret the body
/// for specific query types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryResponse {
    /// Primary query identifier matching the request.
    pub query_id: u8,
    /// Sub-query identifier matching the request.
    pub sub_id: u8,
    /// Raw response payload bytes.
    pub body: Vec<u8>,
}

/// Complete state of a mixer strip as returned by a query (0x18) readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueriedMixerStripState {
    /// Raw level byte (0x00 = unity, higher = more attenuation).
    pub level: u8,
    /// Current pan position.
    pub pan: PanState,
    /// Whether the strip is muted.
    pub muted: bool,
    /// Whether the strip is soloed.
    pub soloed: bool,
}

/// Full readback of both mixer surfaces from a query response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueriedMixerSurfaceReadback {
    /// Strip states indexed as `surfaces[mixer_index][channel_index]`.
    pub surfaces: [[QueriedMixerStripState; 16]; 2],
}

/// Pan position category for startup state readbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPanCategory {
    /// Pan is centered (raw value 0x20).
    Center,
    /// Pan is left of center (raw value < 0x20).
    Left,
    /// Pan is right of center (raw value > 0x20).
    Right,
}

impl StartupPanCategory {
    /// Returns a single-character label (`"C"`, `"L"`, or `"R"`).
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Center => "C",
            Self::Left => "L",
            Self::Right => "R",
        }
    }
}

/// Mixer strip state as returned by startup pan readbacks (query 0x04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupMixerStripState {
    /// Raw level byte.
    pub level: u8,
    /// Current pan position.
    pub pan: PanState,
    /// Whether the strip is muted.
    pub muted: bool,
    /// Whether the strip is soloed.
    pub soloed: bool,
}

impl QueryResponse {
    /// Returns the classified kind of this query response.
    pub fn kind(&self) -> StartupQueryKind {
        StartupQueryKind::from_query_id(self.query_id)
    }

    /// Decodes device metadata from a metadata query reply (`query_id` 0x01).
    ///
    /// Returns `None` if this is not a metadata reply or the body cannot be parsed.
    pub fn metadata(&self) -> Option<DeviceMetadata> {
        if self.query_id != 0x01 {
            return None;
        }

        let parts: Vec<String> = self
            .body
            .split(|byte| *byte == 0)
            .filter(|chunk| !chunk.is_empty())
            .map(|chunk| String::from_utf8_lossy(chunk).trim().to_string())
            .filter(|value| !value.is_empty())
            .collect();

        if parts.len() < 3 {
            return None;
        }

        Some(DeviceMetadata {
            product_name: parts[0].clone(),
            serial: parts[1].clone(),
            hardware_version: parts[2].clone(),
        })
    }

    /// Returns a human-readable summary of this query response.
    ///
    /// Attempts to decode the body into a structured representation
    /// (startup tables, pan categories, selector bitmaps, etc.) and
    /// falls back to a hex dump for unrecognized formats.
    pub fn summary_label(&self) -> String {
        self.summary_indexed_code_table()
            .or_else(|| self.summary_quad_state())
            .or_else(|| self.summary_pan_categories())
            .or_else(|| self.summary_selector_bitmap())
            .or_else(|| self.summary_pair_bank())
            .unwrap_or_else(|| self.summary_fallback())
    }

    fn summary_indexed_code_table(&self) -> Option<String> {
        let entries = self.startup_indexed_code_table()?;
        let preview = entries
            .iter()
            .take(10)
            .map(|(index, code)| format!("{index:02x}:{code:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        Some(format!("Startup indexed code table [{preview}]"))
    }

    fn summary_quad_state(&self) -> Option<String> {
        let bytes = self.startup_quad_state()?;
        Some(format!(
            "Startup quad state [{:02x} {:02x} {:02x} {:02x}]",
            bytes[0], bytes[1], bytes[2], bytes[3]
        ))
    }

    fn summary_pan_categories(&self) -> Option<String> {
        let (surface, categories) = self.startup_pan_category_readback()?;
        let preview = categories
            .iter()
            .map(|c| c.map(|v| v.short_label()).unwrap_or("?"))
            .collect::<Vec<_>>()
            .join(" ");
        Some(format!("Startup {surface:?} pan categories [{preview}]"))
    }

    fn summary_selector_bitmap(&self) -> Option<String> {
        let bitmap = self.selector_bitmap()?;
        let asserted = bitmap
            .iter()
            .enumerate()
            .filter_map(|(i, enabled)| enabled.then_some(format!("{i:02x}")))
            .collect::<Vec<_>>();
        Some(format!(
            "Selector bitmap: {} asserted [{}]",
            asserted.len(),
            asserted.join(" ")
        ))
    }

    fn summary_pair_bank(&self) -> Option<String> {
        let pairs = self.selector_pair_bank()?;
        let preview = pairs
            .iter()
            .take(8)
            .map(|(left, right)| format!("{left:02x}/{right:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        Some(format!(
            "Selector pair bank 0x{:02x}: {} pairs [{preview}]",
            self.sub_id,
            pairs.len(),
        ))
    }

    fn summary_fallback(&self) -> String {
        match self.kind() {
            StartupQueryKind::Metadata => self
                .metadata()
                .map(|m| {
                    format!(
                        "{}: {} (hw {}, serial {})",
                        self.kind().label(),
                        m.product_name,
                        m.hardware_version,
                        m.serial
                    )
                })
                .unwrap_or_else(|| format!("{}: undecoded", self.kind().label())),
            StartupQueryKind::CapabilityDefaults | StartupQueryKind::StatusValue => format!(
                "{}: {} bytes [{}]",
                self.kind().label(),
                self.body.len(),
                self.body
                    .iter()
                    .take(8)
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            StartupQueryKind::Unknown(id) => format!(
                "{} 0x{id:02x}/0x{:02x}: {} bytes",
                self.kind().label(),
                self.sub_id,
                self.body.len()
            ),
        }
    }

    /// Decodes mixer strip assignments from a query reply (`query_id` 0x03).
    ///
    /// Bank 0x05 covers strips 1–4 (early AFX), banks 0x06–0x09 cover strips 5–16.
    pub fn assignment_readback(&self) -> Option<[Option<MixerAssignment>; 16]> {
        if self.query_id != 0x03 || self.body.is_empty() || self.body[0] != self.sub_id {
            return None;
        }

        let mut assignments = [None; 16];
        match self.sub_id {
            0x05 if self.body.len() >= 9 => {
                for (index, chunk) in self.body[1..9].as_chunks::<2>().0.iter().enumerate() {
                    assignments[index] =
                        MixerAssignment::from_ordinary_strip_bytes([chunk[0], chunk[1]]);
                }
                Some(assignments)
            }
            0x06..=0x09 if self.body.len() >= 33 => {
                for (index, chunk) in self.body[9..33].as_chunks::<2>().0.iter().enumerate() {
                    assignments[index + 4] =
                        MixerAssignment::from_ordinary_strip_bytes([chunk[0], chunk[1]]);
                }
                Some(assignments)
            }
            _ => None,
        }
    }

    /// Decodes a selector bitmap from query 0x0b/0x03.
    ///
    /// Returns 24 boolean flags indicating which stereo link selectors are asserted.
    pub fn selector_bitmap(&self) -> Option<[bool; 24]> {
        if self.query_id != 0x0b || self.sub_id != 0x03 || self.body.len() < 24 {
            return None;
        }

        let mut selectors = [false; 24];
        for (index, value) in self.body.iter().take(24).copied().enumerate() {
            selectors[index] = value != 0;
        }
        Some(selectors)
    }

    /// Converts a selector bitmap into per-channel link state for both mixer surfaces.
    ///
    /// Each entry in the returned arrays indicates whether that channel is linked
    /// to its stereo pair.
    pub fn startup_link_readback_from_bitmap(
        &self,
    ) -> Option<[(MixerSurface, [Option<bool>; 16]); 2]> {
        let bitmap = self.selector_bitmap()?;
        let mut mix1 = [None; 16];
        let mut mix2 = [None; 16];

        for (bit, pair_start) in [
            (0_usize, 0_usize),
            (1, 2),
            (2, 4),
            (3, 6),
            (4, 8),
            (5, 10),
            (6, 12),
            (7, 14),
        ] {
            mix1[pair_start] = Some(bitmap[bit]);
            mix1[pair_start + 1] = Some(bitmap[bit]);
        }
        for (bit, pair_start) in [
            (16_usize, 0_usize),
            (17, 2),
            (18, 4),
            (19, 6),
            (20, 8),
            (21, 10),
            (22, 12),
            (23, 14),
        ] {
            mix2[pair_start] = Some(bitmap[bit]);
            mix2[pair_start + 1] = Some(bitmap[bit]);
        }

        Some([(MixerSurface::Mix1, mix1), (MixerSurface::Mix2, mix2)])
    }

    /// Decodes a selector pair bank from query 0x04.
    ///
    /// Returns 32 pairs of (left, right) pan codes.
    pub fn selector_pair_bank(&self) -> Option<Vec<(u8, u8)>> {
        if self.query_id != 0x04 || self.body.len() < 64 {
            return None;
        }

        Some(
            self.body[..64]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|chunk| (chunk[0], chunk[1]))
                .collect(),
        )
    }

    /// Decodes full mixer strip state (level, pan, mute, solo) from a startup pan query (0x04).
    ///
    /// The `sub_id` determines which surface: 0x00 for Mix1, 0x01 for Mix2.
    pub fn startup_pan_state_readback(
        &self,
    ) -> Option<(MixerSurface, [Option<StartupMixerStripState>; 16])> {
        let surface = match self.sub_id {
            0x00 => MixerSurface::Mix1,
            0x01 => MixerSurface::Mix2,
            _ => return None,
        };
        if self.query_id != 0x04 || self.body.len() < 34 {
            return None;
        }

        let mut states = [None; 16];
        for (index, code) in self.body.iter().skip(3).step_by(2).take(16).enumerate() {
            let level = self.body.get(2 + index * 2).copied().unwrap_or(0);
            states[index] = Some(StartupMixerStripState {
                level,
                pan: PanState::from_state_code(*code),
                muted: PanState::state_code_is_muted(*code),
                soloed: PanState::state_code_is_soloed(*code),
            });
        }

        Some((surface, states))
    }

    /// Converts startup pan state readback into simplified pan categories (Left/Center/Right).
    pub fn startup_pan_category_readback(
        &self,
    ) -> Option<(MixerSurface, [Option<StartupPanCategory>; 16])> {
        let (surface, states) = self.startup_pan_state_readback()?;
        let mut categories = [None; 16];
        for (index, state) in states.into_iter().enumerate() {
            let Some(state) = state else {
                continue;
            };
            categories[index] = Some(match state.pan.raw() {
                0x20 => StartupPanCategory::Center,
                0x02 => StartupPanCategory::Left,
                0x3e => StartupPanCategory::Right,
                raw if raw < 0x20 => StartupPanCategory::Left,
                _ => StartupPanCategory::Right,
            });
        }

        Some((surface, categories))
    }

    /// Decodes an indexed code table from query 0x15/0x00.
    ///
    /// Returns 32 pairs of (index, code) bytes.
    pub fn startup_indexed_code_table(&self) -> Option<Vec<(u8, u8)>> {
        if self.query_id != 0x15 || self.sub_id != 0x00 || self.body.len() < 64 {
            return None;
        }

        Some(
            self.body[..64]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|chunk| (chunk[0], chunk[1]))
                .collect(),
        )
    }

    /// Decodes a 4-byte state block from query 0x17/0x00.
    pub fn startup_quad_state(&self) -> Option<[u8; 4]> {
        if self.query_id != 0x17 || self.sub_id != 0x00 || self.body.len() < 4 {
            return None;
        }

        self.body[..4].try_into().ok()
    }

    /// Decodes full dual-surface mixer strip state from query 0x18/0x00.
    ///
    /// Returns level, pan, mute, and solo for all 32 strips (16 per surface).
    pub fn mixer_strip_readback(&self) -> Option<QueriedMixerSurfaceReadback> {
        if self.query_id != 0x18 || self.sub_id != 0x00 || self.body.len() < 64 {
            return None;
        }

        let mut surfaces = [[QueriedMixerStripState::default(); 16]; 2];
        for (index, chunk) in self.body[..64].as_chunks::<2>().0.iter().enumerate() {
            let surface = index / 16;
            let channel = index % 16;
            surfaces[surface][channel] = QueriedMixerStripState {
                level: chunk[0].min(0x5a),
                pan: PanState::from_state_code(chunk[1]),
                muted: PanState::state_code_is_muted(chunk[1]),
                soloed: PanState::state_code_is_soloed(chunk[1]),
            };
        }

        Some(QueriedMixerSurfaceReadback { surfaces })
    }
}

/// Returns the sequence of query requests sent during control panel startup.
///
/// The device responds to each query with a [`QueryResponse`] that can be
/// decoded using the methods on that type.
pub fn control_panel_startup_queries() -> &'static [QueryRequest] {
    const QUERIES: [QueryRequest; 47] = [
        QueryRequest::new(0x01, 0x00),
        QueryRequest::new(0x11, 0x00),
        QueryRequest::new(0x0a, 0x00),
        QueryRequest::new(0x17, 0x00),
        QueryRequest::new(0x18, 0x00),
        QueryRequest::new(0x11, 0x01),
        QueryRequest::new(0x03, 0x00),
        QueryRequest::new(0x03, 0x01),
        QueryRequest::new(0x03, 0x02),
        QueryRequest::new(0x03, 0x03),
        QueryRequest::new(0x03, 0x04),
        QueryRequest::new(0x03, 0x05),
        QueryRequest::new(0x03, 0x06),
        QueryRequest::new(0x03, 0x07),
        QueryRequest::new(0x03, 0x08),
        QueryRequest::new(0x03, 0x09),
        QueryRequest::new(0x0b, 0x00),
        QueryRequest::new(0x16, 0x00),
        QueryRequest::new(0x0a, 0x00),
        QueryRequest::new(0x04, 0x00),
        QueryRequest::new(0x0b, 0x03),
        QueryRequest::new(0x04, 0x01),
        QueryRequest::new(0x0b, 0x03),
        QueryRequest::new(0x04, 0x02),
        QueryRequest::new(0x0b, 0x03),
        QueryRequest::new(0x04, 0x03),
        QueryRequest::new(0x0b, 0x03),
        QueryRequest::new(0x15, 0x00),
        QueryRequest::new(0x19, 0x00),
        QueryRequest::new(0x19, 0x01),
        QueryRequest::new(0x07, 0x27),
        QueryRequest::new(0x07, 0x2c),
        QueryRequest::new(0x07, 0x09),
        QueryRequest::new(0x07, 0x14),
        QueryRequest::new(0x07, 0x4c),
        QueryRequest::new(0x19, 0x02),
        QueryRequest::new(0x19, 0x03),
        QueryRequest::new(0x19, 0x04),
        QueryRequest::new(0x19, 0x05),
        QueryRequest::new(0x19, 0x06),
        QueryRequest::new(0x19, 0x07),
        QueryRequest::new(0x19, 0x08),
        QueryRequest::new(0x19, 0x09),
        QueryRequest::new(0x19, 0x0a),
        QueryRequest::new(0x19, 0x0b),
        QueryRequest::new(0x0b, 0x04),
        QueryRequest::new(0x12, 0x00),
    ];
    &QUERIES
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PanState;

    #[test]
    fn parses_metadata_query_reply() {
        let mut frame = vec![0_u8; 320];
        frame[0..4].copy_from_slice(&0x75_u32.to_le_bytes());
        frame[4..8].copy_from_slice(&0x30_u32.to_le_bytes());
        frame[0x08] = 0x01;
        let body = [
            b'Z', b'e', b'n', b' ', b'G', b'o', b' ', b'S', b'y', b'n', b'e', b'r', b'g', b'y',
            b' ', b'C', b'o', b'r', b'e', 0x00, 0x00, b'4', b'5', b'0', b'2', b'7', b'2', b'1',
            b'0', b'0', b'1', b'3', b'0', b'0', 0x00, 0x00, b'6', b'.', b'6', 0x00,
        ];
        frame[0x10..0x10 + body.len()].copy_from_slice(&body);

        let parsed = crate::frame::Frame::parse(&frame).expect("reply should parse");
        let metadata = parsed
            .as_query_reply()
            .and_then(|reply| reply.metadata())
            .expect("metadata");

        assert_eq!(metadata.product_name, "Zen Go Synergy Core");
        assert_eq!(metadata.serial, "4502721001300");
        assert_eq!(metadata.hardware_version, "6.6");
    }

    #[test]
    fn classifies_grounded_startup_query_reply_kinds() {
        assert_eq!(
            StartupQueryKind::from_query_id(0x01),
            StartupQueryKind::Metadata
        );
        assert_eq!(
            StartupQueryKind::from_query_id(0x00),
            StartupQueryKind::CapabilityDefaults
        );
        assert_eq!(
            StartupQueryKind::from_query_id(0x11),
            StartupQueryKind::StatusValue
        );
        assert_eq!(
            StartupQueryKind::from_query_id(0x7f),
            StartupQueryKind::Unknown(0x7f)
        );
    }

    #[test]
    fn summarizes_non_metadata_query_replies_without_over_decoding() {
        let defaults = QueryResponse {
            query_id: 0x00,
            sub_id: 0x00,
            body: vec![0xaa, 0xbb, 0xcc],
        };
        let status = QueryResponse {
            query_id: 0x11,
            sub_id: 0x00,
            body: vec![0x12],
        };

        assert_eq!(
            defaults.summary_label(),
            "Capability/default block: 3 bytes [aa bb cc]"
        );
        assert_eq!(
            status.summary_label(),
            "Status/capability value: 1 bytes [12]"
        );
    }

    #[test]
    fn decodes_selector_bitmap_from_0x75_0b_03() {
        let reply = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            ],
        };

        let bitmap = reply.selector_bitmap().expect("selector bitmap");
        let asserted = bitmap
            .iter()
            .enumerate()
            .filter_map(|(index, enabled)| enabled.then_some(index as u8))
            .collect::<Vec<_>>();
        assert_eq!(
            asserted,
            vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x11, 0x12, 0x13, 0x14]
        );
        assert_eq!(
            reply.summary_label(),
            "Selector bitmap: 9 asserted [00 01 02 03 04 11 12 13 14]"
        );
    }

    #[test]
    fn decodes_startup_visible_link_pairs_from_0x75_0b_03() {
        let mix1_linked = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            ],
        };
        let unlinked = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            ],
        };
        let mix2_linked = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            ],
        };
        let mix1_high = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00,
            ],
        };
        let mix2_high = QueryResponse {
            query_id: 0x0b,
            sub_id: 0x03,
            body: vec![
                0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01,
            ],
        };

        assert_eq!(
            mix1_linked
                .startup_link_readback_from_bitmap()
                .map(|maps| maps[0].1),
            Some([
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            ])
        );
        assert_eq!(
            unlinked
                .startup_link_readback_from_bitmap()
                .map(|maps| maps[0].1),
            Some([
                Some(false),
                Some(false),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            ])
        );
        assert_eq!(
            mix2_linked
                .startup_link_readback_from_bitmap()
                .map(|maps| maps[1].1),
            Some([
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            ])
        );
        assert_eq!(
            mix1_high
                .startup_link_readback_from_bitmap()
                .map(|maps| maps[0].1),
            Some([
                Some(false),
                Some(false),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
            ])
        );
        assert_eq!(
            mix2_high
                .startup_link_readback_from_bitmap()
                .map(|maps| maps[1].1),
            Some([
                Some(false),
                Some(false),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(true),
            ])
        );
    }

    #[test]
    fn summarizes_selector_pair_bank_conservatively() {
        let reply = QueryResponse {
            query_id: 0x04,
            sub_id: 0x01,
            body: vec![
                0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
            ],
        };

        let pairs = reply.selector_pair_bank().expect("selector pair bank");
        assert_eq!(pairs.len(), 32);
        assert_eq!(pairs[0], (0x00, 0x20));
        assert_eq!(pairs[1], (0x00, 0x60));
        assert_eq!(pairs[2], (0x00, 0x60));
        assert_eq!(pairs[3], (0x00, 0x02));
        assert_eq!(
            reply.summary_label(),
            "Startup Mix2 pan categories [C C L R L R L R L R L R L R L R]"
        );
    }

    #[test]
    fn decodes_startup_pan_categories_from_grounded_0x75_04_mix_banks() {
        let mix1 = QueryResponse {
            query_id: 0x04,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
            ],
        };
        let mix2 = QueryResponse {
            query_id: 0x04,
            sub_id: 0x01,
            body: vec![
                0x00, 0x20, 0x60, 0x20, 0x60, 0x20, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x60, 0x02, 0x60, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e,
                0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
                0x00, 0x3e, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x02,
            ],
        };

        let (surface1, pans1) = mix1.startup_pan_category_readback().expect("mix1 pans");
        let (surface2, pans2) = mix2.startup_pan_category_readback().expect("mix2 pans");
        assert_eq!(surface1, MixerSurface::Mix1);
        assert_eq!(surface2, MixerSurface::Mix2);
        assert_eq!(pans1[0], Some(StartupPanCategory::Center));
        assert_eq!(pans1[1], Some(StartupPanCategory::Center));
        assert_eq!(pans1[2], Some(StartupPanCategory::Left));
        assert_eq!(pans1[3], Some(StartupPanCategory::Right));
        assert_eq!(pans2[0], Some(StartupPanCategory::Center));
        assert_eq!(pans2[1], Some(StartupPanCategory::Center));
        assert_eq!(pans2[2], Some(StartupPanCategory::Left));
        assert_eq!(pans2[3], Some(StartupPanCategory::Right));
        assert_eq!(
            mix1.summary_label(),
            "Startup Mix1 pan categories [C C L R L R L R L R L R L R L R]"
        );
    }

    #[test]
    fn decodes_startup_pan_state_from_grounded_0x75_04_mix_banks() {
        let mix1_ch1 = QueryResponse {
            query_id: 0x04,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x00, 0x5e, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
            ],
        };
        let mix1_pair = QueryResponse {
            query_id: 0x04,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x02, 0x00, 0x3e, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
            ],
        };

        let (surface1, states1) = mix1_ch1.startup_pan_state_readback().expect("mix1 state");
        assert_eq!(surface1, MixerSurface::Mix1);
        assert_eq!(
            states1[0],
            Some(StartupMixerStripState {
                level: 0x00,
                pan: PanState::from_raw(0x1e),
                muted: true,
                soloed: false,
            })
        );
        assert_eq!(
            states1[1],
            Some(StartupMixerStripState {
                level: 0x00,
                pan: PanState::center(),
                muted: true,
                soloed: false,
            })
        );

        let (_, states2) = mix1_pair
            .startup_pan_state_readback()
            .expect("mix1 pair state");
        assert_eq!(
            states2[2],
            Some(StartupMixerStripState {
                level: 0x00,
                pan: PanState::left(),
                muted: false,
                soloed: false,
            })
        );
        assert_eq!(
            states2[3],
            Some(StartupMixerStripState {
                level: 0x00,
                pan: PanState::right(),
                muted: false,
                soloed: false,
            })
        );
    }

    #[test]
    fn startup_pan_state_readback_decodes_solo_flag() {
        let reply = QueryResponse {
            query_id: 0x04,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x00, 0xa0, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
            ],
        };

        let (_, states) = reply
            .startup_pan_state_readback()
            .expect("startup pan state");

        assert!(states[0].expect("ch1").soloed);
    }

    #[test]
    fn decodes_startup_level_from_grounded_0x75_04_mix_banks() {
        let mix1_ch1 = QueryResponse {
            query_id: 0x04,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x12, 0x5e, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
            ],
        };
        let mix2_pair = QueryResponse {
            query_id: 0x04,
            sub_id: 0x01,
            body: vec![
                0x00, 0x20, 0x00, 0x60, 0x00, 0x60, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x00, 0x20, 0x1e, 0x02, 0x1e, 0x3e, 0x00, 0x20,
                0x00, 0x20, 0x00, 0x20, 0x00, 0x20,
            ],
        };

        let (_, states1) = mix1_ch1
            .startup_pan_state_readback()
            .expect("mix1 level state");
        let (_, states2) = mix2_pair
            .startup_pan_state_readback()
            .expect("mix2 pair level state");
        assert_eq!(states1[0].map(|state| state.level), Some(0x12));
        assert_eq!(states2[10].map(|state| state.level), Some(0x1e));
        assert_eq!(states2[11].map(|state| state.level), Some(0x1e));
    }

    #[test]
    fn summarizes_startup_indexed_code_table() {
        let reply = QueryResponse {
            query_id: 0x15,
            sub_id: 0x00,
            body: vec![
                0x00, 0x00, 0x01, 0x10, 0x02, 0x10, 0x03, 0x04, 0x04, 0x04, 0x05, 0x10, 0x06, 0x10,
                0x07, 0x10, 0x08, 0x00, 0x09, 0x0f, 0x0a, 0x00, 0x0b, 0x10, 0x0c, 0x10, 0x0d, 0x10,
                0x0e, 0x00, 0x0f, 0x10, 0x10, 0x00, 0x11, 0x10, 0x12, 0x10, 0x13, 0x00, 0x14, 0x0f,
                0x15, 0x10, 0x16, 0x00, 0x17, 0x00, 0x18, 0x00, 0x19, 0x00, 0x1a, 0x10, 0x1b, 0x00,
                0x1c, 0x10, 0x1d, 0x10, 0x1e, 0x10, 0x1f, 0x10,
            ],
        };

        let entries = reply
            .startup_indexed_code_table()
            .expect("startup indexed code table");
        assert_eq!(entries.len(), 32);
        assert_eq!(entries[0], (0x00, 0x00));
        assert_eq!(entries[3], (0x03, 0x04));
        assert_eq!(entries[9], (0x09, 0x0f));
        assert_eq!(
            reply.summary_label(),
            "Startup indexed code table [00:00 01:10 02:10 03:04 04:04 05:10 06:10 07:10 08:00 09:0f]"
        );
    }

    #[test]
    fn summarizes_startup_quad_state() {
        let reply = QueryResponse {
            query_id: 0x17,
            sub_id: 0x00,
            body: vec![0x5a, 0x00, 0x60, 0x60],
        };

        assert_eq!(reply.startup_quad_state(), Some([0x5a, 0x00, 0x60, 0x60]));
        assert_eq!(reply.summary_label(), "Startup quad state [5a 00 60 60]");
    }

    #[test]
    fn decodes_assignment_readback_from_grounded_0x75_banks() {
        let early_bank = QueryResponse {
            query_id: 0x03,
            sub_id: 0x05,
            body: vec![0x05, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x01, 0x01],
        };
        let ordinary_bank = QueryResponse {
            query_id: 0x03,
            sub_id: 0x06,
            body: vec![
                0x06, 0x03, 0x00, 0x03, 0x01, 0x03, 0x02, 0x03, 0x03, 0x01, 0x02, 0x01, 0x03, 0x01,
                0x04, 0x01, 0x05, 0x01, 0x06, 0x01, 0x07, 0x08, 0x00, 0x08, 0x00, 0x08, 0x00, 0x08,
                0x00, 0x08, 0x00, 0x08, 0x00,
            ],
        };

        let early = early_bank
            .assignment_readback()
            .expect("early assignment bank");
        assert_eq!(early[0], Some(MixerAssignment::Preamp(1)));
        assert_eq!(early[1], Some(MixerAssignment::Preamp(2)));
        assert_eq!(early[2], Some(MixerAssignment::ComputerPlay(1)));
        assert_eq!(early[3], Some(MixerAssignment::ComputerPlay(2)));
        assert!(early[4..].iter().all(|slot| slot.is_none()));

        let ordinary = ordinary_bank
            .assignment_readback()
            .expect("ordinary assignment bank");
        assert!(ordinary[0..4].iter().all(|slot| slot.is_none()));
        assert_eq!(ordinary[4], Some(MixerAssignment::ComputerPlay(3)));
        assert_eq!(ordinary[5], Some(MixerAssignment::ComputerPlay(4)));
        assert_eq!(ordinary[6], Some(MixerAssignment::ComputerPlay(5)));
        assert_eq!(ordinary[7], Some(MixerAssignment::ComputerPlay(6)));
        assert_eq!(ordinary[8], Some(MixerAssignment::ComputerPlay(7)));
        assert_eq!(ordinary[9], Some(MixerAssignment::ComputerPlay(8)));
        assert!(ordinary[10..]
            .iter()
            .all(|slot| *slot == Some(MixerAssignment::Mute)));
    }

    #[test]
    fn decodes_mixer_strip_readback_from_0x75_18_00() {
        let reply = QueryResponse {
            query_id: 0x18,
            sub_id: 0x00,
            body: vec![
                0x00, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x02, 0x60, 0x3e, 0x2e, 0x02, 0x60, 0x3e,
                0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e,
                0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
                0x60, 0x3e, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x02,
            ],
        };

        assert_eq!(reply.body.len(), 64);
        let readback = reply.mixer_strip_readback().expect("strip state readback");
        let mix1 = &readback.surfaces[MixerSurface::Mix1.index()];
        let mix2 = &readback.surfaces[MixerSurface::Mix2.index()];
        assert_eq!(mix1[0].level, 0x00);
        assert_eq!(mix1[0].pan, PanState::center());
        assert!(!mix1[0].muted);
        assert_eq!(mix1[1].level, 0x5a);
        assert_eq!(mix1[1].pan, PanState::center());
        assert_eq!(mix1[2].level, 0x5a);
        assert_eq!(mix1[2].pan, PanState::center());
        assert_eq!(mix1[3].level, 0x5a);
        assert_eq!(mix1[3].pan, PanState::left());
        assert_eq!(mix1[5].level, 0x2e);
        assert_eq!(mix1[5].pan, PanState::left());
        assert_eq!(mix2[0].level, 0x5a);
        assert_eq!(mix2[0].pan, PanState::right());
        assert_eq!(mix2[1].level, 0x5a);
        assert_eq!(mix2[1].pan, PanState::left());
        assert!(mix1.iter().chain(mix2.iter()).all(|strip| !strip.soloed));
    }

    #[test]
    fn mixer_strip_readback_requires_full_dual_surface_payload() {
        let reply = QueryResponse {
            query_id: 0x18,
            sub_id: 0x00,
            body: vec![
                0x12, 0x3e, 0x60, 0x60, 0x60, 0x60, 0x60, 0x02, 0x60, 0x3e, 0x60, 0x20, 0x60, 0x20,
                0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20, 0x60, 0x20,
                0x60, 0x20, 0x60, 0x20,
            ],
        };

        assert!(reply.mixer_strip_readback().is_none());
    }
}
