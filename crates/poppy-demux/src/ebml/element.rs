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
}

impl<'a> EbmlIter<'a> {
    pub fn new(data: &'a [u8], file_offset: u64) -> Self {
        Self {
            data,
            file_offset,
            cursor: 0,
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
            Some(s) => s as usize,
            None => slice.len() - header_len, // Unknown size extends to end
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
pub fn read_uint(payload: &[u8]) -> u64 {
    let mut val = 0u64;
    for &b in payload {
        val = (val << 8) | b as u64;
    }
    val
}

/// Helper to read UTF-8 string from EBML payload.
pub fn read_utf8(payload: &[u8]) -> String {
    String::from_utf8_lossy(payload).into_owned()
}
