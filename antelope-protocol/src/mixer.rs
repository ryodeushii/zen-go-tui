//! Mixer-related types: surfaces, strips, assignments, links, passive decode.

use crate::types::{PanState, Surface};
use crate::types::{
    OFFSET_METER_LANES_END, OFFSET_METER_LANES_START, OFFSET_MIX1_LANE_A, OFFSET_MIX1_LANE_B,
    OFFSET_MIX1_MIRROR_A, OFFSET_MIX1_MIRROR_B, OFFSET_MIX1_PRIMARY, OFFSET_MIX2_LANE_A,
    OFFSET_MIX2_LANE_B, OFFSET_MIX2_PRIMARY, OFFSET_PREAMP1_METER, OFFSET_PREAMP2_METER,
    OFFSET_SURFACE_SELECTOR, SURFACE_CODE_MONITOR_HP1,
};

/// Which mixer surface (mix bus) a strip belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerSurface {
    /// Mix 1 — associated with Monitor/HP1 surface.
    Mix1,
    /// Mix 2 — associated with HP2 surface.
    Mix2,
}

impl MixerSurface {
    /// Returns the zero-based surface index (0 for Mix1, 1 for Mix2).
    pub fn index(self) -> usize {
        match self {
            Self::Mix1 => 0,
            Self::Mix2 => 1,
        }
    }

    /// Returns the raw protocol code for this mixer surface.
    pub fn code(self) -> u8 {
        match self {
            Self::Mix1 => 0x00,
            Self::Mix2 => 0x01,
        }
    }

    /// Maps a front-panel [`Surface`] to the corresponding mixer surface.
    pub fn from_surface(surface: Surface) -> Self {
        match surface {
            Surface::Hp2 => Self::Mix2,
            Surface::MonitorHp1 | Surface::Unknown(_) => Self::Mix1,
        }
    }
}

/// Source signal assigned to a mixer strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerAssignment {
    /// Preamp input channel (1 or 2).
    Preamp(u8),
    /// Computer playback channel (1–8).
    ComputerPlay(u8),
    /// S/PDIF digital input channel (1 or 2).
    SpdifIn(u8),
    /// No signal — strip is muted.
    Mute,
    /// Internal oscillator channel (1 or 2).
    Oscillator(u8),
    /// Emulated microphone input (1 or 2).
    EmuMic(u8),
}

/// Categorizes how a mixer strip interacts with the AFX (effects) pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerStripKind {
    /// Strips 1–4: adjacent to early AFX stages, use a single assignment bank.
    EarlyAfxAdjacent,
    /// Strips 5–16: standard behavior, use multiple assignment banks.
    Ordinary,
}

/// A single mixer strip (channel) on the device.
///
/// Channels 1–4 are [`EarlyAfxAdjacent`](MixerStripKind::EarlyAfxAdjacent);
/// channels 5–16 are [`Ordinary`](MixerStripKind::Ordinary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerStrip {
    /// 1-based channel number.
    pub channel: u8,
    /// Whether this strip is early-AFX-adjacent or ordinary.
    pub kind: MixerStripKind,
}

impl MixerStrip {
    /// Creates a `MixerStrip` for the given channel (1–16), or `None` if out of range.
    pub fn new(channel: u8) -> Option<Self> {
        if !(1..=16).contains(&channel) {
            return None;
        }

        Some(Self {
            channel,
            kind: if channel <= 4 {
                MixerStripKind::EarlyAfxAdjacent
            } else {
                MixerStripKind::Ordinary
            },
        })
    }

    /// Creates a `MixerStrip` only if the channel is an ordinary strip (5–16).
    pub fn ordinary(channel: u8) -> Option<Self> {
        let strip = Self::new(channel)?;
        matches!(strip.kind, MixerStripKind::Ordinary).then_some(strip)
    }

    /// Returns the zero-based index into the assignment bank for this strip.
    pub fn assignment_entry_index(self) -> usize {
        (self.channel - 1) as usize
    }

