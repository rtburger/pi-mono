#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("tui migration pending")]
    Pending,
}
