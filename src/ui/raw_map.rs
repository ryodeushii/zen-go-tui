use std::ops::Range;

use antelope_protocol::{
    control_panel_startup_queries, FrameEndian, FrameOperation, QueryResponse, RuntimeMeterTarget,
    RuntimeProfile, HID_REPORT_SIZE, OFFSET_CLOCK_SOURCE, OFFSET_FRONT_PANEL_BYTES_END,
    OFFSET_FRONT_PANEL_BYTES_START, OFFSET_HP1_MODE, OFFSET_HP1_VOLUME, OFFSET_HP2_MODE,
    OFFSET_HP2_VOLUME, OFFSET_LATE_SHADOW_START, OFFSET_METER_LANES_END, OFFSET_METER_LANES_START,
    OFFSET_MIX1_LANE_A, OFFSET_MIX1_LANE_B, OFFSET_MIX1_MIRROR_A, OFFSET_MIX1_MIRROR_B,
    OFFSET_MIX1_PRIMARY, OFFSET_MIX2_LANE_A, OFFSET_MIX2_LANE_B, OFFSET_MIX2_PRIMARY,
    OFFSET_MONITOR_MODE, OFFSET_MONITOR_VOLUME, OFFSET_PREAMP1_GAIN, OFFSET_PREAMP1_METER,
    OFFSET_PREAMP1_MODE, OFFSET_PREAMP2_GAIN, OFFSET_PREAMP2_METER, OFFSET_PREAMP2_MODE,
    OFFSET_SAMPLE_RATE_CODE, OFFSET_SAMPLE_RATE_HZ_END, OFFSET_SAMPLE_RATE_HZ_START,
    OFFSET_SHARED_SHADOW_0, OFFSET_SHARED_SHADOW_5, OFFSET_STATUS_FLAGS_0, OFFSET_SURFACE_SELECTOR,
    OFFSET_UNKNOWN_6E, SNAPSHOT_PAYLOAD_OFFSET, SNAPSHOT_PAYLOAD_SIZE,
};

use crate::app::{RawMapScope, RawPacketTab};
use crate::traffic::TrafficDirection;

/// Coverage classification used by the RAW view's semantic map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Coverage {
    Used,
    Readback,
    Fixed,
    Opaque,
    Observed,
    Parser,
    Unmapped,
    Padding,
}

impl Coverage {
    pub(crate) const fn rank(self) -> u8 {
        match self {
            Self::Used => 8,
            Self::Readback => 7,
            Self::Fixed => 6,
            Self::Opaque => 5,
            Self::Observed => 4,
            Self::Parser => 3,
            Self::Unmapped => 2,
            Self::Padding => 1,
        }
    }
}

/// Logical protocol domain associated with one RAW map entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawDomain {
    Base,
    Output,
    Preamp,
    Mixer,
    Fx,
    Surround,
    Query,
    Status,
    Parser,
    Unknown,
}

/// A report range and its optional payload-relative counterpart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawMapRange {
    pub(crate) report: Range<usize>,
    pub(crate) payload: Option<Range<usize>>,
}

/// One logical field in a packet semantic map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawMapEntry {
    pub(crate) ranges: Vec<RawMapRange>,
    pub(crate) domain: RawDomain,
    pub(crate) scope: Option<RawMapScope>,
    pub(crate) label: String,
    pub(crate) coverage: Coverage,
    pub(crate) note: String,
}

/// Classification used when rendering one report byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawByteClassification {
    pub(crate) coverage: Coverage,
    pub(crate) selected: bool,
    pub(crate) overlap: bool,
}

/// Semantic descriptors for one selected RAW packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawPacketMap {
    entries: Vec<RawMapEntry>,
    report_len: usize,
}

impl RawPacketMap {
    pub(crate) fn entries(&self) -> &[RawMapEntry] {
        &self.entries
    }

    pub(crate) fn entries_for_scope(&self, scope: RawMapScope) -> Vec<&RawMapEntry> {
        self.entries
            .iter()
            .filter(|entry| scope_matches(entry, scope))
            .collect()
    }

    pub(crate) fn classify(
        &self,
        report_offset: usize,
        scope: RawMapScope,
    ) -> RawByteClassification {
        let all = self
            .entries
            .iter()
            .filter(|entry| entry_contains(entry, report_offset))
            .collect::<Vec<_>>();
        let selected = all
            .iter()
            .copied()
            .filter(|entry| scope_matches(entry, scope))
            .max_by_key(|entry| entry.coverage.rank());
        let chosen = selected.or_else(|| {
            all.iter()
                .copied()
                .max_by_key(|entry| entry.coverage.rank())
        });

        let coverage = chosen.map_or(Coverage::Unmapped, |entry| entry.coverage);
        let selected = selected.is_some_and(|_| {
            !(scope == RawMapScope::All
                && matches!(coverage, Coverage::Unmapped | Coverage::Padding))
        });

        RawByteClassification {
            coverage,
            selected,
            overlap: all.len() > 1,
        }
    }
}

/// Append one logical descriptor to an entry list.
pub(crate) fn add_entry(
    entries: &mut Vec<RawMapEntry>,
    domain: RawDomain,
    scope: Option<RawMapScope>,
    coverage: Coverage,
    label: impl Into<String>,
    note: impl Into<String>,
    ranges: Vec<RawMapRange>,
) {
    entries.push(RawMapEntry {
        ranges,
        domain,
        scope,
        label: label.into(),
        coverage,
        note: note.into(),
    });
}

/// Convert payload-relative ranges to report-relative ranges.
pub(crate) fn payload_ranges(ranges: &[Range<usize>]) -> Vec<RawMapRange> {
    ranges
        .iter()
        .cloned()
        .map(|payload| RawMapRange {
            report: (SNAPSHOT_PAYLOAD_OFFSET + payload.start)
                ..(SNAPSHOT_PAYLOAD_OFFSET + payload.end),
            payload: Some(payload),
        })
        .collect()
}

pub(crate) fn build_raw_packet_map(tab: RawPacketTab, bytes: &[u8]) -> RawPacketMap {
    build_raw_packet_map_for_profile(tab, bytes, None)
}

/// Build a raw map against the selected runtime profile. The compatibility wrapper above is
/// retained for protocol-fixture tests; production rendering always supplies the active profile.
pub(crate) fn build_raw_packet_map_for_profile(
    tab: RawPacketTab,
    bytes: &[u8],
    profile: Option<&RuntimeProfile>,
) -> RawPacketMap {
    let report_len = bytes.len();
    let mut entries = Vec::new();

    match tab {
        RawPacketTab::State73 => match profile {
            Some(profile) => build_profile_snapshot_map(&mut entries, report_len, profile),
            None => build_snapshot_map(&mut entries, report_len),
        },
        RawPacketTab::Query74 => build_query_request_map(&mut entries, bytes, report_len, profile),
        RawPacketTab::Query75 => build_query_reply_map(&mut entries, bytes, report_len, profile),
        RawPacketTab::Auxiliary => build_auxiliary_map(&mut entries, report_len, profile),
        RawPacketTab::DeviceNotification => build_notification_map(&mut entries, report_len),
    }

    let payload_offset = profile_payload_offset(profile);
    let payload = match tab {
        RawPacketTab::State73 => profile
            .is_some()
            .then_some((payload_offset, report_len))
            .or_else(|| {
                Some((
                    SNAPSHOT_PAYLOAD_OFFSET,
                    SNAPSHOT_PAYLOAD_OFFSET + SNAPSHOT_PAYLOAD_SIZE,
                ))
            }),
        RawPacketTab::Auxiliary | RawPacketTab::Query75 => {
            (report_len > payload_offset).then_some((payload_offset, report_len))
        }
        RawPacketTab::Query74 | RawPacketTab::DeviceNotification => None,
    };
    derive_unmapped_complements(&mut entries, report_len, payload);
    annotate_overlaps(&mut entries);
    entries.sort_by(|left, right| {
        first_offset(left)
            .cmp(&first_offset(right))
            .then_with(|| right.coverage.rank().cmp(&left.coverage.rank()))
    });

    RawPacketMap {
        entries,
        report_len,
    }
}

/// Build the diagnostic map for one selected application-transport event.
///
/// Unlike the legacy tab maps, this entry point requires direction, exact profile report geometry,
/// and a matching family envelope before adding semantic labels. Unknown or partial traffic keeps
/// every retained byte and receives only honest UNMAPPED coverage.
pub(crate) fn build_raw_traffic_map(
    direction: TrafficDirection,
    bytes: &[u8],
    profile: Option<&RuntimeProfile>,
) -> RawPacketMap {
    let report_len = bytes.len();
    let mut entries = Vec::new();
    let Some(profile) = profile else {
        return finish_map(entries, report_len, None);
    };
    let Some(expected_len) = profile.transport.report_size.map(usize::from) else {
        return finish_map(entries, report_len, None);
    };
    if report_len != expected_len {
        return finish_map(entries, report_len, None);
    }

    match direction {
        TrafficDirection::Rx => build_profile_rx_traffic_map(&mut entries, bytes, profile),
        TrafficDirection::Tx => build_profile_tx_traffic_map(&mut entries, bytes, profile),
    }
    finish_map(entries, report_len, None)
}

fn finish_map(
    mut entries: Vec<RawMapEntry>,
    report_len: usize,
    payload: Option<(usize, usize)>,
) -> RawPacketMap {
    derive_unmapped_complements(&mut entries, report_len, payload);
    annotate_overlaps(&mut entries);
    entries.sort_by(|left, right| {
        first_offset(left)
            .cmp(&first_offset(right))
            .then_with(|| right.coverage.rank().cmp(&left.coverage.rank()))
    });
    RawPacketMap {
        entries,
        report_len,
    }
}

fn build_profile_rx_traffic_map(
    entries: &mut Vec<RawMapEntry>,
    bytes: &[u8],
    profile: &RuntimeProfile,
) {
    let report_len = bytes.len();
    if profile_frame(profile, "state_report")
        .is_some_and(|frame| frame_fixed_bytes_match(frame, bytes))
    {
        build_profile_snapshot_map(entries, report_len, profile);
        return;
    }

    let Some(readback) = profile.readback.as_ref() else {
        return;
    };
    if bytes.first() != Some(&readback.response_magic) {
        return;
    }
    let discriminator_offset = usize::from(readback.response_discriminator_offset);
    if bytes.get(discriminator_offset) == Some(&readback.response_discriminator) {
        let category = bytes.get(usize::from(readback.category_offset)).copied();
        let index = bytes.get(usize::from(readback.index_offset)).copied();
        if category.zip(index).is_some_and(|(category, index)| {
            readback.allows(antelope_protocol::QueryRequest::new(category, index))
        }) {
            build_query_reply_map(entries, bytes, report_len, Some(profile));
        }
        return;
    }

    if meter_discriminator(profile) == bytes.get(discriminator_offset).copied()
        && profile_frame(profile, "meter_report")
            .is_some_and(|frame| frame_fixed_bytes_match(frame, bytes))
    {
        build_profile_frame_map(entries, "meter_report", report_len, profile);
        add_exact_extent(
            entries,
            RawDomain::Parser,
            Coverage::Fixed,
            "meter report discriminator",
            "Profile/runtime-owned meter family discriminator; kept distinct from 0x75/00 readback.",
            discriminator_offset..discriminator_offset + 1,
            None,
            report_len,
        );
    }
}

fn build_profile_tx_traffic_map(
    entries: &mut Vec<RawMapEntry>,
    bytes: &[u8],
    profile: &RuntimeProfile,
) {
    if profile_query_request_matches(profile, bytes) {
        build_profile_query_request_map(entries, bytes, profile);
        return;
    }

    let mut candidates = profile
        .frames
        .iter()
        .filter(|frame| {
            frame.kind.eq_ignore_ascii_case("command")
                && frame
                    .status
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("confirm")
                && frame_fixed_bytes_match(frame, bytes)
        })
        .map(|frame| {
            let fixed_count = frame
                .operations
                .iter()
                .filter(|operation| matches!(operation, FrameOperation::FixedByte { .. }))
                .count();
            (frame, fixed_count)
        })
        .filter(|(_, fixed_count)| *fixed_count >= 2)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(_, fixed_count)| std::cmp::Reverse(*fixed_count));
    let Some((frame, specificity)) = candidates.first().copied() else {
        return;
    };
    if candidates
        .get(1)
        .is_some_and(|(_, next_specificity)| *next_specificity == specificity)
    {
        return;
    }

    let auraverb = profile
        .auraverb
        .as_ref()
        .is_some_and(|contract| contract.command_frame_id == frame.id);
    let surround_global = profile
        .surround_global
        .as_ref()
        .is_some_and(|contract| contract.command_frame_id == frame.id);
    let routing = (frame.id == "routing_command")
        .then(|| routing_command_match(frame, bytes, profile))
        .flatten();
    if (frame.id == "routing_command" && routing.is_none())
        || (auraverb && !auraverb_tx_matches(bytes, profile))
        || (surround_global && !surround_global_tx_matches(bytes, profile))
        || (!auraverb
            && !surround_global
            && frame.id != "routing_command"
            && !encoder_zero_envelope_matches(frame, bytes, &[]))
    {
        return;
    }

    if let Some(routing) = routing {
        build_profile_routing_command_map(entries, frame, bytes.len(), profile, &routing);
        add_routing_encoder_zero_extents(entries, frame, bytes.len(), &routing);
    } else {
        build_profile_command_map(entries, frame, bytes.len(), profile);
        if !auraverb && !surround_global {
            add_encoder_zero_extents(entries, frame, bytes.len(), &[]);
        }
    }
    if auraverb {
        add_auraverb_tx_extents(entries, bytes, profile);
    }
    if surround_global {
        add_surround_global_tx_extents(entries, bytes, profile);
    }
}

struct RoutingCommandMatch<'a> {
    group: &'a antelope_protocol::RuntimeRoutingGroup,
    destination_range: Range<usize>,
    pairs: Vec<RoutingSourcePairMatch<'a>>,
}

struct RoutingSourcePairMatch<'a> {
    range: Range<usize>,
    bank: u8,
    index: u8,
    domain: &'a antelope_protocol::RuntimeRoutingSourceDomain,
}