    /// Returns the assignment bank IDs that must be written to configure this strip.
    pub fn assignment_write_banks(self) -> &'static [u8] {
        match self.kind {
            MixerStripKind::EarlyAfxAdjacent => &[0x05],
            MixerStripKind::Ordinary if self.channel <= 8 => &[0x03, 0x06, 0x07, 0x08, 0x09],
            MixerStripKind::Ordinary => &[0x06, 0x07, 0x08, 0x09],
        }
    }

    /// Returns whether the given channel has a grounded (valid) assignment mapping.
    pub fn assignment_write_is_grounded(channel: u8) -> bool {
        Self::new(channel).is_some()
    }
}

/// A stereo link pair on a specific mixer surface.
///
/// Each pair covers two consecutive channels (e.g. 1/2, 3/4, …, 15/16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerLinkTarget {
    /// Which mixer surface this link belongs to.
    pub mixer: MixerSurface,
    /// Left channel number (1-based).
    pub left_channel: u8,
    /// Right channel number (1-based).
    pub right_channel: u8,
    /// Raw selector byte used in the protocol.
    pub selector: u8,
}

impl MixerLinkTarget {
    /// Looks up a link pair by its selector code on the given mixer surface.
    ///
    /// Mix1 uses selectors 0x00–0x07, Mix2 uses 0x10–0x17.
    /// The lower 4 bits of the selector give the pair index (0–7),
    /// which maps to channels `pair*2+1` and `pair*2+2`.
    pub fn from_selector(mixer: MixerSurface, selector: u8) -> Option<Self> {
        let (_mixer_base, selector_range) = match mixer {
            MixerSurface::Mix1 => (0x00_u8, 0x00..=0x07),
            MixerSurface::Mix2 => (0x10_u8, 0x10..=0x17),
        };
        if !selector_range.contains(&selector) {
            return None;
        }
        let pair_index = selector & 0x0F;
        let left_channel = pair_index * 2 + 1;
        let right_channel = left_channel + 1;
        Some(Self {
            mixer,
            left_channel,
            right_channel,
            selector,
        })
    }

    /// Looks up a link pair by channel number on the given mixer surface.
    ///
    /// Returns the pair that contains the given channel (as left or right).
    pub fn from_channel(mixer: MixerSurface, channel: u8) -> Option<Self> {
        if !(1..=16).contains(&channel) {
            return None;
        }
        let pair_index = (channel - 1) / 2;
        let selector = match mixer {
            MixerSurface::Mix1 => pair_index,
            MixerSurface::Mix2 => 0x10 | pair_index,
        };
        let left_channel = pair_index * 2 + 1;
        let right_channel = left_channel + 1;
        Some(Self {
            mixer,
            left_channel,
            right_channel,
            selector,
        })
    }

    /// Returns the companion bank ID for link pairs that require a secondary write.
    ///
    /// Only the first two pairs (channels 1/2 and 3/4) on each surface have a companion bank.
    pub fn companion_bank(self) -> Option<u8> {
        let pair_index = self.selector & 0x0F;
        if pair_index < 2 {
            Some(pair_index)
        } else {
            None
        }
    }
}

