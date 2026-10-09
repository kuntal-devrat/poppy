//! Open Bitstream Unit (OBU) parser and container splitter.
//!
//! AV1 specification §5.3: OBU header syntax.

use thiserror::Error;

use crate::leb128::{Leb128Error, read_leb128};

/// AV1 Open Bitstream Unit types (AV1 specification §6.2.2).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObuType {
    SequenceHeader = 1,
    TemporalDelimiter = 2,
    FrameHeader = 3,
    TileGroup = 4,
    Metadata = 5,
    Frame = 6,
    RedundantFrameHeader = 7,
    TileList = 8,
    Padding = 15,
    Reserved = 0,
}

impl ObuType {
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => ObuType::SequenceHeader,
            2 => ObuType::TemporalDelimiter,
            3 => ObuType::FrameHeader,
            4 => ObuType::TileGroup,
            5 => ObuType::Metadata,
            6 => ObuType::Frame,
            7 => ObuType::RedundantFrameHeader,
            8 => ObuType::TileList,
            15 => ObuType::Padding,
            _ => ObuType::Reserved,
        }
    }
}

/// Parsed OBU Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObuHeader {
    pub obu_type: ObuType,
    pub has_extension: bool,
    pub has_size_field: bool,
    pub temporal_id: u8,
    pub spatial_id: u8,
    pub header_len: usize,
}

/// Errors encountered while parsing an OBU.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ObuParseError {
    #[error("OBU buffer too short")]
    TooShort,
    #[error("forbidden bit is set in OBU header")]
    ForbiddenBitSet,
    #[error("failed to parse OBU size: {0}")]
    Leb128(#[from] Leb128Error),
    #[error("OBU payload size exceeds buffer boundaries")]
    PayloadOutOfBounds,
}

impl ObuHeader {
    /// Parses an OBU header from the start of a byte slice.
    pub fn parse(data: &[u8]) -> Result<Self, ObuParseError> {
        if data.is_empty() {
            return Err(ObuParseError::TooShort);
        }

        let first = data[0];
        let obu_forbidden_bit = (first >> 7) & 1;
        if obu_forbidden_bit != 0 {
            return Err(ObuParseError::ForbiddenBitSet);
        }

        let obu_type_raw = (first >> 3) & 0x0F;
        let obu_type = ObuType::from_u8(obu_type_raw);
        let has_extension = ((first >> 2) & 1) != 0;
        let has_size_field = ((first >> 1) & 1) != 0;

        let mut temporal_id = 0;
        let mut spatial_id = 0;
        let mut header_len = 1;

        if has_extension {
            if data.len() < 2 {
                return Err(ObuParseError::TooShort);
            }
            let ext = data[1];
            temporal_id = (ext >> 5) & 0x07;
            spatial_id = (ext >> 3) & 0x03;
            header_len = 2;
        }

        Ok(Self {
            obu_type,
            has_extension,
            has_size_field,
            temporal_id,
            spatial_id,
            header_len,
        })
    }
}

/// A parsed OBU unit referencing its original payload slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObuUnit<'a> {
    pub header: ObuHeader,
    pub payload: &'a [u8],
}

/// Splits a raw AV1 bitstream byte slice into sequential OBU units.
pub fn split_obus(mut data: &[u8]) -> Result<Vec<ObuUnit<'_>>, ObuParseError> {
    let mut obus = Vec::new();

    while !data.is_empty() {
        let header = ObuHeader::parse(data)?;
        let mut offset = header.header_len;

        let payload_size = if header.has_size_field {
            let (size, bytes_read) = read_leb128(&data[offset..])?;
            offset += bytes_read;
            size as usize
        } else {
            data.len() - offset
        };

        let total_end = match offset.checked_add(payload_size) {
            Some(end) => end,
            None => return Err(ObuParseError::PayloadOutOfBounds),
        };

        if total_end > data.len() {
            return Err(ObuParseError::PayloadOutOfBounds);
        }

        let payload = &data[offset..total_end];
        obus.push(ObuUnit { header, payload });

        data = &data[total_end..];
    }

    Ok(obus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_obu_header_parsing() {
        // Temporal delimiter: type=2 (0b00010000), no ext, has_size_field=1 (bit 1) -> 0b00010010 = 0x12
        let data = [0x12, 0x00];
        let header = ObuHeader::parse(&data).unwrap();
        assert_eq!(header.obu_type, ObuType::TemporalDelimiter);
        assert!(!header.has_extension);
        assert!(header.has_size_field);
    }
}
