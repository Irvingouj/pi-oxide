import ConsoleOutput from "./ConsoleOutput.tsx";
import ContactFormDemo from "./ContactFormDemo.tsx";
import CounterDemo from "./CounterDemo.tsx";
import TodoListDemo from "./TodoListDemo.tsx";

export default function DemoArea() {
	return (
		<div className="w-1/2 p-6 overflow-y-auto border-r border-border bg-bg">
			<h2 className="text-xl text-text font-semibold tracking-tight mb-4">Demo Page</h2>
			<CounterDemo />
			<ContactFormDemo />
			<TodoListDemo />
			<ConsoleOutput />
		</div>
	);
}