impl MixerAssignment {
    /// Returns the complete list of assignable source choices available in the UI.
    pub fn grounded_choices() -> &'static [MixerAssignment] {
        const CHOICES: [MixerAssignment; 17] = [
            MixerAssignment::Mute,
            MixerAssignment::Preamp(1),
            MixerAssignment::Preamp(2),
            MixerAssignment::ComputerPlay(1),
            MixerAssignment::ComputerPlay(2),
            MixerAssignment::ComputerPlay(3),
            MixerAssignment::ComputerPlay(4),
            MixerAssignment::ComputerPlay(5),
            MixerAssignment::ComputerPlay(6),
            MixerAssignment::ComputerPlay(7),
            MixerAssignment::ComputerPlay(8),
            MixerAssignment::SpdifIn(1),
            MixerAssignment::SpdifIn(2),
            MixerAssignment::Oscillator(1),
            MixerAssignment::Oscillator(2),
            MixerAssignment::EmuMic(1),
            MixerAssignment::EmuMic(2),
        ];
        &CHOICES
    }

    /// Decodes a `MixerAssignment` from the two-byte encoding used in ordinary strip readbacks.
    pub fn from_ordinary_strip_bytes(bytes: [u8; 2]) -> Option<Self> {
        match bytes {
            [0x00, 0x00] => Some(Self::Preamp(1)),
            [0x00, 0x01] => Some(Self::Preamp(2)),
            [0x01, 0x00..=0x07] => Some(Self::ComputerPlay(bytes[1] + 1)),
            [0x02, 0x00] => Some(Self::SpdifIn(1)),
            [0x02, 0x01] => Some(Self::SpdifIn(2)),
            [0x08, 0x00] => Some(Self::Mute),
            [0x09, 0x00] => Some(Self::Oscillator(1)),
            [0x09, 0x01] => Some(Self::Oscillator(2)),
            [0x0a, 0x00] => Some(Self::EmuMic(1)),
            [0x0a, 0x01] => Some(Self::EmuMic(2)),
            _ => None,
        }
    }

    /// Encodes this assignment into the two-byte format used for ordinary strips.
    ///
    /// # Panics
    ///
    /// Panics if the assignment is not a grounded (encodable) variant.
    pub fn ordinary_strip_bytes(self) -> [u8; 2] {
        match self {
            Self::Preamp(1) => [0x00, 0x00],
            Self::Preamp(2) => [0x00, 0x01],
            Self::ComputerPlay(channel @ 1..=8) => [0x01, channel - 1],
            Self::SpdifIn(1) => [0x02, 0x00],
            Self::SpdifIn(2) => [0x02, 0x01],
            Self::Mute => [0x08, 0x00],
            Self::Oscillator(1) => [0x09, 0x00],
            Self::Oscillator(2) => [0x09, 0x01],
            Self::EmuMic(1) => [0x0a, 0x00],
            Self::EmuMic(2) => [0x0a, 0x01],
            _ => panic!("unsupported grounded assignment variant"),
        }
    }

    /// Returns a human-readable label (e.g. `"Preamp 1"`, `"Computer Play 3"`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Preamp(1) => "Preamp 1",
            Self::Preamp(2) => "Preamp 2",
            Self::ComputerPlay(1) => "Computer Play 1",
            Self::ComputerPlay(2) => "Computer Play 2",
            Self::ComputerPlay(3) => "Computer Play 3",
            Self::ComputerPlay(4) => "Computer Play 4",
            Self::ComputerPlay(5) => "Computer Play 5",
            Self::ComputerPlay(6) => "Computer Play 6",
            Self::ComputerPlay(7) => "Computer Play 7",
            Self::ComputerPlay(8) => "Computer Play 8",
            Self::SpdifIn(1) => "SPDIF In 1",
            Self::SpdifIn(2) => "SPDIF In 2",
            Self::Mute => "Mute",
            Self::Oscillator(1) => "Oscillator 1",
            Self::Oscillator(2) => "Oscillator 2",
            Self::EmuMic(1) => "Emu Mic 1",
            Self::EmuMic(2) => "Emu Mic 2",
            _ => "Unknown",
        }
    }

    /// Returns a compact label for UI display (e.g. `"P1"`, `"C3"`, `"M"`).
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Preamp(1) => "P1",
            Self::Preamp(2) => "P2",
            Self::ComputerPlay(1) => "C1",
            Self::ComputerPlay(2) => "C2",
            Self::ComputerPlay(3) => "C3",
            Self::ComputerPlay(4) => "C4",
            Self::ComputerPlay(5) => "C5",
            Self::ComputerPlay(6) => "C6",
            Self::ComputerPlay(7) => "C7",
            Self::ComputerPlay(8) => "C8",
            Self::SpdifIn(1) => "S1",
            Self::SpdifIn(2) => "S2",
            Self::Mute => "M",
            Self::Oscillator(1) => "O1",
            Self::Oscillator(2) => "O2",
            Self::EmuMic(1) => "E1",
            Self::EmuMic(2) => "E2",
            _ => "?",
        }
    }
}

