//! Bitstream filter (BSF) utilities.
//!
//! Converts between container bitstream formats:
//! - AVCC (length-prefixed NAL units, standard in MP4/ISOBMFF)
//! - Annex-B (start-code-prefixed NAL units with `0x000001` or `0x00000001`,
//!   required by MPEG-TS, bare elementary streams, and Annex-B decoders).

use bytes::Bytes;

/// Errors arising during bitstream filtering.
#[derive(Debug, thiserror::Error)]
pub enum BsfError {
    #[error("unsupported NAL length size: {0} (must be 1, 2, or 4)")]
    UnsupportedNalLengthSize(u8),

    #[error(
        "truncated NAL unit: expected {expected} bytes at offset {offset}, but only {available} available"
    )]
    TruncatedNal {
        offset: usize,
        expected: usize,
        available: usize,
    },
}

/// Start code prefixes for Annex-B streams.
pub const ANNEX_B_START_CODE_4: [u8; 4] = [0x00, 0x00, 0x00, 0x01];
pub const ANNEX_B_START_CODE_3: [u8; 3] = [0x00, 0x00, 0x01];

/// Converts AVCC-framed NAL units (length-prefixed) into Annex-B format
/// (4-byte start codes `00 00 00 01`).
///
/// If `param_sets` (e.g. SPS and PPS NAL units) are provided (typically on keyframes),
/// they will be prepended with 4-byte start codes before any slice data.
///
/// # Arguments
/// - `data`: Raw AVCC packet data from the container.
/// - `nal_length_size`: Length size in bytes: 1, 2, or 4 (from `avcC.lengthSizeMinusOne + 1`).
/// - `param_sets`: Optional list of parameter set NAL units (without start codes or length headers).
/// - `output`: Destination vector to append the Annex-B stream to.
pub fn avcc_to_annex_b(
    data: &[u8],
    nal_length_size: u8,
    param_sets: Option<&[Bytes]>,
    output: &mut Vec<u8>,
) -> Result<(), BsfError> {
    if nal_length_size != 1 && nal_length_size != 2 && nal_length_size != 4 {
        return Err(BsfError::UnsupportedNalLengthSize(nal_length_size));
    }

    let nls = nal_length_size as usize;

    // Estimate capacity to reduce reallocations
    let param_sets_len: usize = param_sets.map_or(0, |ps| ps.iter().map(|s| 4 + s.len()).sum());
    output.reserve(param_sets_len + data.len() + 32);

    // If parameter sets are provided (e.g. on keyframe), emit them first with 4-byte start codes
    if let Some(ps_list) = param_sets {
        for ps in ps_list {
            output.extend_from_slice(&ANNEX_B_START_CODE_4);
            output.extend_from_slice(ps);
        }
    }

    let mut cursor = 0;
    while cursor < data.len() {
        if cursor + nls > data.len() {
            return Err(BsfError::TruncatedNal {
                offset: cursor,
                expected: nls,
                available: data.len() - cursor,
            });
        }

        let nal_len = match nal_length_size {
            1 => data[cursor] as usize,
            2 => u16::from_be_bytes([data[cursor], data[cursor + 1]]) as usize,
            4 => u32::from_be_bytes([
                data[cursor],
                data[cursor + 1],
                data[cursor + 2],
                data[cursor + 3],
            ]) as usize,
            _ => unreachable!(),
        };

        cursor += nls;

        if cursor + nal_len > data.len() {
            return Err(BsfError::TruncatedNal {
                offset: cursor,
                expected: nal_len,
                available: data.len() - cursor,
            });
        }

        output.extend_from_slice(&ANNEX_B_START_CODE_4);
        output.extend_from_slice(&data[cursor..cursor + nal_len]);
        cursor += nal_len;
    }

    Ok(())
}

