use super::*;

pub(crate) fn convert_actions_to_directives(
    actions: Vec<pi_core::AgentAction>,
) -> Result<Vec<HostDirective>, serde_json::Error> {
    let mut directives = Vec::new();

    for action in actions {
        match action {
            pi_core::AgentAction::StreamLlm { context, .. } => {
                let projected_context: LlmContext = context.try_into()?;
                directives.push(HostDirective::StreamLlm {
                    context: projected_context,
                });
            }
            pi_core::AgentAction::Summarize { context, .. } => {
                let summary_context: LlmContext = context.try_into()?;
                directives.push(HostDirective::Summarize {
                    context: summary_context,
                });
            }
            pi_core::AgentAction::ExecuteTools { calls } => {
                directives.push(HostDirective::ExecuteTools {
                    calls: calls
                        .into_iter()
                        .map(|c| c.try_into())
                        .collect::<Result<Vec<_>, _>>()?,
                });
            }
            pi_core::AgentAction::CancelTools {
                tool_call_ids,
                reason,
            } => {
                directives.push(HostDirective::CancelTools {
                    tool_call_ids: tool_call_ids
                        .into_iter()
                        .map(|id| id.try_into())
                        .collect::<Result<Vec<_>, _>>()?,
                    reason: reason.try_into()?,
                });
            }
            pi_core::AgentAction::WaitForInput { mode } => {
                directives.push(HostDirective::WaitForInput {
                    mode: mode.try_into()?,
                });
            }
            pi_core::AgentAction::Finished => {
                directives.push(HostDirective::Finished);
                directives.push(HostDirective::Persist);
            }
        }
    }
    Ok(directives)
}

pub(crate) fn convert_events(
    events: Vec<pi_core::AgentEvent>,
) -> Result<Vec<AgentEvent>, serde_json::Error> {
    events
        .into_iter()
        .map(|e| e.try_into())
        .collect::<Result<Vec<_>, _>>()
}
