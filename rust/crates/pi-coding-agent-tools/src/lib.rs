#[derive(Debug, thiserror::Error)]
pub enum CodingAgentToolsError {
    #[error("coding-agent tools migration pending")]
    Pending,
}
