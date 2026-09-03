//! Pure checked helpers for normalized profile frame encoding.

use crate::driver::{ControlValue, DriverError};
use crate::profile::{
    FrameEndian, FrameOperation, ReadbackDefinition, RuntimeFrame, RuntimeProfile,
};
use crate::QueryRequest;

pub(crate) fn report_size(profile: &RuntimeProfile) -> Result<usize, DriverError> {
    profile
        .transport
        .report_size
        .map(usize::from)
        .ok_or_else(|| DriverError::InvalidAction("profile transport has no report size".into()))
}

pub(crate) fn validate_operations(
    frame: &RuntimeFrame,
    report_size: usize,
) -> Result<(), DriverError> {
    use std::collections::HashSet;
    let mut semantics = HashSet::new();
    let mut fixed_offsets = HashSet::new();
    for operation in &frame.operations {
        let end = match operation {
            FrameOperation::FixedByte { offset, .. } | FrameOperation::BitField { offset, .. } => {
                usize::from(*offset) + 1
            }
            FrameOperation::Scalar { offset, width, .. } => {
                usize::from(*offset) + usize::from(*width)
            }
            FrameOperation::Indexed {
                base,
                stride,
                width,
                max_index,
                ..
            } => {
                let max = max_index.ok_or_else(|| {
                    DriverError::InvalidAction(format!(
                        "frame {} indexed operation has no finite domain",
                        frame.id
                    ))
                })?;
                if *stride == 0 || *width == 0 {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} indexed operation has zero stride/width",
                        frame.id
                    )));
                }
                usize::from(*base)
                    .checked_add(usize::from(*stride) * usize::from(max))
                    .and_then(|offset| offset.checked_add(usize::from(*width)))
                    .ok_or_else(|| {
                        DriverError::InvalidAction(format!(
                            "frame {} indexed operation span overflows",
                            frame.id
                        ))
                    })?
            }
            FrameOperation::PairIndex {
                base,
                stride,
                width,
                max_index,
                ..
            } => {
                let max = max_index.ok_or_else(|| {
                    DriverError::InvalidAction(format!(
                        "frame {} pair operation has no finite domain",
                        frame.id
                    ))
                })?;
                if *stride == 0
                    || *width != 1
                    || u32::from(*stride) * u32::from(max) > u32::from(u8::MAX)
                {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} pair operation value exceeds byte",
                        frame.id
                    )));
                }
                usize::from(*base) + 1
            }
            FrameOperation::AllowedValues { .. } | FrameOperation::UncompiledFormula { .. } => 0,
        };
        if end > report_size {
            return Err(DriverError::InvalidAction(format!(
                "frame {} operation span {end} exceeds report size {report_size}",
                frame.id
            )));
        }
        let semantic = match operation {
            FrameOperation::Scalar {
                field,
                offset,
                width,
                endian,
            } => {
                if field.trim().is_empty() {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} scalar has no semantic field",
                        frame.id
                    )));
                }
                if !(1..=4).contains(width) {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} scalar {field:?} width {width} is outside executable range 1..=4",
                        frame.id
                    )));
                }
                if (*width == 1 && *endian != FrameEndian::NotApplicable)
                    || (*width > 1 && *endian == FrameEndian::NotApplicable)
                {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} scalar {field:?} width {width} has invalid endianness {endian:?}",
                        frame.id
                    )));
                }
                Some(field.clone())
            }
            FrameOperation::BitField {
                field, mask, shift, ..
            } => {
                if field.trim().is_empty() {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} bit field has no semantic name",
                        frame.id
                    )));
                }
                if *mask == 0 {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} bit field {field:?} has zero mask",
                        frame.id
                    )));
                }
                if *shift >= 8 {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} bit field {field:?} shift {shift} is outside 0..8",
                        frame.id
                    )));
                }
                if (*mask >> *shift) == 0 {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} bit field {field:?} mask {mask:#04x} is below shift {shift}",
                        frame.id
                    )));
                }
                Some(field.clone())
            }
            FrameOperation::Indexed { index_field, .. } => {
                if index_field.trim().is_empty() {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} indexed field has no semantic name",
                        frame.id
                    )));
                }
                Some(format!("indexed:{index_field}"))
            }
            FrameOperation::PairIndex { pair_field, .. } => {
                if pair_field.trim().is_empty() {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} pair field has no semantic name",
                        frame.id
                    )));
                }
                Some(format!("pair_index:{pair_field}"))
            }
            FrameOperation::FixedByte { offset, .. } => {
                if !fixed_offsets.insert(*offset) {
                    return Err(DriverError::InvalidAction(format!(
                        "frame {} has ambiguous fixed offset {offset}",
                        frame.id
                    )));
                }
                None
            }
            FrameOperation::AllowedValues { .. } | FrameOperation::UncompiledFormula { .. } => None,
        };
        if let Some(key) = semantic {
            if !semantics.insert(key.clone()) {
                return Err(DriverError::InvalidAction(format!(
                    "frame {} has ambiguous semantic {}",
                    frame.id, key
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: &str, operations: Vec<FrameOperation>) -> RuntimeFrame {
        RuntimeFrame {
            id: id.into(),
            kind: "test".into(),
            status: "confirmed".into(),
            report_size: Some(8),
            operations,
            metadata: String::new(),
        }
    }

    #[test]
    fn rejects_duplicate_scalar_field_even_at_distinct_offsets() {
        let result = validate_operations(
            &frame(
                "state_report",
                vec![
                    FrameOperation::Scalar {
                        field: "status_byte".into(),
                        offset: 1,
                        width: 1,
                        endian: FrameEndian::NotApplicable,
                    },
                    FrameOperation::Scalar {
                        field: "status_byte".into(),
                        offset: 2,
                        width: 1,
                        endian: FrameEndian::NotApplicable,
                    },
                ],
            ),
            8,
        );
        assert!(
            result.is_err(),
            "duplicate scalar semantic must be rejected"
        );
    }

    #[test]
    fn rejects_duplicate_scalar_and_bit_field_semantic_name() {
        let result = validate_operations(
            &frame(
                "state_report",
                vec![
                    FrameOperation::Scalar {
                        field: "status_byte".into(),
                        offset: 1,
                        width: 1,
                        endian: FrameEndian::NotApplicable,
                    },
                    FrameOperation::BitField {
                        field: "status_byte".into(),
                        offset: 2,
                        mask: 0x01,
                        shift: 0,
                    },
                ],
            ),
            8,
        );
        assert!(
            result.is_err(),
            "scalar and bit-field semantic names must share namespace"
        );
    }

    #[test]
    fn rejects_duplicate_bit_field_with_distinct_masks() {
        let result = validate_operations(
            &frame(
                "state_report",
                vec![
                    FrameOperation::BitField {
                        field: "mask".into(),
                        offset: 1,
                        mask: 0x04,
                        shift: 2,
                    },
                    FrameOperation::BitField {
                        field: "mask".into(),
                        offset: 1,
                        mask: 0x08,
                        shift: 3,
                    },
                ],
            ),
            8,
        );
        assert!(result.is_err(), "duplicate bit semantic must be rejected");
    }

    #[test]
    fn zen_go_sparse_safe_query_codec_preserves_frame_and_rejects_absent_pair() {
        let profile = crate::load_profile_pack(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/device/generated_profiles.json"
        )))
        .expect("generated profile pack")
        .profiles
        .into_iter()
        .find(|entry| entry.profile.identity.pid == 0xa015)
        .expect("Zen Go profile")
        .profile;
        let readback = profile.readback.as_ref().expect("readback metadata");
        let frame = super::encode_query(
            &profile,
            readback,
            QueryRequest {
                query_id: 0x04,
                sub_id: 3,
            },
        )
        .expect("explicitly safe query");
        assert_eq!(&frame[0..8], &[0x74, 0, 0, 0, 0x10, 0, 0, 0]);
        assert_eq!(frame[8], 0x04);
        assert_eq!(frame[12], 3);
        assert!(matches!(
            super::encode_query(
                &profile,
                readback,
                QueryRequest {
                    query_id: 0x04,
                    sub_id: 4,
                },
            ),
            Err(DriverError::UnsupportedAction(_))
        ));
    }
}

pub(crate) fn allocate(
    profile: &RuntimeProfile,
    frame: &RuntimeFrame,
) -> Result<Vec<u8>, DriverError> {
    if !is_confirmed(&frame.status) {
        return Err(DriverError::UnsupportedAction(format!(
            "unconfirmed frame {}",
            frame.id
        )));
    }
    validate_operations(frame, report_size(profile)?)?;
    if frame
        .operations
        .iter()
        .any(|operation| matches!(operation, FrameOperation::UncompiledFormula { .. }))
    {
        return Err(DriverError::UnsupportedAction(format!(
            "uncompiled formula in frame {}",
            frame.id
        )));
    }
    let size = report_size(profile)?;
    if frame.report_size.map(usize::from).unwrap_or(size) != size {
        return Err(DriverError::InvalidAction(format!(
            "frame {} report size differs from transport",
            frame.id
        )));
    }
    let mut bytes = vec![0; size];
    for operation in &frame.operations {
        if let FrameOperation::FixedByte { offset, value } = operation {
            let slot = bytes.get_mut(usize::from(*offset)).ok_or_else(|| {
                DriverError::InvalidAction(format!(
                    "frame {} fixed offset {offset} outside report",
                    frame.id
                ))
            })?;
            *slot = *value;
        }
    }
    Ok(bytes)
}

fn scalar(frame: &RuntimeFrame, field: &str) -> Result<(u16, u8, FrameEndian), DriverError> {
    let mut matches = frame
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FrameOperation::Scalar {
                field: candidate,
                offset,
                width,
                endian,
            } if candidate == field => Some((*offset, *width, *endian)),
            _ => None,
        });
    let result = matches.next().ok_or_else(|| {
        DriverError::InvalidAction(format!(
            "frame {} missing scalar semantic {field}",
            frame.id
        ))
    })?;
    if matches.next().is_some() {
        return Err(DriverError::InvalidAction(format!(
            "frame {} ambiguous scalar semantic {field}",
            frame.id
        )));
    }
    Ok(result)
}

