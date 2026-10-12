//! EBML element parser and iterator.

use super::vint::{read_vint_id, read_vint_size};
use crate::DemuxError;

/// A parsed EBML element.
#[derive(Debug, Clone, Copy)]
pub struct EbmlElement<'a> {
    pub id: u32,
    pub payload: &'a [u8],
    pub offset: u64,
}

/// Iterator over sibling EBML elements in a byte buffer.
pub struct EbmlIter<'a> {
    data: &'a [u8],
    file_offset: u64,
    cursor: usize,
    /// Whether `unknown size` (all-1s VINT) is allowed. Per Matroska spec this
    /// is only legal for `Segment` (top-level, to EOF) and in live streams for
    /// `Cluster`. Inner elements with unknown size must be rejected instead of
    /// swallowing all following siblings.
    allow_unknown_size: bool,
}

impl<'a> EbmlIter<'a> {
    pub fn new(data: &'a [u8], file_offset: u64) -> Self {
        Self {
            data,
            file_offset,
            cursor: 0,
            allow_unknown_size: false,
        }
    }

    /// Top-level iterator where `Segment` with unknown size (to EOF) is legal.
    pub fn new_top_level(data: &'a [u8], file_offset: u64) -> Self {
        Self {
            data,
            file_offset,
            cursor: 0,
            allow_unknown_size: true,
        }
    }
}

impl<'a> Iterator for EbmlIter<'a> {
    type Item = Result<EbmlElement<'a>, DemuxError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.data.len() {
            return None;
        }

        let current_offset = self.file_offset + self.cursor as u64;
        let slice = &self.data[self.cursor..];

        let (id, id_len) = match read_vint_id(slice, current_offset) {
            Ok(res) => res,
            Err(e) => return Some(Err(e)),
        };

        let (size_opt, size_len) =
            match read_vint_size(&slice[id_len..], current_offset + id_len as u64) {
                Ok(res) => res,
                Err(e) => return Some(Err(e)),
            };

        let header_len = id_len + size_len;
        let size = match size_opt {
            Some(s) => {
                // u64 -> usize checked for 32-bit platforms.
                match usize::try_from(s) {
                    Ok(v) => v,
                    Err(_) => {
                        return Some(Err(DemuxError::InvalidBox {
                            offset: current_offset,
                            detail: format!("EBML element {id:#x} size {s} does not fit in usize"),
                        }));
                    }
                }
            }
            None => {
                if !self.allow_unknown_size {
                    return Some(Err(DemuxError::InvalidBox {
                        offset: current_offset,
                        detail: format!(
                            "EBML element {id:#x} has unknown size inside container (only Segment may be unknown)"
                        ),
                    }));
                }
                slice.len() - header_len // Unknown size extends to end
            }
        };

        let total_len = match header_len.checked_add(size) {
            Some(l) => l,
            None => {
                return Some(Err(DemuxError::InvalidBox {
                    offset: current_offset,
                    detail: "EBML element size overflow".into(),
                }));
            }
        };

        if total_len > slice.len() {
            return Some(Err(DemuxError::UnexpectedEof {
                offset: current_offset + header_len as u64,
            }));
        }

        let payload = &slice[header_len..total_len];
        self.cursor += total_len;

        Some(Ok(EbmlElement {
            id,
            payload,
            offset: current_offset,
        }))
    }
}

/// Helper to read unsigned integer from EBML payload (1 to 8 bytes big endian).
/// Longer payloads truncate with an error via `try_read_uint`; this legacy
/// helper saturates to the low 8 bytes for compatibility.
pub fn read_uint(payload: &[u8]) -> u64 {
    try_read_uint(payload).unwrap_or(0)
}

/// Checked variant: rejects empty (>8B) payloads instead of silently
/// truncating high bits.
pub fn try_read_uint(payload: &[u8]) -> Result<u64, crate::DemuxError> {
    if payload.len() > 8 {
        return Err(crate::DemuxError::InvalidBox {
            offset: 0,
            detail: format!("EBML uint of {} bytes exceeds 8-byte max", payload.len()),
        });
    }
    let mut val = 0u64;
    for &b in payload {
        val = (val << 8) | b as u64;
    }
    Ok(val)
}

/// Helper to read UTF-8 string from EBML payload.
/// Uses lossy conversion for compatibility; use `try_read_utf8` for strict.
pub fn read_utf8(payload: &[u8]) -> String {
    String::from_utf8_lossy(payload).into_owned()
}

/// Strict variant: rejects invalid UTF-8 (e.g. corrupt CodecID) instead of
/// substituting U+FFFD and misclassifying as Unknown(0).
pub fn try_read_utf8(payload: &[u8]) -> Result<String, crate::DemuxError> {
    std::str::from_utf8(payload)
        .map(|s| s.to_owned())
        .map_err(|_| crate::DemuxError::InvalidBox {
            offset: 0,
            detail: "EBML string is not valid UTF-8".into(),
        })
}
