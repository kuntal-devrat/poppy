//! # poppy-audio
//!
//! Pure-Rust audio decoding engine and playback sink for the Poppy media pipeline.
//!
//! ## Example
//!
//! ```
//! use poppy_audio::AudioClock;
//!
//! let clock = AudioClock::new();
//! assert_eq!(clock.current_pts_seconds(), 0.0);
//!
//! clock.reset_to(1.5);
//! assert!((clock.current_pts_seconds() - 1.5).abs() < 1e-3);
//! ```

pub mod decoder;
pub mod error;
pub mod sink;

pub use decoder::{AudioDecoder, create_audio_decoder};
pub use error::AudioError;
pub use sink::{AudioClock, AudioSink};
