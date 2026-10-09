//! Decoded Picture Buffer (DPB) state machine for H.264 reference frames.
//!
//! Implements ITU-T H.264 §8.2.5 (sliding window marking) and DPB output reordering.

use super::ffi::*;

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
}

/// The Decoded Picture Buffer manager.
pub struct Dpb {
    pub max_ref_frames: usize,
    pub frames: Vec<DpbFrame>,
}

impl Dpb {
    pub fn new(max_ref_frames: usize) -> Self {
        Self {
            max_ref_frames: max_ref_frames.max(1),
            frames: Vec::new(),
        }
    }

    /// Clear all references on IDR frame.
    pub fn clear_on_idr(&mut self) -> Vec<DpbFrame> {
        let mut ready_for_output = Vec::new();
        for f in &mut self.frames {
            f.is_reference = false;
            if f.needed_for_output {
                ready_for_output.push(f.clone());
                f.needed_for_output = false;
            }
        }
        self.frames.clear();
        ready_for_output
    }

    /// Insert a newly decoded frame into the DPB.
    pub fn insert(&mut self, frame: DpbFrame) -> Vec<DpbFrame> {
        let mut output_frames = Vec::new();

        // Sliding window: if reference frames exceed max_ref_frames, unmark oldest
        let ref_count = self.frames.iter().filter(|f| f.is_reference).count();
        if ref_count >= self.max_ref_frames {
            // Find oldest short-term reference frame
            if let Some(oldest) = self.frames.iter_mut().find(|f| f.is_reference && !f.is_long_term) {
                oldest.is_reference = false;
                if oldest.needed_for_output {
                    output_frames.push(oldest.clone());
                    oldest.needed_for_output = false;
                }
            }
        }

        self.frames.push(frame);

        // Remove frames that are no longer reference and already outputted
        self.frames.retain(|f| f.is_reference || f.needed_for_output);

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
                };
            } else {
                *slot = VAPictureH264::default();
            }
        }
    }

    /// Fill `RefPicList0[32]` for `VASliceParameterBufferH264`.
    pub fn fill_ref_pic_list0(&self, cur_poc: i32, list: &mut [VAPictureH264; 32]) {
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

        for (i, slot) in list.iter_mut().enumerate() {
            if let Some(f) = short_terms.get(i) {
                *slot = VAPictureH264 {
                    picture_id: f.surface_id,
                    frame_idx: f.frame_num,
                    flags: VA_PICTURE_H264_SHORT_TERM_REFERENCE,
                    top_field_order_cnt: f.poc,
                    bottom_field_order_cnt: f.poc,
                };
            } else {
                *slot = VAPictureH264::default();
            }
        }
    }

    /// Drain all remaining frames for flush at EOS.
    pub fn drain(&mut self) -> Vec<DpbFrame> {
        let mut res = Vec::new();
        for f in &mut self.frames {
            if f.needed_for_output {
                res.push(f.clone());
                f.needed_for_output = false;
            }
        }
        self.frames.clear();
        res
    }
}
