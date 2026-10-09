//! Bitstream reader for AV1 syntax parsing.
//!
//! AV1 specification §4.10: Bitstream syntax parsing primitives.

use thiserror::Error;

/// Errors encountered while reading AV1 bitstream data.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BitstreamError {
    #[error("unexpected end of bitstream: need {need} bits, have {have}")]
    UnexpectedEof { need: usize, have: usize },
    #[error("invalid bitstream syntax: {0}")]
    InvalidSyntax(&'static str),
}

/// Bit-level reader for AV1 bitstream payloads.
pub struct BitReader<'a> {
    data: &'a [u8],
    bit_cursor: usize,
}

impl<'a> BitReader<'a> {
    /// Creates a new bit reader over a slice of bytes.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_cursor: 0,
        }
    }

    /// Total bits remaining in the buffer.
    pub fn bits_remaining(&self) -> usize {
        let total = self.data.len() * 8;
        total.saturating_sub(self.bit_cursor)
    }

    /// Current number of bits read so far.
    pub fn bits_read(&self) -> usize {
        self.bit_cursor
    }

    /// Read a single bit as boolean.
    pub fn read_bit(&mut self) -> Result<bool, BitstreamError> {
        let bit = self.read_bits(1)?;
        Ok(bit != 0)
    }

    /// Read up to 32 bits into a `u32` (most-significant bit first).
    pub fn read_bits(&mut self, n: usize) -> Result<u32, BitstreamError> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(BitstreamError::InvalidSyntax(
                "cannot read more than 32 bits at once",
            ));
        }
        if self.bits_remaining() < n {
            return Err(BitstreamError::UnexpectedEof {
                need: n,
                have: self.bits_remaining(),
            });
        }

        let mut val = 0u32;
        for _ in 0..n {
            let byte_idx = self.bit_cursor / 8;
            let bit_in_byte = 7 - (self.bit_cursor % 8);
            let bit = ((self.data[byte_idx] >> bit_in_byte) & 1) as u32;
            val = (val << 1) | bit;
            self.bit_cursor += 1;
        }

        Ok(val)
    }

    /// Read non-symmetric unsigned integer `ns(n)` (AV1 spec §4.10.3).
    pub fn read_ns(&mut self, n: u32) -> Result<u32, BitstreamError> {
        if n <= 1 {
            return Ok(0);
        }

        let w = (32 - (n - 1).leading_zeros()) as usize;
        let m = (1u32 << w) - n;
        let v = self.read_bits(w - 1)?;
        if v < m {
            Ok(v)
        } else {
            let extra_bit = self.read_bit()? as u32;
            Ok((v << 1) - m + extra_bit)
        }
    }

    /// Read signed integer `su(n)` (sign bit in most significant bit).
    pub fn read_su(&mut self, n: usize) -> Result<i32, BitstreamError> {
        if n == 0 {
            return Ok(0);
        }
        let unsigned = self.read_bits(n)?;
        let sign_bit = 1u32 << (n - 1);
        if (unsigned & sign_bit) != 0 {
            let val = unsigned as i32 - (1i32 << n);
            Ok(val)
        } else {
            Ok(unsigned as i32)
        }
    }

    /// Read unsigned variable-length integer `uvlc()` (AV1 spec §4.10.4).
    pub fn read_uvlc(&mut self) -> Result<u32, BitstreamError> {
        let mut leading_zeros = 0usize;
        while !self.read_bit()? {
            leading_zeros += 1;
            if leading_zeros >= 32 {
                return Err(BitstreamError::InvalidSyntax(
                    "uvlc leading zeros exceeded 32",
                ));
            }
        }

        if leading_zeros >= 32 {
            return Ok(0xFFFFFFFF);
        }

        let value = self.read_bits(leading_zeros)?;
        Ok((1u32 << leading_zeros) - 1 + value)
    }

    /// Skip `n` bits.
    pub fn skip_bits(&mut self, n: usize) -> Result<(), BitstreamError> {
        if self.bits_remaining() < n {
            return Err(BitstreamError::UnexpectedEof {
                need: n,
                have: self.bits_remaining(),
            });
        }
        self.bit_cursor += n;
        Ok(())
    }

    /// Align to next byte boundary.
    pub fn byte_align(&mut self) {
        let remainder = self.bit_cursor % 8;
        if remainder != 0 {
            self.bit_cursor += 8 - remainder;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_bits() {
        let data = [0b10110010, 0b11001100];
        let mut r = BitReader::new(&data);
        assert!(r.read_bit().unwrap());
        assert!(!r.read_bit().unwrap());
        assert_eq!(r.read_bits(4).unwrap(), 0b1100);
        assert_eq!(r.read_bits(2).unwrap(), 0b10);
        assert_eq!(r.read_bits(8).unwrap(), 0b11001100);
    }

    #[test]
    fn test_read_su() {
        let data = [0b11110000]; // 4-bit negative: 0b1111 -> -1
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_su(4).unwrap(), -1);
    }
}
