import { useCallback, useState } from "react";

interface InputBarProps {
	onSend: (text: string) => void;
	onStop: () => void;
	onSteer: (text: string) => void;
	onReset: () => void;
	isRunning: boolean;
}

export default function InputBar({
	onSend,
	onStop,
	onSteer,
	onReset,
	isRunning,
}: InputBarProps) {
	const [text, setText] = useState("");

	const handleSend = useCallback(() => {
		const trimmed = text.trim();
		if (!trimmed) return;
		onSend(trimmed);
		setText("");
	}, [text, onSend]);

	const handleSteer = useCallback(() => {
		const trimmed = text.trim();
		if (!trimmed) return;
		onSteer(trimmed);
		setText("");
	}, [text, onSteer]);

	return (
		<div
			style={{
				padding: "12px 16px",
				background: "#16213e",
				borderTop: "1px solid #0f3460",
				display: "flex",
				gap: "8px",
				flexShrink: 0,
			}}
		>
			<textarea
				id="user-input"
				rows={2}
				placeholder="Ask the agent to inspect or interact with this page..."
				value={text}
				onChange={(e) => setText(e.target.value)}
				onKeyDown={(e) => {
					if (e.key === "Enter" && !e.shiftKey) {
						e.preventDefault();
						if (isRunning) {
							handleSteer();
						} else {
							handleSend();
						}
					}
				}}
				style={{
					flex: 1,
					background: "#0f3460",
					border: "1px solid #533483",
					color: "#e0e0e0",
					padding: "8px 12px",
					borderRadius: "4px",
					fontSize: "14px",
					resize: "none",
				}}
			/>
			{isRunning && (
				<>
					<button
						type="button"
						onClick={handleSteer}
						style={{
							background: "#533483",
							color: "white",
							border: "none",
							padding: "8px 16px",
							borderRadius: "4px",
							cursor: "pointer",
							fontSize: "14px",
						}}
					>
						Steer
					</button>
					<button
						type="button"
						onClick={onStop}
						style={{
							background: "#e94560",
							color: "white",
							border: "none",
							padding: "8px 16px",
							borderRadius: "4px",
							cursor: "pointer",
							fontSize: "14px",
						}}
					>
						Stop
					</button>
				</>
			)}
			<button
				id="send-btn"
				type="button"
				onClick={handleSend}
				disabled={isRunning || !text.trim()}
				style={{
					background: isRunning || !text.trim() ? "#555" : "#e94560",
					color: "white",
					border: "none",
					padding: "8px 20px",
					borderRadius: "4px",
					cursor: isRunning || !text.trim() ? "not-allowed" : "pointer",
					fontSize: "14px",
				}}
			>
				Send
			</button>
			<button
				type="button"
				onClick={onReset}
				style={{
					background: "#555",
					color: "white",
					border: "none",
					padding: "8px 16px",
					borderRadius: "4px",
					cursor: "pointer",
					fontSize: "14px",
				}}
			>
				Reset
			</button>
		</div>
	);
}
