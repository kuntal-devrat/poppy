//! VP9 Superframe index parser.
//!
//! In VP9, multiple coded frames (e.g. non-displayed reference update frames
//! followed by displayed frames) can be bundled into a single container packet
//! using a "superframe" structure (VP9 Spec Annex B).
//!
//! The superframe index is stored at the end of the packet, framed by matching
//! marker bytes:
//!
//! ```text
//! [Frame 0] [Frame 1] ... [Marker Byte] [Size 0] [Size 1] ... [Marker Byte]
//! ```
//!
//! Marker byte layout (1 byte):
//! - Bit 7..5: `0b110` (Marker code `0xC0`)
//! - Bit 4..3: `bytes_per_framesize_minus_1` (0=1 byte, 1=2 bytes, 2=3 bytes, 3=4 bytes)
//! - Bit 2..0: `frames_in_superframe_minus_1` (0..7 -> 1..8 frames)

use crate::bitstream::Vp9ParserError;

/// Parse a VP9 packet buffer, unpacking superframe chunks into individual frame buffers.
///
/// If the packet is not a superframe (or contains only a single frame), returns
/// a single-element slice containing the entire input buffer.
pub fn parse_superframe_frames(packet: &[u8]) -> Result<Vec<&[u8]>, Vp9ParserError> {
    if packet.is_empty() {
        return Ok(Vec::new());
    }

    let last_byte = packet[packet.len() - 1];

    // Marker must start with 0b110 (0xC0..=0xDF)
    if (last_byte & 0xE0) != 0xC0 {
        return Ok(vec![packet]);
    }

    let bytes_per_size = (((last_byte >> 3) & 0x03) + 1) as usize;
    let num_frames = ((last_byte & 0x07) + 1) as usize;
    let index_size = 2 + num_frames * bytes_per_size;

    if packet.len() < index_size {
        // Not enough data for a valid superframe index; treat as regular frame
        return Ok(vec![packet]);
    }

    let first_marker_byte = packet[packet.len() - index_size];
    if first_marker_byte != last_byte {
        // Markers do not match, not a valid superframe index
        return Ok(vec![packet]);
    }

    // Parse little-endian frame sizes
    let mut frame_sizes = Vec::with_capacity(num_frames);
    let mut offset = packet.len() - index_size + 1;

    for _ in 0..num_frames {
        let mut size = 0usize;
        for b in 0..bytes_per_size {
            size |= (packet[offset + b] as usize) << (b * 8);
        }
        frame_sizes.push(size);
        offset += bytes_per_size;
    }

    // Validate and extract individual frame slices
    let mut frames = Vec::with_capacity(num_frames);
    let mut pos = 0usize;

    for (i, &size) in frame_sizes.iter().enumerate() {
        if pos + size > packet.len().saturating_sub(index_size) {
            return Err(Vp9ParserError::InvalidSuperframeSize {
                frame_idx: i,
                frame_size: size,
                packet_size: packet.len(),
            });
        }
        frames.push(&packet[pos..pos + size]);
        pos += size;
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_superframe_packet() {
        let dummy = vec![0x12, 0x34, 0x56, 0x78];
        let frames = parse_superframe_frames(&dummy).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], &dummy[..]);
    }

    #[test]
    fn test_superframe_two_frames() {
        let f1 = vec![0x01, 0x02, 0x03];
        let f2 = vec![0x04, 0x05, 0x06, 0x07];

        // Marker byte: 0b110_00_001 = 0xC1
        // bytes_per_size = 1 (bits 4..3 = 00 -> 1 byte)
        // num_frames = 2 (bits 2..0 = 001 -> 2 frames)
        let marker = 0b1100_0001; // 0xC1
        let mut packet = Vec::new();
        packet.extend_from_slice(&f1); // len = 3
        packet.extend_from_slice(&f2); // len = 4

        // Index: [marker, size1, size2, marker]
        packet.push(marker);
        packet.push(3); // size1
        packet.push(4); // size2
        packet.push(marker);

        let frames = parse_superframe_frames(&packet).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], &f1[..]);
        assert_eq!(frames[1], &f2[..]);
    }
}
