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
		<div className="px-4 py-3 bg-surface border-t border-border flex gap-2 flex-shrink-0 backdrop-blur-[22px]">
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
				className="flex-1 bg-surface-solid text-text border border-border rounded-xl px-3 py-2.5 text-sm resize-none outline-none focus:border-accent focus:ring-4 focus:ring-accent-soft transition-all duration-150 ease-out"
			/>
			{isRunning && (
				<>
					<button
						type="button"
						onClick={handleSteer}
						className="bg-text text-bg rounded-full px-4 py-2.5 text-sm font-medium hover:text-text-muted transition-colors duration-150 ease-out"
					>
						Steer
					</button>
					<button
						type="button"
						onClick={onStop}
						className="bg-danger text-white rounded-full px-4 py-2.5 text-sm font-medium hover:opacity-90 transition-opacity duration-150 ease-out"
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
				className="bg-text text-bg rounded-full px-5 py-2.5 text-sm font-medium hover:text-text-muted transition-colors duration-150 ease-out disabled:bg-surface-muted disabled:text-text-faint disabled:cursor-not-allowed"
			>
				Send
			</button>
			<button
				type="button"
				onClick={onReset}
				className="bg-surface-muted text-text border border-border rounded-full px-4 py-2.5 text-sm font-medium hover:bg-surface-solid transition-colors duration-150 ease-out"
			>
				Reset
			</button>
		</div>
	);
}
