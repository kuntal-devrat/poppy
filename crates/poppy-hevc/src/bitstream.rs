//! H.265 / HEVC bitstream reader and Exp-Golomb decoder.
//!
//! Implements ITU-T H.265 §7.2, §7.3.1 (emulation prevention byte removal),
//! and §9.2 (Exponential-Golomb coded syntax elements).

/// Bitstream parsing errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BitstreamError {
    #[error("unexpected end of bitstream: needed {needed} bits, {available} remaining")]
    UnexpectedEof { needed: usize, available: usize },

    #[error("Exp-Golomb value overflowed 32 bits (leading zeros: {leading_zeros})")]
    ExpGolombOverflow { leading_zeros: usize },

    #[error("invalid bit count {0} (must be 1..=32)")]
    InvalidBitCount(usize),

    #[error("invalid syntax element: {0}")]
    InvalidSyntax(String),
}

/// Strips H.265 emulation prevention bytes (`0x00 0x00 0x03`) to produce
/// raw byte sequence payload (RBSP).
///
/// Reference: ITU-T H.265 §7.3.1.1
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

    /// Returns true if the bit position is byte-aligned.
    #[inline]
    pub fn is_byte_aligned(&self) -> bool {
        self.bit_cursor.trailing_zeros() >= 3
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

    /// Skips `n` bits in the bitstream.
    pub fn skip_bits(&mut self, n: usize) -> Result<(), BitstreamError> {
        let remaining = self.bits_remaining();
        if remaining < n {
            return Err(BitstreamError::UnexpectedEof {
                needed: n,
                available: remaining,
            });
        }
        self.bit_cursor += n;
        Ok(())
    }

    /// Read an unsigned Exponential-Golomb coded integer (`ue(v)`).
    ///
    /// Reference: ITU-T H.265 §9.2
    pub fn read_ue(&mut self) -> Result<u32, BitstreamError> {
        let mut leading_zeros = 0usize;

        while self.read_bits(1)? == 0 {
            leading_zeros += 1;
            if leading_zeros >= 32 {
                return Err(BitstreamError::ExpGolombOverflow { leading_zeros });
            }
        }

        if leading_zeros == 0 {
            return Ok(0);
        }

        let info = self.read_bits(leading_zeros)?;
        let val = (1u32 << leading_zeros) - 1 + info;
        Ok(val)
    }

    /// Read a signed Exponential-Golomb coded integer (`se(v)`).
    ///
    /// Mapping: code_num = 2*|val| (val > 0), code_num = 2*|val| - 1 (val < 0).
    ///
    /// Reference: ITU-T H.265 §9.2
    pub fn read_se(&mut self) -> Result<i32, BitstreamError> {
        let code_num = self.read_ue()?;
        if code_num == 0 {
            return Ok(0);
        }

        let sign = if code_num % 2 == 1 { 1i32 } else { -1i32 };
        let val = code_num.div_ceil(2) as i32 * sign;
        Ok(val)
    }

    /// Returns `true` if more RBSP data exists before `rbsp_trailing_bits()`.
    ///
    /// Reference: ITU-T H.265 §7.3.2.11
    pub fn has_more_rbsp_data(&self) -> bool {
        let remaining = self.bits_remaining();
        if remaining == 0 {
            return false;
        }

        let byte_idx = self.bit_cursor / 8;
        let bit_in_byte = 7 - (self.bit_cursor % 8);

        // Check if current bit is 1
        let bit = (self.data[byte_idx] >> bit_in_byte) & 1;
        if bit == 0 {
            return true;
        }

        // If current bit is 1, check all subsequent bits in the remaining stream
        let mut idx = self.bit_cursor + 1;
        while idx < self.data.len() * 8 {
            let b_idx = idx / 8;
            let b_offset = 7 - (idx % 8);
            if (self.data[b_idx] >> b_offset) & 1 != 0 {
                return true;
            }
            idx += 1;
        }

        false
    }
}
