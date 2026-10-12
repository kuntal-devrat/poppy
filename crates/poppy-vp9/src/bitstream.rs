//! Bitstream reader for VP9 uncompressed headers.
//!
//! VP9 bitstreams are read MSB-first for uncompressed headers, with specific
//! signed-magnitude encoding for quantization deltas and loop filter deltas,
//! and little-endian integers for superframe index sizes.

use thiserror::Error;

/// Errors encountered while reading or parsing a VP9 bitstream.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Vp9ParserError {
    #[error("Unexpected end of bitstream (needed {needed_bits} bits, had {available_bits})")]
    UnexpectedEof {
        needed_bits: usize,
        available_bits: usize,
    },

    #[error("Invalid frame marker: expected 0b10 (2), got {0}")]
    InvalidFrameMarker(u8),

    #[error("Invalid sync code: expected 0x498342, got 0x{0:06X}")]
    InvalidSyncCode(u32),

    #[error("Invalid superframe index marker: expected 0xC0..=0xDF, got 0x{0:02X}")]
    InvalidSuperframeMarker(u8),

    #[error("Superframe frame {frame_idx} size {frame_size} exceeds packet size {packet_size}")]
    InvalidSuperframeSize {
        frame_idx: usize,
        frame_size: usize,
        packet_size: usize,
    },

    #[error("Reserved bit violation: expected 0, got {0}")]
    ReservedBitMismatch(u32),

    #[error("Invalid profile: {0}")]
    InvalidProfile(u8),
}

/// Bit-level cursor for VP9 uncompressed headers.
pub struct Vp9BitReader<'a> {
    data: &'a [u8],
    bit_offset: usize,
}

impl<'a> Vp9BitReader<'a> {
    /// Create a new bit reader wrapping the provided slice.
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_offset: 0,
        }
    }

    /// Total number of bits remaining in the bitstream.
    #[inline]
    pub fn bits_remaining(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.bit_offset)
    }

    /// Total number of bits consumed so far.
    #[inline]
    pub fn bits_read(&self) -> usize {
        self.bit_offset
    }

    /// Total number of whole bytes consumed (rounded up if mid-byte).
    #[inline]
    pub fn byte_offset(&self) -> usize {
        self.bit_offset.div_ceil(8)
    }

    /// Read a single bit as a boolean.
    #[inline]
    pub fn read_bit(&mut self) -> Result<bool, Vp9ParserError> {
        Ok(self.read_bits(1)? != 0)
    }

    /// Read an unsigned integer of `n` bits (0..=32) MSB-first.
    pub fn read_bits(&mut self, n: usize) -> Result<u32, Vp9ParserError> {
        if n == 0 {
            return Ok(0);
        }
        if self.bits_remaining() < n {
            return Err(Vp9ParserError::UnexpectedEof {
                needed_bits: n,
                available_bits: self.bits_remaining(),
            });
        }

        let mut value = 0u32;
        for _ in 0..n {
            let byte_idx = self.bit_offset / 8;
            let bit_in_byte = 7 - (self.bit_offset % 8);
            let bit = ((self.data[byte_idx] >> bit_in_byte) & 1) as u32;
            value = (value << 1) | bit;
            self.bit_offset += 1;
        }

        Ok(value)
    }

    /// Read a signed integer encoded as `n` bits of magnitude followed by 1 sign bit.
    /// In VP9: if `sign == 1`, value is `-magnitude`, else `+magnitude`.
    pub fn read_signed_bits(&mut self, n: usize) -> Result<i32, Vp9ParserError> {
        let magnitude = self.read_bits(n)? as i32;
        let sign = self.read_bit()?;
        Ok(if sign { -magnitude } else { magnitude })
    }

    /// Read delta Q: 1 bit flag. If set, reads a 4-bit signed magnitude value (`read_signed_bits(4)`).
    /// If not set, returns 0.
    pub fn read_delta_q(&mut self) -> Result<i8, Vp9ParserError> {
        if self.read_bit()? {
            Ok(self.read_signed_bits(4)? as i8)
        } else {
            Ok(0)
        }
    }

    /// Read a little-endian unsigned integer of `num_bytes` (1..=4).
    pub fn read_le(&mut self, num_bytes: usize) -> Result<u32, Vp9ParserError> {
        let mut value = 0u32;
        for b in 0..num_bytes {
            let byte_val = self.read_bits(8)?;
            value |= byte_val << (b * 8);
        }
        Ok(value)
    }

    /// Skip to the next byte boundary.
    pub fn align_to_byte(&mut self) {
        let rem = self.bit_offset % 8;
        if rem != 0 {
            self.bit_offset += 8 - rem;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vp9_bit_reader_basic() {
        let data = [0b1011_0001, 0b1100_1010];
        let mut r = Vp9BitReader::new(&data);
        assert_eq!(r.read_bits(2).unwrap(), 0b10);
        assert_eq!(r.read_bits(4).unwrap(), 0b1100);
        assert_eq!(r.read_bits(2).unwrap(), 0b01);
        assert_eq!(r.byte_offset(), 1);
        assert_eq!(r.read_bits(8).unwrap(), 0b1100_1010);
        assert_eq!(r.byte_offset(), 2);
    }

    #[test]
    fn test_vp9_signed_bits() {
        // magnitude 5 (0b101), sign 1 (negative) -> 0b1011
        let data = [0b1011_0000];
        let mut r = Vp9BitReader::new(&data);
        assert_eq!(r.read_signed_bits(3).unwrap(), -5);

        // magnitude 5 (0b101), sign 0 (positive) -> 0b1010
        let data2 = [0b1010_0000];
        let mut r2 = Vp9BitReader::new(&data2);
        assert_eq!(r2.read_signed_bits(3).unwrap(), 5);
    }

    #[test]
    fn test_vp9_delta_q() {
        // flag=0 -> 0
        let data = [0b0000_0000];
        let mut r = Vp9BitReader::new(&data);
        assert_eq!(r.read_delta_q().unwrap(), 0);

        // flag=1, magnitude=3 (0011), sign=1 (neg) -> -3
        let data2 = [0b1001_1100];
        let mut r2 = Vp9BitReader::new(&data2);
        assert_eq!(r2.read_delta_q().unwrap(), -3);
    }
}
