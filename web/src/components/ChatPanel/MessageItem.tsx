import type { AgentMessage, AgentContentBlock } from "@pi-oxide/pi-host-web";

interface MessageItemProps {
	message: AgentMessage;
}

const roleClasses: Record<string, string> = {
	user: "bg-surface-muted ml-auto rounded-2xl rounded-br-md",
	assistant: "bg-surface border border-border rounded-2xl rounded-bl-md shadow-sm backdrop-blur-[22px]",
	tool_result:
		"bg-surface-muted border border-border text-xs font-mono rounded-2xl rounded-bl-md shadow-sm",
	error: "bg-danger/10 border border-danger text-danger rounded-2xl",
	steer: "bg-surface-muted border border-dashed border-border text-text-muted text-xs italic ml-auto rounded-2xl rounded-br-md shadow-sm",
};

function getMessageType(message: AgentMessage): string {
	if (message.role === "tool_result") return "tool_result";
	return message.role;
}

function renderContentBlock(
	block: AgentContentBlock,
	index: number,
): React.ReactNode {
	switch (block.type) {
		case "text":
			return <span key={index}>{block.text}</span>;
		case "image":
			return (
				<img
					key={index}
					src={`data:${block.mimeType};base64,${block.data}`}
					alt="agent-generated"
					className="max-w-full rounded-xl mt-1"
				/>
			);
		case "tool_call":
			return (
				<div
					key={index}
					className="mt-1 px-2 py-1 bg-surface-muted border border-border rounded-xl text-xs font-mono"
				>
					<span className="text-danger font-bold">{block.name}</span>{" "}
					<span className="text-text-muted">({block.id})</span>
					<div className="text-text-muted mt-0.5">
						{JSON.stringify(block.arguments, null, 2)}
					</div>
				</div>
			);
		case "file":
			return (
				<div
					key={index}
					className="mt-1 px-2 py-1 bg-surface rounded-xl text-xs"
				>
					📎 File ({block.mimeType})
				</div>
			);
		default:
			return null;
	}
}

export default function MessageItem({ message }: MessageItemProps) {
	const type = getMessageType(message);

	const baseClasses =
		"mb-3 px-3 py-2 rounded-2xl text-sm leading-relaxed max-w-[90%]";

	if (type === "tool_result") {
		const text = message.content
			.filter((c): c is { type: "text"; text: string } => c.type === "text")
			.map((c) => c.text)
			.join("");
		const toolCallBlock = message.content.find((c) => c.type === "tool_call");
		const toolName =
			toolCallBlock && toolCallBlock.type === "tool_call"
				? toolCallBlock.name
				: "tool";
		return (
			<div className={`${baseClasses} ${roleClasses.tool_result || ""}`}>
				<span className="text-danger font-bold">{toolName}</span>{" "}
				<span className="text-text-muted">{message.tool_call_id}</span>
				{text && (
					<div className="text-text whitespace-pre-wrap max-h-[200px] overflow-y-auto mt-1">
						{text}
					</div>
				)}
			</div>
		);
	}

	return (
		<div className={`${baseClasses} ${roleClasses[type] || ""}`}>
			{message.content.map((block, i) => renderContentBlock(block, i))}
		</div>
	);
}
