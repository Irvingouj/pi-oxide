import { useState } from "react";
import { useUIStore } from "../../stores/uiStore.ts";

interface Todo {
	id: string;
	text: string;
}

export default function TodoListDemo() {
	const [todos, setTodos] = useState<Todo[]>([
		{ id: "1", text: "Build browser agent" },
		{ id: "2", text: "Test with real LLM" },
		{ id: "3", text: "Ship it" },
	]);
	const [input, setInput] = useState("");
	const addConsoleLine = useUIStore((state) => state.addConsoleLine);

	const addTodo = (text: string) => {
		const trimmed = text.trim();
		if (!trimmed) return;
		const id = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
		setTodos((prev) => [...prev, { id, text: trimmed }]);
		setInput("");
		addConsoleLine(`Todo added: ${trimmed}`);
	};

	return (
		<div className="bg-surface border border-border rounded-2xl shadow-sm p-5 mb-5 backdrop-blur-[22px]">
			<h3 className="text-text-muted text-sm font-semibold mb-2">
				Todo List
			</h3>
			<ul className="mb-2 pl-5">
				{todos.map((todo) => (
					<li key={todo.id} className="text-text text-sm">
						{todo.text}
					</li>
				))}
			</ul>
			<div className="flex gap-2">
				<input
					type="text"
					placeholder="Add a todo..."
					value={input}
					onChange={(e) => setInput(e.target.value)}
					onKeyDown={(e) => {
						if (e.key === "Enter") addTodo(input);
					}}
					className="flex-1 bg-surface-solid text-text border border-border rounded-xl px-3 py-2 text-sm outline-none focus:border-accent focus:ring-4 focus:ring-accent-soft transition-all duration-150 ease-out"
				/>
				<button
					type="button"
					onClick={() => addTodo(input)}
					className="bg-text text-bg rounded-full px-4 py-2.5 text-sm font-medium hover:text-text-muted transition-colors duration-150 ease-out"
				>
					Add
				</button>
			</div>
		</div>
	);
}
