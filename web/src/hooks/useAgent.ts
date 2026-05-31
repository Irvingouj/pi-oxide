/**
 * useAgent hook — bridges the WASM HostAgent lifecycle to React/Zustand.
 *
 * Uses the new HostDirective protocol. No local projection state or artifacts.
 */

import type {
	AgentEvent,
	AgentMessage,
	PersistData,
} from "@pi-oxide/pi-host-web";
import { useCallback, useEffect, useRef, useState } from "react";
import { LiveBrowserRuntime } from "../browser/liveRuntime.ts";
import {
	createHostAgentInstance,
	type HostAgent,
	runTurnWithHostAgent,
	stopAgent,
} from "../services/agentService.ts";
import { createLlmProvider } from "../services/llmService.ts";
import {
	ARTIFACT_TOOLS,
	BROWSER_TOOLS,
	createArtifactToolRegistry,
	createToolRegistry,
} from "../services/toolService.ts";
import { useAgentStore } from "../stores/agentStore.ts";
import { useConfigStore } from "../stores/configStore.ts";
import { useSessionStore } from "../stores/sessionStore.ts";

const SESSION_ID = "browser-default-session";
const MAX_TOOL_RESULT_DISPLAY_CHARS = 500;

function isPersistData(v: unknown): v is PersistData {
	return (
		typeof v === "object" &&
		v !== null &&
		"T" in (v as Record<string, unknown>) &&
		"A" in (v as Record<string, unknown>) &&
		typeof (v as Record<string, unknown>).budget === "object"
	);
}

function eventToStoreAction(
	event: AgentEvent,
	store: ReturnType<typeof useAgentStore.getState>,
) {
	switch (event.type) {
		case "message_start": {
			if (event.message?.role === "assistant") {
				store.addMessage({
					id: `msg-${Date.now()}`,
					type: "assistant",
					text: "",
				});
			}
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
			const text =
				result.content
					?.map((c) => c.text)
					.filter((t): t is string => t !== undefined)
					.join("\n") ?? "";
			store.setToolResult(
				event.tool_call_id,
				text.slice(0, MAX_TOOL_RESULT_DISPLAY_CHARS),
			);
			break;
		}
		case "queue_update": {
			if (event.steer.length > 0) {
				store.addMessage({
					id: `steer-${Date.now()}`,
					type: "steer",
					text: `Steer queued: ${event.steer.length} message(s)`,
				});
			}
			break;
		}
		case "agent_end": {
			store.setRunning(false);
			break;
		}
		case "tool_execution_update":
		case "tool_execution_cancelled":
		case "turn_start":
		case "turn_end":
		case "agent_start":
		case "save_point":
		case "settled": {
			break;
		}
	}
}

export function useAgent() {
	const [hostAgent, setHostAgent] = useState<HostAgent | null>(null);
	const abortControllerRef = useRef<AbortController | null>(null);
	const runtimeRef = useRef(new LiveBrowserRuntime());

	const store = useAgentStore();
	const configStore = useConfigStore();
	const sessionStore = useSessionStore();

	useEffect(() => {
		let cancelled = false;

		async function init() {
			store.setStatus("Loading WASM...");
			await sessionStore.loadSession(SESSION_ID);
			if (cancelled) return;

			const restored = sessionStore.restoredState;
			const agent = await createHostAgentInstance(
				SESSION_ID,
				restored && isPersistData(restored) ? restored : undefined,
			);
			if (cancelled) {
				agent.destroy();
				return;
			}

			setHostAgent(agent);
			store.setStatus(restored ? "Session restored" : "Ready");
			store.setRunning(false);
		}

		init();
		return () => {
			cancelled = true;
		};
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [store.setStatus, store.setRunning, sessionStore.loadSession]);

	useEffect(() => {
		return () => {
			if (hostAgent) {
				try {
					hostAgent.destroy();
				} catch {
					/* ignore */
				}
			}
		};
	}, [hostAgent]);

	const sendPrompt = useCallback(
		async (text: string) => {
			if (!hostAgent || store.isRunning || !text.trim()) return;
			store.addMessage({ id: `user-${Date.now()}`, type: "user", text });
			store.setRunning(true);
			store.setError(null);

			const abortController = new AbortController();
			abortControllerRef.current = abortController;

			const tools = {
				...createToolRegistry(runtimeRef.current),
				...createArtifactToolRegistry(
					() => hostAgent.handle,
					undefined,
					() => hostAgent.getSessionId(),
				),
			};
			const llmProvider = createLlmProvider(abortController.signal);

			try {
				const result = await runTurnWithHostAgent(hostAgent, text, {
					llm: llmProvider,
					tools,
					llmTools: [...BROWSER_TOOLS, ...ARTIFACT_TOOLS],
					onEvent: (event) => eventToStoreAction(event, store),
					onPersist: async (data) => {
						await sessionStore.saveSession(SESSION_ID, data);
					},
					signal: abortController.signal,
				});

				if (result.aborted) {
					store.addMessage({
						id: `abort-${Date.now()}`,
						type: "assistant",
						text: "Stopped by user.",
					});
				}
			} catch (e) {
				store.addMessage({
					id: `err-${Date.now()}`,
					type: "error",
					text: e instanceof Error ? e.message : String(e),
				});
			} finally {
				store.setRunning(false);
				abortControllerRef.current = null;
			}
		},
		[hostAgent, store, sessionStore],
	);

	const steerPrompt = useCallback(
		async (text: string) => {
			if (!hostAgent || !text.trim()) return;
			const message: AgentMessage = {
				role: "user",
				content: [{ type: "text", text }],
				timestamp: Date.now(),
			};
			try {
				const step = hostAgent.steer(message);
				for (const event of step.events) {
					eventToStoreAction(event, store);
				}
			} catch (e) {
				store.addMessage({
					id: `err-${Date.now()}`,
					type: "error",
					text: e instanceof Error ? e.message : String(e),
				});
			}
		},
		[hostAgent, store],
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
