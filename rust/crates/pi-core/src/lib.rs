pub type PiResult<T, E = PiError> = Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum PiError {
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
