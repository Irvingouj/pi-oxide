import { useEffect, useMemo } from "react";
import { useAgent } from "@pi-oxide/pi-host-web/react";
import { defineModel, memoryStore } from "@pi-oxide/pi-host-web";
import type { AgentConfig } from "@pi-oxide/pi-host-web";
import ChatPanel from "./ChatPanel/ChatPanel.tsx";
import DemoArea from "./DemoArea/DemoArea.tsx";
import Header from "./Header.tsx";

export default function App() {
	const config = useMemo<AgentConfig>(
		() => ({
			sessionId: "demo",
			model: defineModel({
				id: "dummy",
				generate: async () => ({
					content: [{ type: "text", text: "" }],
					stopReason: "end" as const,
				}),
			}),
			store: memoryStore(),
		}),
		[],
	);

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
		window.__sendPrompt = sendPrompt;
		window.__stopPrompt = stopPrompt;
		window.__steerPrompt = steerPrompt;
		window.__resetAgent = resetAgent;
	}, [sendPrompt, stopPrompt, steerPrompt, resetAgent]);

	return (
		<div
			style={{
				display: "flex",
				flexDirection: "column",
				height: "100vh",
				background: "#1a1a2e",
				color: "#e0e0e0",
				fontFamily:
					"-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
			}}
		>
			<Header status={status.state} />
			<div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
				<DemoArea />
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
			</div>
		</div>
	);
}
