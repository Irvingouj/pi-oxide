import { useState } from "react";

export default function TodoListDemo() {
	const [todos, setTodos] = useState([
		"Build browser agent",
		"Test with real LLM",
		"Ship it",
	]);
	const [input, setInput] = useState("");

	return (
		<div
			style={{
				background: "#16213e",
				border: "1px solid #0f3460",
				borderRadius: "8px",
				padding: "16px",
				marginBottom: "16px",
			}}
		>
			<h3 style={{ color: "#533483", marginBottom: "8px", fontSize: "14px" }}>
				Todo List
			</h3>
			<ul style={{ marginBottom: "8px", paddingLeft: "20px" }}>
				{todos.map((t) => (
					<li key={t} style={{ color: "#e0e0e0", fontSize: "14px" }}>
						{t}
					</li>
				))}
			</ul>
			<div style={{ display: "flex", gap: "8px" }}>
				<input
					type="text"
					placeholder="Add a todo..."
					value={input}
					onChange={(e) => setInput(e.target.value)}
					onKeyDown={(e) => {
						if (e.key === "Enter") {
							const text = input.trim();
							if (!text) return;
							setTodos((prev) => [...prev, text]);
							setInput("");
							console.log("Todo added:", text);
						}
					}}
					style={{
						flex: 1,
						padding: "6px 10px",
						background: "#0f3460",
						border: "1px solid #533483",
						color: "#e0e0e0",
						borderRadius: "4px",
						fontSize: "14px",
					}}
				/>
				<button
					type="button"
					onClick={() => {
						const text = input.trim();
						if (!text) return;
						setTodos((prev) => [...prev, text]);
						setInput("");
						console.log("Todo added:", text);
					}}
					style={{
						background: "#533483",
						color: "white",
						border: "none",
						padding: "8px 16px",
						borderRadius: "4px",
						cursor: "pointer",
					}}
				>
					Add
				</button>
			</div>
		</div>
	);
}
