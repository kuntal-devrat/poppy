//! Shared error types for poppy-core.

/// Errors originating from core type operations.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// A timestamp conversion overflowed.
    #[error("timestamp overflow: {context}")]
    TimestampOverflow { context: String },
}
