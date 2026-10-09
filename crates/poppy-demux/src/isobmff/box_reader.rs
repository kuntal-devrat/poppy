//! ISOBMFF box header reader.
//!
//! Parses the 8-byte (or 16-byte extended) header that begins every box in an
//! ISOBMFF container. This is the lowest-level building block of the MP4 parser.
//!
//! # Box Header Layout (ISO 14496-12, §4.2)
//!
//! ```text
//! Standard (8 bytes):
//! ┌──────────────────┬──────────────────┐
//! │   size (u32 BE)  │  type (4 × u8)   │
//! └──────────────────┴──────────────────┘
//!
//! Extended (16 bytes, when size == 1):
//! ┌──────────────────┬──────────────────┬──────────────────────────────────┐
//! │   size=1 (u32)   │  type (4 × u8)   │         largesize (u64 BE)       │
//! └──────────────────┴──────────────────┴──────────────────────────────────┘
//!
//! FullBox (adds 4 bytes after type):
//! ┌──────────────────┬──────────────────┬────────┬─────────────┐
//! │   size (u32 BE)  │  type (4 × u8)   │ ver u8 │ flags 3×u8  │
//! └──────────────────┴──────────────────┴────────┴─────────────┘
//! ```

use crate::DemuxError;

/// A parsed ISOBMFF box header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxHeader {
    /// The FourCC type of this box (e.g., `*b"moov"`, `*b"mdat"`).
    pub box_type: [u8; 4],

    /// Total size of the box in bytes, **including** the header itself.
    /// If `0`, the box extends to the end of the file.
    pub size: u64,

    /// Size of the header itself (8 for standard, 16 for extended).
    /// The box body starts at `file_offset + header_size`.
    pub header_size: u8,
}

impl BoxHeader {
    /// The minimum number of bytes needed to read a standard box header.
    pub const MIN_HEADER_SIZE: usize = 8;

    /// Parse a box header from the given byte slice.
    ///
    /// The slice must contain at least 8 bytes. If the box uses an extended
    /// size (first 4 bytes == `0x00000001`), 16 bytes are required.
    ///
    /// Returns the parsed header and the number of bytes consumed.
    pub fn parse(data: &[u8], file_offset: u64) -> Result<Self, DemuxError> {
        if data.len() < 8 {
            return Err(DemuxError::UnexpectedEof {
                offset: file_offset,
            });
        }

        let size_u32 = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let box_type: [u8; 4] = [data[4], data[5], data[6], data[7]];

        let (size, header_size) = match size_u32 {
            // Extended size: read 8 more bytes for u64 size
            1 => {
                if data.len() < 16 {
                    return Err(DemuxError::UnexpectedEof {
                        offset: file_offset,
                    });
                }
                let large_size = u64::from_be_bytes([
                    data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
                ]);
                (large_size, 16u8)
            }
            // Box extends to end of file
            0 => (0u64, 8u8),
            // Normal size
            s => (s as u64, 8u8),
        };

        // Sanity: size must be at least header_size (unless it's 0 = "to EOF")
        if size != 0 && size < header_size as u64 {
            return Err(DemuxError::InvalidBox {
                offset: file_offset,
                detail: format!(
                    "box '{}' has size {} which is smaller than its header ({})",
                    fourcc_to_string(&box_type),
                    size,
                    header_size
                ),
            });
        }

        Ok(BoxHeader {
            box_type,
            size,
            header_size,
        })
    }

    /// Returns the size of the box body (total size minus header).
    /// Returns `None` if the box extends to EOF (size == 0).
    pub fn body_size(&self) -> Option<u64> {
        if self.size == 0 {
            None
        } else {
            Some(self.size - self.header_size as u64)
        }
    }

    /// Returns the FourCC as a human-readable string.
    pub fn type_str(&self) -> String {
        fourcc_to_string(&self.box_type)
    }

    /// Returns true if this box type is a known container (has children).
    pub fn is_container(&self) -> bool {
        matches!(
            &self.box_type,
            b"moov"
                | b"trak"
                | b"mdia"
                | b"minf"
                | b"stbl"
                | b"dinf"
                | b"edts"
                | b"udta"
                | b"moof"
                | b"traf"
                | b"mvex"
                | b"sinf"
                | b"schi"
        )
    }
}

