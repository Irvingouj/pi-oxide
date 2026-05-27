import { useEffect, useState } from "react";

export default function ConsoleOutput() {
	const [lines, setLines] = useState<string[]>(["Ready."]);

	useEffect(() => {
		const handler = (e: Event) => {
			const detail = (e as CustomEvent).detail;
			if (typeof detail === "string") {
				setLines((prev) => [...prev, detail]);
			}
		};
		window.addEventListener("demo-console", handler);
		return () => window.removeEventListener("demo-console", handler);
	}, []);

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
				Console Output
			</h3>
			<div
				style={{
					background: "#0a0a1a",
					border: "1px solid #333",
					borderRadius: "4px",
					padding: "8px",
					fontFamily: "monospace",
					fontSize: "12px",
					maxHeight: "120px",
					overflowY: "auto",
					whiteSpace: "pre-wrap",
					color: "#aaa",
				}}
			>
				{lines.join("\n")}
			</div>
		</div>
	);
}
