import InputBar from "./InputBar.tsx";
import MessageList from "./MessageList.tsx";
import type {
	AgentMessage,
	AgentToolRun,
	AgentStatus,
	AgentError,
} from "@pi-oxide/pi-host-web";

interface ChatPanelProps {
	onSend: (text: string) => void;
	onStop: () => void;
	onSteer: (text: string) => void;
	onReset: () => void;
	isRunning: boolean;
	messages: AgentMessage[];
	toolCalls: AgentToolRun[];
	status: AgentStatus;
	error: AgentError | null;
}

export default function ChatPanel({
	onSend,
	onStop,
	onSteer,
	onReset,
	isRunning,
	messages,
	toolCalls,
	status,
	error,
}: ChatPanelProps) {
	return (
		<div className="w-1/2 flex flex-col bg-bg">
			<MessageList
				messages={messages}
				toolCalls={toolCalls}
				status={status}
				error={error}
			/>
			<InputBar
				onSend={onSend}
				onStop={onStop}
				onSteer={onSteer}
				onReset={onReset}
				isRunning={isRunning}
			/>
		</div>
	);
}