fn routing_command_match<'a>(
    frame: &antelope_protocol::RuntimeFrame,
    bytes: &[u8],
    profile: &'a RuntimeProfile,
) -> Option<RoutingCommandMatch<'a>> {
    if frame.report_size.map(usize::from) != Some(bytes.len()) || profile.routing_groups.is_empty()
    {
        return None;
    }

    let mut destinations = frame
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FrameOperation::Scalar {
                field,
                offset,
                width,
                endian,
            } if field == "destination" => {
                Some((usize::from(*offset), usize::from(*width), *endian))
            }
            _ => None,
        });
    let (destination_offset, destination_width, destination_endian) = destinations.next()?;
    if destinations.next().is_some() {
        return None;
    }
    let destination_range =
        destination_offset..destination_offset.checked_add(destination_width)?;
    let destination = read_profile_scalar(bytes, destination_range.clone(), destination_endian)?;
    let mut groups = profile
        .routing_groups
        .iter()
        .filter(|group| group.destination == destination);
    let group = groups.next()?;
    if groups.next().is_some() || group.channel_count == 0 {
        return None;
    }

    let mut layouts = frame
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FrameOperation::Indexed {
                base,
                stride,
                index_field,
                width: 2,
                max_index: Some(max_index),
            } if index_field == "source_pair" => Some((
                usize::from(*base),
                usize::from(*stride),
                usize::from(*max_index),
            )),
            FrameOperation::Indexed {
                base,
                stride,
                index_field,
                width: 1,
                max_index: Some(max_index),
            } if index_field == "channel" => Some((
                usize::from(*base),
                usize::from(*stride),
                usize::from(*max_index),
            )),
            _ => None,
        });
    let (base, stride, max_index) = layouts.next()?;
    if layouts.next().is_some()
        || frame
            .operations
            .iter()
            .filter(|operation| matches!(operation, FrameOperation::Indexed { .. }))
            .count()
            != 1
        || stride < 2
    {
        return None;
    }
    let declared_max = profile
        .routing_groups
        .iter()
        .map(|group| usize::from(group.channel_count))
        .max()?;
    if max_index.checked_add(1) != Some(declared_max)
        || usize::from(group.channel_count) > declared_max
    {
        return None;
    }

    let mut pairs = Vec::with_capacity(usize::from(group.channel_count));
    for channel in 0..usize::from(group.channel_count) {
        let start = base.checked_add(stride.checked_mul(channel)?)?;
        let range = start..start.checked_add(2)?;
        if ranges_overlap(&range, &destination_range)
            || frame.operations.iter().any(|operation| {
                matches!(operation, FrameOperation::FixedByte { offset, .. } if range.contains(&usize::from(*offset)))
            })
        {
            return None;
        }
        let pair = bytes.get(range.clone())?;
        let mut domains = group.source_domains.iter().filter(|domain| {
            domain.bank == pair[0]
                && domain.index_count > 0
                && domain.index_count <= 256
                && domain
                    .status
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("confirm")
                && !domain.evidence.trim().is_empty()
        });
        let domain = domains.next()?;
        if domains.next().is_some() || u16::from(pair[1]) >= domain.index_count {
            return None;
        }
        pairs.push(RoutingSourcePairMatch {
            range,
            bank: pair[0],
            index: pair[1],
            domain,
        });
    }

    let mut occupied = vec![false; bytes.len()];
    for range in frame
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FrameOperation::FixedByte { offset, .. } => {
                let start = usize::from(*offset);
                Some(start..start + 1)
            }
            _ => None,
        })
        .chain(std::iter::once(destination_range.clone()))
        .chain(pairs.iter().map(|pair| pair.range.clone()))
    {
        for occupied in occupied.get_mut(range)? {
            *occupied = true;
        }
    }
    if bytes
        .iter()
        .zip(occupied)
        .any(|(byte, occupied)| !occupied && *byte != 0)
    {
        return None;
    }

    Some(RoutingCommandMatch {
        group,
        destination_range,
        pairs,
    })
}

fn read_profile_scalar(bytes: &[u8], range: Range<usize>, endian: FrameEndian) -> Option<u16> {
    let value = bytes.get(range)?;
    let decoded = match endian {
        FrameEndian::NotApplicable if value.len() == 1 => u32::from(value[0]),
        FrameEndian::Little if (2..=4).contains(&value.len()) => value
            .iter()
            .enumerate()
            .fold(0_u32, |decoded, (shift, byte)| {
                decoded | (u32::from(*byte) << (shift * 8))
            }),
        FrameEndian::Big if (2..=4).contains(&value.len()) => value
            .iter()
            .fold(0_u32, |decoded, byte| (decoded << 8) | u32::from(*byte)),
        _ => return None,
    };
    u16::try_from(decoded).ok()
}

fn build_profile_routing_command_map(
    entries: &mut Vec<RawMapEntry>,
    frame: &antelope_protocol::RuntimeFrame,
    report_len: usize,
    profile: &RuntimeProfile,
    routing: &RoutingCommandMatch<'_>,
) {
    let payload_offset = profile_payload_offset(Some(profile));
    for operation in &frame.operations {
        if matches!(operation, FrameOperation::FixedByte { .. }) {
            add_profile_operation(entries, &frame.id, operation, payload_offset, report_len);
        }
    }
    add_bounded_entry(
        entries,
        RawDomain::Base,
        Some(RawMapScope::Base),
        Coverage::Used,
        format!("{} {} destination", frame.id, routing.group.name),
        "Selected destination group validated against the active profile topology.",
        vec![profile_range(
            routing.destination_range.start,
            routing.destination_range.len(),
            payload_offset,
        )],
        report_len,
    );
    for (channel, pair) in routing.pairs.iter().enumerate() {
        add_bounded_entry(
            entries,
            RawDomain::Base,
            Some(RawMapScope::Base),
            Coverage::Used,
            format!(
                "{} {} channel {:02} source pair",
                frame.id,
                routing.group.name,
                channel + 1
            ),
            format!(
                "Profile-authoritative complete routing source pair: {} bank {:#04x}, index {}; channel order is positional.",
                pair.domain.name, pair.bank, pair.index
            ),
            vec![profile_range(
                pair.range.start,
                pair.range.len(),
                payload_offset,
            )],
            report_len,
        );
    }
}

fn add_routing_encoder_zero_extents(
    entries: &mut Vec<RawMapEntry>,
    frame: &antelope_protocol::RuntimeFrame,
    report_len: usize,
    routing: &RoutingCommandMatch<'_>,
) {
    let mut occupied = vec![false; report_len];
    for range in frame
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FrameOperation::FixedByte { offset, .. } => {
                let start = usize::from(*offset);
                Some(start..start + 1)
            }
            _ => None,
        })
        .chain(std::iter::once(routing.destination_range.clone()))
        .chain(routing.pairs.iter().map(|pair| pair.range.clone()))
    {
        for byte in occupied
            .iter_mut()
            .take(range.end.min(report_len))
            .skip(range.start.min(report_len))
        {
            *byte = true;
        }
    }
    let mut start = None;
    for offset in 0..=report_len {
        let fixed_zero = offset < report_len && !occupied[offset];
        match (start, fixed_zero) {
            (None, true) => start = Some(offset),
            (Some(run_start), false) => {
                add_exact_extent(
                    entries,
                    RawDomain::Parser,
                    Coverage::Fixed,
                    format!("{} encoder zero envelope", frame.id),
                    "Validated zero-initialized bytes outside the selected destination's complete ordered source pairs.",
                    run_start..offset,
                    None,
                    report_len,
                );
                start = None;
            }
            _ => {}
        }
    }
}

fn build_profile_frame_map(
    entries: &mut Vec<RawMapEntry>,
    frame_id: &str,
    report_len: usize,
    profile: &RuntimeProfile,
) {
    let payload_offset = profile_payload_offset(Some(profile));
    if let Some(frame) = profile_frame(profile, frame_id) {
        for operation in &frame.operations {
            add_profile_operation(entries, frame_id, operation, payload_offset, report_len);
        }
    }
}

fn build_profile_command_map(
    entries: &mut Vec<RawMapEntry>,
    frame: &antelope_protocol::RuntimeFrame,
    report_len: usize,
    profile: &RuntimeProfile,
) {
    let payload_offset = profile_payload_offset(Some(profile));
    for operation in &frame.operations {
        match operation {
            FrameOperation::Indexed { .. } => {
                // The frame does not carry which zero-valued indexed slot was selected. Keep the
                // candidate extent unmapped rather than claiming every possible slot was used.
            }
            FrameOperation::PairIndex {
                base, pair_field, ..
            } => add_bounded_entry(
                entries,
                profile_domain(pair_field),
                profile_scope(profile_domain(pair_field)),
                Coverage::Used,
                format!("{} {}", frame.id, pair_field),
                "Profile encoder writes the selected pair code at this single byte.",
                vec![profile_range(usize::from(*base), 1, payload_offset)],
                report_len,
            ),
            _ => add_profile_operation(entries, &frame.id, operation, payload_offset, report_len),
        }
    }
}

fn operation_possible_ranges(operation: &FrameOperation) -> Vec<Range<usize>> {
    match operation {
        FrameOperation::FixedByte { offset, .. } | FrameOperation::BitField { offset, .. } => {
            vec![usize::from(*offset)..usize::from(*offset) + 1]
        }
        FrameOperation::Scalar { offset, width, .. } => {
            vec![usize::from(*offset)..usize::from(*offset) + usize::from(*width)]
        }
        FrameOperation::Indexed {
            base,
            stride,
            width,
            max_index: Some(max_index),
            ..
        } => (0..=usize::from(*max_index))
            .map(|index| {
                let start = usize::from(*base) + index * usize::from(*stride);
                start..start + usize::from(*width)
            })
            .collect(),
        FrameOperation::PairIndex { base, .. } => {
            vec![usize::from(*base)..usize::from(*base) + 1]
        }
        FrameOperation::AllowedValues { .. }
        | FrameOperation::UncompiledFormula { .. }
        | FrameOperation::Indexed {
            max_index: None, ..
        } => Vec::new(),
    }
}

fn encoder_occupied_bytes(
    frame: &antelope_protocol::RuntimeFrame,
    report_len: usize,
    extra_ranges: &[Range<usize>],
) -> Vec<bool> {
    let mut occupied = vec![false; report_len];
    for range in frame
        .operations
        .iter()
        .flat_map(operation_possible_ranges)
        .chain(extra_ranges.iter().cloned())
    {
        for byte in occupied
            .iter_mut()
            .take(range.end.min(report_len))
            .skip(range.start.min(report_len))
        {
            *byte = true;
        }
    }
    occupied
}

fn encoder_zero_envelope_matches(
    frame: &antelope_protocol::RuntimeFrame,
    bytes: &[u8],
    extra_ranges: &[Range<usize>],
) -> bool {
    let occupied = encoder_occupied_bytes(frame, bytes.len(), extra_ranges);
    bytes
        .iter()
        .zip(occupied)
        .all(|(byte, occupied)| occupied || *byte == 0)
}

fn add_encoder_zero_extents(
    entries: &mut Vec<RawMapEntry>,
    frame: &antelope_protocol::RuntimeFrame,
    report_len: usize,
    extra_ranges: &[Range<usize>],
) {
    let occupied = encoder_occupied_bytes(frame, report_len, extra_ranges);
    let mut start = None;
    for offset in 0..=report_len {
        let is_zero_envelope = offset < report_len && !occupied[offset];
        match (start, is_zero_envelope) {
            (None, true) => start = Some(offset),
            (Some(run_start), false) => {
                add_exact_extent(
                    entries,
                    RawDomain::Parser,
                    Coverage::Fixed,
                    format!("{} encoder zero envelope", frame.id),
                    "Validated zero-initialized bytes outside the frame operation extents.",
                    run_start..offset,
                    None,
                    report_len,
                );
                start = None;
            }
            _ => {}
        }
    }
}

fn frame_fixed_bytes_match(frame: &antelope_protocol::RuntimeFrame, bytes: &[u8]) -> bool {
    let mut fixed = 0usize;
    for operation in &frame.operations {
        if let FrameOperation::FixedByte { offset, value } = operation {
            fixed += 1;
            if bytes.get(usize::from(*offset)) != Some(value) {
                return false;
            }
        }
    }
    fixed > 0
}

fn profile_query_request_matches(profile: &RuntimeProfile, bytes: &[u8]) -> bool {
    let Some(readback) = profile.readback.as_ref() else {
        return false;
    };
    let category_offset = usize::from(readback.category_offset);
    let index_offset = usize::from(readback.index_offset);
    let Some(category) = bytes.get(category_offset).copied() else {
        return false;
    };
    let Some(index) = bytes.get(index_offset).copied() else {
        return false;
    };
    bytes.first() == Some(&readback.request_magic)
        && bytes.get(4..8) == Some(readback.request_subcommand.to_le_bytes().as_slice())
        && readback.allows(antelope_protocol::QueryRequest::new(category, index))
        && bytes.iter().enumerate().all(|(offset, byte)| {
            offset == 0
                || (4..8).contains(&offset)
                || offset == category_offset
                || offset == index_offset
                || *byte == 0
        })
}

fn meter_discriminator(profile: &RuntimeProfile) -> Option<u8> {
    let readback = profile.readback.as_ref()?;
    let frame = profile_frame(profile, "meter_report")?;
    profile_fixed_byte(
        profile,
        "meter_report",
        usize::from(readback.response_discriminator_offset),
    )
    .or_else(|| {
        ((profile.identity.vid, profile.identity.pid) == (0x23e5, 0xa221)
            && frame
                .metadata
                .contains("\"status\":\"superseded_for_per_channel\""))
        .then_some(0x1f)
    })
}

fn build_profile_query_request_map(
    entries: &mut Vec<RawMapEntry>,
    bytes: &[u8],
    profile: &RuntimeProfile,
) {
    let readback = profile.readback.as_ref().expect("request shape checked");
    let report_len = bytes.len();
    let category_offset = usize::from(readback.category_offset);
    let index_offset = usize::from(readback.index_offset);
    for (label, range) in [
        ("query request family", 0..1),
        ("query request subcommand", 4..8),
    ] {
        add_bounded_entry(
            entries,
            RawDomain::Query,
            Some(RawMapScope::Query),
            Coverage::Fixed,
            label,
            "Profile query encoder fixed extent.",
            vec![RawMapRange {
                report: range,
                payload: None,
            }],
            report_len,
        );
    }
    for (label, offset) in [
        ("query category", category_offset),
        ("query index", index_offset),
    ] {
        add_bounded_entry(
            entries,
            RawDomain::Query,
            Some(RawMapScope::Query),
            Coverage::Used,
            label,
            "Profile-bounded query selector emitted by the encoder.",
            vec![RawMapRange {
                report: offset..offset + 1,
                payload: None,
            }],
            report_len,
        );
    }
}

fn profile_payload_offset(profile: Option<&RuntimeProfile>) -> usize {
    profile
        .and_then(|profile| profile.readback.as_ref())
        .map_or(SNAPSHOT_PAYLOAD_OFFSET, |readback| {
            usize::from(readback.data_offset)
        })
}

fn profile_frame<'a>(
    profile: &'a RuntimeProfile,
    frame_id: &str,
) -> Option<&'a antelope_protocol::RuntimeFrame> {
    profile.frames.iter().find(|frame| frame.id == frame_id)
}

fn profile_fixed_byte(profile: &RuntimeProfile, frame_id: &str, offset: usize) -> Option<u8> {
    profile_frame(profile, frame_id)?
        .operations
        .iter()
        .find_map(|operation| match operation {
            FrameOperation::FixedByte {
                offset: operation_offset,
                value,
            } if usize::from(*operation_offset) == offset => Some(*value),
            _ => None,
        })
}

fn profile_range(offset: usize, width: usize, payload_offset: usize) -> RawMapRange {
    let end = offset.saturating_add(width);
    let payload = (offset >= payload_offset)
        .then(|| (offset - payload_offset)..(end.saturating_sub(payload_offset)));
    RawMapRange {
        report: offset..end,
        payload,
    }
}

fn add_exact_extent(
    entries: &mut Vec<RawMapEntry>,
    domain: RawDomain,
    coverage: Coverage,
    label: impl Into<String>,
    note: impl Into<String>,
    range: Range<usize>,
    payload_offset: Option<usize>,
    report_len: usize,
) {
    let payload = payload_offset.and_then(|offset| {
        (range.start >= offset).then(|| (range.start - offset)..(range.end - offset))
    });
    add_bounded_entry(
        entries,
        domain,
        profile_scope(domain),
        coverage,
        label,
        note,
        vec![RawMapRange {
            report: range,
            payload,
        }],
        report_len,
    );
}

