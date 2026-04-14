use crate::{AgentEvent, AgentMessage, AgentTool};
use indexmap::IndexSet;
use pi_events::Model;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingLevel {
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentTool>,
}

impl AgentContext {
    pub fn new(system_prompt: impl Into<String>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            messages: Vec::new(),
            tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentState {
    pub system_prompt: String,
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub tools: Vec<AgentTool>,
    pub messages: Vec<AgentMessage>,
    pub is_streaming: bool,
    pub streaming_message: Option<AgentMessage>,
    pub pending_tool_calls: IndexSet<String>,
    pub error_message: Option<String>,
}

impl AgentState {
    pub fn new(model: Model) -> Self {
        Self {
            system_prompt: String::new(),
            model,
            thinking_level: ThinkingLevel::Off,
            tools: Vec::new(),
            messages: Vec::new(),
            is_streaming: false,
            streaming_message: None,
            pending_tool_calls: IndexSet::new(),
            error_message: None,
        }
    }

    pub fn context_snapshot(&self) -> AgentContext {
        AgentContext {
            system_prompt: self.system_prompt.clone(),
            messages: self.messages.clone(),
            tools: self.tools.clone(),
        }
    }

    pub fn begin_run(&mut self) {
        self.is_streaming = true;
        self.streaming_message = None;
        self.error_message = None;
    }

    pub fn apply_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::AgentStart | AgentEvent::TurnStart => {}
            AgentEvent::AgentEnd { .. } => {
                self.streaming_message = None;
            }
            AgentEvent::TurnEnd { message, .. } => {
                self.error_message = message.error_message.clone();
            }
            AgentEvent::MessageStart { message } | AgentEvent::MessageUpdate { message, .. } => {
                self.streaming_message = Some(message.clone());
            }
            AgentEvent::MessageEnd { message } => {
                self.streaming_message = None;
                self.messages.push(message.clone());
            }
            AgentEvent::ToolExecutionStart { tool_call_id, .. } => {
                self.pending_tool_calls.insert(tool_call_id.clone());
            }
            AgentEvent::ToolExecutionUpdate { .. } => {}
            AgentEvent::ToolExecutionEnd { tool_call_id, .. } => {
                self.pending_tool_calls.shift_remove(tool_call_id);
            }
        }
    }

    pub fn finish_run(&mut self) {
        self.is_streaming = false;
        self.streaming_message = None;
        self.pending_tool_calls.clear();
    }
}
