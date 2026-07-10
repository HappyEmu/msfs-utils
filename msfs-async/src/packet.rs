use crate::{Error, Result};

/// Copy a value from a validated packet byte range.
///
/// Callers remain responsible for ensuring every bit pattern is valid for
/// `T`; this function validates only the byte range and alignment-independent
/// copy.
pub(crate) fn decode_value<T: Copy>(packet: &[u8], offset: usize) -> Result<T> {
    let expected = offset
        .checked_add(std::mem::size_of::<T>())
        .ok_or(Error::InvalidPacket {
            expected: usize::MAX,
            actual: packet.len(),
        })?;
    if packet.len() < expected {
        return Err(Error::InvalidPacket {
            expected,
            actual: packet.len(),
        });
    }

    // SAFETY: the bounds check proves that a complete `T` is present, the
    // caller supplies a type for which every bit pattern is valid, and
    // `read_unaligned` does not require the packet offset to satisfy T's
    // alignment.
    Ok(unsafe { packet.as_ptr().add(offset).cast::<T>().read_unaligned() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_an_unaligned_value() {
        let mut packet = vec![0_u8; 9];
        packet[1..9].copy_from_slice(&42_u64.to_ne_bytes());

        assert_eq!(decode_value::<u64>(&packet, 1).unwrap(), 42);
    }

    #[test]
    fn rejects_a_truncated_value() {
        let error = decode_value::<u64>(&[0; 8], 1).unwrap_err();
        assert_eq!(
            error,
            Error::InvalidPacket {
                expected: 9,
                actual: 8,
            }
        );
    }

    #[test]
    fn rejects_offset_overflow() {
        let error = decode_value::<u64>(&[], usize::MAX).unwrap_err();
        assert_eq!(
            error,
            Error::InvalidPacket {
                expected: usize::MAX,
                actual: 0,
            }
        );
    }
}