fn add_profile_special_readback(
    entries: &mut Vec<RawMapEntry>,
    bytes: &[u8],
    profile: &RuntimeProfile,
) -> bool {
    let Some(readback) = profile.readback.as_ref() else {
        return false;
    };
    let category = bytes.get(usize::from(readback.category_offset)).copied();
    let index = bytes.get(usize::from(readback.index_offset)).copied();

    if let Some(contract) = profile.auraverb.as_ref() {
        if category == Some(contract.readback_category)
            && index == Some(contract.readback_index)
            && auraverb_readback_matches(bytes, profile)
        {
            add_auraverb_rx_extents(entries, bytes, profile);
            return true;
        }
    }
    if let Some(contract) = profile.surround_speaker_eq.as_ref() {
        if category == Some(contract.readback_category)
            && index.is_some_and(|index| u16::from(index) < contract.record_count)
            && surround_speaker_eq_readback_matches(bytes, profile)
        {
            add_surround_speaker_eq_rx_extents(entries, bytes, profile);
            return true;
        }
    }
    if let Some(contract) = profile.surround_global.as_ref() {
        if category == Some(contract.readback_category)
            && index == Some(contract.readback_index)
            && surround_global_readback_matches(bytes, profile)
        {
            add_surround_global_rx_extents(entries, bytes, profile);
            return true;
        }
    }
    false
}

fn exact_profile_report(bytes: &[u8], profile: &RuntimeProfile) -> bool {
    profile
        .transport
        .report_size
        .is_some_and(|size| bytes.len() == usize::from(size))
}

fn auraverb_readback_matches(bytes: &[u8], profile: &RuntimeProfile) -> bool {
    let Some(contract) = profile.auraverb.as_ref() else {
        return false;
    };
    let Some(readback) = profile.readback.as_ref() else {
        return false;
    };
    let data = usize::from(readback.data_offset);
    let block_start = data + usize::from(contract.readback_block_offset);
    let block_end = block_start + usize::from(contract.readback_block_size);
    let record_end = data + usize::from(contract.readback_record_size);
    let Some(block) = bytes.get(block_start..block_end) else {
        return false;
    };
    let Some(command_origin) = auraverb_command_origin(contract) else {
        return false;
    };
    let Some(wet_offset) = usize::from(contract.wet_offset).checked_sub(command_origin) else {
        return false;
    };
    let Some(enabled_offset) = usize::from(contract.enabled_offset).checked_sub(command_origin)
    else {
        return false;
    };
    exact_profile_report(bytes, profile)
        && bytes.get(..contract.readback_header.len()) == Some(contract.readback_header.as_slice())
        && bytes.get(data) == Some(&contract.readback_body_header)
        && record_end == usize::from(contract.fixed_tail_offset)
        && bytes
            .get(record_end..)
            .is_some_and(|tail| tail.iter().all(|byte| *byte == 0))
        && block.get(wet_offset) == Some(&contract.wet_constant)
        && block.get(usize::from(contract.terminator_offset)) == Some(&contract.terminator_constant)
        && block
            .get(enabled_offset)
            .is_some_and(|value| matches!(value, 0 | 1))
        && contract.fields.iter().all(|field| {
            block
                .get(usize::from(field.readback_offset))
                .is_some_and(|value| (contract.range.0..=contract.range.1).contains(value))
        })
}

fn add_auraverb_rx_extents(entries: &mut Vec<RawMapEntry>, bytes: &[u8], profile: &RuntimeProfile) {
    let contract = profile.auraverb.as_ref().expect("AuraVerb shape checked");
    let readback = profile.readback.as_ref().expect("AuraVerb readback");
    let data = usize::from(readback.data_offset);
    let block = data + usize::from(contract.readback_block_offset);
    let record_end = data + usize::from(contract.readback_record_size);
    add_exact_extent(
        entries,
        RawDomain::Fx,
        Coverage::Fixed,
        "AuraVerb readback header",
        "Exact profile category 0x0a, index, family, discriminator, and header.",
        0..data,
        None,
        bytes.len(),
    );
    add_exact_extent(
        entries,
        RawDomain::Fx,
        Coverage::Fixed,
        "AuraVerb body header",
        "Validated fixed record header.",
        data..data + 1,
        Some(data),
        bytes.len(),
    );
    for field in &contract.fields {
        let offset = block + usize::from(field.readback_offset);
        add_exact_extent(
            entries,
            RawDomain::Fx,
            Coverage::Readback,
            format!("AuraVerb Mix 1 {}", field.name),
            "Runtime-consumed typed AuraVerb field; annotation does not grant write authority.",
            offset..offset + 1,
            Some(data),
            bytes.len(),
        );
    }
    let command_origin = auraverb_command_origin(contract).expect("AuraVerb shape checked");
    let wet_offset = usize::from(contract.wet_offset) - command_origin;
    let enabled_offset = usize::from(contract.enabled_offset) - command_origin;
    for (label, relative, coverage) in [
        ("AuraVerb wet constant", wet_offset, Coverage::Fixed),
        ("AuraVerb enabled", enabled_offset, Coverage::Readback),
        (
            "AuraVerb terminator",
            usize::from(contract.terminator_offset),
            Coverage::Fixed,
        ),
    ] {
        let offset = block + relative;
        add_exact_extent(
            entries,
            RawDomain::Fx,
            coverage,
            label,
            "Validated by the profile-owned AuraVerb decoder.",
            offset..offset + 1,
            Some(data),
            bytes.len(),
        );
    }
    let block_end = block + usize::from(contract.readback_block_size);
    add_exact_extent(
        entries,
        RawDomain::Fx,
        Coverage::Opaque,
        "AuraVerb preserved Mix 2-4 record bytes",
        "Present inside the validated record but not consumed by the Mix-1 runtime state.",
        block_end..record_end,
        Some(data),
        bytes.len(),
    );
    add_exact_extent(
        entries,
        RawDomain::Fx,
        Coverage::Fixed,
        "AuraVerb fixed zero tail",
        "Every byte is validated zero by the runtime decoder.",
        record_end..bytes.len(),
        Some(data),
        bytes.len(),
    );
}

fn auraverb_command_origin(contract: &antelope_protocol::RuntimeAuraVerbContract) -> Option<usize> {
    contract
        .fields
        .iter()
        .map(|field| usize::from(field.command_offset))
        .min()
}

fn auraverb_tx_matches(bytes: &[u8], profile: &RuntimeProfile) -> bool {
    let Some(contract) = profile.auraverb.as_ref() else {
        return false;
    };
    let Some(frame) = profile_frame(profile, &contract.command_frame_id) else {
        return false;
    };
    let operation_end = frame
        .operations
        .iter()
        .filter_map(operation_end)
        .max()
        .unwrap_or(0);
    let scalar = |field: &str| {
        frame
            .operations
            .iter()
            .find_map(|operation| match operation {
                FrameOperation::Scalar {
                    field: name,
                    offset,
                    width: 1,
                    ..
                } if name == field => bytes.get(usize::from(*offset)).copied(),
                _ => None,
            })
    };
    exact_profile_report(bytes, profile)
        && encoder_zero_envelope_matches(frame, bytes, &[])
        && scalar("target") == u8::try_from(contract.target).ok()
        && scalar("enabled").is_some_and(|value| matches!(value, 0 | 1))
        && contract.fields.iter().all(|field| {
            scalar(&format!("field_{}", field.id))
                .is_some_and(|value| (contract.range.0..=contract.range.1).contains(&value))
        })
        && bytes
            .get(operation_end..)
            .is_some_and(|tail| tail.iter().all(|byte| *byte == 0))
}

fn add_auraverb_tx_extents(entries: &mut Vec<RawMapEntry>, bytes: &[u8], profile: &RuntimeProfile) {
    let contract = profile.auraverb.as_ref().expect("AuraVerb frame matched");
    let Some(frame) = profile_frame(profile, &contract.command_frame_id) else {
        return;
    };
    let end = frame
        .operations
        .iter()
        .filter_map(operation_end)
        .max()
        .unwrap_or(0);
    if bytes
        .get(end..)
        .is_some_and(|tail| tail.iter().all(|byte| *byte == 0))
    {
        add_exact_extent(
            entries,
            RawDomain::Fx,
            Coverage::Fixed,
            "AuraVerb encoder zero tail",
            "Fixed by the validated whole-state command encoder.",
            end..bytes.len(),
            None,
            bytes.len(),
        );
    }
}

fn surround_speaker_eq_readback_matches(bytes: &[u8], profile: &RuntimeProfile) -> bool {
    let Some(contract) = profile.surround_speaker_eq.as_ref() else {
        return false;
    };
    let index = bytes.get(usize::from(contract.index_offset)).copied();
    let bands_start = usize::from(contract.data_offset + contract.candidate_head_size);
    exact_profile_report(bytes, profile)
        && bytes.get(..contract.header_prefix.len()) == Some(contract.header_prefix.as_slice())
        && index.is_some_and(|index| u16::from(index) < contract.record_count)
        && bytes.get(usize::from(contract.index_offset) + 1..usize::from(contract.data_offset))
            == Some(contract.header_suffix.as_slice())
        && bytes
            .get(usize::from(contract.fixed_tail_offset)..)
            .is_some_and(|tail| tail.iter().all(|byte| *byte == 0))
        && (0..usize::from(contract.band_count)).all(|band| {
            let start = bands_start + band * usize::from(contract.band_stride);
            let read_u16 = |relative: u16| {
                let offset = start + usize::from(relative);
                bytes
                    .get(offset..offset + 2)
                    .map(|value| u16::from_le_bytes([value[0], value[1]]))
            };
            read_u16(contract.frequency_offset).is_some_and(|value| {
                contract.frequency_range.0 <= value && value <= contract.frequency_range.1
            }) && read_u16(contract.q_offset).is_some_and(|value| {
                contract.q_raw_range.0 <= value && value <= contract.q_raw_range.1
            }) && read_u16(contract.gain_offset)
                .map(|value| value as i16)
                .is_some_and(|value| {
                    contract.gain_raw_range.0 <= value && value <= contract.gain_raw_range.1
                })
                && bytes
                    .get(start + usize::from(contract.mode_offset))
                    .is_some()
        })
}

fn add_surround_speaker_eq_rx_extents(
    entries: &mut Vec<RawMapEntry>,
    bytes: &[u8],
    profile: &RuntimeProfile,
) {
    let contract = profile
        .surround_speaker_eq
        .as_ref()
        .expect("speaker EQ shape checked");
    let data = usize::from(contract.data_offset);
    add_exact_extent(
        entries,
        RawDomain::Surround,
        Coverage::Fixed,
        "Surround speaker EQ readback header",
        "Exact RX family, category 0x1a, discriminator, and fixed header bytes.",
        0..usize::from(contract.index_offset),
        None,
        bytes.len(),
    );
    add_exact_extent(
        entries,
        RawDomain::Surround,
        Coverage::Readback,
        "Surround speaker EQ index",
        "Validated profile-bounded speaker index; read-only contract.",
        usize::from(contract.index_offset)..usize::from(contract.index_offset) + 1,
        None,
        bytes.len(),
    );
    add_exact_extent(
        entries,
        RawDomain::Surround,
        Coverage::Fixed,
        "Surround speaker EQ header suffix",
        "Fixed bytes validated before band decoding.",
        usize::from(contract.index_offset) + 1..data,
        None,
        bytes.len(),
    );
    add_exact_extent(
        entries,
        RawDomain::Surround,
        Coverage::Opaque,
        "Surround speaker EQ candidate head",
        "Observed bytes remain opaque; no engineering meaning or write authority is claimed.",
        data..data + usize::from(contract.candidate_head_size),
        Some(data),
        bytes.len(),
    );
    let bands = data + usize::from(contract.candidate_head_size);
    for band in 0..usize::from(contract.band_count) {
        let start = bands + band * usize::from(contract.band_stride);
        for (label, relative, width) in [
            ("frequency", contract.frequency_offset, 2usize),
            ("Q", contract.q_offset, 2),
            ("gain", contract.gain_offset, 2),
            ("raw mode (unknown, read-only)", contract.mode_offset, 1),
        ] {
            let offset = start + usize::from(relative);
            add_exact_extent(
                entries,
                RawDomain::Surround,
                Coverage::Readback,
                format!("Surround speaker EQ band {:02} {label}", band + 1),
                "Runtime-consumed read-only band field; raw mode values are retained without semantic promotion.",
                offset..offset + width,
                Some(data),
                bytes.len(),
            );
        }
    }
    add_exact_extent(
        entries,
        RawDomain::Surround,
        Coverage::Fixed,
        "Surround speaker EQ fixed zero tail",
        "Every byte from the profile-owned tail offset is validated zero.",
        usize::from(contract.fixed_tail_offset)..bytes.len(),
        Some(data),
        bytes.len(),
    );
}

fn surround_global_readback_matches(bytes: &[u8], profile: &RuntimeProfile) -> bool {
    let Some(contract) = profile.surround_global.as_ref() else {
        return false;
    };
    let Some(readback) = profile.readback.as_ref() else {
        return false;
    };
    let data = usize::from(readback.data_offset);
    let tail = data + usize::from(contract.template_size);
    exact_profile_report(bytes, profile)
        && bytes.get(..contract.readback_header.len()) == Some(contract.readback_header.as_slice())
        && bytes
            .get(tail..)
            .is_some_and(|tail| tail.iter().all(|byte| *byte == 0))
}

fn add_surround_global_rx_extents(
    entries: &mut Vec<RawMapEntry>,
    bytes: &[u8],
    profile: &RuntimeProfile,
) {
    let contract = profile
        .surround_global
        .as_ref()
        .expect("Surround global shape checked");
    let data = usize::from(profile.readback.as_ref().expect("readback").data_offset);
    let template_end = data + usize::from(contract.template_size);
    add_exact_extent(
        entries,
        RawDomain::Surround,
        Coverage::Fixed,
        "Surround global readback header",
        "Exact RX family, discriminator, category 0x1b, index, and fixed header.",
        0..data,
        None,
        bytes.len(),
    );
    add_exact_extent(
        entries,
        RawDomain::Surround,
        Coverage::Opaque,
        "Surround global preserved template",
        "Whole validated template is retained, but only overlaid fields are semantically decoded.",
        data..template_end,
        Some(data),
        bytes.len(),
    );
    add_surround_global_fields(entries, bytes.len(), profile, data, true);
    add_exact_extent(
        entries,
        RawDomain::Surround,
        Coverage::Fixed,
        "Surround global fixed zero tail",
        "Every byte after the RX template extent is validated zero.",
        template_end..bytes.len(),
        Some(data),
        bytes.len(),
    );
}

fn surround_global_tx_matches(bytes: &[u8], profile: &RuntimeProfile) -> bool {
    let Some(contract) = profile.surround_global.as_ref() else {
        return false;
    };
    let payload = usize::from(contract.payload_offset);
    let template_end = payload + usize::from(contract.template_size);
    let read_u16 = |offset: u16| {
        let offset = usize::from(offset);
        bytes
            .get(offset..offset + 2)
            .map(|value| u16::from_le_bytes([value[0], value[1]]))
    };
    let flags_a = bytes.get(usize::from(contract.flags_a_offset)).copied();
    let flags_b = bytes.get(usize::from(contract.flags_b_offset)).copied();
    let writable_format = flags_a.zip(flags_b).is_some_and(|(flags_a, flags_b)| {
        contract.formats.iter().any(|format| {
            format.writable
                && flags_a & contract.flags_a_mask == format.flags_a & contract.flags_a_mask
                && flags_b & contract.flags_b_mask == format.flags_b & contract.flags_b_mask
        })
    });
    let Some(frame) = profile_frame(profile, &contract.command_frame_id) else {
        return false;
    };
    exact_profile_report(bytes, profile)
        && template_end == usize::from(contract.fixed_tail_offset)
        && encoder_zero_envelope_matches(frame, bytes, &[payload..template_end])
        && writable_format
        && bytes
            .get(usize::from(contract.delay_offset))
            .is_some_and(|value| {
                contract.delay_range.0 <= u16::from(*value)
                    && u16::from(*value) <= contract.delay_range.1
            })
        && read_u16(contract.level_offset)
            .is_some_and(|value| contract.level_range.0 <= value && value <= contract.level_range.1)
        && bytes
            .get(template_end..)
            .is_some_and(|tail| tail.iter().all(|byte| *byte == 0))
}

