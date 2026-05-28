import InputBar from "./InputBar.tsx";
import MessageList from "./MessageList.tsx";

interface ChatPanelProps {
	onSend: (text: string) => void;
	onStop: () => void;
	onSteer: (text: string) => void;
	isRunning: boolean;
}

export default function ChatPanel({
	onSend,
	onStop,
	onSteer,
	isRunning,
}: ChatPanelProps) {
	return (
		<div
			data-running={isRunning}
			style={{
				width: "50%",
				display: "flex",
				flexDirection: "column",
			}}
		>
			<MessageList />
			<InputBar
				onSend={onSend}
				onStop={onStop}
				onSteer={onSteer}
				isRunning={isRunning}
			/>
		</div>
	);
}