pub(crate) fn scalar_offset(frame: &RuntimeFrame, field: &str) -> Result<u16, DriverError> {
    scalar(frame, field).map(|value| value.0)
}

pub(crate) fn write_scalar(
    frame: &RuntimeFrame,
    bytes: &mut [u8],
    field: &str,
    value: i32,
) -> Result<(), DriverError> {
    let (offset, width, endian) = scalar(frame, field)?;
    let width = usize::from(width);
    let bits = width.checked_mul(8).ok_or_else(|| {
        DriverError::InvalidAction(format!("frame {} scalar {field} width overflow", frame.id))
    })?;
    if bits == 0 || bits > 32 {
        return Err(DriverError::InvalidAction(format!(
            "frame {} scalar {field} unsupported width {width}",
            frame.id
        )));
    }
    let signed_min = -(1_i64 << (bits - 1));
    let unsigned_max = (1_i64 << bits) - 1;
    let numeric = i64::from(value);
    if numeric < signed_min || numeric > unsigned_max {
        return Err(DriverError::InvalidAction(format!(
            "{field} value {value} does not fit width {width}"
        )));
    }
    let encoded = (numeric as i128) & ((1_i128 << bits) - 1);
    let range = usize::from(offset)..usize::from(offset) + width;
    let target = bytes
        .get_mut(range)
        .ok_or_else(|| DriverError::InvalidAction(format!("{field} scalar span outside report")))?;
    for (index, slot) in target.iter_mut().enumerate() {
        let shift = match endian {
            FrameEndian::NotApplicable if width == 1 => 0,
            FrameEndian::Little => index * 8,
            FrameEndian::Big => (width - 1 - index) * 8,
            _ => {
                return Err(DriverError::InvalidAction(format!(
                    "{field} scalar has invalid declared endianness"
                )))
            }
        };
        *slot = ((encoded >> shift) & 0xff) as u8;
    }
    Ok(())
}

