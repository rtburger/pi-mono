#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("agent migration pending")]
    Pending,
}
