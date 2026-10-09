//! H.264 NAL (Network Abstraction Layer) unit parser.
//!
//! Implements ITU-T H.264 §7.3.1 / Table 7-1.

/// NAL unit types defined in ITU-T H.264 Table 7-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NalUnitType {
    /// Coded slice of a non-IDR picture (type 1)
    SliceNonIdr,
    /// Coded slice data partition A (type 2)
    SlicePartitionA,
    /// Coded slice data partition B (type 3)
    SlicePartitionB,
    /// Coded slice data partition C (type 4)
    SlicePartitionC,
    /// Coded slice of an IDR picture (type 5)
    SliceIdr,
    /// Supplemental enhancement information (type 6)
    Sei,
    /// Sequence parameter set (type 7)
    Sps,
    /// Picture parameter set (type 8)
    Pps,
    /// Access unit delimiter (type 9)
    Aud,
    /// End of sequence (type 10)
    EndOfSequence,
    /// End of stream (type 11)
    EndOfStream,
    /// Filler data (type 12)
    FillerData,
    /// Sequence parameter set extension (type 13)
    SpsExtension,
    /// Prefix NAL unit (type 14)
    PrefixNal,
    /// Subset sequence parameter set (type 15)
    SubsetSps,
    /// Coded slice of an auxiliary picture (type 19)
    SliceAux,
    /// Coded slice extension (type 20)
    SliceExtension,
    /// Other / reserved / unspecified
    Other(u8),
}

impl NalUnitType {
    pub fn from_u8(val: u8) -> Self {
        match val & 0x1F {
            1 => Self::SliceNonIdr,
            2 => Self::SlicePartitionA,
            3 => Self::SlicePartitionB,
            4 => Self::SlicePartitionC,
            5 => Self::SliceIdr,
            6 => Self::Sei,
            7 => Self::Sps,
            8 => Self::Pps,
            9 => Self::Aud,
            10 => Self::EndOfSequence,
            11 => Self::EndOfStream,
            12 => Self::FillerData,
            13 => Self::SpsExtension,
            14 => Self::PrefixNal,
            15 => Self::SubsetSps,
            19 => Self::SliceAux,
            20 => Self::SliceExtension,
            other => Self::Other(other),
        }
    }

    /// True if this NAL unit contains a coded slice (types 1 or 5).
    pub fn is_slice(&self) -> bool {
        matches!(self, Self::SliceNonIdr | Self::SliceIdr)
    }

    /// True if this is an IDR slice (random access keyframe point).
    pub fn is_idr(&self) -> bool {
        matches!(self, Self::SliceIdr)
    }
}

/// A parsed NAL unit header (1 byte in H.264 baseline/high).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NalHeader {
    /// Must be 0 for standard streams. If 1, bit errors detected.
    pub forbidden_zero_bit: bool,
    /// Reference indicator (0 = non-ref, 1..=3 = reference).
    pub nal_ref_idc: u8,
    /// NAL unit type.
    pub unit_type: NalUnitType,
}

impl NalHeader {
    pub fn parse(first_byte: u8) -> Self {
        let forbidden_zero_bit = (first_byte & 0x80) != 0;
        let nal_ref_idc = (first_byte >> 5) & 0x03;
        let unit_type = NalUnitType::from_u8(first_byte & 0x1F);

        Self {
            forbidden_zero_bit,
            nal_ref_idc,
            unit_type,
        }
    }
}

/// A parsed NAL unit slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NalUnit<'a> {
    pub header: NalHeader,
    /// The payload of the NAL unit (excluding the 1-byte NAL header).
    pub payload: &'a [u8],
}

impl<'a> NalUnit<'a> {
    /// Parse a NAL unit from a raw buffer starting with the NAL header byte.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        if data.is_empty() {
            return None;
        }

        let header = NalHeader::parse(data[0]);
        let payload = &data[1..];

        Some(Self { header, payload })
    }
}

/// Splits an Annex-B byte stream (`00 00 01` or `00 00 00 01` start codes) into NAL units.
pub fn find_annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut i = 0;
    let mut start_indices = Vec::new();

    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                // 3-byte start code 00 00 01
                start_indices.push((i, i + 3));
                i += 3;
                continue;
            } else if i + 3 < data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                // 4-byte start code 00 00 00 01
                start_indices.push((i, i + 4));
                i += 4;
                continue;
            }
        }
        i += 1;
    }

    for idx in 0..start_indices.len() {
        let (_, nal_data_start) = start_indices[idx];
        let nal_data_end = if idx + 1 < start_indices.len() {
            start_indices[idx + 1].0
        } else {
            data.len()
        };

        if nal_data_start < nal_data_end {
            nals.push(&data[nal_data_start..nal_data_end]);
        }
    }

    nals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nal_header_parse() {
        // 0x67 = 0110 0111 -> forbidden=0, ref_idc=3, type=7 (SPS)
        let h1 = NalHeader::parse(0x67);
        assert!(!h1.forbidden_zero_bit);
        assert_eq!(h1.nal_ref_idc, 3);
        assert_eq!(h1.unit_type, NalUnitType::Sps);

        // 0x68 = 0110 1000 -> forbidden=0, ref_idc=3, type=8 (PPS)
        let h2 = NalHeader::parse(0x68);
        assert_eq!(h2.unit_type, NalUnitType::Pps);

        // 0x65 = 0110 0101 -> forbidden=0, ref_idc=3, type=5 (IDR)
        let h3 = NalHeader::parse(0x65);
        assert_eq!(h3.unit_type, NalUnitType::SliceIdr);
        assert!(h3.unit_type.is_idr());
        assert!(h3.unit_type.is_slice());

        // 0x41 = 0100 0001 -> forbidden=0, ref_idc=2, type=1 (Non-IDR)
        let h4 = NalHeader::parse(0x41);
        assert_eq!(h4.unit_type, NalUnitType::SliceNonIdr);
        assert!(!h4.unit_type.is_idr());
        assert!(h4.unit_type.is_slice());
    }

    #[test]
    fn test_annex_b_splitting() {
        let stream = [
            0, 0, 0, 1, 0x67, 1, 2, 3, // SPS
            0, 0, 0, 1, 0x68, 4, 5, // PPS
            0, 0, 1, 0x65, 6, // IDR with 3-byte start code
        ];

        let nals = find_annex_b_nals(&stream);
        assert_eq!(nals.len(), 3);
        assert_eq!(nals[0], &[0x67, 1, 2, 3]);
        assert_eq!(nals[1], &[0x68, 4, 5]);
        assert_eq!(nals[2], &[0x65, 6]);
    }
}
