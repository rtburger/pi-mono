use pi_ai::AiError;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum AgentError {
    #[error("Agent is already processing a prompt. Use steer() or followUp() to queue messages, or wait for completion.")]
    AlreadyProcessingPrompt,
    #[error("Agent is already processing. Wait for completion before continuing.")]
    AlreadyProcessingContinue,
    #[error("No messages to continue from")]
    NoMessagesToContinue,
    #[error("Cannot continue: no messages in context")]
    EmptyContext,
    #[error("Cannot continue from message role: assistant")]
    CannotContinueFromAssistant,
    #[error("assistant stream ended without a terminal event")]
    MissingTerminalEvent,
    #[error(transparent)]
    Ai(#[from] AiError),
}
