/**
 * useAgent hook — bridges the WASM agent lifecycle to React/Zustand.
 *
 * Replaces the old bootstrap() function. Creates the agent on mount,
 * wires events to the agentStore, and exposes send/steer/stop actions.
 */

import type { Agent, AgentEvent } from "@pi-oxide/pi-host-web";
import { useCallback, useEffect, useRef, useState } from "react";
import { LiveBrowserRuntime } from "../browser/liveRuntime.ts";
import {
	createAgent,
	getSessionState,
	runTurn,
	steerAgent,
	stopAgent,
} from "../services/agentService.ts";
import { createLlmProvider } from "../services/llmService.ts";
import { runProjection } from "../services/projectionService.ts";
import { createToolRegistry } from "../services/toolService.ts";
import { useAgentStore } from "../stores/agentStore.ts";
import { useSessionStore } from "../stores/sessionStore.ts";

const SESSION_ID = "browser-default-session";

function eventToStoreAction(
	event: AgentEvent,
	store: ReturnType<typeof useAgentStore.getState>,
) {
	switch (event.type) {
		case "message_start": {
			store.addMessage({
				id: `msg-${Date.now()}`,
				type: "assistant",
				text: "",
			});
			break;
		}
		case "message_update": {
			const delta = event.delta as Record<string, unknown>;
			if (delta.kind === "text_delta" && typeof delta.text === "string") {
				store.appendToLastAssistant(delta.text);
			}
			break;
		}
		case "message_end": {
			store.removeEmptyAssistant();
			break;
		}
		case "tool_execution_start": {
			store.addMessage({
				id: `tool-${event.tool_call_id}`,
				type: "tool",
				toolName: event.tool_name,
				toolCallId: event.tool_call_id,
			});
			break;
		}
		case "tool_execution_end": {
			const result = event.result as { content?: Array<{ text?: string }> };
			const text = result.content?.map((c) => c.text).join("\n") ?? "";
			store.setToolResult(event.tool_call_id, text.slice(0, 500));
			break;
		}
		case "queue_update": {
			if (event.steer.length > 0) {
				store.addMessage({
					id: `steer-${Date.now()}`,
					type: "steer",
					text: `Steer queued: ${steer.length} message(s)`,
				});
			}
			break;
		}
		case "finished":
		case "wait_for_input": {
			store.setRunning(false);
			break;
		}
	}
}

export function useAgent() {
	const [agent, setAgent] = useState<Agent | null>(null);
	const abortControllerRef = useRef<AbortController | null>(null);
	const runtimeRef = useRef(new LiveBrowserRuntime());

	const store = useAgentStore();
	const sessionStore = useSessionStore();

	// Create agent on mount
	useEffect(() => {
		let cancelled = false;

		async function init() {
			store.setStatus("Loading WASM...");
			await sessionStore.loadSession(SESSION_ID);
			if (cancelled) return;

			const a = await createAgent(SESSION_ID, sessionStore.restoredState);
			if (cancelled) {
				a.destroy();
				return;
			}

			setAgent(a);
			store.setStatus(
				sessionStore.restoredState ? "Session restored" : "Ready",
			);
			store.setRunning(false);
		}

		init();
		return () => {
			cancelled = true;
		};
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [
		store.setStatus,
		store.setRunning,
		sessionStore.restoredState,
		sessionStore.loadSession,
	]);

	// Cleanup on unmount
	useEffect(() => {
		return () => {
			if (agent) {
				try {
					agent.destroy();
				} catch {
					/* ignore */
				}
			}
		};
	}, [agent]);

	const sendPrompt = useCallback(
		async (text: string) => {
			if (!agent || store.isRunning || !text.trim()) return;
			store.addMessage({ id: `user-${Date.now()}`, type: "user", text });
			store.setRunning(true);
			store.setError(null);

			const abortController = new AbortController();
			abortControllerRef.current = abortController;

			const tools = createToolRegistry(runtimeRef.current);
			const llmProvider = createLlmProvider(abortController.signal);

			const result = await runTurn(agent, text, {
				llm: {
					async call(context, signal) {
						const projected = runProjection(
							context.system_prompt,
							context.messages,
						);
						return llmProvider.call(
							{ ...context, messages: projected },
							signal,
						);
					},
				},
				tools,
				onEvent: (event) => eventToStoreAction(event, store),
				signal: abortController.signal,
			});

			if (result.aborted) {
				store.addMessage({
					id: `abort-${Date.now()}`,
					type: "assistant",
					text: "Stopped by user.",
				});
			} else if (result.error) {
				store.addMessage({
					id: `err-${Date.now()}`,
					type: "error",
					text: result.error,
				});
			}

			store.setRunning(false);
			abortControllerRef.current = null;

			// Persist session
			try {
				const state = getSessionState(agent);
				await sessionStore.saveSession(SESSION_ID, state);
			} catch (e) {
				console.warn("session save failed:", e);
			}
		},
		[agent, store, sessionStore],
	);

	const steerPrompt = useCallback(
		async (text: string) => {
			if (!agent || !store.isRunning || !text.trim()) return;
			try {
				const events = steerAgent(agent, text);
				for (const event of events) {
					eventToStoreAction(event, store);
				}
			} catch (e: unknown) {
				console.warn("steer failed:", e);
			}
		},
		[agent, store],
	);

	const stopPrompt = useCallback(() => {
		stopAgent(abortControllerRef.current);
	}, []);

	return {
		sendPrompt,
		steerPrompt,
		stopPrompt,
		isRunning: store.isRunning,
		status: store.status,
	};
}
