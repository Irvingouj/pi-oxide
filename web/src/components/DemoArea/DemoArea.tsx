import ConsoleOutput from "./ConsoleOutput.tsx";
import ContactFormDemo from "./ContactFormDemo.tsx";
import CounterDemo from "./CounterDemo.tsx";
import TodoListDemo from "./TodoListDemo.tsx";

export default function DemoArea() {
	return (
		<div
			style={{
				width: "50%",
				padding: "24px",
				overflowY: "auto",
				borderRight: "1px solid #0f3460",
			}}
		>
			<h2 style={{ color: "#e94560", marginBottom: "12px", fontSize: "18px" }}>
				Demo Page
			</h2>
			<CounterDemo />
			<ContactFormDemo />
			<TodoListDemo />
			<ConsoleOutput />
		</div>
	);
}
