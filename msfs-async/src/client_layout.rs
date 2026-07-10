use crate::{Error, Result};

pub(crate) type FieldDefinition = (usize, usize, u32, f32);
pub(crate) type SdkDefinition = (usize, u32, f32);

/// Validate field ranges and add explicit entries for every padding range.
pub(crate) fn plan(
    mut fields: Vec<FieldDefinition>,
    type_size: usize,
) -> Result<Vec<SdkDefinition>> {
    if type_size > u32::MAX as usize
        || fields
            .iter()
            .any(|(offset, size, _, _)| offset.checked_add(*size).is_none_or(|end| end > type_size))
    {
        return Err(Error::InvalidClientDataDefinition);
    }
    fields.sort_by_key(|(offset, _, _, _)| *offset);

    let mut definitions = Vec::with_capacity(fields.len() * 2 + 1);
    let mut cursor = 0;
    for (offset, size, sdk_size_or_type, epsilon) in fields {
        if offset < cursor {
            return Err(Error::InvalidClientDataDefinition);
        }
        if offset > cursor {
            definitions.push((cursor, (offset - cursor) as u32, 0.0));
        }
        definitions.push((offset, sdk_size_or_type, epsilon));
        cursor = offset + size;
    }
    if cursor < type_size {
        definitions.push((cursor, (type_size - cursor) as u32, 0.0));
    }
    Ok(definitions)
}

/// Encode declared fields into a reusable zero-initialized buffer, leaving padding zero.
///
/// # Safety
///
/// Every supplied range must identify initialized bytes of `data`, not padding.
pub(crate) unsafe fn encode_into<T>(
    data: &T,
    fields: &[FieldDefinition],
    bytes: &mut Vec<u8>,
) -> Result<()> {
    bytes.resize(std::mem::size_of::<T>(), 0);
    bytes.fill(0);
    let source = std::ptr::from_ref(data).cast::<u8>();
    for (offset, size, _, _) in fields {
        let end = offset
            .checked_add(*size)
            .ok_or(Error::InvalidClientDataDefinition)?;
        if end > bytes.len() {
            return Err(Error::InvalidClientDataDefinition);
        }
        // SAFETY: required by this function's contract; the destination range
        // was checked above and cannot overlap the source object.
        unsafe {
            std::ptr::copy_nonoverlapping(
                source.add(*offset),
                bytes.as_mut_ptr().add(*offset),
                *size,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_interior_and_trailing_padding() {
        let definitions = plan(
            vec![(0, 1, (-1_i32) as u32, 0.0), (4, 4, (-3_i32) as u32, 0.5)],
            12,
        )
        .unwrap();

        assert_eq!(
            definitions,
            vec![
                (0, (-1_i32) as u32, 0.0),
                (1, 3, 0.0),
                (4, (-3_i32) as u32, 0.5),
                (8, 4, 0.0),
            ]
        );
    }

    #[test]
    fn rejects_overlapping_and_out_of_bounds_fields() {
        assert_eq!(
            plan(vec![(0, 4, 4, 0.0), (2, 4, 4, 0.0)], 8),
            Err(Error::InvalidClientDataDefinition)
        );
        assert_eq!(
            plan(vec![(8, 1, 1, 0.0)], 8),
            Err(Error::InvalidClientDataDefinition)
        );
    }

    #[test]
    fn encoding_keeps_padding_zero() {
        #[repr(C)]
        struct Padded {
            byte: u8,
            word: u32,
        }

        let value = Padded {
            byte: 7,
            word: 0x1122_3344,
        };
        let fields = vec![
            (0, 1, (-1_i32) as u32, 0.0),
            (std::mem::offset_of!(Padded, word), 4, (-3_i32) as u32, 0.0),
        ];
        let mut bytes = Vec::new();
        // SAFETY: both ranges are initialized fields of `value`.
        unsafe { encode_into(&value, &fields, &mut bytes) }.unwrap();

        assert_eq!(bytes[0], 7);
        assert_eq!(&bytes[1..4], &[0, 0, 0]);
        assert_eq!(&bytes[4..8], &0x1122_3344_u32.to_ne_bytes());

        let capacity = bytes.capacity();
        // SAFETY: both ranges are initialized fields of `value`.
        unsafe { encode_into(&value, &fields, &mut bytes) }.unwrap();
        assert_eq!(bytes.capacity(), capacity);
    }
}
