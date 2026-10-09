//! EBML Variable Size Integer (VINT) parser per RFC 8794.

use crate::DemuxError;

/// Reads an EBML Element ID from `data`.
///
/// In RFC 8794 §5, Element IDs preserve the length descriptor bits.
/// Returns `(element_id, bytes_consumed)`.
pub fn read_vint_id(data: &[u8], offset: u64) -> Result<(u32, usize), DemuxError> {
    if data.is_empty() {
        return Err(DemuxError::UnexpectedEof { offset });
    }

    let first = data[0];
    let len = first.leading_zeros() as usize + 1;
    if len > 4 {
        return Err(DemuxError::InvalidBox {
            offset,
            detail: format!("EBML element ID length {len} exceeds 4 bytes"),
        });
    }

    if data.len() < len {
        return Err(DemuxError::UnexpectedEof { offset });
    }

    let mut id = 0u32;
    for &b in &data[..len] {
        id = (id << 8) | b as u32;
    }

    Ok((id, len))
}

/// Reads an EBML Element Data Size from `data`.
///
/// In RFC 8794 §6, Data Sizes mask out the leading length descriptor bit.
/// If all value bits are 1, it represents an unknown/undefined size (`None`).
/// Returns `(Option<size>, bytes_consumed)`.
pub fn read_vint_size(data: &[u8], offset: u64) -> Result<(Option<u64>, usize), DemuxError> {
    if data.is_empty() {
        return Err(DemuxError::UnexpectedEof { offset });
    }

    let first = data[0];
    let len = first.leading_zeros() as usize + 1;
    if len > 8 {
        return Err(DemuxError::InvalidBox {
            offset,
            detail: format!("EBML data size length {len} exceeds 8 bytes"),
        });
    }

    if data.len() < len {
        return Err(DemuxError::UnexpectedEof { offset });
    }

    // Mask out the length descriptor bit from the first byte
    let mask = (1u8 << (8 - len)) - 1;
    let mut val = (first & mask) as u64;

    for &b in &data[1..len] {
        val = (val << 8) | b as u64;
    }

    // Check for unknown size (all value bits == 1)
    let unknown_marker = (1u64 << (7 * len)) - 1;
    if val == unknown_marker {
        Ok((None, len))
    } else {
        Ok((Some(val), len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vint_id() {
        // 0x1A45DFA3 -> EBML Header (4 bytes, leading 0001...)
        let data = [0x1A, 0x45, 0xDF, 0xA3];
        let (id, len) = read_vint_id(&data, 0).unwrap();
        assert_eq!(id, 0x1A45DFA3);
        assert_eq!(len, 4);

        // 0xAE -> TrackEntry (1 byte, leading 1...)
        let data_ae = [0xAE, 0x00];
        let (id_ae, len_ae) = read_vint_id(&data_ae, 0).unwrap();
        assert_eq!(id_ae, 0xAE);
        assert_eq!(len_ae, 1);
    }

    #[test]
    fn test_vint_size() {
        // 1 byte: 0x85 -> len 1, size = 5 (0x85 & 0x7F = 5)
        let (sz, len) = read_vint_size(&[0x85], 0).unwrap();
        assert_eq!(sz, Some(5));
        assert_eq!(len, 1);

        // 2 bytes: 0x40 0x10 -> len 2, size = 16
        let (sz2, len2) = read_vint_size(&[0x40, 0x10], 0).unwrap();
        assert_eq!(sz2, Some(16));
        assert_eq!(len2, 2);

        // Unknown size 1 byte: 0xFF
        let (sz_unk, len_unk) = read_vint_size(&[0xFF], 0).unwrap();
        assert_eq!(sz_unk, None);
        assert_eq!(len_unk, 1);
    }
}