/// Complete state of a single mixer channel, as decoded from query responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerChannelState {
    /// 1-based channel number.
    pub channel: u8,
    /// Raw level byte, if known.
    pub level: Option<u8>,
    /// Raw meter reading byte, if observed.
    pub meter: Option<u8>,
    /// Whether the channel is muted, if known.
    pub muted: Option<bool>,
    /// Whether the channel is soloed, if known.
    pub soloed: Option<bool>,
    /// Current pan position.
    pub pan: PanState,
    /// Source signal assigned to this strip, if known.
    pub assignment: Option<MixerAssignment>,
    /// Whether this channel is linked to its stereo pair, if known.
    pub linked: Option<bool>,
}

impl MixerChannelState {
    /// Creates a `MixerChannelState` with all fields unknown except channel number and center pan.
    pub fn unknown(channel: u8) -> Self {
        Self {
            channel,
            level: None,
            meter: None,
            muted: None,
            soloed: None,
            pan: PanState::center(),
            assignment: None,
            linked: None,
        }
    }

    /// Creates a `MixerChannelState` with known values.
    pub fn known(
        channel: u8,
        level: Option<u8>,
        muted: Option<bool>,
        pan: PanState,
        assignment: Option<MixerAssignment>,
        linked: Option<bool>,
    ) -> Self {
        Self {
            channel,
            level,
            meter: None,
            muted,
            soloed: None,
            pan,
            assignment,
            linked,
        }
    }

    /// Returns the display-friendly dB value (0 to -90 dB), or `None` if level is unknown.
    pub fn display_db(self) -> Option<i16> {
        self.level.map(|raw| -(raw.min(0x5a) as i16))
    }

    /// Returns the gain as a normalized 0.0–1.0 ratio, or `None` if level is unknown.
    pub fn gain_ratio(self) -> Option<f64> {
        self.level
            .map(|raw| (1.0 - (raw.min(0x5a) as f64 / 90.0)).clamp(0.0, 1.0))
    }

    /// Returns the meter reading as a normalized 0.0–1.0 ratio, or `None` if no meter is set.
    pub fn meter_ratio(self) -> Option<f64> {
        self.meter.map(crate::types::meter_ratio)
    }

    /// Returns the meter reading as a display-friendly dB value, or `None` if no meter is set.
    pub fn meter_db(self) -> Option<i16> {
        self.meter.and_then(crate::types::meter_display_db)
    }
}

/// Partially resolved state for a mixer strip, decoded passively from snapshot frames.
///
/// Unlike [`MixerChannelState`], this is derived from observing snapshot payloads
/// rather than explicit query responses, so fields may be unresolved (`None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MixerPassiveStripState {
    /// Raw meter reading byte, if observed.
    pub meter: Option<u8>,
    /// Whether the strip is muted, if determinable from the snapshot.
    pub muted: Option<bool>,
    /// Pan position, if determinable from the snapshot.
    pub pan: Option<PanState>,
    /// Whether the strip is linked to its stereo pair, if determinable.
    pub linked: Option<bool>,
}

impl MixerPassiveStripState {
    /// Creates a `MixerPassiveStripState` with all fields unresolved.
    pub const fn unresolved() -> Self {
        Self {
            meter: None,
            muted: None,
            pan: None,
            linked: None,
        }
    }
}

/// Passive mixer state decoded from a snapshot frame payload.
///
/// Contains partially resolved strip states for both mixer surfaces,
/// plus any observed preamp meter readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixerPassiveDecode {
    /// Strip states indexed as `surfaces[mixer_index][channel_index]`.
    pub surfaces: [[MixerPassiveStripState; 16]; 2],
    /// Preamp 1 meter reading observed from the snapshot, if any.
    pub observed_preamp1_meter: Option<u8>,
    /// Preamp 2 meter reading observed from the snapshot, if any.
    pub observed_preamp2_meter: Option<u8>,
}

impl Default for MixerPassiveDecode {
    fn default() -> Self {
        Self {
            surfaces: [[MixerPassiveStripState::unresolved(); 16]; 2],
            observed_preamp1_meter: None,
            observed_preamp2_meter: None,
        }
    }
}

