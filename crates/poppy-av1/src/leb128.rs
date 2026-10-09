//! LEB128 (Little Endian Base 128) variable length integer parser.
//!
//! AV1 specification §4.10.5: `leb128()` syntax.

use thiserror::Error;

/// Error encountered while parsing LEB128 encoded integers.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Leb128Error {
    #[error("unexpected end of data while parsing leb128 integer")]
    UnexpectedEof,
    #[error("leb128 integer exceeded maximum allowed 8 bytes")]
    TooManyBytes,
}

/// Reads an unsigned LEB128 integer from a byte slice.
///
/// Returns `(value, bytes_read)`.
pub fn read_leb128(data: &[u8]) -> Result<(u64, usize), Leb128Error> {
    let mut value = 0u64;
    let mut shift = 0;

    for (i, &byte) in data.iter().enumerate() {
        if i >= 8 {
            return Err(Leb128Error::TooManyBytes);
        }

        value |= ((byte & 0x7F) as u64) << shift;
        if (byte & 0x80) == 0 {
            return Ok((value, i + 1));
        }
        shift += 7;
    }

    Err(Leb128Error::UnexpectedEof)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leb128_single_byte() {
        assert_eq!(read_leb128(&[0x00]).unwrap(), (0, 1));
        assert_eq!(read_leb128(&[0x7F]).unwrap(), (127, 1));
        assert_eq!(read_leb128(&[0x20]).unwrap(), (32, 1));
    }

    #[test]
    fn test_leb128_multi_byte() {
        // 624485 = 0xE5 0x8E 0x26
        let data = [0xE5, 0x8E, 0x26];
        assert_eq!(read_leb128(&data).unwrap(), (624485, 3));
    }

    #[test]
    fn test_leb128_eof() {
        assert_eq!(read_leb128(&[0x80, 0x80]), Err(Leb128Error::UnexpectedEof));
    }
}
