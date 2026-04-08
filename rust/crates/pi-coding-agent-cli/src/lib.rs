#[derive(Debug, thiserror::Error)]
pub enum CodingAgentCliError {
    #[error("coding-agent cli migration pending")]
    Pending,
}
