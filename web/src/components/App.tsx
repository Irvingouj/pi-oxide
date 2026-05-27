import { useEffect } from "react";
import { useAgent } from "../hooks/useAgent.ts";
import ChatPanel from "./ChatPanel/ChatPanel.tsx";
import DemoArea from "./DemoArea/DemoArea.tsx";
import Header from "./Header.tsx";

export default function App() {
	const { sendPrompt, steerPrompt, stopPrompt, isRunning, status } = useAgent();

	useEffect(() => {
		window.__sendPrompt = sendPrompt;
		window.__stopPrompt = stopPrompt;
		window.__steerPrompt = steerPrompt;
	}, [sendPrompt, stopPrompt, steerPrompt]);

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
			<Header status={status} />
			<div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
				<DemoArea />
				<ChatPanel
					onSend={sendPrompt}
					onStop={stopPrompt}
					onSteer={steerPrompt}
					isRunning={isRunning}
				/>
			</div>
		</div>
	);
}
