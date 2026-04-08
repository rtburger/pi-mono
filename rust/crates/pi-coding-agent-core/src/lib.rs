#[derive(Debug, thiserror::Error)]
pub enum CodingAgentCoreError {
    #[error("coding-agent core migration pending")]
    Pending,
}
