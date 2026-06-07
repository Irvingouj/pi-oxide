import { useEffect, useRef } from "react";
import MessageItem from "./MessageItem.tsx";
import type {
	AgentMessage,
	AgentToolRun,
	AgentStatus,
	AgentError,
} from "@pi-oxide/pi-host-web";

interface MessageListProps {
	messages: AgentMessage[];
	toolCalls: AgentToolRun[];
	status: AgentStatus;
	error: AgentError | null;
}

export default function MessageList({
	messages,
	toolCalls,
	status,
	error,
}: MessageListProps) {
	const ref = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (ref.current) {
			ref.current.scrollTop = ref.current.scrollHeight;
		}
	}, [messages.length, toolCalls.length]);

	const isActive =
		status.state !== "idle" &&
		status.state !== "completed" &&
		status.state !== "failed" &&
		status.state !== "aborted";

	return (
		<div ref={ref} className="flex-1 overflow-y-auto p-4 bg-bg">
			{isActive && (
				<div className="px-3 py-1.5 mb-2 bg-surface border border-border rounded-xl text-xs text-text-muted text-center font-medium">
					{status.state}
					{status.message ? ` — ${status.message}` : ""}
				</div>
			)}

			{error && (
				<div className="px-3 py-2 mb-3 bg-danger/10 border border-danger rounded-xl text-danger text-sm">
					<strong>Error:</strong> {error.message} ({error.code})
				</div>
			)}

			{messages.map((msg) => (
				<MessageItem key={msg.id} message={msg} />
			))}

			{toolCalls.map((tool) => (
				<div
					key={tool.id}
					className="mb-3 px-3 py-2 rounded-2xl bg-surface border border-border text-xs font-mono shadow-sm backdrop-blur-[22px]"
				>
					<span className="text-danger font-bold">{tool.name}</span>{" "}
					<span className="text-text-muted">({tool.status})</span>
					{tool.error && (
						<div className="text-danger mt-1">{tool.error.message}</div>
					)}
					{tool.output !== undefined && (
						<div className="text-text whitespace-pre-wrap max-h-[200px] overflow-y-auto mt-1">
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