pub(crate) fn write_bit_field(
    frame: &RuntimeFrame,
    bytes: &mut [u8],
    field: &str,
    value: i32,
) -> Result<(), DriverError> {
    let mut matches = frame
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FrameOperation::BitField {
                field: candidate,
                offset,
                mask,
                shift,
            } if candidate == field => Some((*offset, *mask, *shift)),
            _ => None,
        });
    let (offset, mask, shift) = matches.next().ok_or_else(|| {
        DriverError::InvalidAction(format!("frame {} missing bit semantic {field}", frame.id))
    })?;
    if matches.next().is_some() {
        return Err(DriverError::InvalidAction(format!(
            "frame {} ambiguous bit semantic {field}",
            frame.id
        )));
    }
    let maximum = i32::from(mask >> shift);
    if !(0..=maximum).contains(&value) {
        return Err(DriverError::InvalidAction(format!(
            "bit field {field} value {value} outside 0..={maximum}"
        )));
    }
    let slot = bytes.get_mut(usize::from(offset)).ok_or_else(|| {
        DriverError::InvalidAction(format!("bit field {field} offset outside report"))
    })?;
    *slot = (*slot & !mask) | (((value as u8) << shift) & mask);
    Ok(())
}

pub(crate) fn write_indexed_bytes(
    frame: &RuntimeFrame,
    bytes: &mut [u8],
    field: &str,
    index: u16,
    value: &[u8],
) -> Result<(), DriverError> {
    let mut matches = frame
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FrameOperation::Indexed {
                base,
                stride,
                index_field,
                width,
                max_index,
            } if index_field == field => Some((*base, *stride, *width, *max_index)),
            _ => None,
        });
    let (base, stride, width, max_index) = matches.next().ok_or_else(|| {
        DriverError::InvalidAction(format!(
            "frame {} missing indexed semantic {field}",
            frame.id
        ))
    })?;
    if matches.next().is_some() {
        return Err(DriverError::InvalidAction(format!(
            "frame {} ambiguous indexed semantic {field}",
            frame.id
        )));
    }
    let max_index = max_index.ok_or_else(|| {
        DriverError::InvalidAction(format!("indexed semantic {field} has no finite domain"))
    })?;
    if index > max_index || value.len() != usize::from(width) {
        return Err(DriverError::InvalidAction(format!(
            "indexed semantic {field} index/width outside domain"
        )));
    }
    let offset = u32::from(base)
        .checked_add(u32::from(stride) * u32::from(index))
        .ok_or_else(|| DriverError::InvalidAction(format!("indexed semantic {field} overflow")))?;
    let start = usize::try_from(offset)
        .map_err(|_| DriverError::InvalidAction(format!("indexed semantic {field} overflow")))?;
    bytes
        .get_mut(start..start + value.len())
        .ok_or_else(|| {
            DriverError::InvalidAction(format!("indexed semantic {field} outside report"))
        })?
        .copy_from_slice(value);
    Ok(())
}

