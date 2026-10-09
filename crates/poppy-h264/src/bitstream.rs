//! H.264 bitstream reader and Exp-Golomb decoder.
//!
//! Implements ITU-T H.264 §7.2, §7.3.1 (emulation prevention byte removal),
//! and §9.1 (Exponential-Golomb coded syntax elements).

/// Bitstream parsing errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BitstreamError {
    #[error("unexpected end of bitstream: needed {needed} bits, {available} remaining")]
    UnexpectedEof { needed: usize, available: usize },

    #[error("Exp-Golomb value overflowed 32 bits (leading zeros: {leading_zeros})")]
    ExpGolombOverflow { leading_zeros: usize },

    #[error("invalid bit count {0} (must be 1..=32)")]
    InvalidBitCount(usize),
}

/// Strips H.264 emulation prevention bytes (`0x00 0x00 0x03`) to produce
/// raw byte sequence payload (RBSP).
///
/// Reference: ITU-T H.264 §7.3.1
pub fn remove_emulation_prevention(nal_payload: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(nal_payload.len());
    let mut zero_count = 0;

    for &b in nal_payload {
        if zero_count >= 2 && b == 0x03 {
            // Drop emulation prevention byte 0x03, reset count
            zero_count = 0;
            continue;
        }

        rbsp.push(b);

        if b == 0x00 {
            zero_count += 1;
        } else {
            zero_count = 0;
        }
    }

    rbsp
}

/// Bit-level reader for RBSP data.
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Current bit index in `data` (0-indexed).
    bit_cursor: usize,
}

