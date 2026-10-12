//! Decoded Picture Buffer (DPB) state machine for H.264 reference frames.
//!
//! Implements ITU-T H.264 Annex C (bumping process), §8.2.5 (sliding window marking),
//! and §8.2.4 (reference picture list construction for P- and B-slices).

use super::ffi::*;
use poppy_core::InterlaceMode;
use poppy_h264::RefPicListModification;

/// A frame stored in the Decoded Picture Buffer.
#[derive(Debug, Clone)]
pub struct DpbFrame {
    pub surface_id: VASurfaceID,
    pub frame_num: u32,
    pub poc: i32,
    pub is_reference: bool,
    pub is_long_term: bool,
    pub pts: Option<i64>,
    pub needed_for_output: bool,
    pub width: u32,
    pub height: u32,
    pub interlace_mode: InterlaceMode,
    pub is_concealed: bool,
}

impl Default for DpbFrame {
    fn default() -> Self {
        Self {
            surface_id: 0,
            frame_num: 0,
            poc: 0,
            is_reference: false,
            is_long_term: false,
            pts: None,
            needed_for_output: false,
            width: 1920,
            height: 1080,
            interlace_mode: InterlaceMode::Progressive,
            is_concealed: false,
        }
    }
}

/// The Decoded Picture Buffer manager.
pub struct Dpb {
    pub max_ref_frames: usize,
    pub max_reorder_frames: usize,
    pub frames: Vec<DpbFrame>,
}

impl Dpb {
    pub fn new(max_ref_frames: usize) -> Self {
        let max_refs = max_ref_frames.max(1);
        Self {
            max_ref_frames: max_refs,
            max_reorder_frames: max_refs,
            frames: Vec::new(),
        }
    }

    pub fn with_reorder(max_ref_frames: usize, max_reorder_frames: usize) -> Self {
        Self {
            max_ref_frames: max_ref_frames.max(1),
            max_reorder_frames: max_reorder_frames.max(1),
            frames: Vec::new(),
        }
    }

    /// Clear all frames and references from DPB immediately.
    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// Clear all references on IDR frame and bump any un-outputted frames in POC order.
    pub fn clear_on_idr(&mut self) -> Vec<DpbFrame> {
        let mut ready_for_output: Vec<DpbFrame> = self
            .frames
            .iter()
            .filter(|f| f.needed_for_output)
            .cloned()
            .collect();

        // Output in ascending POC order (ITU-T H.264 §C.4.5.1)
        ready_for_output.sort_by_key(|f| f.poc);

        self.frames.clear();
        ready_for_output
    }

    /// Insert a newly decoded frame into the DPB and run the bumping process.
    pub fn insert(&mut self, frame: DpbFrame) -> Vec<DpbFrame> {
        let mut output_frames = Vec::new();

        self.frames.push(frame);

        // 1. Sliding window reference marking (ITU-T H.264 §8.2.5.3)
        // If reference frames exceed max_ref_frames, unmark the oldest short-term reference frame.
        while self.frames.iter().filter(|f| f.is_reference).count() > self.max_ref_frames {
            if let Some(oldest) = self
                .frames
                .iter_mut()
                .find(|f| f.is_reference && !f.is_long_term)
            {
                oldest.is_reference = false;
            } else {
                break;
            }
        }

        // 2. Bumping process (ITU-T H.264 §C.4.5.2)
        // While un-outputted frames exceed max_reorder_frames, bump the frame with the smallest POC.
        while self.frames.iter().filter(|f| f.needed_for_output).count() > self.max_reorder_frames {
            // Find index of frame with minimum POC among those needing output
            let min_idx = self
                .frames
                .iter()
                .enumerate()
                .filter(|(_, f)| f.needed_for_output)
                .min_by_key(|(_, f)| f.poc)
                .map(|(i, _)| i);

            if let Some(idx) = min_idx {
                let bumped = self.frames[idx].clone();
                self.frames[idx].needed_for_output = false;
                output_frames.push(bumped);
            } else {
                break;
            }
        }

        // 3. Cleanup: remove frames that are no longer reference and already outputted
        self.frames
            .retain(|f| f.is_reference || f.needed_for_output);

        output_frames
    }

