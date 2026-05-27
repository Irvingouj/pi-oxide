use super::Agent;
use crate::events::{AgentEvent, QueueMode};
use crate::message::AgentMessage;
use tracing::debug;

impl Agent {
    /// Inject a steering message mid-run.
    pub(crate) fn steer(&mut self, message: AgentMessage) -> Vec<AgentEvent> {
        self.steering_queue.push(message);
        debug!(
            queued = self.steering_queue.len(),
            "steering message queued"
        );
        vec![AgentEvent::QueueUpdate {
            steer: self.steering_queue.clone(),
            follow_up: self.follow_up_queue.clone(),
        }]
    }

    /// Queue a follow-up message for after the run would otherwise stop.
    pub(crate) fn follow_up(&mut self, message: AgentMessage) {
        self.follow_up_queue.push(message);
        debug!(
            queued = self.follow_up_queue.len(),
            "follow-up message queued"
        );
    }

    pub(crate) fn drain_steering(&mut self) -> Vec<AgentMessage> {
        if self.steering_mode == QueueMode::All {
            std::mem::take(&mut self.steering_queue)
        } else {
            if self.steering_queue.is_empty() {
                vec![]
            } else {
                vec![self.steering_queue.remove(0)]
            }
        }
    }

    pub(crate) fn drain_follow_up(&mut self) -> Vec<AgentMessage> {
        if self.follow_up_mode == QueueMode::All {
            std::mem::take(&mut self.follow_up_queue)
        } else {
            if self.follow_up_queue.is_empty() {
                vec![]
            } else {
                vec![self.follow_up_queue.remove(0)]
            }
        }
    }
}
