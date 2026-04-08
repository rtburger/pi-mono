#[derive(Debug, thiserror::Error)]
pub enum CodingAgentTuiError {
    #[error("coding-agent tui migration pending")]
    Pending,
}