/// Converts HVCC-framed HEVC NAL units (length-prefixed) into Annex-B format
/// (4-byte start codes `00 00 00 01`).
///
/// If parameter sets (`vps_list`, `sps_list`, `pps_list`) are provided (typically on keyframes),
/// they will be prepended with 4-byte start codes before any slice data.
pub fn hevc_to_annex_b(
    data: &[u8],
    nal_length_size: u8,
    vps_list: Option<&[Bytes]>,
    sps_list: Option<&[Bytes]>,
    pps_list: Option<&[Bytes]>,
    output: &mut Vec<u8>,
) -> Result<(), BsfError> {
    if nal_length_size != 1 && nal_length_size != 2 && nal_length_size != 4 {
        return Err(BsfError::UnsupportedNalLengthSize(nal_length_size));
    }

    let nls = nal_length_size as usize;

    let vps_len: usize = vps_list.map_or(0, |l| l.iter().map(|s| 4 + s.len()).sum());
    let sps_len: usize = sps_list.map_or(0, |l| l.iter().map(|s| 4 + s.len()).sum());
    let pps_len: usize = pps_list.map_or(0, |l| l.iter().map(|s| 4 + s.len()).sum());
    output.reserve(vps_len + sps_len + pps_len + data.len() + 32);

    if let Some(list) = vps_list {
        for nalu in list {
            output.extend_from_slice(&ANNEX_B_START_CODE_4);
            output.extend_from_slice(nalu);
        }
    }

    if let Some(list) = sps_list {
        for nalu in list {
            output.extend_from_slice(&ANNEX_B_START_CODE_4);
            output.extend_from_slice(nalu);
        }
    }

    if let Some(list) = pps_list {
        for nalu in list {
            output.extend_from_slice(&ANNEX_B_START_CODE_4);
            output.extend_from_slice(nalu);
        }
    }

    let mut cursor = 0;
    while cursor < data.len() {
        if cursor + nls > data.len() {
            return Err(BsfError::TruncatedNal {
                offset: cursor,
                expected: nls,
                available: data.len() - cursor,
            });
        }

        let nal_len = match nal_length_size {
            1 => data[cursor] as usize,
            2 => u16::from_be_bytes([data[cursor], data[cursor + 1]]) as usize,
            4 => u32::from_be_bytes([
                data[cursor],
                data[cursor + 1],
                data[cursor + 2],
                data[cursor + 3],
            ]) as usize,
            _ => unreachable!(),
        };

        cursor += nls;

        if cursor + nal_len > data.len() {
            return Err(BsfError::TruncatedNal {
                offset: cursor,
                expected: nal_len,
                available: data.len() - cursor,
            });
        }

        output.extend_from_slice(&ANNEX_B_START_CODE_4);
        output.extend_from_slice(&data[cursor..cursor + nal_len]);
        cursor += nal_len;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_avcc_to_annex_b_4byte_len() {
        let mut avcc = Vec::new();
        let nal1 = b"Hello";
        let nal2 = b"World123";

        avcc.extend_from_slice(&(nal1.len() as u32).to_be_bytes());
        avcc.extend_from_slice(nal1);
        avcc.extend_from_slice(&(nal2.len() as u32).to_be_bytes());
        avcc.extend_from_slice(nal2);

        let mut out = Vec::new();
        avcc_to_annex_b(&avcc, 4, None, &mut out).unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(nal1);
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(nal2);

        assert_eq!(out, expected);
    }

    #[test]
    fn test_avcc_to_annex_b_with_param_sets() {
        let sps = Bytes::from_static(b"SPS_DATA");
        let pps = Bytes::from_static(b"PPS_DATA");
        let nal = b"SLICE";

        let mut avcc = Vec::new();
        avcc.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        avcc.extend_from_slice(nal);

        let mut out = Vec::new();
        let params = [sps.clone(), pps.clone()];
        avcc_to_annex_b(&avcc, 4, Some(&params), &mut out).unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(b"SPS_DATA");
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(b"PPS_DATA");
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(b"SLICE");

        assert_eq!(out, expected);
    }

    #[test]
    fn test_hevc_to_annex_b_with_vps_sps_pps() {
        let vps = Bytes::from_static(b"VPS");
        let sps = Bytes::from_static(b"SPS");
        let pps = Bytes::from_static(b"PPS");
        let slice = b"HEVC_SLICE";

        let mut hvcc = Vec::new();
        hvcc.extend_from_slice(&(slice.len() as u32).to_be_bytes());
        hvcc.extend_from_slice(slice);

        let mut out = Vec::new();
        let vps_list = [vps];
        let sps_list = [sps];
        let pps_list = [pps];
        hevc_to_annex_b(
            &hvcc,
            4,
            Some(&vps_list),
            Some(&sps_list),
            Some(&pps_list),
            &mut out,
        )
        .unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(b"VPS");
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(b"SPS");
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(b"PPS");
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(b"HEVC_SLICE");

        assert_eq!(out, expected);
    }

    #[test]
    fn test_invalid_nal_length_size() {
        let mut out = Vec::new();
        let err = avcc_to_annex_b(&[0, 1, 2], 3, None, &mut out).unwrap_err();
        match err {
            BsfError::UnsupportedNalLengthSize(3) => {}
            _ => panic!("unexpected error"),
        }
    }
}