impl std::fmt::Display for BoxHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.size == 0 {
            write!(f, "[{} → EOF]", self.type_str())
        } else {
            write!(f, "[{} size={}]", self.type_str(), self.size)
        }
    }
}

/// Parse the version and flags from a FullBox header.
///
/// FullBox adds 4 bytes after the standard box header:
/// - 1 byte version
/// - 3 bytes flags
///
/// Returns `(version, flags)`.
pub fn parse_fullbox_header(data: &[u8], file_offset: u64) -> Result<(u8, u32), DemuxError> {
    if data.len() < 4 {
        return Err(DemuxError::UnexpectedEof {
            offset: file_offset,
        });
    }

    let version = data[0];
    let flags = u32::from_be_bytes([0, data[1], data[2], data[3]]);

    Ok((version, flags))
}

/// Convert a FourCC byte array to a printable string.
///
/// Non-printable bytes are replaced with `.` for readability.
pub fn fourcc_to_string(fourcc: &[u8; 4]) -> String {
    fourcc
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_box() {
        // ftyp box: size=20, type="ftyp"
        let data = [
            0x00, 0x00, 0x00, 0x14, // size = 20
            b'f', b't', b'y',
            b'p', // type = "ftyp"
                  // ... body would follow
        ];

        let header = BoxHeader::parse(&data, 0).unwrap();
        assert_eq!(header.box_type, *b"ftyp");
        assert_eq!(header.size, 20);
        assert_eq!(header.header_size, 8);
        assert_eq!(header.body_size(), Some(12));
    }

    #[test]
    fn parse_extended_box() {
        let data = vec![
            0x00, 0x00, 0x00, 0x01, // size = 1 (signals extended)
            b'm', b'd', b'a', b't', // type = "mdat"
            0x00, 0x00, 0x00, 0x01, // largesize (upper 32 bits) = 1
            0x00, 0x00, 0x00, 0x00, // largesize (lower 32 bits) = 0
        ];
        // largesize = 0x100000000 = 4 GiB

        let header = BoxHeader::parse(&data, 0).unwrap();
        assert_eq!(header.box_type, *b"mdat");
        assert_eq!(header.size, 0x1_0000_0000);
        assert_eq!(header.header_size, 16);
        assert_eq!(header.body_size(), Some(0x1_0000_0000 - 16));
    }

    #[test]
    fn parse_box_extends_to_eof() {
        let data = [
            0x00, 0x00, 0x00, 0x00, // size = 0 (extends to EOF)
            b'm', b'd', b'a', b't', // type = "mdat"
        ];

        let header = BoxHeader::parse(&data, 0).unwrap();
        assert_eq!(header.size, 0);
        assert_eq!(header.body_size(), None);
    }

    #[test]
    fn parse_too_short() {
        let data = [0x00, 0x00, 0x00];
        let result = BoxHeader::parse(&data, 42);
        assert!(result.is_err());
    }

    #[test]
    fn parse_invalid_size_too_small() {
        let data = [
            0x00, 0x00, 0x00, 0x04, // size = 4 (less than header_size=8)
            b'f', b'r', b'e', b'e',
        ];
        let result = BoxHeader::parse(&data, 0);
        assert!(result.is_err());
    }

    #[test]
    fn container_detection() {
        let header = BoxHeader {
            box_type: *b"moov",
            size: 100,
            header_size: 8,
        };
        assert!(header.is_container());

        let header = BoxHeader {
            box_type: *b"mdat",
            size: 100,
            header_size: 8,
        };
        assert!(!header.is_container());
    }

    #[test]
    fn fourcc_display() {
        assert_eq!(fourcc_to_string(b"moov"), "moov");
        assert_eq!(fourcc_to_string(b"ftyp"), "ftyp");
        // Non-printable bytes
        assert_eq!(fourcc_to_string(&[0x00, 0x01, b'a', b'b']), "..ab");
    }

    #[test]
    fn fullbox_header() {
        let data = [0x01, 0x00, 0x00, 0x01]; // version=1, flags=1
        let (version, flags) = parse_fullbox_header(&data, 0).unwrap();
        assert_eq!(version, 1);
        assert_eq!(flags, 1);
    }
}
