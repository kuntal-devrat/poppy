//! Linux VA-API hardware decode backend.

pub mod av1_decoder;
pub mod decoder;
pub mod device;
pub mod dpb;
pub mod ffi;
pub mod hevc_decoder;
pub mod surface;
pub mod vp9_decoder;

pub use av1_decoder::VaapiAv1Decoder;
pub use decoder::VaapiH264Decoder;
pub use device::VaDevice;
pub use hevc_decoder::VaapiHevcDecoder;
pub use surface::{DmaBufSurface, SurfacePool};
pub use vp9_decoder::VaapiVp9Decoder;