impl MixerPassiveDecode {
    /// Returns the passive strip state for the given mixer surface and channel (1–16).
    pub fn strip(&self, mixer: MixerSurface, channel: u8) -> Option<MixerPassiveStripState> {
        let index = channel.checked_sub(1)? as usize;
        self.surfaces
            .get(mixer.index())
            .and_then(|surface| surface.get(index))
            .copied()
    }
}

fn decode_strip_meter(payload: &[u8], channel: u8) -> Option<u8> {
    let meter_lanes = payload.get(OFFSET_METER_LANES_START..=OFFSET_METER_LANES_END)?;
    if meter_lanes.iter().all(|lane| *lane == 0x00) {
        return None;
    }

    meter_lanes
        .get(channel.checked_sub(1)? as usize)
        .copied()
        .filter(|raw| *raw <= 0x60)
}

fn decode_preamp_meter(payload: &[u8], offset: usize) -> Option<u8> {
    payload
        .get(offset)
        .copied()
        .filter(|raw| *raw != 0x00 && *raw <= 0x49)
}

fn decode_mute_from_group(
    payload: &[u8],
    primary_a: usize,
    primary_b: usize,
    shadow_a: usize,
    shadow_b: usize,
    shadow_c: usize,
    shadow_d: usize,
) -> Option<bool> {
    let values = [
        payload.get(primary_a).copied()?,
        payload.get(primary_b).copied()?,
        payload.get(shadow_a).copied()?,
        payload.get(shadow_b).copied()?,
        payload.get(shadow_c).copied()?,
        payload.get(shadow_d).copied()?,
    ];

    let all_51 = values.iter().all(|value| *value == 0x51);
    let all_active = values
        .iter()
        .all(|value| matches!(*value, 0x49 | 0x4b | 0x4c | 0x4e | 0x51));
    if all_51 {
        Some(true)
    } else if all_active {
        Some(false)
    } else {
        None
    }
}

fn decode_pan_from_group(
    payload: &[u8],
    primary_a: usize,
    primary_b: usize,
    shadow_a: usize,
    shadow_b: usize,
    tail_a: usize,
    tail_b: usize,
) -> Option<PanState> {
    let samples = [
        payload.get(primary_a).copied()?,
        payload.get(primary_b).copied()?,
        payload.get(shadow_a).copied()?,
        payload.get(shadow_b).copied()?,
        payload.get(tail_a).copied()?,
        payload.get(tail_b).copied()?,
    ];
    let value =
        (samples.iter().map(|&sample| sample as u16).sum::<u16>() / samples.len() as u16) as u8;
    let centered = match value {
        0x49..=0x4c => 0x20,
        0x4d..=0x4e => 0x1e,
        _ => return None,
    };
    Some(PanState::from_raw(centered))
}

fn decode_link_state(payload: &[u8]) -> Option<bool> {
    let head_a = payload.get(OFFSET_MIX1_PRIMARY).copied()?;
    let head_b = payload.get(OFFSET_MIX2_PRIMARY).copied()?;
    let tails = [
        payload.get(OFFSET_MIX1_LANE_A).copied()?,
        payload.get(OFFSET_MIX1_LANE_B).copied()?,
        payload.get(OFFSET_MIX1_MIRROR_A).copied()?,
        payload.get(OFFSET_MIX1_MIRROR_B).copied()?,
        payload.get(OFFSET_MIX2_LANE_A).copied()?,
        payload.get(OFFSET_MIX2_LANE_B).copied()?,
    ];
    let values = [
        head_a, head_b, tails[0], tails[1], tails[2], tails[3], tails[4], tails[5],
    ];

    if values.iter().all(|value| *value == 0x49)
        || (head_a == 0x51 && head_b == 0x51 && tails.iter().all(|value| *value == 0x4e))
    {
        Some(true)
    } else if head_a == 0x4e
        && head_b == 0x4e
        && tails.iter().all(|value| matches!(*value, 0x4c | 0x4e))
    {
        Some(false)
    } else {
        None
    }
}

