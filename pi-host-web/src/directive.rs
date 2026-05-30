use super::*;

pub(crate) fn convert_actions_to_directives(
    host_state: &mut HostState,
    actions: Vec<pi_core::AgentAction>,
) -> Result<Vec<HostDirective>, serde_json::Error> {
    let mut directives = Vec::new();
    let system_prompt = host_state.system_prompt.clone();

    for action in actions {
        match action {
            pi_core::AgentAction::StreamLlm { context, .. } => {
                // Clear any stale compaction plan from a previous turn.
                host_state.pending_compaction_plans.clear();
                let (projected_messages, report) =
                    host_state.project(&system_prompt, &context.messages);
                eprintln!("DEBUG: report = {:?}", report);
                let projected_context = LlmContext {
                    system_prompt: system_prompt.clone(),
                    messages: projected_messages
                        .into_iter()
                        .map(|m| m.try_into())
                        .collect::<Result<Vec<_>, _>>()?,
                    tools: context
                        .tools
                        .into_iter()
                        .map(|t| t.try_into())
                        .collect::<Result<Vec<_>, _>>()?,
                };
                directives.push(HostDirective::StreamLlm {
                    context: projected_context,
                });
                if report.needs_compaction {
                    let plan = host_state.plan_compaction();
                    eprintln!("DEBUG: plan = {:?}", plan);
                    if let Some(plan) = plan {
                        host_state.pending_compaction_plans.push((host_state.leaf_id.clone(), plan.clone()));
                        directives.push(HostDirective::Compact {
                            compact_up_to: host_state.leaf_id.clone(),
                            reason: CompactReason::OverBudget {
                                estimated_tokens: report.estimated_tokens,
                                budget_tokens: host_state.budget.max_context_tokens,
                            },
                        });
                    }
                }
            }
            pi_core::AgentAction::ExecuteTools { calls } => {
                directives.push(HostDirective::ExecuteTools {
                    calls: calls
                        .into_iter()
                        .map(|c| c.try_into())
                        .collect::<Result<Vec<_>, _>>()?,
                });
            }
            pi_core::AgentAction::CancelTools { tool_call_ids, reason } => {
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
            pi_core::AgentAction::Finished { .. } => {
                directives.push(HostDirective::Finished);
                directives.push(HostDirective::Persist);
            }
        }
    }
    Ok(directives)
}

pub(crate) fn convert_events(events: Vec<pi_core::AgentEvent>) -> Result<Vec<AgentEvent>, serde_json::Error> {
    events.into_iter().map(|e| e.try_into()).collect::<Result<Vec<_>, _>>()
}