fn add_surround_global_tx_extents(
    entries: &mut Vec<RawMapEntry>,
    bytes: &[u8],
    profile: &RuntimeProfile,
) {
    let contract = profile
        .surround_global
        .as_ref()
        .expect("Surround global frame matched");
    let payload = usize::from(contract.payload_offset);
    let template_end = payload + usize::from(contract.template_size);
    if bytes
        .get(usize::from(contract.fixed_tail_offset)..)
        .is_none_or(|tail| tail.iter().any(|byte| *byte != 0))
        || template_end != usize::from(contract.fixed_tail_offset)
    {
        return;
    }
    add_exact_extent(
        entries,
        RawDomain::Surround,
        Coverage::Opaque,
        "Surround global copied whole template",
        "The encoder copies this complete validated template; unoverlaid bytes remain preserved opaque and annotation grants no write authority.",
        payload..template_end,
        Some(payload),
        bytes.len(),
    );
    add_surround_global_fields(entries, bytes.len(), profile, payload, false);
    add_exact_extent(
        entries,
        RawDomain::Surround,
        Coverage::Fixed,
        "Surround global encoder zero tail",
        "Fixed zero extent after the complete copied template.",
        template_end..bytes.len(),
        Some(payload),
        bytes.len(),
    );
}

fn add_surround_global_fields(
    entries: &mut Vec<RawMapEntry>,
    report_len: usize,
    profile: &RuntimeProfile,
    report_payload_start: usize,
    rx: bool,
) {
    let contract = profile.surround_global.as_ref().expect("Surround contract");
    let translate = |contract_offset: u16| {
        report_payload_start + usize::from(contract_offset - contract.payload_offset)
    };
    for (label, offset, width) in [
        (
            "Surround global format flags A",
            contract.flags_a_offset,
            1usize,
        ),
        ("Surround global format flags B", contract.flags_b_offset, 1),
        ("Surround global delay", contract.delay_offset, 1),
        ("Surround global level", contract.level_offset, 2),
        ("Surround global mask word 1", contract.mask_offsets[0], 2),
        ("Surround global mask word 2", contract.mask_offsets[1], 2),
        ("Surround global mask word 3", contract.mask_offsets[2], 2),
    ] {
        let offset = translate(offset);
        add_exact_extent(
            entries,
            RawDomain::Surround,
            if rx {
                Coverage::Readback
            } else {
                Coverage::Used
            },
            label,
            if rx {
                "Runtime-decoded field; unknown format flag combinations remain accepted read-only."
            } else {
                "Typed or authorization-relevant field inside the copied command template."
            },
            offset..offset + width,
            Some(report_payload_start),
            report_len,
        );
    }
}

fn operation_end(operation: &FrameOperation) -> Option<usize> {
    match operation {
        FrameOperation::FixedByte { offset, .. } | FrameOperation::BitField { offset, .. } => {
            Some(usize::from(*offset) + 1)
        }
        FrameOperation::Scalar { offset, width, .. } => {
            Some(usize::from(*offset) + usize::from(*width))
        }
        FrameOperation::Indexed {
            base,
            stride,
            width,
            max_index: Some(max_index),
            ..
        } => Some(
            usize::from(*base)
                + usize::from(*stride) * usize::from(*max_index)
                + usize::from(*width),
        ),
        FrameOperation::PairIndex { base, .. } => Some(usize::from(*base) + 1),
        FrameOperation::AllowedValues { .. }
        | FrameOperation::UncompiledFormula { .. }
        | FrameOperation::Indexed {
            max_index: None, ..
        } => None,
    }
}

fn profile_domain(field: &str) -> RawDomain {
    let field = field.to_ascii_lowercase();
    if field.contains("physical_meter")
        || field.contains("preamp")
        || field.contains("input")
        || field.contains("gain")
        || field.contains("phantom")
        || field.contains("phase")
        || field.contains("mode")
    {
        return RawDomain::Preamp;
    }
    if field.contains("bus") || field.contains("output") {
        return RawDomain::Output;
    }
    if field.contains("meter") || field.contains("mix") {
        return RawDomain::Mixer;
    }
    RawDomain::Base
}

fn profile_scope(domain: RawDomain) -> Option<RawMapScope> {
    match domain {
        RawDomain::Base => Some(RawMapScope::Base),
        RawDomain::Output => Some(RawMapScope::Outputs),
        RawDomain::Preamp => Some(RawMapScope::Preamps),
        RawDomain::Mixer => Some(RawMapScope::Mixer),
        RawDomain::Fx | RawDomain::Surround => Some(RawMapScope::Status),
        _ => None,
    }
}

fn add_profile_operation(
    entries: &mut Vec<RawMapEntry>,
    frame_id: &str,
    operation: &FrameOperation,
    payload_offset: usize,
    report_len: usize,
) {
    let note = format!("Active profile {frame_id} frame mapping; no cross-device byte overlay.");
    let mut add_used = |field: &str, ranges: Vec<RawMapRange>| {
        let domain = profile_domain(field);
        add_bounded_entry(
            entries,
            domain,
            profile_scope(domain),
            Coverage::Used,
            format!("{frame_id} {field}"),
            note.clone(),
            ranges,
            report_len,
        );
    };

    match operation {
        FrameOperation::FixedByte { offset, value } => add_bounded_entry(
            entries,
            RawDomain::Parser,
            Some(RawMapScope::Parser),
            Coverage::Fixed,
            format!("{frame_id} fixed 0x{value:02x}"),
            "Profile-declared fixed byte validated for this frame.",
            vec![profile_range(usize::from(*offset), 1, payload_offset)],
            report_len,
        ),
        FrameOperation::Scalar {
            field,
            offset,
            width,
            ..
        } => {
            add_used(
                field,
                vec![profile_range(
                    usize::from(*offset),
                    usize::from(*width),
                    payload_offset,
                )],
            );
        }
        FrameOperation::BitField { field, offset, .. } => add_used(
            field,
            vec![profile_range(usize::from(*offset), 1, payload_offset)],
        ),
        FrameOperation::Indexed {
            base,
            stride,
            index_field,
            width,
            max_index,
        } => {
            let Some(max_index) = max_index else {
                return;
            };
            for index in 0..=usize::from(*max_index) {
                let offset =
                    usize::from(*base).saturating_add(index.saturating_mul(usize::from(*stride)));
                add_used(
                    &format!("{index_field}[{index}]"),
                    vec![profile_range(offset, usize::from(*width), payload_offset)],
                );
            }
        }
        FrameOperation::PairIndex {
            base,
            stride,
            pair_field,
            width,
            max_index,
        } => {
            let Some(max_index) = max_index else {
                return;
            };
            for index in 0..=usize::from(*max_index) {
                let offset =
                    usize::from(*base).saturating_add(index.saturating_mul(usize::from(*stride)));
                add_used(
                    &format!("{pair_field}[{index}]"),
                    vec![profile_range(offset, usize::from(*width), payload_offset)],
                );
            }
        }
        FrameOperation::AllowedValues { .. } | FrameOperation::UncompiledFormula { .. } => {}
    }
}

fn build_profile_snapshot_map(
    entries: &mut Vec<RawMapEntry>,
    report_len: usize,
    profile: &RuntimeProfile,
) {
    let payload_offset = profile_payload_offset(Some(profile));
    add_bounded_entry(
        entries,
        RawDomain::Parser,
        Some(RawMapScope::Parser),
        Coverage::Parser,
        "frame envelope and header",
        "Active profile state_report frame area.",
        vec![RawMapRange {
            report: 0..payload_offset,
            payload: None,
        }],
        report_len,
    );

    if let Some(frame) = profile_frame(profile, "state_report") {
        for operation in &frame.operations {
            add_profile_operation(
                entries,
                "state_report",
                operation,
                payload_offset,
                report_len,
            );
        }
    }

    if let Some(state_report) = profile.state_report.as_ref() {
        for meter in &state_report.candidate_preamp_meters {
            let label = format!("candidate preamp {} meter", meter.input_index + 1);
            add_bounded_entry(
                entries,
                RawDomain::Preamp,
                Some(RawMapScope::Preamps),
                Coverage::Observed,
                label,
                "Profile candidate only; this is not a confirmed physical-input capability.",
                vec![profile_range(
                    SNAPSHOT_PAYLOAD_OFFSET.saturating_add(meter.offset),
                    1,
                    payload_offset,
                )],
                report_len,
            );
        }
    }

    for mapping in profile
        .meter_mappings
        .iter()
        .filter(|mapping| mapping.frame_id == "state_report")
    {
        let (domain, label) = match mapping.target {
            RuntimeMeterTarget::MixMaster => {
                let name = profile
                    .mixer(mapping.target_index as u8)
                    .map(|mixer| mixer.name.clone())
                    .unwrap_or_else(|| format!("Mix {}", mapping.target_index + 1));
                (RawDomain::Mixer, format!("{name} master meter"))
            }
            RuntimeMeterTarget::MixerStrip => {
                let name = profile
                    .mixer(mapping.target_index as u8)
                    .map(|mixer| mixer.name.clone())
                    .unwrap_or_else(|| format!("Mix {}", mapping.target_index + 1));
                (
                    RawDomain::Mixer,
                    format!("{name} strip {} meter", mapping.lane),
                )
            }
            RuntimeMeterTarget::PhysicalOutput => {
                let name = profile
                    .outputs
                    .iter()
                    .find(|output| output.id == mapping.target_index)
                    .map(|output| output.name.clone())
                    .unwrap_or_else(|| format!("output {}", mapping.target_index));
                (RawDomain::Output, format!("{name} output meter"))
            }
        };
        let gate_note = mapping.byte_equals.map_or_else(String::new, |predicate| {
            format!(
                " Gated by report @{} == {}; source report @{}.",
                predicate.offset, predicate.value, mapping.offset
            )
        });
        add_bounded_entry(
            entries,
            domain,
            profile_scope(domain),
            Coverage::Observed,
            label.clone(),
            format!(
                "Profile-owned observed meter mapping ({}). One lane only; no stereo L/R inference.{gate_note} {}",
                mapping.status_text, mapping.evidence
            ),
            vec![profile_range(mapping.offset, 1, payload_offset)],
            report_len,
        );
        if let Some(predicate) = mapping.byte_equals {
            add_bounded_entry(
                entries,
                domain,
                profile_scope(domain),
                Coverage::Used,
                format!("{label} selector gate"),
                format!(
                    "Runtime byte_equals predicate: report @{} must equal {}; gates source report @{}.",
                    predicate.offset, predicate.value, mapping.offset
                ),
                vec![profile_range(predicate.offset, 1, payload_offset)],
                report_len,
            );
        }
    }
}

