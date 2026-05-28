//! Headless integration test for async tool execution.

#[cfg(test)]
mod tests {
    use std::path::Path;

    use pi_core::{
        AgentMessage, AgentOptions, AgentRuntime, Model, ModelCapabilities, ModelCost, ModelId,
        ModelName, ProviderName, ToolDefinition,
    };

    use crate::extension::{BashExtension, BuiltinExtension, Extension, ExtensionContext};
    use crate::llm::LlmClient;

    fn dummy_model() -> Model {
        Model {
            id: ModelId("accounts/fireworks/routers/kimi-k2p6-turbo".to_string()),
            name: ModelName("kimi".to_string()),
            api: pi_core::ApiName("fireworks".to_string()),
            provider: ProviderName("fireworks".to_string()),
            base_url: None,
            reasoning: false,
            context_window: 4096,
            max_tokens: 1024,
            capabilities: ModelCapabilities {
                vision: false,
                json_mode: true,
                function_calling: true,
                streaming: true,
            },
            cost: ModelCost::default(),
        }
    }

    fn build_tools() -> Vec<ToolDefinition> {
        let mut defs = BuiltinExtension::new().tools();
        defs.extend(BashExtension::new().tools());
        defs
    }

    #[test]
    fn smoke_async_bash_with_fireworks() {
        let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
        if api_key.is_empty() {
            eprintln!("SKIP: no ANTHROPIC_API_KEY set");
            return;
        }

        let llm_client = LlmClient::new(
            &api_key,
            "https://api.fireworks.ai/inference",
            "accounts/fireworks/routers/kimi-k2p6-turbo",
        );

        let options = AgentOptions {
            system_prompt: "You are a helpful coding assistant. Use tools when needed.".to_string(),
            model: dummy_model(),
            thinking_level: Default::default(),
            steering_mode: Default::default(),
            follow_up_mode: Default::default(),
            tool_execution_mode: Default::default(),
            session_id: None,
            messages: vec![],
            session_state: None,
        };
        let tools = build_tools();

        let mut runtime = AgentRuntime::new(options);

        // Start turn
        let AgentRuntime::Idle(idle) = runtime else {
            panic!("expected Idle");
        };
        let t = idle.start_turn(
            AgentMessage::user("run 'sleep 1 && echo hello from bash'"),
            tools,
        );
        println!("start_turn events: {:?}", t.events);
        println!("start_turn actions: {:?}", t.actions);
        runtime = t.state.into_runtime();

        // Expect StreamLlm
        let context = match t.actions.into_iter().next() {
            Some(pi_core::AgentAction::StreamLlm { context, .. }) => context,
            other => panic!("expected StreamLlm, got {:?}", other),
        };

        // Stream LLM
        let mut stream = llm_client
            .stream_sync(&context.system_prompt, &context.messages, &context.tools)
            .expect("stream_sync failed");

        let mut chunks = vec![];
        for chunk in stream.by_ref() {
            println!("LLM chunk: {:?}", chunk);
            chunks.push(chunk);
            if chunks.len() > 200 {
                panic!("too many chunks, likely streaming forever");
            }
        }

        // Build LlmResult
        let assistant_msg = pi_core::AssistantMessage {
            content: if stream.tool_calls().is_empty() {
                vec![pi_core::Content::Text(pi_core::message::TextContent {
                    text: chunks
                        .iter()
                        .filter_map(|c| {
                            if let pi_core::LlmChunk::TextDelta { text } = c {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .concat(),
                })]
            } else {
                stream
                    .tool_calls()
                    .into_iter()
                    .map(|tc| {
                        pi_core::Content::ToolCall(pi_core::ToolCall {
                            id: pi_core::ToolCallId::new(&tc.id),
                            name: pi_core::ToolName::new(&tc.name),
                            arguments: pi_core::ToolArguments::new(tc.input),
                        })
                    })
                    .collect()
            },
            api: pi_core::ApiName("fireworks".to_string()),
            provider: ProviderName("fireworks".to_string()),
            model: ModelId("accounts/fireworks/routers/kimi-k2p6-turbo".to_string()),
            stop_reason: match stream.stop_reason().unwrap_or("end_turn") {
                "end_turn" => pi_core::StopReason::EndTurn,
                "max_tokens" => pi_core::StopReason::MaxTokens,
                "tool_use" => pi_core::StopReason::ToolUse,
                _ => pi_core::StopReason::EndTurn,
            },
            error_message: None,
            timestamp: pi_core::timestamp::current_timestamp(),
            usage: pi_core::message::TokenUsage::default(),
        };

        let (events, actions, new_runtime) = match runtime {
            AgentRuntime::Streaming(streaming) => streaming
                .finish_llm(pi_core::LlmResult::Ok(assistant_msg))
                .into_parts(),
            _ => panic!("expected Streaming, got non-Streaming AgentRuntime"),
        };
        runtime = new_runtime;
        println!("finish_llm events: {:?}", events);
        println!("finish_llm actions: {:?}", actions);

        // Execute tools
        let mut running_tasks: Vec<(
            pi_core::ToolCallId,
            String,
            Box<dyn crate::extension::ToolEventStream>,
        )> = vec![];
        for action in actions {
            match action {
                pi_core::AgentAction::ExecuteTools { calls } => {
                    for call in calls {
                        let ctx = ExtensionContext {
                            cwd: Path::new("/tmp").to_path_buf(),
                        };
                        let outcome = if call.name.as_str() == "bash" {
                            BashExtension::new().execute(&call, &ctx)
                        } else {
                            BuiltinExtension::new().execute(&call, &ctx)
                        };
                        match outcome {
                            crate::extension::ExtensionOutcome::Complete(result) => {
                                println!("Sync tool result: {:?}", result);
                                runtime = match runtime {
                                    AgentRuntime::WaitingTools(waiting) => {
                                        let transition =
                                            waiting.on_tool_done(call.id.clone(), result);
                                        match transition {
                                            pi_core::ToolTransition::Ready(t) => {
                                                t.state.into_runtime()
                                            }
                                            pi_core::ToolTransition::Finished(t) => {
                                                t.state.into_runtime()
                                            }
                                            pi_core::ToolTransition::WaitingTools(t) => {
                                                t.state.into_runtime()
                                            }
                                        }
                                    }
                                    other => other,
                                };
                            }
                            crate::extension::ExtensionOutcome::Running(stream) => {
                                running_tasks.push((
                                    call.id.clone(),
                                    call.name.as_str().to_string(),
                                    stream,
                                ));
                            }
                        }
                    }
                }
                other => println!("other action: {:?}", other),
            }
        }

        // Poll running tasks — use typestate transitions for async completions
        let start = std::time::Instant::now();
        while !running_tasks.is_empty() && start.elapsed() < std::time::Duration::from_secs(30) {
            let mut remaining = vec![];
            for (id, name, mut stream) in running_tasks {
                let mut done = false;
                while let Some(event) = stream.try_recv() {
                    match event {
                        crate::extension::ToolEvent::Update(u) => {
                            println!("Tool update: {} {:?} {}", name, u.stream, u.chunk);
                        }
                        crate::extension::ToolEvent::Done(result) => {
                            println!("Tool done: {} {:?}", name, result);
                            runtime = match runtime {
                                AgentRuntime::WaitingTools(waiting) => {
                                    let transition = waiting.on_tool_done(id.clone(), result);
                                    match transition {
                                        pi_core::ToolTransition::WaitingTools(t) => {
                                            t.state.into_runtime()
                                        }
                                        pi_core::ToolTransition::Ready(t) => t.state.into_runtime(),
                                        pi_core::ToolTransition::Finished(t) => {
                                            t.state.into_runtime()
                                        }
                                    }
                                }
                                other => other,
                            };
                            done = true;
                        }
                    }
                }
                if !done {
                    remaining.push((id, name, stream));
                }
            }
            running_tasks = remaining;
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            running_tasks.is_empty(),
            "tools did not complete within 30s"
        );

        // Auto-continue
        let actions = match runtime {
            AgentRuntime::ReadyToContinue(ready) => {
                let transition = ready.continue_turn();
                println!("continue_turn events: {:?}", transition.events);
                println!("continue_turn actions: {:?}", transition.actions);
                runtime = transition.state.into_runtime();
                transition.actions
            }
            _ => {
                println!(
                    "Not ReadyToContinue after tools: agent runtime not in ReadyToContinue state"
                );
                return;
            }
        };

        // Stream second LLM response
        if let Some(pi_core::AgentAction::StreamLlm { context, .. }) = actions.into_iter().next() {
            println!("Streaming second LLM response...");
            let mut stream = llm_client
                .stream_sync(&context.system_prompt, &context.messages, &context.tools)
                .expect("second stream_sync failed");

            let mut chunks = vec![];
            for chunk in stream.by_ref() {
                println!("Second LLM chunk: {:?}", chunk);
                chunks.push(chunk);
                if chunks.len() > 200 {
                    panic!("too many chunks in second stream");
                }
            }
            println!("Second stream done. stop_reason={:?}", stream.stop_reason());
        }
    }
}
