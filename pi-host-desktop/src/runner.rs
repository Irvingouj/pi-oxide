//! Desktop runner: tokio async event loop driving the synchronous pi-core agent.
//!
//! The runner owns the tokio runtime, makes HTTP requests, executes shell commands,
//! and calls into core via synchronous callbacks after each async operation completes.

use pi_core::{Agent, AgentAction, AgentEvent, AgentMessage, LlmChunk, LlmResult};

pub struct DesktopRunner {
    agent: Agent,
}

impl DesktopRunner {
    pub fn new(agent: Agent) -> Self {
        Self { agent }
    }

    /// Entry point for running a prompt.
    pub async fn run_prompt(&mut self, text: String) {
        let prompt = AgentMessage::user(text);
        let (_events, actions) = self.agent.start_turn(prompt);
        self.handle_actions(actions).await;
    }

    async fn handle_actions(&mut self, actions: Vec<AgentAction>) {
        for action in actions {
            match action {
                AgentAction::StreamLlm { context, .. } => {
                    self.stream_llm(context).await;
                }
                AgentAction::ExecuteTools { calls } => {
                    self.execute_tools(calls).await;
                }
                AgentAction::Finished { .. } => {
                    // Run complete
                }
                _ => {}
            }
        }
    }

    async fn stream_llm(&mut self, _context: pi_core::LlmContext) {
        // TODO: HTTP request via reqwest, feed chunks back to core
        unimplemented!("stream_llm")
    }

    async fn execute_tools(&mut self, _calls: Vec<pi_core::ToolCall>) {
        // TODO: spawn tool execution, call back into core when done
        unimplemented!("execute_tools")
    }
}