fn build_snapshot_map(entries: &mut Vec<RawMapEntry>, report_len: usize) {
    add_bounded_entry(
        entries,
        RawDomain::Parser,
        Some(RawMapScope::Parser),
        Coverage::Parser,
        "frame envelope and header",
        "Parser-known 0x73 frame area.",
        vec![RawMapRange {
            report: 0..SNAPSHOT_PAYLOAD_OFFSET,
            payload: None,
        }],
        report_len,
    );
    add_snapshot_entry(
        entries,
        RawDomain::Base,
        Some(RawMapScope::Base),
        Coverage::Used,
        "status flags 0-1",
        "Typed snapshot status bytes.",
        OFFSET_STATUS_FLAGS_0..(OFFSET_STATUS_FLAGS_0 + 1),
        report_len,
    );
    add_snapshot_entry(
        entries,
        RawDomain::Base,
        Some(RawMapScope::Base),
        Coverage::Used,
        "sample-rate code",
        "Decoded through SampleRate.",
        OFFSET_SAMPLE_RATE_CODE..(OFFSET_SAMPLE_RATE_CODE + 1),
        report_len,
    );
    add_snapshot_entry(
        entries,
        RawDomain::Base,
        Some(RawMapScope::Base),
        Coverage::Used,
        "clock source",
        "Decoded as the active profile's raw clock-source enum value.",
        OFFSET_CLOCK_SOURCE..(OFFSET_CLOCK_SOURCE + 1),
        report_len,
    );
    add_snapshot_entry(
        entries,
        RawDomain::Base,
        Some(RawMapScope::Base),
        Coverage::Used,
        "sample-rate Hz",
        "Big-endian rate value.",
        OFFSET_SAMPLE_RATE_HZ_START..OFFSET_SAMPLE_RATE_HZ_END,
        report_len,
    );
    add_snapshot_entry(
        entries,
        RawDomain::Base,
        Some(RawMapScope::Base),
        Coverage::Unmapped,
        "front-panel cluster",
        "Preserved. Individual controls are not decoded.",
        OFFSET_FRONT_PANEL_BYTES_START..(OFFSET_FRONT_PANEL_BYTES_END + 1),
        report_len,
    );

    for (domain, label, note, payload_offset) in [
        (
            RawDomain::Output,
            "Monitor output level",
            "Output attenuation value.",
            OFFSET_MONITOR_VOLUME,
        ),
        (
            RawDomain::Output,
            "Monitor output mode",
            "Normal, dim, or mute.",
            OFFSET_MONITOR_MODE,
        ),
        (
            RawDomain::Output,
            "HP1 output level",
            "Output attenuation value.",
            OFFSET_HP1_VOLUME,
        ),
        (
            RawDomain::Output,
            "HP1 output mode",
            "Normal, dim, or mute.",
            OFFSET_HP1_MODE,
        ),
        (
            RawDomain::Output,
            "HP2 output level",
            "Output attenuation value.",
            OFFSET_HP2_VOLUME,
        ),
        (
            RawDomain::Output,
            "HP2 output mode",
            "Normal, dim, or mute.",
            OFFSET_HP2_MODE,
        ),
    ] {
        add_snapshot_entry(
            entries,
            domain,
            Some(RawMapScope::Outputs),
            Coverage::Used,
            label,
            note,
            payload_offset..(payload_offset + 1),
            report_len,
        );
    }

    for (label, note, payload_offset) in [
        (
            "preamp 1 gain",
            "Typed preamp cluster.",
            OFFSET_PREAMP1_GAIN,
        ),
        (
            "preamp 2 gain",
            "Typed preamp cluster.",
            OFFSET_PREAMP2_GAIN,
        ),
        (
            "preamp 1 mode, phantom bit, phase bit",
            "Low nibble is mode. Bits 0x10 and 0x40 are phantom and phase.",
            OFFSET_PREAMP1_MODE,
        ),
        (
            "preamp 2 mode, phantom bit, phase bit",
            "Low nibble is mode. Bits 0x10 and 0x40 are phantom and phase.",
            OFFSET_PREAMP2_MODE,
        ),
    ] {
        add_snapshot_entry(
            entries,
            RawDomain::Preamp,
            Some(RawMapScope::Preamps),
            Coverage::Used,
            label,
            note,
            payload_offset..(payload_offset + 1),
            report_len,
        );
    }
    add_snapshot_entry(
        entries,
        RawDomain::Preamp,
        Some(RawMapScope::Preamps),
        Coverage::Used,
        "preamp 2 phase bit",
        "0x40 phase bit in combined preamp 2 mode, phantom, phase byte.",
        OFFSET_PREAMP2_MODE..(OFFSET_PREAMP2_MODE + 1),
        report_len,
    );

    add_snapshot_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Used,
        "active mixer surface selector",
        "Mix1 or Mix2 selection.",
        OFFSET_SURFACE_SELECTOR..(OFFSET_SURFACE_SELECTOR + 1),
        report_len,
    );
    add_snapshot_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Unmapped,
        "unknown control byte",
        "Documented unknown byte.",
        OFFSET_UNKNOWN_6E..(OFFSET_UNKNOWN_6E + 1),
        report_len,
    );

    let meter_lane_count = OFFSET_METER_LANES_END - OFFSET_METER_LANES_START + 1;
    for lane in 0..meter_lane_count {
        let payload_offset = OFFSET_METER_LANES_START + lane;
        add_snapshot_entry(
            entries,
            RawDomain::Mixer,
            Some(RawMapScope::Mixer),
            Coverage::Observed,
            format!("CH{:02} observed meter lane", lane + 1),
            "n ranges from 0 through 15. The app applies this shared observation to both mixer surfaces.",
            payload_offset..(payload_offset + 1),
            report_len,
        );
    }

    for (label, payload_offset) in [
        ("observed preamp 1 meter lane", OFFSET_PREAMP1_METER),
        ("observed preamp 2 meter lane", OFFSET_PREAMP2_METER),
    ] {
        add_snapshot_entry(
            entries,
            RawDomain::Preamp,
            Some(RawMapScope::Preamps),
            Coverage::Observed,
            label,
            "Narrow observed meter range.",
            payload_offset..(payload_offset + 1),
            report_len,
        );
    }

    add_snapshot_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Observed,
        "late mixer correlation lanes",
        "Preserve correlated Mix1 and Mix2 lanes. Do not claim one control per byte.",
        OFFSET_LATE_SHADOW_START..OFFSET_SHARED_SHADOW_0,
        report_len,
    );
    add_snapshot_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Unmapped,
        "shared late shadow bytes",
        "No per-control claim.",
        OFFSET_SHARED_SHADOW_0..(OFFSET_SHARED_SHADOW_5 + 1),
        report_len,
    );

    add_bounded_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Observed,
        "active mixer CH01 mute correlation",
        "Narrow passive decode. Not a standalone byte field.",
        payload_ranges(&[
            OFFSET_MIX1_PRIMARY..(OFFSET_MIX1_PRIMARY + 1),
            OFFSET_MIX2_PRIMARY..(OFFSET_MIX2_PRIMARY + 1),
            OFFSET_MIX1_LANE_A..(OFFSET_MIX1_MIRROR_A + 1),
        ]),
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Observed,
        "active mixer CH01 pan correlation",
        "Narrow passive decode. Unresolved outside documented codebook.",
        payload_ranges(&[
            OFFSET_MIX1_PRIMARY..(OFFSET_MIX1_PRIMARY + 1),
            OFFSET_MIX2_PRIMARY..(OFFSET_MIX2_PRIMARY + 1),
            OFFSET_MIX1_LANE_A..(OFFSET_MIX1_LANE_A + 1),
            OFFSET_MIX1_MIRROR_B..(OFFSET_MIX2_LANE_B + 1),
        ]),
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Observed,
        "active mixer CH01/CH02 link correlation",
        "Applied only to active-surface CH01/CH02. not a standalone byte field.",
        payload_ranges(&[
            OFFSET_MIX1_PRIMARY..(OFFSET_MIX1_PRIMARY + 1),
            OFFSET_MIX2_PRIMARY..(OFFSET_MIX2_PRIMARY + 1),
            OFFSET_MIX1_LANE_A..(OFFSET_MIX2_LANE_B + 1),
        ]),
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Observed,
        "Mix1 CH01/CH02 link correlation",
        "Applied only to Mix1 CH01/CH02. not a standalone byte field.",
        payload_ranges(&[
            OFFSET_MIX1_PRIMARY..(OFFSET_MIX1_PRIMARY + 1),
            OFFSET_MIX2_PRIMARY..(OFFSET_MIX2_PRIMARY + 1),
            OFFSET_MIX1_LANE_A..(OFFSET_MIX2_LANE_B + 1),
        ]),
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Observed,
        "Mix1 late lane A/B",
        "Correlated lane pair.",
        payload_ranges(&[
            OFFSET_MIX1_LANE_A..(OFFSET_MIX1_LANE_A + 1),
            OFFSET_MIX1_LANE_B..(OFFSET_MIX1_LANE_B + 1),
        ]),
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Observed,
        "Mix1 late lane A/B mirror",
        "Mirror of the Mix1 pair.",
        payload_ranges(&[
            OFFSET_MIX1_MIRROR_A..(OFFSET_MIX1_MIRROR_A + 1),
            OFFSET_MIX1_MIRROR_B..(OFFSET_MIX1_MIRROR_B + 1),
        ]),
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Observed,
        "Mix2 late lane A/B",
        "Correlated lane pair.",
        payload_ranges(&[
            OFFSET_MIX2_LANE_A..(OFFSET_MIX2_LANE_A + 1),
            OFFSET_MIX2_LANE_B..(OFFSET_MIX2_LANE_B + 1),
        ]),
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Mixer,
        Some(RawMapScope::Mixer),
        Coverage::Unmapped,
        "shared late shadow bytes",
        "No per-control claim.",
        payload_ranges(&[OFFSET_SHARED_SHADOW_0..OFFSET_SHARED_SHADOW_5]),
        report_len,
    );

    add_bounded_entry(
        entries,
        RawDomain::Unknown,
        None,
        Coverage::Padding,
        "fixed snapshot padding",
        "Snapshot payload ends at report 0xf6.",
        vec![RawMapRange {
            report: (SNAPSHOT_PAYLOAD_OFFSET + SNAPSHOT_PAYLOAD_SIZE)..HID_REPORT_SIZE,
            payload: None,
        }],
        report_len,
    );
}

fn build_query_request_map(
    entries: &mut Vec<RawMapEntry>,
    bytes: &[u8],
    report_len: usize,
    profile: Option<&RuntimeProfile>,
) {
    add_query_header(entries, report_len, profile);

    if let Some(profile) = profile {
        let request_magic = profile
            .readback
            .as_ref()
            .map(|readback| readback.request_magic);
        if request_magic != bytes.first().copied() {
            add_query_body_unresolved(
                entries,
                report_len,
                "Frame does not match active profile 0x74 request magic.".to_string(),
                true,
                report_len.saturating_sub(profile_payload_offset(Some(profile))),
            );
            return;
        }
    }

    let Some(response) = query_response(bytes, profile) else {
        return;
    };
    let known = control_panel_startup_queries()
        .iter()
        .any(|query| query.query_id == response.query_id && query.sub_id == response.sub_id);
    if known {
        add_bounded_entry(
            entries,
            RawDomain::Query,
            Some(RawMapScope::Query),
            Coverage::Padding,
            "known request padding",
            "encode_query writes no request body fields.",
            vec![RawMapRange {
                report: SNAPSHOT_PAYLOAD_OFFSET..HID_REPORT_SIZE,
                payload: None,
            }],
            report_len,
        );
    } else {
        add_query_body_unresolved(
            entries,
            report_len,
            format!(
                "Unknown 0x74 query request 0x{:02x}/0x{:02x}.",
                response.query_id, response.sub_id
            ),
            false,
            report_len.saturating_sub(SNAPSHOT_PAYLOAD_OFFSET),
        );
    }
}

fn build_query_reply_map(
    entries: &mut Vec<RawMapEntry>,
    bytes: &[u8],
    report_len: usize,
    profile: Option<&RuntimeProfile>,
) {
    add_query_header(entries, report_len, profile);

    if let Some(profile) = profile {
        let Some(readback) = profile.readback.as_ref() else {
            add_query_body_unresolved(
                entries,
                report_len,
                "Active profile has no readback discriminator.".to_string(),
                true,
                report_len.saturating_sub(profile_payload_offset(Some(profile))),
            );
            return;
        };
        let matches_response = bytes.first() == Some(&readback.response_magic)
            && bytes.get(usize::from(readback.response_discriminator_offset))
                == Some(&readback.response_discriminator);
        if !matches_response {
            add_query_body_unresolved(
                entries,
                report_len,
                format!(
                    "Frame does not match active profile 0x75 readback discriminator 0x{:02x}; body remains unresolved.",
                    readback.response_discriminator
                ),
                true,
                report_len.saturating_sub(profile_payload_offset(Some(profile))),
            );
            return;
        }
    }

    if profile.is_some_and(|profile| add_profile_special_readback(entries, bytes, profile)) {
        return;
    }

    let Some(response) = query_response(bytes, profile) else {
        add_query_body_unresolved(
            entries,
            report_len,
            "Short 0x75 frame; query body shape unavailable.".to_string(),
            true,
            report_len.saturating_sub(SNAPSHOT_PAYLOAD_OFFSET),
        );
        return;
    };

    let mut grounded = false;
    match response.query_id {
        0x03 if response.sub_id == 0x05
            && response.body.len() == 9
            && response.assignment_readback().is_some() =>
        {
            grounded = true;
            add_assignment_entries(entries, &response, 1, 4, report_len);
        }
        0x03 if (0x06..=0x09).contains(&response.sub_id)
            && response.body.len() == 33
            && response.assignment_readback().is_some() =>
        {
            grounded = true;
            add_assignment_entries(entries, &response, 5, 16, report_len);
        }
        0x0b if response.sub_id == 0x03 => {
            if response.body.len() == 24 && response.selector_bitmap().is_some() {
                grounded = true;
                add_selector_entries(entries, &response, report_len);
            }
        }
        0x04 if matches!(response.sub_id, 0x00 | 0x01) => {
            if response.body.len() == 34 && response.startup_pan_state_readback().is_some() {
                grounded = true;
                add_pan_state_entries(entries, &response, report_len);
            }
        }
        0x18 if response.sub_id == 0x00 => {
            if response.body.len() == 64 && response.mixer_strip_readback().is_some() {
                grounded = true;
                add_mixer_strip_entries(entries, &response, report_len);
            }
        }
        0x01 => {
            if response.metadata().is_some() {
                grounded = add_metadata_entries(entries, &response, report_len);
            }
        }
        0x15 if response.sub_id == 0x00 => {
            if response.body.len() == 64 && response.startup_indexed_code_table().is_some() {
                grounded = true;
                add_indexed_entries(entries, &response, report_len);
            }
        }
        0x17 if response.sub_id == 0x00 => {
            if response.body.len() == 4 && response.startup_quad_state().is_some() {
                grounded = true;
                add_quad_entries(entries, &response, report_len);
            }
        }
        0x11 => {
            grounded = !response.body.is_empty();
            add_query_body_entry(
                entries,
                RawDomain::Status,
                Some(RawMapScope::Status),
                Coverage::Unmapped,
                "status/capability body",
                "Status or capability body has no field decoder.",
                0..response.body.len(),
                report_len,
            );
        }
        _ => {}
    }

    if !grounded {
        add_query_body_unresolved(
            entries,
            report_len,
            format!(
                "No grounded 0x75 body shape for query 0x{:02x}/0x{:02x}.",
                response.query_id, response.sub_id
            ),
            true,
            effective_query_body_len(&response, report_len),
        );
    }
}

fn build_auxiliary_map(
    entries: &mut Vec<RawMapEntry>,
    report_len: usize,
    profile: Option<&RuntimeProfile>,
) {
    let frame_label = match profile {
        None => "0x83".to_string(),
        Some(profile) => profile_fixed_byte(profile, "meter_report", 0).map_or_else(
            || "active profile".to_string(),
            |magic| format!("0x{magic:02x}"),
        ),
    };
    add_bounded_entry(
        entries,
        RawDomain::Parser,
        Some(RawMapScope::Parser),
        Coverage::Parser,
        "frame envelope and header",
        "Parser-known auxiliary frame area for the active profile.",
        vec![RawMapRange {
            report: 0..SNAPSHOT_PAYLOAD_OFFSET,
            payload: None,
        }],
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Unknown,
        Some(RawMapScope::Unmapped),
        Coverage::Unmapped,
        "unmapped auxiliary payload",
        format!("{frame_label} payload is preserved without a grounded decoder."),
        vec![RawMapRange {
            report: SNAPSHOT_PAYLOAD_OFFSET..HID_REPORT_SIZE,
            payload: Some(0..(HID_REPORT_SIZE - SNAPSHOT_PAYLOAD_OFFSET)),
        }],
        report_len,
    );
}

fn build_notification_map(entries: &mut Vec<RawMapEntry>, report_len: usize) {
    add_bounded_entry(
        entries,
        RawDomain::Parser,
        Some(RawMapScope::Parser),
        Coverage::Parser,
        "notification frame bytes",
        "Parser accepts exactly six notification bytes.",
        vec![RawMapRange {
            report: 0..6,
            payload: None,
        }],
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Unknown,
        None,
        Coverage::Padding,
        "fixed notification padding",
        "App copy is padded after the six-byte notification.",
        vec![RawMapRange {
            report: 6..HID_REPORT_SIZE,
            payload: None,
        }],
        report_len,
    );
}

fn add_query_header(
    entries: &mut Vec<RawMapEntry>,
    report_len: usize,
    profile: Option<&RuntimeProfile>,
) {
    let payload_offset = profile_payload_offset(profile);
    let (category_offset, index_offset) = profile
        .and_then(|profile| profile.readback.as_ref())
        .map(|readback| {
            (
                usize::from(readback.category_offset),
                usize::from(readback.index_offset),
            )
        })
        .unwrap_or((0x08, 0x0c));
    add_bounded_entry(
        entries,
        RawDomain::Parser,
        Some(RawMapScope::Parser),
        Coverage::Parser,
        "frame envelope and header",
        "Parser-known query frame area.",
        vec![RawMapRange {
            report: 0..payload_offset,
            payload: None,
        }],
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Parser,
        Some(RawMapScope::Parser),
        Coverage::Parser,
        "query ID",
        "Parser-known query identifier.",
        vec![RawMapRange {
            report: category_offset..(category_offset + 1),
            payload: None,
        }],
        report_len,
    );
    add_bounded_entry(
        entries,
        RawDomain::Parser,
        Some(RawMapScope::Parser),
        Coverage::Parser,
        "sub-ID",
        "Parser-known sub-query identifier.",
        vec![RawMapRange {
            report: index_offset..(index_offset + 1),
            payload: None,
        }],
        report_len,
    );
}

fn add_assignment_entries(
    entries: &mut Vec<RawMapEntry>,
    response: &QueryResponse,
    first_channel: usize,
    last_channel: usize,
    report_len: usize,
) {
    for channel in first_channel..=last_channel {
        let body_offset = if response.sub_id == 0x05 {
            1 + (channel - 1) * 2
        } else {
            9 + (channel - 5) * 2
        };
        add_query_body_entry(
            entries,
            RawDomain::Mixer,
            Some(RawMapScope::Mixer),
            Coverage::Readback,
            format!("CH{:02} assignment", channel),
            format!(
                "Assignment bank 0x{:02x}, body pair offset 0x{:02x}.",
                response.sub_id, body_offset
            ),
            body_offset..(body_offset + 2),
            report_len,
        );
    }
}

fn add_selector_entries(
    entries: &mut Vec<RawMapEntry>,
    response: &QueryResponse,
    report_len: usize,
) {
    let _bitmap = response.selector_bitmap().expect("selector shape checked");
    for index in 0..24 {
        let label = match index {
            0..=7 => {
                let first_channel = index * 2 + 1;
                format!(
                    "Mix1 CH{:02}/CH{:02} link selector",
                    first_channel,
                    first_channel + 1
                )
            }
            8..=15 => format!("selector bitmap byte {index:02}"),
            16..=23 => {
                let first_channel = (index - 16) * 2 + 1;
                format!(
                    "Mix2 CH{:02}/CH{:02} link selector",
                    first_channel,
                    first_channel + 1
                )
            }
            _ => unreachable!(),
        };
        add_query_body_entry(
            entries,
            RawDomain::Mixer,
            Some(RawMapScope::Mixer),
            Coverage::Readback,
            label,
            "Complete selector bitmap byte is read back; value may be selected or unselected.",
            index..(index + 1),
            report_len,
        );
    }
}

