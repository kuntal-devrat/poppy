//! Linux VA-API hardware decode backend.

pub mod decoder;
pub mod device;
pub mod dpb;
pub mod ffi;
pub mod surface;

pub use decoder::VaapiH264Decoder;
pub use device::VaDevice;
pub use surface::{DmaBufSurface, SurfacePool};
