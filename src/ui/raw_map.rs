use std::ops::Range;

use antelope_protocol::{
    control_panel_startup_queries, FrameOperation, QueryResponse, RuntimeMeterTarget,
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

/// Coverage classification used by the RAW view's semantic map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Coverage {
    Used,
    Readback,
    Observed,
    Parser,
    Unmapped,
    Padding,
}

impl Coverage {
    pub(crate) const fn rank(self) -> u8 {
        match self {
            Self::Used => 6,
            Self::Readback => 5,
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
            Coverage::Parser,
            format!("{frame_id} magic 0x{value:02x}"),
            "Profile-declared report discriminator.",
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
        add_bounded_entry(
            entries,
            domain,
            profile_scope(domain),
            Coverage::Observed,
            label,
            format!(
                "Profile-owned observed meter mapping ({}). One lane only; no stereo L/R inference. {}",
                mapping.status_text, mapping.evidence
            ),
            vec![profile_range(mapping.offset, 1, payload_offset)],
            report_len,
        );
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
        "Decoded through ClockSource.",
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

    fn builtin_profile(pid: u16) -> antelope_protocol::RuntimeProfile {
        crate::device::ProfileCatalog::builtin()
            .find(0x23e5, pid)
            .unwrap_or_else(|| panic!("missing built-in profile for pid {pid:#06x}"))
            .profile()
            .clone()
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

    #[test]
    fn active_orion_profile_maps_four_single_lane_mix_masters_at_full_offsets() {
        let profile = builtin_profile(0xa221);
        let map =
            build_raw_packet_map_for_profile(RawPacketTab::State73, &[0; 320], Some(&profile));
        let observed = map
            .entries()
            .iter()
            .filter(|entry| {
                entry.coverage == Coverage::Observed
                    && entry.domain == RawDomain::Mixer
                    && entry.label.contains("master meter")
            })
            .collect::<Vec<_>>();

        assert_eq!(observed.len(), 4);
        assert_eq!(
            observed
                .iter()
                .map(|entry| entry.ranges[0].report.start)
                .collect::<Vec<_>>(),
            vec![157, 158, 159, 160]
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
                || entry.label.contains("output meter")));
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
                            && mapping.target == antelope_protocol::RuntimeMeterTarget::MixMaster
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