pub(crate) fn write_pair_index(
    frame: &RuntimeFrame,
    bytes: &mut [u8],
    field: &str,
    pair: u16,
) -> Result<(), DriverError> {
    let mut matches = frame
        .operations
        .iter()
        .filter_map(|operation| match operation {
            FrameOperation::PairIndex {
                base,
                stride,
                pair_field,
                width,
                max_index,
            } if pair_field == field => Some((*base, *stride, *width, *max_index)),
            _ => None,
        });
    let (base, stride, width, max_index) = matches.next().ok_or_else(|| {
        DriverError::InvalidAction(format!("frame {} missing pair semantic {field}", frame.id))
    })?;
    if matches.next().is_some() {
        return Err(DriverError::InvalidAction(format!(
            "frame {} ambiguous pair semantic {field}",
            frame.id
        )));
    }
    if pair
        > max_index.ok_or_else(|| {
            DriverError::InvalidAction(format!("pair semantic {field} has no finite domain"))
        })?
        || width != 1
    {
        return Err(DriverError::InvalidAction(format!(
            "pair semantic {field} outside domain"
        )));
    }
    let value = u32::from(stride)
        .checked_mul(u32::from(pair))
        .ok_or_else(|| DriverError::InvalidAction(format!("pair semantic {field} overflow")))?;
    *bytes.get_mut(usize::from(base)).ok_or_else(|| {
        DriverError::InvalidAction(format!("pair semantic {field} outside report"))
    })? = u8::try_from(value)
        .map_err(|_| DriverError::InvalidAction(format!("pair semantic {field} exceeds byte")))?;
    Ok(())
}