impl<'a> BitReader<'a> {
    /// Creates a new `BitReader` over the given RBSP slice.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_cursor: 0,
        }
    }

    /// Number of bits read so far from the start.
    #[inline]
    pub fn bits_read(&self) -> usize {
        self.bit_cursor
    }

    /// Number of bits remaining in the buffer.
    #[inline]
    pub fn bits_remaining(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.bit_cursor)
    }

    /// Read a single bit as a boolean.
    #[inline]
    pub fn read_bit(&mut self) -> Result<bool, BitstreamError> {
        let bit = self.read_bits(1)?;
        Ok(bit != 0)
    }

    /// Read `n` bits (1 to 32) as an unsigned integer.
    pub fn read_bits(&mut self, n: usize) -> Result<u32, BitstreamError> {
        if n == 0 || n > 32 {
            return Err(BitstreamError::InvalidBitCount(n));
        }

        let remaining = self.bits_remaining();
        if remaining < n {
            return Err(BitstreamError::UnexpectedEof {
                needed: n,
                available: remaining,
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

    /// Read an unsigned Exp-Golomb integer `ue(v)`.
    ///
    /// Reference: ITU-T H.264 §9.1
    pub fn read_ue(&mut self) -> Result<u32, BitstreamError> {
        let mut leading_zeros = 0usize;

        while !self.read_bit()? {
            leading_zeros += 1;
            if leading_zeros > 31 {
                return Err(BitstreamError::ExpGolombOverflow { leading_zeros });
            }
        }

        if leading_zeros == 0 {
            return Ok(0);
        }

        let suffix = self.read_bits(leading_zeros)?;
        let val = (1u32 << leading_zeros) - 1 + suffix;
        Ok(val)
    }

    /// Read a signed Exp-Golomb integer `se(v)`.
    ///
    /// Reference: ITU-T H.264 §9.1.1
    pub fn read_se(&mut self) -> Result<i32, BitstreamError> {
        let code_num = self.read_ue()?;
        if code_num == 0 {
            return Ok(0);
        }

        if code_num % 2 == 1 {
            Ok(code_num.div_ceil(2) as i32)
        } else {
            Ok(-((code_num / 2) as i32))
        }
    }

    /// Read a truncated Exp-Golomb integer `te(v)`.
    pub fn read_te(&mut self, max: u32) -> Result<u32, BitstreamError> {
        if max > 1 {
            self.read_ue()
        } else {
            let b = self.read_bit()?;
            Ok(if b { 0 } else { 1 })
        }
    }

    /// Check if more RBSP data exists prior to `rbsp_trailing_bits()`.
    ///
    /// Reference: ITU-T H.264 §7.2
    pub fn has_more_rbsp_data(&self) -> bool {
        let remaining = self.bits_remaining();
        if remaining == 0 {
            return false;
        }

        // Find the stop bit (the last '1' bit in the bitstream)
        // If the very next bit is '1' and everything after until the end is '0',
        // then this is rbsp_trailing_bits().
        let mut cur = self.bit_cursor;
        let total = self.data.len() * 8;

        // Peak next bit
        let byte_idx = cur / 8;
        let bit_in_byte = 7 - (cur % 8);
        let next_bit = (self.data[byte_idx] >> bit_in_byte) & 1;

        if next_bit == 0 {
            // Still more data before stop bit
            return true;
        }

        // The current bit is 1. Check if all remaining bits until total are 0.
        cur += 1;
        while cur < total {
            let b_idx = cur / 8;
            let bit_pos = 7 - (cur % 8);
            if ((self.data[b_idx] >> bit_pos) & 1) != 0 {
                return true;
            }
            cur += 1;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_emulation_prevention() {
        // 00 00 03 01 -> 00 00 01
        let input = [0x00, 0x00, 0x03, 0x01, 0xAA, 0x00, 0x00, 0x03, 0x00];
        let rbsp = remove_emulation_prevention(&input);
        assert_eq!(rbsp, vec![0x00, 0x00, 0x01, 0xAA, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_read_ue() {
        // Bit sequences for ue(v):
        // 1          -> 0
        // 0 1 0      -> 1
        // 0 1 1      -> 2
        // 0 0 1 0 0  -> 3
        // 0 0 1 0 1  -> 4
        // 0 0 1 1 0  -> 5
        // 0 0 1 1 1  -> 6
        //
        // Pack: 1_010_011_0 (0xA6), 0100_0010 (0x42), 1011_0001 (0xB1)
        // 1 (0) | 010 (1) | 011 (2) | 00100 (3) | 00101 (4)
        // Bits: 1 010 011 0 0100 0010 1 = 16 bits
        let data = [0b1010_0110, 0b0100_0010, 0b1000_0000];
        let mut reader = BitReader::new(&data);

        assert_eq!(reader.read_ue().unwrap(), 0);
        assert_eq!(reader.read_ue().unwrap(), 1);
        assert_eq!(reader.read_ue().unwrap(), 2);
        assert_eq!(reader.read_ue().unwrap(), 3);
        assert_eq!(reader.read_ue().unwrap(), 4);
    }

    #[test]
    fn test_read_se() {
        // code_num -> se value
        // 0 -> 0
        // 1 -> 1
        // 2 -> -1
        // 3 -> 2
        // 4 -> -2
        let data = [0b1010_0110, 0b0100_0010, 0b1000_0000];
        let mut reader = BitReader::new(&data);

        assert_eq!(reader.read_se().unwrap(), 0);
        assert_eq!(reader.read_se().unwrap(), 1);
        assert_eq!(reader.read_se().unwrap(), -1);
        assert_eq!(reader.read_se().unwrap(), 2);
        assert_eq!(reader.read_se().unwrap(), -2);
    }

    #[test]
    fn test_read_bits() {
        let data = [0xA5, 0x5A]; // 10100101 01011010
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read_bits(4).unwrap(), 0b1010);
        assert_eq!(reader.read_bits(4).unwrap(), 0b0101);
        assert_eq!(reader.read_bits(8).unwrap(), 0x5A);
        assert_eq!(reader.bits_remaining(), 0);
        assert!(reader.read_bits(1).is_err());
    }
}
