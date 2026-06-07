import { useEffect, useMemo } from "react";
import { useAgent } from "../react/useAgent.ts";
import {
	defineModel,
	memoryStore,
	openaiCompatible,
} from "@pi-oxide/pi-host-web";
import type { AgentConfig } from "@pi-oxide/pi-host-web";
import { useConfigStore } from "../stores/configStore.ts";
import ChatPanel from "./ChatPanel/ChatPanel.tsx";
import DemoArea from "./DemoArea/DemoArea.tsx";
import ErrorBoundary from "./ErrorBoundary.tsx";
import ErrorFallback from "./ErrorFallback.tsx";
import Header from "./Header.tsx";

export default function App() {
	const { apiKey, baseUrl, model } = useConfigStore();

	const config = useMemo<AgentConfig>(() => {
		const hasCredentials = apiKey.trim() && baseUrl.trim() && model.trim();
		return {
			sessionId: "demo",
			model: hasCredentials
				? openaiCompatible({
						apiKey: apiKey.trim(),
						baseUrl: baseUrl.trim(),
						model: model.trim(),
					})
				: defineModel({
						id: "dummy",
						generate: async () => ({
							content: [
								{
									type: "text",
									text: "Please configure an API key, base URL, and model in the header above to use the agent.",
								},
							],
							stopReason: "end" as const,
						}),
					}),
			store: memoryStore(),
		};
	}, [apiKey, baseUrl, model]);

	const { send, steer, stop, reset, status, messages, toolCalls, error } =
		useAgent(config);

	const sendPrompt = async (text: string) => {
		await send(text);
	};
	const steerPrompt = async (text: string) => {
		await steer(text);
	};
	const stopPrompt = () => stop();
	const resetAgent = () => reset();
	const isRunning =
		status.state !== "idle" &&
		status.state !== "completed" &&
		status.state !== "failed" &&
		status.state !== "aborted";

	useEffect(() => {
		if (import.meta.env.DEV) {
			window.__sendPrompt = sendPrompt;
			window.__stopPrompt = stopPrompt;
			window.__steerPrompt = steerPrompt;
			window.__resetAgent = resetAgent;
		}
	}, [sendPrompt, stopPrompt, steerPrompt, resetAgent]);

	return (
		<div className="flex flex-col h-screen bg-bg text-text font-sans">
			<Header status={status.state} />
			<div className="flex flex-1 overflow-hidden">
				<ErrorBoundary fallback={<ErrorFallback title="Demo Area crashed" />}>
					<DemoArea />
				</ErrorBoundary>
				<ErrorBoundary fallback={<ErrorFallback title="Chat Panel crashed" />}>
					<ChatPanel
						onSend={sendPrompt}
						onStop={stopPrompt}
						onSteer={steerPrompt}
						onReset={resetAgent}
						isRunning={isRunning}
						messages={messages}
						toolCalls={toolCalls}
						status={status}
						error={error}
					/>
				</ErrorBoundary>
			</div>
		</div>
	);
}