pub(crate) fn value_i32(
    value: ControlValue,
    value_type: &str,
    range: Option<(i32, i32)>,
    allowed: &[(i32, String)],
    field: &str,
) -> Result<i32, DriverError> {
    let value = match (value_type.to_ascii_lowercase().as_str(), value) {
        ("bool", ControlValue::Bool(value)) => i32::from(value),
        ("enum", ControlValue::Enum(value)) | ("enum", ControlValue::Int(value)) => value,
        (_, ControlValue::Int(value)) | (_, ControlValue::Enum(value)) => value,
        _ => {
            return Err(DriverError::InvalidAction(format!(
                "{field} control value type mismatch"
            )))
        }
    };
    if let Some((minimum, maximum)) = range {
        if !(minimum..=maximum).contains(&value) {
            return Err(DriverError::InvalidAction(format!(
                "{field} value {value} outside {minimum}..={maximum}"
            )));
        }
    }
    if !allowed.is_empty() && !allowed.iter().any(|(candidate, _)| *candidate == value) {
        return Err(DriverError::InvalidAction(format!(
            "{field} enum value {value} is not allowed"
        )));
    }
    Ok(value)
}

pub(crate) fn encode_query(
    profile: &RuntimeProfile,
    readback: &ReadbackDefinition,
    query: QueryRequest,
) -> Result<Vec<u8>, DriverError> {
    let explicitly_safe = readback
        .safe_queries
        .iter()
        .any(|safe| safe.category == query.query_id && safe.index == query.sub_id);
    let bounded = readback.category_counts.iter().any(|category| {
        category.category == query.query_id && u16::from(query.sub_id) < category.count
    });
    if !explicitly_safe && !bounded {
        return Err(DriverError::UnsupportedAction(
            "query pair is not in profile readback safety data".into(),
        ));
    }
    let mut bytes = vec![0; report_size(profile)?];
    bytes[0] = readback.request_magic;
    bytes
        .get_mut(4..8)
        .ok_or_else(|| DriverError::InvalidAction("query subcommand field truncated".into()))?
        .copy_from_slice(&readback.request_subcommand.to_le_bytes());
    *bytes
        .get_mut(usize::from(readback.category_offset))
        .ok_or_else(|| {
            DriverError::InvalidAction("query category offset outside report".into())
        })? = query.query_id;
    *bytes
        .get_mut(usize::from(readback.index_offset))
        .ok_or_else(|| DriverError::InvalidAction("query index offset outside report".into()))? =
        query.sub_id;
    Ok(bytes)
}

pub(crate) fn fixed_byte(frame: &RuntimeFrame, offset: u16) -> Option<u8> {
    frame
        .operations
        .iter()
        .find_map(|operation| match operation {
            FrameOperation::FixedByte {
                offset: candidate,
                value,
            } if *candidate == offset => Some(*value),
            _ => None,
        })
}

pub(crate) fn is_confirmed(status: &str) -> bool {
    status.trim().to_ascii_lowercase().starts_with("confirm")
}