    /// Fill `reference_frames[16]` for `VAPictureParameterBufferH264`.
    pub fn fill_reference_frames(&self, ref_frames: &mut [VAPictureH264; 16]) {
        for (i, slot) in ref_frames.iter_mut().enumerate() {
            if let Some(f) = self.frames.iter().filter(|f| f.is_reference).nth(i) {
                *slot = VAPictureH264 {
                    picture_id: f.surface_id,
                    frame_idx: f.frame_num,
                    flags: if f.is_long_term {
                        VA_PICTURE_H264_LONG_TERM_REFERENCE
                    } else {
                        VA_PICTURE_H264_SHORT_TERM_REFERENCE
                    },
                    top_field_order_cnt: f.poc,
                    bottom_field_order_cnt: f.poc,
                    va_reserved: [0; 4],
                };
            } else {
                *slot = VAPictureH264::default();
            }
        }
    }

    /// Fill `RefPicList0[32]` for `VASliceParameterBufferH264` (P-slices and B-slices).
    pub fn fill_ref_pic_list0(
        &self,
        cur_poc: i32,
        cur_frame_num: u32,
        max_frame_num: u32,
        modifications: &[RefPicListModification],
        list: &mut [VAPictureH264; 32],
    ) {
        // H.264 §8.2.4.2: Short-term reference frames sorted by POC
        let mut short_terms: Vec<&DpbFrame> = self
            .frames
            .iter()
            .filter(|f| f.is_reference && !f.is_long_term)
            .collect();

        // Sort descending for POC < cur_poc, ascending for POC > cur_poc
        short_terms.sort_by(|a, b| {
            let a_less = a.poc < cur_poc;
            let b_less = b.poc < cur_poc;
            if a_less && b_less {
                b.poc.cmp(&a.poc) // descending
            } else if !a_less && !b_less {
                a.poc.cmp(&b.poc) // ascending
            } else if a_less {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });

        // Apply RPLR modifications (ITU-T H.264 §8.2.4.3)
        if !modifications.is_empty() && max_frame_num > 0 {
            let mut pic_num_pred = cur_frame_num as i32;
            let mut ref_idx = 0usize;
            for m in modifications {
                match m.modification_of_pic_nums_idc {
                    0 => {
                        let diff = m.value as i32 + 1;
                        pic_num_pred = (pic_num_pred - diff).rem_euclid(max_frame_num as i32);
                        if let Some(pos) = short_terms
                            .iter()
                            .position(|f| (f.frame_num as i32) == pic_num_pred)
                        {
                            let frame = short_terms.remove(pos);
                            short_terms.insert(ref_idx.min(short_terms.len()), frame);
                            ref_idx += 1;
                        }
                    }
                    1 => {
                        let diff = m.value as i32 + 1;
                        pic_num_pred = (pic_num_pred + diff).rem_euclid(max_frame_num as i32);
                        if let Some(pos) = short_terms
                            .iter()
                            .position(|f| (f.frame_num as i32) == pic_num_pred)
                        {
                            let frame = short_terms.remove(pos);
                            short_terms.insert(ref_idx.min(short_terms.len()), frame);
                            ref_idx += 1;
                        }
                    }
                    _ => break,
                }
            }
        }

        for (i, slot) in list.iter_mut().enumerate() {
            if let Some(f) = short_terms.get(i) {
                *slot = VAPictureH264 {
                    picture_id: f.surface_id,
                    frame_idx: f.frame_num,
                    flags: VA_PICTURE_H264_SHORT_TERM_REFERENCE,
                    top_field_order_cnt: f.poc,
                    bottom_field_order_cnt: f.poc,
                    va_reserved: [0; 4],
                };
            } else {
                *slot = VAPictureH264::default();
            }
        }
    }

    /// Fill `RefPicList1[32]` for `VASliceParameterBufferH264` (B-slices).
    pub fn fill_ref_pic_list1(
        &self,
        cur_poc: i32,
        cur_frame_num: u32,
        max_frame_num: u32,
        modifications: &[RefPicListModification],
        list: &mut [VAPictureH264; 32],
    ) {
        let mut short_terms: Vec<&DpbFrame> = self
            .frames
            .iter()
            .filter(|f| f.is_reference && !f.is_long_term)
            .collect();

        // H.264 §8.2.4.2.4: For RefPicList1, frames with POC > cur_poc come first in ascending order,
        // followed by frames with POC < cur_poc in descending order.
        short_terms.sort_by(|a, b| {
            let a_greater = a.poc > cur_poc;
            let b_greater = b.poc > cur_poc;
            if a_greater && b_greater {
                a.poc.cmp(&b.poc) // ascending
            } else if !a_greater && !b_greater {
                b.poc.cmp(&a.poc) // descending
            } else if a_greater {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });

        // Apply RPLR modifications (ITU-T H.264 §8.2.4.3)
        if !modifications.is_empty() && max_frame_num > 0 {
            let mut pic_num_pred = cur_frame_num as i32;
            let mut ref_idx = 0usize;
            for m in modifications {
                match m.modification_of_pic_nums_idc {
                    0 => {
                        let diff = m.value as i32 + 1;
                        pic_num_pred = (pic_num_pred - diff).rem_euclid(max_frame_num as i32);
                        if let Some(pos) = short_terms
                            .iter()
                            .position(|f| (f.frame_num as i32) == pic_num_pred)
                        {
                            let frame = short_terms.remove(pos);
                            short_terms.insert(ref_idx.min(short_terms.len()), frame);
                            ref_idx += 1;
                        }
                    }
                    1 => {
                        let diff = m.value as i32 + 1;
                        pic_num_pred = (pic_num_pred + diff).rem_euclid(max_frame_num as i32);
                        if let Some(pos) = short_terms
                            .iter()
                            .position(|f| (f.frame_num as i32) == pic_num_pred)
                        {
                            let frame = short_terms.remove(pos);
                            short_terms.insert(ref_idx.min(short_terms.len()), frame);
                            ref_idx += 1;
                        }
                    }
                    _ => break,
                }
            }
        }

        for (i, slot) in list.iter_mut().enumerate() {
            if let Some(f) = short_terms.get(i) {
                *slot = VAPictureH264 {
                    picture_id: f.surface_id,
                    frame_idx: f.frame_num,
                    flags: VA_PICTURE_H264_SHORT_TERM_REFERENCE,
                    top_field_order_cnt: f.poc,
                    bottom_field_order_cnt: f.poc,
                    va_reserved: [0; 4],
                };
            } else {
                *slot = VAPictureH264::default();
            }
        }
    }

    /// Drain all remaining frames for flush at EOS, outputting in strictly ascending POC order.
    pub fn drain(&mut self) -> Vec<DpbFrame> {
        let mut res: Vec<DpbFrame> = self
            .frames
            .iter()
            .filter(|f| f.needed_for_output)
            .cloned()
            .collect();

        res.sort_by_key(|f| f.poc);
        self.frames.clear();
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dpb_b_frame_bumping_order() {
        let mut dpb = Dpb::with_reorder(4, 2);

        // Decode order: Frame 0 (POC 0), Frame 2 (POC 4), Frame 1 (POC 2)
        // With max_reorder_frames = 2:
        let out0 = dpb.insert(DpbFrame {
            surface_id: 10,
            frame_num: 0,
            poc: 0,
            is_reference: true,
            is_long_term: false,
            pts: Some(0),
            needed_for_output: true,
            ..Default::default()
        });
        assert_eq!(out0.len(), 0);

        let out1 = dpb.insert(DpbFrame {
            surface_id: 11,
            frame_num: 1,
            poc: 4,
            is_reference: true,
            is_long_term: false,
            pts: Some(40),
            needed_for_output: true,
            ..Default::default()
        });
        assert_eq!(out1.len(), 0);

        // Third frame triggers bumping since count(needed_for_output) becomes 3 > max_reorder(2)
        let out2 = dpb.insert(DpbFrame {
            surface_id: 12,
            frame_num: 2,
            poc: 2,
            is_reference: false,
            is_long_term: false,
            pts: Some(20),
            needed_for_output: true,
            ..Default::default()
        });
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].poc, 0); // Smallest POC (0) is bumped first!

        // Drain at EOS
        let drained = dpb.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].poc, 2); // Then POC 2
        assert_eq!(drained[1].poc, 4); // Then POC 4
    }

    #[test]
    fn test_dpb_sliding_window_marking() {
        let mut dpb = Dpb::new(2);

        dpb.insert(DpbFrame {
            surface_id: 1,
            frame_num: 0,
            poc: 0,
            is_reference: true,
            is_long_term: false,
            pts: None,
            needed_for_output: false,
            ..Default::default()
        });
        dpb.insert(DpbFrame {
            surface_id: 2,
            frame_num: 1,
            poc: 2,
            is_reference: true,
            is_long_term: false,
            pts: None,
            needed_for_output: false,
            ..Default::default()
        });

        assert_eq!(dpb.frames.iter().filter(|f| f.is_reference).count(), 2);

        // Inserting 3rd reference frame unmarks oldest
        dpb.insert(DpbFrame {
            surface_id: 3,
            frame_num: 2,
            poc: 4,
            is_reference: true,
            is_long_term: false,
            pts: None,
            needed_for_output: false,
            ..Default::default()
        });

        assert_eq!(dpb.frames.iter().filter(|f| f.is_reference).count(), 2);
        assert!(
            !dpb.frames
                .iter()
                .any(|f| f.surface_id == 1 && f.is_reference)
        );
    }
}
