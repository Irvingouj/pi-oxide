import type { ChatMessage } from "../../stores/agentStore.ts";

interface MessageItemProps {
	message: ChatMessage;
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
	tool: {
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

export default function MessageItem({ message }: MessageItemProps) {
	switch (message.type) {
		case "tool":
			return (
				<div className="msg-tool" style={styles.tool}>
					<span style={{ color: "#e94560", fontWeight: "bold" }}>
						{message.toolName}
					</span>{" "}
					<span style={{ color: "#888" }}>{message.toolCallId}</span>
					{message.toolResult && (
						<div
							style={{
								color: "#53c285",
								whiteSpace: "pre-wrap",
								maxHeight: "200px",
								overflowY: "auto",
								marginTop: "4px",
							}}
						>
							{message.toolResult}
						</div>
					)}
				</div>
			);
		default:
			return (
				<div
					className={`msg-${message.type}`}
					style={styles[message.type] || baseStyle}
				>
					{message.text}
				</div>
			);
	}
}