/// Decodes passive mixer state from a snapshot frame payload.
///
/// Extracts meter readings, mute flags, pan positions, and link states
/// by correlating bytes across multiple regions of the payload.
pub fn decode_passive_mixer_state(payload: &[u8]) -> MixerPassiveDecode {
    let mut decode = MixerPassiveDecode::default();

    let shared_mute = decode_mute_from_group(
        payload,
        OFFSET_MIX1_PRIMARY,
        OFFSET_MIX2_PRIMARY,
        OFFSET_MIX1_LANE_A,
        OFFSET_MIX1_LANE_B,
        OFFSET_MIX1_MIRROR_A,
        OFFSET_MIX1_MIRROR_B,
    );
    let shared_pan = decode_pan_from_group(
        payload,
        OFFSET_MIX1_PRIMARY,
        OFFSET_MIX2_PRIMARY,
        OFFSET_MIX1_LANE_A,
        OFFSET_MIX1_MIRROR_B,
        OFFSET_MIX2_LANE_A,
        OFFSET_MIX2_LANE_B,
    );

    let active_mixer = MixerSurface::from_surface(Surface::from_code(
        *payload
            .get(OFFSET_SURFACE_SELECTOR)
            .unwrap_or(&SURFACE_CODE_MONITOR_HP1),
    ));
    decode.observed_preamp1_meter = decode_preamp_meter(payload, OFFSET_PREAMP1_METER);
    decode.observed_preamp2_meter = decode_preamp_meter(payload, OFFSET_PREAMP2_METER);
    for channel in 1..=16 {
        let meter = decode_strip_meter(payload, channel);
        for mixer in [MixerSurface::Mix1, MixerSurface::Mix2] {
            if let Some(slot) = decode.surfaces[mixer.index()].get_mut(channel as usize - 1) {
                slot.meter = meter;
            }
        }
    }
    if let Some(slot) = decode.surfaces[active_mixer.index()].get_mut(0) {
        slot.muted = shared_mute;
        slot.pan = shared_pan;
    }

    if let Some(linked) = decode_link_state(payload) {
        let targets: &[(MixerSurface, u8)] = match active_mixer {
            MixerSurface::Mix1 => &[(MixerSurface::Mix1, 1), (MixerSurface::Mix1, 2)],
            MixerSurface::Mix2 => &[(MixerSurface::Mix2, 1), (MixerSurface::Mix2, 2)],
        };
        for (mixer, channel) in targets {
            if let Some(slot) = decode.surfaces[mixer.index()].get_mut(*channel as usize - 1) {
                slot.linked = Some(linked);
            }
        }
    }

    decode
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PanState;

    #[test]
    fn ordinary_strip_index_map_stays_out_of_early_afx_range() {
        assert_eq!(MixerStrip::ordinary(4), None);
        assert_eq!(
            MixerStrip::ordinary(5).map(|strip| strip.assignment_entry_index()),
            Some(4)
        );
        assert_eq!(
            MixerStrip::ordinary(16).map(|strip| strip.assignment_entry_index()),
            Some(15)
        );
    }

    #[test]
    fn assignment_write_is_grounded_for_all_visible_strips() {
        for channel in 1..=16 {
            assert!(MixerStrip::assignment_write_is_grounded(channel));
        }
    }

    #[test]
    fn link_target_mapping_covers_full_visible_pair_map() {
        assert_eq!(
            MixerLinkTarget::from_selector(MixerSurface::Mix1, 0x03),
            Some(MixerLinkTarget {
                mixer: MixerSurface::Mix1,
                left_channel: 7,
                right_channel: 8,
                selector: 0x03,
            })
        );
        assert_eq!(
            MixerLinkTarget::from_selector(MixerSurface::Mix1, 0x01),
            Some(MixerLinkTarget {
                mixer: MixerSurface::Mix1,
                left_channel: 3,
                right_channel: 4,
                selector: 0x01,
            })
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 2).map(|target| target.selector),
            Some(0x10)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 4).map(|target| target.selector),
            Some(0x11)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 12).map(|target| target.selector),
            Some(0x05)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 7).map(|target| target.selector),
            Some(0x13)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 1)
                .and_then(|target| target.companion_bank()),
            Some(0x00)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix1, 7)
                .and_then(|target| target.companion_bank()),
            None
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 1)
                .and_then(|target| target.companion_bank()),
            Some(0x00)
        );
        assert_eq!(
            MixerLinkTarget::from_channel(MixerSurface::Mix2, 3)
                .and_then(|target| target.companion_bank()),
            Some(0x01)
        );
    }

    #[test]
    fn passive_link_state_decode_matches_grounded_mix1_and_mix2_signatures() {
        let mut payload = vec![0_u8; 0xe6];
        payload[0x8f] = 0x51;
        payload[0xcf] = 0x51;
        payload[0xda] = 0x4e;
        payload[0xdb] = 0x4e;
        payload[0xdc] = 0x4e;
        payload[0xdd] = 0x4e;
        payload[0xde] = 0x4e;
        payload[0xdf] = 0x4e;
        assert_eq!(decode_link_state(&payload), Some(true));

        payload[0x8f] = 0x4e;
        payload[0xcf] = 0x4e;
        payload[0xda] = 0x4c;
        payload[0xdb] = 0x4c;
        payload[0xdc] = 0x4c;
        payload[0xdd] = 0x4c;
        payload[0xde] = 0x4c;
        payload[0xdf] = 0x4c;
        assert_eq!(decode_link_state(&payload), Some(false));

        payload[0x8f] = 0x49;
        payload[0xcf] = 0x49;
        payload[0xda] = 0x49;
        payload[0xdb] = 0x49;
        payload[0xdc] = 0x49;
        payload[0xdd] = 0x49;
        payload[0xde] = 0x49;
        payload[0xdf] = 0x49;
        assert_eq!(decode_link_state(&payload), Some(true));
    }

    #[test]
    fn mixer_channel_state_tracks_assignment_pan_and_link() {
        let state = MixerChannelState::known(
            16,
            Some(0x22),
            Some(true),
            PanState::from_raw(0x3e),
            Some(MixerAssignment::ComputerPlay(8)),
            Some(false),
        );

        assert_eq!(state.channel, 16);
        assert_eq!(state.assignment, Some(MixerAssignment::ComputerPlay(8)));
        assert_eq!(state.pan, PanState::from_raw(0x3e));
        assert_eq!(state.linked, Some(false));
    }

    #[test]
    fn mixer_assignment_decodes_grounded_ordinary_strip_values() {
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x00, 0x00]),
            Some(MixerAssignment::Preamp(1))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x00, 0x01]),
            Some(MixerAssignment::Preamp(2))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x01, 0x00]),
            Some(MixerAssignment::ComputerPlay(1))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x01, 0x06]),
            Some(MixerAssignment::ComputerPlay(7))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x01, 0x07]),
            Some(MixerAssignment::ComputerPlay(8))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x02, 0x00]),
            Some(MixerAssignment::SpdifIn(1))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x02, 0x01]),
            Some(MixerAssignment::SpdifIn(2))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x08, 0x00]),
            Some(MixerAssignment::Mute)
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x09, 0x00]),
            Some(MixerAssignment::Oscillator(1))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x09, 0x01]),
            Some(MixerAssignment::Oscillator(2))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x0a, 0x00]),
            Some(MixerAssignment::EmuMic(1))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x0a, 0x01]),
            Some(MixerAssignment::EmuMic(2))
        );
        assert_eq!(
            MixerAssignment::from_ordinary_strip_bytes([0x03, 0x00]),
            None
        );
    }

    #[test]
    fn mixer_level_display_uses_inverse_db_scale() {
        let unity =
            MixerChannelState::known(1, Some(0x00), Some(false), PanState::center(), None, None);
        let silence =
            MixerChannelState::known(1, Some(0x60), Some(false), PanState::center(), None, None);

        assert_eq!(unity.display_db(), Some(0));
        assert_eq!(silence.display_db(), Some(-90));
        assert_eq!(unity.gain_ratio(), Some(1.0));
        assert_eq!(silence.gain_ratio(), Some(0.0));
    }
}
