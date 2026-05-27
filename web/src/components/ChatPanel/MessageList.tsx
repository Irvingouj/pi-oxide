import { useEffect, useRef } from "react";
import { useAgentStore } from "../../stores/agentStore.ts";
import MessageItem from "./MessageItem.tsx";

export default function MessageList() {
	const messages = useAgentStore((s) => s.messages);
	const ref = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (ref.current && messages.length > 0) {
			ref.current.scrollTop = ref.current.scrollHeight;
		}
	}, [messages.length]);

	return (
		<div
			ref={ref}
			style={{
				flex: 1,
				overflowY: "auto",
				padding: "16px",
			}}
		>
			{messages.map((msg) => (
				<MessageItem key={msg.id} message={msg} />
			))}
		</div>
	);
}
