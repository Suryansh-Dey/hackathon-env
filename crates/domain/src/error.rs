use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("invalid id: {0}")]
    InvalidId(String),
    #[error("unknown category: {0}")]
    UnknownCategory(String),
    #[error("arithmetic overflow")]
    Overflow,
}

impl DomainError {
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }
}