fn add_pan_state_entries(
    entries: &mut Vec<RawMapEntry>,
    response: &QueryResponse,
    report_len: usize,
) {
    let (surface, _states) = response
        .startup_pan_state_readback()
        .expect("pan-state shape checked");
    let surface_label = match surface {
        antelope_protocol::MixerSurface::Mix1 => "Mix1",
        antelope_protocol::MixerSurface::Mix2 => "Mix2",
    };
    for channel in 1..=16 {
        let body_offset = 2 + (channel - 1) * 2;
        add_query_body_entry(
            entries,
            RawDomain::Mixer,
            Some(RawMapScope::Mixer),
            Coverage::Readback,
            format!("{surface_label} CH{channel:02} level"),
            format!("Startup pan/state body level byte offset 0x{body_offset:02x}."),
            body_offset..(body_offset + 1),
            report_len,
        );
        add_query_body_entry(
            entries,
            RawDomain::Mixer,
            Some(RawMapScope::Mixer),
            Coverage::Readback,
            format!("{surface_label} CH{channel:02} pan/mute/solo state"),
            format!(
                "Startup pan/state body state byte offset 0x{:02x}; decodes pan, mute, and solo.",
                body_offset + 1
            ),
            (body_offset + 1)..(body_offset + 2),
            report_len,
        );
    }
}

fn add_mixer_strip_entries(
    entries: &mut Vec<RawMapEntry>,
    response: &QueryResponse,
    report_len: usize,
) {
    let _readback = response
        .mixer_strip_readback()
        .expect("mixer strip shape checked");
    for surface_index in 0..2 {
        let surface_label = if surface_index == 0 { "Mix1" } else { "Mix2" };
        for channel in 1..=16 {
            let body_offset = surface_index * 32 + (channel - 1) * 2;
            add_query_body_entry(
                entries,
                RawDomain::Mixer,
                Some(RawMapScope::Mixer),
                Coverage::Readback,
                format!("{surface_label} CH{channel:02} level"),
                format!("Full strip readback level byte offset 0x{body_offset:02x}."),
                body_offset..(body_offset + 1),
                report_len,
            );
            add_query_body_entry(
                entries,
                RawDomain::Mixer,
                Some(RawMapScope::Mixer),
                Coverage::Readback,
                format!("{surface_label} CH{channel:02} pan/mute/solo state"),
                format!(
                    "Full strip readback state byte offset 0x{:02x}; decodes pan, mute, and solo.",
                    body_offset + 1
                ),
                (body_offset + 1)..(body_offset + 2),
                report_len,
            );
        }
    }
}

fn metadata_body_ranges(body: &[u8]) -> Option<Vec<Range<usize>>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for chunk in body.split(|byte| *byte == 0) {
        let end = start + chunk.len();
        let value = String::from_utf8_lossy(chunk).trim().to_string();
        if !chunk.is_empty() && !value.is_empty() {
            ranges.push(start..end);
            if ranges.len() == 3 {
                return Some(ranges);
            }
        }
        start = end.saturating_add(1);
    }
    None
}

fn add_metadata_entries(
    entries: &mut Vec<RawMapEntry>,
    response: &QueryResponse,
    report_len: usize,
) -> bool {
    let Some(ranges) = metadata_body_ranges(&response.body) else {
        return false;
    };
    for (label, range) in [
        ("product name", ranges[0].clone()),
        ("serial", ranges[1].clone()),
        ("hardware version", ranges[2].clone()),
    ] {
        add_query_body_entry(
            entries,
            RawDomain::Query,
            Some(RawMapScope::Metadata),
            Coverage::Readback,
            label,
            "NUL-separated metadata field in existing decoder order.",
            range,
            report_len,
        );
    }
    true
}

fn add_indexed_entries(
    entries: &mut Vec<RawMapEntry>,
    response: &QueryResponse,
    report_len: usize,
) {
    let table = response
        .startup_indexed_code_table()
        .expect("indexed-table shape checked");
    for index in 0..table.len() {
        let body_offset = index * 2;
        add_query_body_entry(
            entries,
            RawDomain::Query,
            Some(RawMapScope::Mixer),
            Coverage::Readback,
            format!("indexed entry {index:02}"),
            format!("Indexed code-table pair offset 0x{body_offset:02x}."),
            body_offset..(body_offset + 2),
            report_len,
        );
    }
}

fn add_quad_entries(entries: &mut Vec<RawMapEntry>, response: &QueryResponse, report_len: usize) {
    let _quad = response
        .startup_quad_state()
        .expect("quad-state shape checked");
    for index in 0..4 {
        add_query_body_entry(
            entries,
            RawDomain::Query,
            Some(RawMapScope::Mixer),
            Coverage::Readback,
            format!("quad state byte {index}"),
            format!("Startup quad-state byte offset 0x{index:02x}."),
            index..(index + 1),
            report_len,
        );
    }
}

fn add_query_body_entry(
    entries: &mut Vec<RawMapEntry>,
    domain: RawDomain,
    scope: Option<RawMapScope>,
    coverage: Coverage,
    label: impl Into<String>,
    note: impl Into<String>,
    body_range: Range<usize>,
    report_len: usize,
) {
    add_bounded_entry(
        entries,
        domain,
        scope,
        coverage,
        label,
        note,
        payload_ranges(std::slice::from_ref(&body_range)),
        report_len,
    );
}

fn add_query_body_unresolved(
    entries: &mut Vec<RawMapEntry>,
    report_len: usize,
    note: String,
    payload: bool,
    body_len: usize,
) {
    let body_len = body_len.min(report_len.saturating_sub(SNAPSHOT_PAYLOAD_OFFSET));
    if payload {
        add_query_body_entry(
            entries,
            RawDomain::Unknown,
            Some(RawMapScope::Unmapped),
            Coverage::Unmapped,
            "unresolved query body",
            note,
            0..body_len,
            report_len,
        );
    } else {
        add_bounded_entry(
            entries,
            RawDomain::Unknown,
            Some(RawMapScope::Unmapped),
            Coverage::Unmapped,
            "unresolved query body",
            note,
            vec![RawMapRange {
                report: SNAPSHOT_PAYLOAD_OFFSET..(SNAPSHOT_PAYLOAD_OFFSET + body_len),
                payload: None,
            }],
            report_len,
        );
    }
}

fn effective_query_body_len(response: &QueryResponse, report_len: usize) -> usize {
    let available = report_len.saturating_sub(SNAPSHOT_PAYLOAD_OFFSET);
    if response.body.is_empty() {
        available
    } else {
        response.body.len().min(available)
    }
}

fn query_response(bytes: &[u8], profile: Option<&RuntimeProfile>) -> Option<QueryResponse> {
    let payload_offset = profile_payload_offset(profile);
    let (category_offset, index_offset) = profile
        .and_then(|profile| profile.readback.as_ref())
        .map(|readback| {
            (
                usize::from(readback.category_offset),
                usize::from(readback.index_offset),
            )
        })
        .unwrap_or((0x08, 0x0c));
    (bytes.len() >= payload_offset
        && bytes.get(category_offset).is_some()
        && bytes.get(index_offset).is_some())
    .then(|| {
        let body = declared_query_body_len_at(bytes, payload_offset)
            .map(|body_len| bytes[payload_offset..payload_offset + body_len].to_vec())
            .unwrap_or_default();
        QueryResponse {
            query_id: bytes[category_offset],
            sub_id: bytes[index_offset],
            body,
        }
    })
}

fn declared_query_body_len(bytes: &[u8]) -> Option<usize> {
    declared_query_body_len_at(bytes, SNAPSHOT_PAYLOAD_OFFSET)
}

fn declared_query_body_len_at(bytes: &[u8], payload_offset: usize) -> Option<usize> {
    let declared_total = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
    let body_len = declared_total.checked_sub(payload_offset)?;
    let available = bytes.len().saturating_sub(payload_offset);
    Some(body_len.min(available))
}

fn add_snapshot_entry(
    entries: &mut Vec<RawMapEntry>,
    domain: RawDomain,
    scope: Option<RawMapScope>,
    coverage: Coverage,
    label: impl Into<String>,
    note: impl Into<String>,
    payload_range: Range<usize>,
    report_len: usize,
) {
    add_bounded_entry(
        entries,
        domain,
        scope,
        coverage,
        label,
        note,
        payload_ranges(std::slice::from_ref(&payload_range)),
        report_len,
    );
}

fn add_bounded_entry(
    entries: &mut Vec<RawMapEntry>,
    domain: RawDomain,
    scope: Option<RawMapScope>,
    coverage: Coverage,
    label: impl Into<String>,
    note: impl Into<String>,
    ranges: Vec<RawMapRange>,
    report_len: usize,
) {
    let ranges = ranges
        .into_iter()
        .filter_map(|range| {
            let end = range.report.end.min(report_len);
            if range.report.start >= end {
                return None;
            }
            let visible_len = end - range.report.start;
            let payload = range.payload.map(|payload| {
                let payload_end = payload.start.saturating_add(visible_len).min(payload.end);
                payload.start..payload_end
            });
            Some(RawMapRange {
                report: range.report.start..end,
                payload,
            })
        })
        .collect::<Vec<_>>();
    if !ranges.is_empty() {
        add_entry(entries, domain, scope, coverage, label, note, ranges);
    }
}

fn scope_matches(entry: &RawMapEntry, scope: RawMapScope) -> bool {
    match scope {
        RawMapScope::All => true,
        RawMapScope::Unmapped => entry.coverage == Coverage::Unmapped,
        _ => entry.scope == Some(scope),
    }
}

fn entry_contains(entry: &RawMapEntry, report_offset: usize) -> bool {
    entry
        .ranges
        .iter()
        .any(|range| range.report.contains(&report_offset))
}

fn derive_unmapped_complements(
    entries: &mut Vec<RawMapEntry>,
    report_len: usize,
    payload: Option<(usize, usize)>,
) {
    let mut offset = 0;
    while offset < report_len {
        if entries.iter().any(|entry| entry_contains(entry, offset)) {
            offset += 1;
            continue;
        }
        let start = offset;
        while offset < report_len && !entries.iter().any(|entry| entry_contains(entry, offset)) {
            offset += 1;
        }
        let end = offset;
        let ranges = report_complement_ranges(start..end, payload);
        add_entry(
            entries,
            RawDomain::Unknown,
            None,
            Coverage::Unmapped,
            format!("unmapped report 0x{start:02x}..0x{end:02x}"),
            "No grounded decoder mapping.",
            ranges,
        );
    }
}

fn report_complement_ranges(
    report_range: Range<usize>,
    payload: Option<(usize, usize)>,
) -> Vec<RawMapRange> {
    let Some((payload_start, payload_end)) = payload else {
        return vec![RawMapRange {
            report: report_range,
            payload: None,
        }];
    };

    let mut ranges = Vec::new();
    let mut cursor = report_range.start;
    while cursor < report_range.end {
        let end = if cursor < payload_start {
            report_range.end.min(payload_start)
        } else if cursor < payload_end {
            report_range.end.min(payload_end)
        } else {
            report_range.end
        };
        let payload_range = (cursor >= payload_start && cursor < payload_end)
            .then(|| (cursor - payload_start)..(end - payload_start));
        ranges.push(RawMapRange {
            report: cursor..end,
            payload: payload_range,
        });
        cursor = end;
    }
    ranges
}

fn annotate_overlaps(entries: &mut [RawMapEntry]) {
    let overlap = (0..entries.len())
        .map(|index| {
            (0..entries.len()).any(|other| {
                index != other
                    && entries[index].ranges.iter().any(|left| {
                        entries[other]
                            .ranges
                            .iter()
                            .any(|right| ranges_overlap(&left.report, &right.report))
                    })
            })
        })
        .collect::<Vec<_>>();

    for (entry, has_overlap) in entries.iter_mut().zip(overlap) {
        if has_overlap && !entry.note.contains("OVERLAP") {
            if !entry.note.is_empty() {
                entry.note.push(' ');
            }
            entry.note.push_str("OVERLAP");
        }
    }
}

fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn first_offset(entry: &RawMapEntry) -> usize {
    entry
        .ranges
        .first()
        .map_or(usize::MAX, |range| range.report.start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry<'a>(map: &'a RawPacketMap, label: &str) -> &'a RawMapEntry {
        map.entries()
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("missing raw map entry: {label}"))
    }

    fn builtin_entry(pid: u16) -> antelope_protocol::RuntimeEntry {
        crate::device::ProfileCatalog::builtin()
            .find(0x23e5, pid)
            .unwrap_or_else(|| panic!("missing built-in profile for pid {pid:#06x}"))
            .clone()
    }

    fn builtin_profile(pid: u16) -> antelope_protocol::RuntimeProfile {
        builtin_entry(pid).profile
    }

    fn query_bytes(query_id: u8, sub_id: u8, body: &[u8]) -> [u8; 320] {
        let mut bytes = [0_u8; 320];
        bytes[0..4].copy_from_slice(&0x75_u32.to_le_bytes());
        let body_len = body.len().min(bytes.len() - SNAPSHOT_PAYLOAD_OFFSET);
        let declared_total = SNAPSHOT_PAYLOAD_OFFSET + body_len;
        bytes[4..8].copy_from_slice(&(declared_total as u32).to_le_bytes());
        bytes[0x08] = query_id;
        bytes[0x0c] = sub_id;
        bytes[SNAPSHOT_PAYLOAD_OFFSET..SNAPSHOT_PAYLOAD_OFFSET + body_len]
            .copy_from_slice(&body[..body_len]);
        bytes
    }

    fn hex_fixture(text: &str) -> Vec<u8> {
        text.split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).expect("fixture byte"))
            .collect()
    }

    fn only_unmapped(map: &RawPacketMap) -> bool {
        map.entries()
            .iter()
            .all(|entry| entry.coverage == Coverage::Unmapped)
    }

    #[test]
    fn selected_traffic_maps_complete_routing_group_preserves_every_ordered_source_pair() {
        use antelope_protocol::{Action, DeviceDriver, ProfileDriver, RoutingSource};

        let profile_entry = builtin_entry(0xa221);
        let profile = profile_entry.profile.clone();
        let driver = ProfileDriver::new(profile_entry).expect("Orion profile driver");
        let sources = (0..16)
            .map(|index| RoutingSource { bank: 3, index })
            .collect::<Vec<_>>();
        let frame = driver
            .encode(Action::SetRoutingGroup {
                destination: 0,
                changed_channel: None,
                sources,
            })
            .expect("complete routing group")
            .frames
            .remove(0);

        let map = build_raw_traffic_map(TrafficDirection::Tx, &frame, Some(&profile));
        for channel in 0..16 {
            let pair = entry(
                &map,
                &format!(
                    "routing_command line_out channel {:02} source pair",
                    channel + 1
                ),
            );
            assert_eq!(pair.coverage, Coverage::Used);
            assert_eq!(
                pair.ranges[0].report,
                (19 + channel * 2)..(21 + channel * 2)
            );
        }
        assert_eq!(&frame[19..21], &[3, 0]);
        assert!(
            entry(&map, "routing_command line_out channel 01 source pair")
                .note
                .contains("bank 0x03, index 0")
        );
        assert_eq!(map.classify(20, RawMapScope::All).coverage, Coverage::Used);
        assert_eq!(map.classify(51, RawMapScope::All).coverage, Coverage::Fixed);

        let zero_frame = driver
            .encode(Action::SetRoutingGroup {
                destination: 1,
                changed_channel: None,
                sources: vec![RoutingSource { bank: 0, index: 0 }; 2],
            })
            .expect("zero-valued active routing pairs")
            .frames
            .remove(0);
        let zero_map = build_raw_traffic_map(TrafficDirection::Tx, &zero_frame, Some(&profile));
        assert_eq!(&zero_frame[19..23], &[0, 0, 0, 0]);
        let zero_pair = entry(
            &zero_map,
            "routing_command headphone_1 channel 02 source pair",
        );
        assert_eq!(zero_pair.ranges[0].report, 21..23);
        assert!(zero_pair.note.contains("bank 0x00, index 0"));

        let mut invalid_source = frame.clone();
        invalid_source[20] = 16;
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Tx,
            &invalid_source,
            Some(&profile)
        )));
        let mut invalid_destination = frame.clone();
        invalid_destination[18] = 15;
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Tx,
            &invalid_destination,
            Some(&profile)
        )));
        let mut invalid_header = frame.clone();
        invalid_header[17] ^= 1;
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Tx,
            &invalid_header,
            Some(&profile)
        )));
        let mut invalid_tail = frame.clone();
        invalid_tail[51] = 1;
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Tx,
            &invalid_tail,
            Some(&profile)
        )));

        let mut invalid_geometry = profile.clone();
        let routing = invalid_geometry
            .frames
            .iter_mut()
            .find(|candidate| candidate.id == "routing_command")
            .expect("routing frame");
        let indexed = routing
            .operations
            .iter_mut()
            .find_map(|operation| match operation {
                FrameOperation::Indexed { max_index, .. } => Some(max_index),
                _ => None,
            })
            .expect("routing indexed geometry");
        *indexed = Some(14);
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Tx,
            &frame,
            Some(&invalid_geometry)
        )));

        let zen = builtin_profile(0xa015);
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Tx,
            &frame,
            Some(&zen)
        )));
    }

    #[test]
    fn selected_traffic_maps_auraverb_rx_and_tx_exact_profile_extents() {
        let profile = builtin_profile(0xa221);
        let rx = hex_fixture(include_str!(
            "../../antelope-protocol/tests/fixtures/orion/auraverb/readback_mix1_poweron.hex"
        ));
        let rx_map = build_raw_traffic_map(TrafficDirection::Rx, &rx, Some(&profile));
        assert_eq!(
            entry(&rx_map, "AuraVerb body header").ranges[0].report,
            16..17
        );
        assert_eq!(
            entry(&rx_map, "AuraVerb Mix 1 room_size").ranges[0].report,
            17..18
        );
        assert_eq!(entry(&rx_map, "AuraVerb enabled").ranges[0].report, 26..27);
        assert_eq!(
            entry(&rx_map, "AuraVerb terminator").ranges[0].report,
            27..28
        );
        assert_eq!(
            entry(&rx_map, "AuraVerb preserved Mix 2-4 record bytes").ranges[0].report,
            28..59
        );
        assert_eq!(
            entry(&rx_map, "AuraVerb fixed zero tail").ranges[0].report,
            59..320
        );

        let tx = hex_fixture(include_str!(
            "../../antelope-protocol/tests/fixtures/orion/auraverb/color_0.hex"
        ));
        let tx_map = build_raw_traffic_map(TrafficDirection::Tx, &tx, Some(&profile));
        assert_eq!(
            entry(&tx_map, "auraverb_command target").ranges[0].report,
            18..19
        );
        assert_eq!(
            entry(&tx_map, "auraverb_command field_6").ranges[0].report,
            19..20
        );
        assert_eq!(
            entry(&tx_map, "auraverb_command enabled").ranges[0].report,
            28..29
        );
        assert_eq!(
            entry(&tx_map, "AuraVerb encoder zero tail").ranges[0].report,
            29..320
        );
    }

    #[test]
    fn selected_traffic_maps_surround_global_rx_and_tx_without_calling_template_semantic() {
        let profile = builtin_profile(0xa221);
        let rx = hex_fixture(include_str!(
            "../../antelope-protocol/tests/fixtures/orion/surround_global_20_readback.hex"
        ));
        let rx_map = build_raw_traffic_map(TrafficDirection::Rx, &rx, Some(&profile));
        let rx_template = entry(&rx_map, "Surround global preserved template");
        assert_eq!(rx_template.coverage, Coverage::Opaque);
        assert_eq!(rx_template.ranges[0].report, 16..167);
        assert_eq!(
            entry(&rx_map, "Surround global delay").ranges[0].report,
            18..19
        );
        assert_eq!(
            entry(&rx_map, "Surround global level").ranges[0].report,
            20..22
        );
        assert_eq!(
            entry(&rx_map, "Surround global fixed zero tail").ranges[0].report,
            167..320
        );

        let tx = hex_fixture(include_str!(
            "../../antelope-protocol/tests/fixtures/orion/surround_global_20_eq_post.hex"
        ));
        let tx_map = build_raw_traffic_map(TrafficDirection::Tx, &tx, Some(&profile));
        let tx_template = entry(&tx_map, "Surround global copied whole template");
        assert_eq!(tx_template.coverage, Coverage::Opaque);
        assert_eq!(tx_template.ranges[0].report, 18..169);
        assert_eq!(
            entry(&tx_map, "Surround global delay").ranges[0].report,
            20..21
        );
        assert_eq!(
            entry(&tx_map, "Surround global level").ranges[0].report,
            22..24
        );
        assert_eq!(
            entry(&tx_map, "Surround global encoder zero tail").ranges[0].report,
            169..320
        );
    }

    #[test]
    fn selected_traffic_maps_speaker_eq_head_bands_mode_and_tail_at_exact_offsets() {
        let profile = builtin_profile(0xa221);
        let mut rx = hex_fixture(include_str!(
            "../../antelope-protocol/tests/fixtures/orion/surround_speaker_eq/l.hex"
        ));
        rx[26] = 0xff;
        let map = build_raw_traffic_map(TrafficDirection::Rx, &rx, Some(&profile));

        let head = entry(&map, "Surround speaker EQ candidate head");
        assert_eq!(head.coverage, Coverage::Opaque);
        assert_eq!(head.ranges[0].report, 16..20);
        assert_eq!(
            entry(&map, "Surround speaker EQ band 01 frequency").ranges[0].report,
            20..22
        );
        assert_eq!(
            entry(&map, "Surround speaker EQ band 01 Q").ranges[0].report,
            22..24
        );
        assert_eq!(
            entry(&map, "Surround speaker EQ band 01 gain").ranges[0].report,
            24..26
        );
        assert_eq!(
            entry(
                &map,
                "Surround speaker EQ band 01 raw mode (unknown, read-only)"
            )
            .ranges[0]
                .report,
            26..27
        );
        assert_eq!(
            entry(
                &map,
                "Surround speaker EQ band 16 raw mode (unknown, read-only)"
            )
            .ranges[0]
                .report,
            131..132
        );
        assert_eq!(
            entry(&map, "Surround speaker EQ fixed zero tail").ranges[0].report,
            132..320
        );
    }

    #[test]
    fn selected_traffic_rejects_wrong_direction_profile_header_index_and_truncation() {
        let orion = builtin_profile(0xa221);
        let zen = builtin_profile(0xa015);
        let aura_rx = hex_fixture(include_str!(
            "../../antelope-protocol/tests/fixtures/orion/auraverb/readback_mix1_poweron.hex"
        ));
        let aura_tx = hex_fixture(include_str!(
            "../../antelope-protocol/tests/fixtures/orion/auraverb/color_0.hex"
        ));
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Tx,
            &aura_rx,
            Some(&orion)
        )));
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Rx,
            &aura_tx,
            Some(&orion)
        )));
        assert!(
            !build_raw_traffic_map(TrafficDirection::Rx, &aura_rx, Some(&zen))
                .entries()
                .iter()
                .any(|item| {
                    item.label.contains("AuraVerb")
                        || item.label.contains("Surround")
                        || item.coverage == Coverage::Readback
                })
        );
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Rx,
            &aura_rx[..100],
            Some(&orion)
        )));
        let unknown = vec![0x99; 320];
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Rx,
            &unknown,
            Some(&orion)
        )));
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Tx,
            &unknown,
            Some(&orion)
        )));

        let mut bad_header = aura_rx.clone();
        bad_header[2] = 1;
        assert!(
            !build_raw_traffic_map(TrafficDirection::Rx, &bad_header, Some(&orion))
                .entries()
                .iter()
                .any(|item| item.label.contains("AuraVerb Mix 1"))
        );

        let mut speaker = hex_fixture(include_str!(
            "../../antelope-protocol/tests/fixtures/orion/surround_speaker_eq/l.hex"
        ));
        speaker[12] = 16;
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Rx,
            &speaker,
            Some(&orion)
        )));
    }

    #[test]
    fn profile_meter_predicates_annotate_selector_and_each_gated_source_from_metadata() {
        let profile = builtin_profile(0xa221);
        let mut bytes = vec![0_u8; 320];
        bytes[0] = 0x73;
        let map = build_raw_traffic_map(TrafficDirection::Rx, &bytes, Some(&profile));
        let gated = profile
            .meter_mappings
            .iter()
            .filter(|mapping| mapping.byte_equals.is_some())
            .collect::<Vec<_>>();
        assert_eq!(gated.len(), 13);
        for mapping in gated {
            let label = format!("Mix 2 strip {} meter selector gate", mapping.lane);
            let gate = entry(&map, &label);
            let predicate = mapping.byte_equals.expect("gated mapping");
            assert_eq!(
                gate.ranges[0].report,
                predicate.offset..predicate.offset + 1
            );
            assert!(gate
                .note
                .contains(&format!("gates source report @{}", mapping.offset)));
            assert!(map.entries().iter().any(|item| {
                item.label == format!("Mix 2 strip {} meter", mapping.lane)
                    && item.ranges[0].report == (mapping.offset..mapping.offset + 1)
            }));
        }
        assert_eq!(
            profile
                .meter_mappings
                .iter()
                .filter_map(|mapping| mapping.byte_equals.map(|predicate| predicate.offset))
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([121])
        );
        assert_eq!(
            profile
                .meter_mappings
                .iter()
                .filter(|mapping| mapping.byte_equals.is_some())
                .map(|mapping| mapping.offset)
                .collect::<Vec<_>>(),
            (144..=156).collect::<Vec<_>>()
        );
    }

    #[test]
    fn selected_traffic_keeps_orion_75_1f_distinct_and_reuses_typed_legacy_query_map() {
        let orion = builtin_profile(0xa221);
        let zen = builtin_profile(0xa015);
        let mut meter = vec![0_u8; 320];
        meter[0] = 0x75;
        meter[1] = 0x1f;
        let orion_meter = build_raw_traffic_map(TrafficDirection::Rx, &meter, Some(&orion));
        assert_eq!(
            entry(&orion_meter, "meter report discriminator").ranges[0].report,
            1..2
        );
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Rx,
            &meter,
            Some(&zen)
        )));

        let mut body = vec![0_u8; 34];
        body[0] = 0;
        let query = query_bytes(0x04, 0x00, &body);
        let typed = build_raw_traffic_map(TrafficDirection::Rx, &query, Some(&zen));
        assert_eq!(entry(&typed, "Mix1 CH01 level").ranges[0].report, 18..19);
    }

    #[test]
    fn selected_traffic_maps_validated_profile_query_and_general_command_operations() {
        let profile = builtin_profile(0xa221);
        let query = antelope_protocol::encode_profile_query(
            &profile,
            antelope_protocol::QueryRequest::new(0x1b, 0),
        )
        .expect("safe profile query");
        let query_map = build_raw_traffic_map(TrafficDirection::Tx, &query, Some(&profile));
        assert_eq!(
            entry(&query_map, "query request family").coverage,
            Coverage::Fixed
        );
        assert_eq!(entry(&query_map, "query category").ranges[0].report, 8..9);
        assert_eq!(entry(&query_map, "query index").ranges[0].report, 12..13);

        let frame = profile_frame(&profile, "global_command").expect("global command");
        let mut command = vec![0_u8; 320];
        for operation in &frame.operations {
            if let FrameOperation::FixedByte { offset, value } = operation {
                command[usize::from(*offset)] = *value;
            }
        }
        let scalar = frame
            .operations
            .iter()
            .find_map(|operation| match operation {
                FrameOperation::Scalar {
                    field,
                    offset,
                    width,
                    ..
                } => Some((field.clone(), usize::from(*offset), usize::from(*width))),
                _ => None,
            })
            .expect("command scalar");
        let command_map = build_raw_traffic_map(TrafficDirection::Tx, &command, Some(&profile));
        assert_eq!(
            entry(&command_map, &format!("global_command {}", scalar.0)).ranges[0].report,
            scalar.1..scalar.1 + scalar.2
        );
        assert!(command_map.entries().iter().any(|item| {
            item.label == "global_command encoder zero envelope"
                && item.coverage == Coverage::Fixed
                && item.ranges[0].report == (1..4)
        }));

        command[1] = 1;
        assert!(only_unmapped(&build_raw_traffic_map(
            TrafficDirection::Tx,
            &command,
            Some(&profile)
        )));
    }

    #[test]
    fn active_orion_profile_maps_six_provisional_single_lane_outputs_at_full_offsets() {
        let profile = builtin_profile(0xa221);
        let map =
            build_raw_packet_map_for_profile(RawPacketTab::State73, &[0; 320], Some(&profile));
        let observed = map
            .entries()
            .iter()
            .filter(|entry| {
                entry.coverage == Coverage::Observed
                    && entry.domain == RawDomain::Output
                    && entry.label.contains("output meter")
            })
            .collect::<Vec<_>>();

        assert_eq!(observed.len(), 6);
        assert_eq!(
            observed
                .iter()
                .map(|entry| entry.ranges[0].report.start)
                .collect::<Vec<_>>(),
            vec![157, 158, 159, 160, 177, 178]
        );
        assert!(observed.iter().all(|entry| entry.ranges.len() == 1));
        assert!(observed.iter().all(|entry| !entry.label.contains(" L ")));
        assert!(observed.iter().all(|entry| !entry.label.contains(" R ")));
        assert_eq!(
            map.classify(161, RawMapScope::All).coverage,
            Coverage::Unmapped
        );
        assert!(!map
            .entries()
            .iter()
            .any(|entry| entry.label.contains("physical preamp")
                || entry.label.contains("master meter")));
    }

    #[test]
    fn active_zen_go_profile_highlights_candidate_preamp_meters_at_full_report_offsets() {
        let profile = builtin_profile(0xa015);
        let map =
            build_raw_packet_map_for_profile(RawPacketTab::State73, &[0; 320], Some(&profile));

        assert_eq!(
            entry(&map, "candidate preamp 1 meter").ranges[0].report,
            0xde..0xdf
        );
        assert_eq!(
            entry(&map, "candidate preamp 2 meter").ranges[0].report,
            0xdf..0xe0
        );
        for offset in [0xde, 0xdf] {
            let classification = map.classify(offset, RawMapScope::Preamps);
            assert!(classification.selected, "offset {offset:#x}");
        }
    }

    #[test]
    fn active_profile_readback_discriminator_keeps_orion_meter_bytes_unresolved() {
        let profile = builtin_profile(0xa221);
        let mut bytes = [0_u8; 320];
        bytes[0] = 0x75;
        bytes[1] = 0x1f;
        let map = build_raw_packet_map_for_profile(RawPacketTab::Query75, &bytes, Some(&profile));

        assert!(map
            .entries()
            .iter()
            .all(|entry| entry.coverage != Coverage::Readback));
        assert!(map
            .entries()
            .iter()
            .any(|entry| entry.label == "unresolved query body"));
    }

    #[test]
    fn active_profiles_do_not_invent_meter_lanes_or_exceed_short_report_bounds() {
        let catalog = crate::device::ProfileCatalog::builtin();
        for entry in catalog.entries() {
            let profile = entry.profile();
            let bytes = vec![0_u8; 160];
            let map =
                build_raw_packet_map_for_profile(RawPacketTab::State73, &bytes, Some(profile));
            assert!(map.entries().iter().all(|item| {
                item.ranges
                    .iter()
                    .all(|range| range.report.end <= bytes.len())
            }));
            let expected = profile
                .meter_mappings
                .iter()
                .filter(|mapping| {
                    mapping.frame_id == "state_report" && mapping.offset < bytes.len()
                })
                .count();
            let observed = map
                .entries()
                .iter()
                .filter(|item| {
                    item.coverage == Coverage::Observed
                        && item.label.contains("meter")
                        && item.domain == RawDomain::Mixer
                })
                .count();
            assert_eq!(
                observed,
                profile
                    .meter_mappings
                    .iter()
                    .filter(|mapping| {
                        mapping.frame_id == "state_report"
                            && mapping.offset < bytes.len()
                            && matches!(
                                mapping.target,
                                antelope_protocol::RuntimeMeterTarget::MixMaster
                                    | antelope_protocol::RuntimeMeterTarget::MixerStrip
                            )
                    })
                    .count(),
                "profile {}",
                entry.id
            );
            assert!(expected >= observed);
        }
    }

    #[test]
    fn snapshot_maps_exact_base_output_and_preamp_offsets() {
        let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);

        assert_eq!(entry(&map, "clock source").ranges[0].report, 0x13..0x14);
        assert_eq!(
            entry(&map, "clock source").ranges[0].payload,
            Some(0x03..0x04)
        );
        assert_eq!(entry(&map, "HP1 output mode").ranges[0].report, 0x1f..0x20);
        assert_eq!(entry(&map, "preamp 2 gain").ranges[0].report, 0x29..0x2a);
        assert_eq!(
            entry(&map, "preamp 2 mode, phantom bit, phase bit").ranges[0].payload,
            Some(0x1b..0x1c)
        );
    }

    #[test]
    fn snapshot_maps_every_meter_lane_to_exact_channel() {
        let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);

        for channel in 1..=16 {
            let label = format!("CH{channel:02} observed meter lane");
            let item = entry(&map, &label);
            assert_eq!(item.coverage, Coverage::Observed);
            assert_eq!(
                item.ranges[0].report,
                (0x9e + channel - 1)..(0x9f + channel - 1)
            );
            assert_eq!(
                item.ranges[0].payload,
                Some((0x8e + channel - 1)..(0x8f + channel - 1))
            );
        }
    }

    #[test]
    fn mixer_correlation_keeps_non_contiguous_ranges_and_warning_note() {
        let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
        let item = entry(&map, "active mixer CH01/CH02 link correlation");

        assert_eq!(item.ranges.len(), 3);
        assert!(item.note.contains("not a standalone byte field"));
        assert!(item.note.contains("OVERLAP"));
        assert_eq!(item.ranges[0].report, 0x9f..0xa0);
        assert_eq!(item.ranges[1].report, 0xdf..0xe0);
        assert_eq!(item.ranges[2].report, 0xea..0xf0);
    }

    #[test]
    fn snapshot_status_maps_only_flag_zero_without_overlapping_sample_rate() {
        let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
        let status = entry(&map, "status flags 0-1");

        assert_eq!(status.ranges[0].report, 0x10..0x11);
        assert_eq!(status.ranges[0].payload, Some(0x00..0x01));
        assert!(!status
            .ranges
            .iter()
            .any(|range| range.report.contains(&0x11)));
        assert_eq!(
            map.classify(0x12, RawMapScope::All).coverage,
            Coverage::Used
        );
        assert!(!map.classify(0x12, RawMapScope::All).overlap);
    }

    #[test]
    fn snapshot_padding_and_unmapped_complements_preserve_report_offsets() {
        let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
        let padding = entry(&map, "fixed snapshot padding");

        assert_eq!(padding.coverage, Coverage::Padding);
        assert_eq!(padding.ranges[0].report, 0xf6..0x140);
        assert_eq!(
            map.classify(0xf6, RawMapScope::All).coverage,
            Coverage::Padding
        );
        assert!(map
            .entries()
            .iter()
            .any(|item| item.coverage == Coverage::Unmapped && item.label.contains("0x22")));
    }

    #[test]
    fn snapshot_output_and_preamp_labels_are_field_specific() {
        let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);

        for label in [
            "Monitor output level",
            "Monitor output mode",
            "HP1 output level",
            "HP1 output mode",
            "HP2 output level",
            "HP2 output mode",
            "preamp 1 gain",
            "preamp 2 gain",
            "preamp 1 mode, phantom bit, phase bit",
            "preamp 2 mode, phantom bit, phase bit",
        ] {
            assert_eq!(entry(&map, label).coverage, Coverage::Used);
        }
    }

    #[test]
    fn overlap_precedence_keeps_selected_scope_and_one_byte_classification() {
        let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
        let classification = map.classify(0x9f, RawMapScope::Mixer);

        assert_eq!(classification.coverage, Coverage::Observed);
        assert!(classification.selected);
        assert!(classification.overlap);
        assert_eq!(
            map.classify(0x13, RawMapScope::Outputs).coverage,
            Coverage::Used
        );
        assert_eq!(
            map.classify(0x13, RawMapScope::Mixer).coverage,
            Coverage::Used
        );
        assert!(!map.classify(0x13, RawMapScope::Mixer).selected);
    }

    #[test]
    fn marker_only_assignment_body_does_not_use_padded_tail() {
        let bytes = query_bytes(0x03, 0x05, &[0x05]);
        let map = build_raw_packet_map(RawPacketTab::Query75, &bytes);

        assert!(map
            .entries()
            .iter()
            .all(|item| !item.label.ends_with("assignment")));
        let unresolved = entry(&map, "unresolved query body");
        assert_eq!(unresolved.ranges[0].report, 0x10..0x11);
    }

    #[test]
    fn valid_assignment_readback_uses_total_frame_length_header() {
        let mut body = vec![0_u8; 9];
        body[0] = 0x05;
        let bytes = query_bytes(0x03, 0x05, &body);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 0x19);
        let map = build_raw_packet_map(RawPacketTab::Query75, &bytes);

        assert_eq!(entry(&map, "CH01 assignment").coverage, Coverage::Readback);
        assert_eq!(entry(&map, "CH04 assignment").ranges[0].report, 0x17..0x19);
    }

    #[test]
    fn declared_query_total_below_header_is_rejected() {
        let mut bytes = query_bytes(0x03, 0x05, &[0x05; 9]);
        bytes[4..8].copy_from_slice(&0x0f_u32.to_le_bytes());

        assert_eq!(declared_query_body_len(&bytes), None);
    }

    #[test]
    fn empty_declared_query_body_stays_unresolved() {
        let mut bytes = query_bytes(0x17, 0x00, &[]);
        bytes[4..8].copy_from_slice(&0_u32.to_le_bytes());
        let map = build_raw_packet_map(RawPacketTab::Query75, &bytes);

        assert!(map
            .entries()
            .iter()
            .all(|item| !item.label.starts_with("quad state byte")));
        let unresolved = entry(&map, "unresolved query body");
        assert_eq!(unresolved.ranges[0].report, 0x10..0x140);
    }

    #[test]
    fn invalid_query_shape_does_not_create_readback_labels() {
        let bytes = query_bytes(0x03, 0x05, &[0x06]);
        let map = build_raw_packet_map(RawPacketTab::Query75, &bytes);

        assert!(map
            .entries()
            .iter()
            .all(|item| item.coverage != Coverage::Readback));
        assert!(map
            .entries()
            .iter()
            .any(|item| item.label == "unresolved query body"));
    }

    #[test]
    fn atomic_preamp_and_mixer_labels_are_present() {
        let snapshot = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
        let phase = entry(&snapshot, "preamp 2 phase bit");
        assert_eq!(phase.coverage, Coverage::Used);
        assert_eq!(phase.ranges[0].report, 0x2b..0x2c);
        assert_eq!(phase.ranges[0].payload, Some(0x1b..0x1c));

        let link = entry(&snapshot, "Mix1 CH01/CH02 link correlation");
        assert_eq!(link.coverage, Coverage::Observed);
        assert_eq!(link.ranges.len(), 3);
        assert_eq!(link.ranges[2].report, 0xea..0xf0);
    }

    #[test]
    fn recognized_assignment_pairs_require_decoder_shape() {
        let mut body = vec![0_u8; 9];
        body[0] = 0x05;
        let map = build_raw_packet_map(RawPacketTab::Query75, &query_bytes(0x03, 0x05, &body));

        for channel in 1..=4 {
            let item = entry(&map, &format!("CH{channel:02} assignment"));
            let body_start = 1 + (channel - 1) * 2;
            assert_eq!(item.coverage, Coverage::Readback);
            assert_eq!(
                item.ranges[0].report,
                (SNAPSHOT_PAYLOAD_OFFSET + body_start)..(SNAPSHOT_PAYLOAD_OFFSET + body_start + 2)
            );
            assert!(item.note.contains("bank 0x05"));
        }

        let invalid =
            build_raw_packet_map(RawPacketTab::Query75, &query_bytes(0x03, 0x05, &[0x06]));
        assert!(invalid
            .entries()
            .iter()
            .all(|item| !item.label.ends_with("assignment")));
    }

    #[test]
    fn selector_bitmap_maps_all_readback_bytes_to_pair_labels() {
        let body = [0x00_u8; 24];
        let map = build_raw_packet_map(RawPacketTab::Query75, &query_bytes(0x0b, 0x03, &body));

        assert_eq!(
            entry(&map, "Mix1 CH01/CH02 link selector").ranges[0].report,
            0x10..0x11
        );
        assert_eq!(
            entry(&map, "Mix1 CH15/CH16 link selector").ranges[0].report,
            0x17..0x18
        );
        assert_eq!(
            entry(&map, "Mix2 CH01/CH02 link selector").ranges[0].report,
            0x20..0x21
        );
        assert_eq!(
            entry(&map, "Mix2 CH15/CH16 link selector").ranges[0].report,
            0x27..0x28
        );
        assert_eq!(
            map.entries()
                .iter()
                .filter(|item| item.coverage == Coverage::Readback)
                .count(),
            24
        );
    }

    #[test]
    fn startup_pan_state_maps_level_and_complete_state_pairs() {
        let mut body = vec![0_u8; 34];
        body[0] = 0x00;
        let map = build_raw_packet_map(RawPacketTab::Query75, &query_bytes(0x04, 0x00, &body));

        assert_eq!(entry(&map, "Mix1 CH01 level").ranges[0].report, 0x12..0x13);
        assert_eq!(
            entry(&map, "Mix1 CH01 pan/mute/solo state").ranges[0].report,
            0x13..0x14
        );
        assert_eq!(
            entry(&map, "Mix1 CH16 pan/mute/solo state").ranges[0].report,
            0x31..0x32
        );
        assert!(entry(&map, "Mix1 CH01 pan/mute/solo state")
            .note
            .contains("pan, mute, and solo"));
    }

    #[test]
    fn full_strip_readback_maps_both_surfaces_in_order() {
        let body = [0_u8; 64];
        let map = build_raw_packet_map(RawPacketTab::Query75, &query_bytes(0x18, 0x00, &body));

        assert_eq!(entry(&map, "Mix1 CH01 level").ranges[0].report, 0x10..0x11);
        assert_eq!(
            entry(&map, "Mix1 CH16 pan/mute/solo state").ranges[0].report,
            0x2f..0x30
        );
        assert_eq!(entry(&map, "Mix2 CH01 level").ranges[0].report, 0x30..0x31);
        assert_eq!(
            entry(&map, "Mix2 CH16 pan/mute/solo state").ranges[0].report,
            0x4f..0x50
        );
    }

    #[test]
    fn metadata_indexed_table_and_quad_state_use_exact_body_ranges() {
        let metadata_body = b"Product\0Serial\0Hardware\0";
        let metadata = build_raw_packet_map(
            RawPacketTab::Query75,
            &query_bytes(0x01, 0x00, metadata_body),
        );
        assert_eq!(
            entry(&metadata, "product name").ranges[0].payload,
            Some(0..7)
        );
        assert_eq!(entry(&metadata, "serial").ranges[0].payload, Some(8..14));
        assert_eq!(
            entry(&metadata, "hardware version").ranges[0].payload,
            Some(15..23)
        );

        let indexed =
            build_raw_packet_map(RawPacketTab::Query75, &query_bytes(0x15, 0x00, &[0_u8; 64]));
        assert_eq!(
            entry(&indexed, "indexed entry 31").ranges[0].report,
            0x4e..0x50
        );

        let quad = build_raw_packet_map(
            RawPacketTab::Query75,
            &query_bytes(0x17, 0x00, &[1, 2, 3, 4]),
        );
        assert_eq!(
            entry(&quad, "quad state byte 3").ranges[0].payload,
            Some(3..4)
        );
    }

    #[test]
    fn unknown_query_and_status_body_keep_identifiers_and_unmapped_body() {
        let unknown =
            build_raw_packet_map(RawPacketTab::Query75, &query_bytes(0xfe, 0xaa, &[1, 2, 3]));
        assert_eq!(entry(&unknown, "query ID").ranges[0].report, 0x08..0x09);
        assert_eq!(entry(&unknown, "sub-ID").ranges[0].report, 0x0c..0x0d);
        assert_eq!(
            entry(&unknown, "unresolved query body").coverage,
            Coverage::Unmapped
        );

        let status =
            build_raw_packet_map(RawPacketTab::Query75, &query_bytes(0x11, 0x00, &[1, 2, 3]));
        assert_eq!(
            entry(&status, "status/capability body").coverage,
            Coverage::Unmapped
        );
        assert_eq!(
            entry(&status, "status/capability body").domain,
            RawDomain::Status
        );
    }

    #[test]
    fn request_padding_is_guarded_by_startup_query_pairs() {
        let known = build_raw_packet_map(RawPacketTab::Query74, &query_bytes(0x03, 0x05, &[]));
        assert_eq!(
            entry(&known, "known request padding").coverage,
            Coverage::Padding
        );
        assert_eq!(
            entry(&known, "known request padding").ranges[0].report,
            0x10..0x140
        );

        let unknown = build_raw_packet_map(RawPacketTab::Query74, &query_bytes(0xfe, 0xaa, &[]));
        assert_eq!(
            entry(&unknown, "unresolved query body").coverage,
            Coverage::Unmapped
        );
    }

    #[test]
    fn auxiliary_payload_is_unmapped_and_notification_tail_is_padding() {
        let auxiliary = build_raw_packet_map(RawPacketTab::Auxiliary, &[0; 320]);
        let payload = entry(&auxiliary, "unmapped auxiliary payload");
        assert_eq!(payload.coverage, Coverage::Unmapped);
        assert_eq!(payload.ranges[0].report, 0x10..0x140);
        assert_eq!(payload.ranges[0].payload, Some(0..0x130));

        let notification = build_raw_packet_map(RawPacketTab::DeviceNotification, &[0; 320]);
        let padding = entry(&notification, "fixed notification padding");
        assert_eq!(padding.coverage, Coverage::Padding);
        assert_eq!(padding.ranges[0].report, 0x06..0x140);
        assert_eq!(
            notification.classify(0x06, RawMapScope::All).coverage,
            Coverage::Padding
        );
    }

    #[test]
    fn raw_map_entries_are_ordered_by_report_offset_then_coverage() {
        let map = build_raw_packet_map(RawPacketTab::State73, &[0; 320]);
        let starts = map.entries().iter().map(first_offset).collect::<Vec<_>>();
        assert!(starts.windows(2).all(|window| window[0] <= window[1]));
        assert!(map
            .entries_for_scope(RawMapScope::Unmapped)
            .iter()
            .all(|item| item.coverage == Coverage::Unmapped));
    }
}
