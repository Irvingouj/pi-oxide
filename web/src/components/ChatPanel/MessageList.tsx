import { useEffect, useRef } from "react";
import MessageItem from "./MessageItem.tsx";
import type { AgentMessage, AgentToolRun, AgentStatus, AgentError } from "@pi-oxide/pi-host-web";

interface MessageListProps {
	messages: AgentMessage[];
	toolCalls: AgentToolRun[];
	status: AgentStatus;
	error: AgentError | null;
}

export default function MessageList({ messages, toolCalls, status, error }: MessageListProps) {
	const ref = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (ref.current) {
			ref.current.scrollTop = ref.current.scrollHeight;
		}
	}, [messages.length, toolCalls.length]);

	return (
		<div
			ref={ref}
			style={{
				flex: 1,
				overflowY: "auto",
				padding: "16px",
			}}
		>
			{/* Status indicator */}
			{status.state !== "idle" && status.state !== "completed" && status.state !== "failed" && status.state !== "aborted" && (
				<div
					style={{
						padding: "6px 12px",
						marginBottom: "8px",
						background: "#0f3460",
						borderRadius: "4px",
						fontSize: "12px",
						color: "#888",
						textAlign: "center",
					}}
				>
					{status.state}
					{status.message ? ` — ${status.message}` : ""}
				</div>
			)}

			{/* Error display */}
			{error && (
				<div
					style={{
						padding: "8px 12px",
						marginBottom: "12px",
						background: "#2e0a0a",
						border: "1px solid #e94560",
						borderRadius: "4px",
						color: "#e94560",
						fontSize: "13px",
					}}
				>
					<strong>Error:</strong> {error.message} ({error.code})
				</div>
			)}

			{/* Messages */}
			{messages.map((msg) => (
				<MessageItem key={msg.id} message={msg} />
			))}

			{/* Tool calls */}
			{toolCalls.map((tool) => (
				<div
					key={tool.id}
					style={{
						marginBottom: "12px",
						padding: "8px 12px",
						borderRadius: "8px",
						background: "#1a0a2e",
						border: "1px solid #533483",
						fontSize: "12px",
						fontFamily: "monospace",
					}}
				>
					<span style={{ color: "#e94560", fontWeight: "bold" }}>
						{tool.name}
					</span>{" "}
					<span style={{ color: "#888" }}>({tool.status})</span>
					{tool.error && (
						<div style={{ color: "#e94560", marginTop: "4px" }}>
							{tool.error.message}
						</div>
					)}
					{tool.output !== undefined && (
						<div
							style={{
								color: "#53c285",
								whiteSpace: "pre-wrap",
								maxHeight: "200px",
								overflowY: "auto",
								marginTop: "4px",
							}}
						>
							{typeof tool.output === "string"
								? tool.output
								: JSON.stringify(tool.output, null, 2)}
						</div>
					)}
				</div>
			))}
		</div>
	);
}
