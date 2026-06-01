import type { AgentMessage, AgentContentBlock } from "@pi-oxide/pi-host-web";

interface MessageItemProps {
	message: AgentMessage;
}

const baseStyle: React.CSSProperties = {
	marginBottom: "12px",
	padding: "8px 12px",
	borderRadius: "8px",
	fontSize: "14px",
	lineHeight: 1.5,
	maxWidth: "90%",
};

const styles: Record<string, React.CSSProperties> = {
	user: {
		...baseStyle,
		background: "#0f3460",
		marginLeft: "auto",
		borderBottomRightRadius: "2px",
	},
	assistant: {
		...baseStyle,
		background: "#16213e",
		border: "1px solid #0f3460",
		borderBottomLeftRadius: "2px",
	},
	tool_result: {
		...baseStyle,
		background: "#1a0a2e",
		border: "1px solid #533483",
		fontSize: "12px",
		fontFamily: "monospace",
		borderBottomLeftRadius: "2px",
	},
	error: {
		...baseStyle,
		background: "#2e0a0a",
		border: "1px solid #e94560",
		color: "#e94560",
	},
	steer: {
		...baseStyle,
		background: "#1a0a2e",
		border: "1px dashed #533483",
		color: "#888",
		fontSize: "12px",
		fontStyle: "italic",
		marginLeft: "auto",
		borderBottomRightRadius: "2px",
	},
};

function getMessageType(message: AgentMessage): string {
	if (message.role === "tool_result") return "tool_result";
	return message.role;
}

function renderContentBlock(block: AgentContentBlock, index: number): React.ReactNode {
	switch (block.type) {
		case "text":
			return <span key={index}>{block.text}</span>;
		case "image":
			return (
				<img
					key={index}
					src={`data:${block.mimeType};base64,${block.data}`}
					alt="agent-generated"
					style={{ maxWidth: "100%", borderRadius: "4px", marginTop: "4px" }}
				/>
			);
		case "tool_call":
			return (
				<div
					key={index}
					style={{
						marginTop: "4px",
						padding: "4px 8px",
						background: "#1a0a2e",
						border: "1px solid #533483",
						borderRadius: "4px",
						fontSize: "12px",
						fontFamily: "monospace",
					}}
				>
					<span style={{ color: "#e94560", fontWeight: "bold" }}>
						{block.name}
					</span>{" "}
					<span style={{ color: "#888" }}>({block.id})</span>
					<div style={{ color: "#888", marginTop: "2px" }}>
						{JSON.stringify(block.arguments, null, 2)}
					</div>
				</div>
			);
		case "file":
			return (
				<div
					key={index}
					style={{
						marginTop: "4px",
						padding: "4px 8px",
						background: "#0f3460",
						borderRadius: "4px",
						fontSize: "12px",
					}}
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
			<div className="msg-tool" style={styles.tool_result}>
				<span style={{ color: "#e94560", fontWeight: "bold" }}>
					{toolName}
				</span>{" "}
				<span style={{ color: "#888" }}>{message.tool_call_id}</span>
				{text && (
					<div
						style={{
							color: "#53c285",
							whiteSpace: "pre-wrap",
							maxHeight: "200px",
							overflowY: "auto",
							marginTop: "4px",
						}}
					>
						{text}
					</div>
				)}
			</div>
		);
	}

	return (
		<div className={`msg-${type}`} style={styles[type] || baseStyle}>
			{message.content.map((block, i) => renderContentBlock(block, i))}
		</div>
	);
}
