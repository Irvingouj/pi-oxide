import { useState } from "react";

export default function CounterDemo() {
	const [counter, setCounter] = useState(0);

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
				Counter
			</h3>
			<div
				style={{
					fontSize: "48px",
					fontWeight: "bold",
					textAlign: "center",
					color: "#e94560",
					margin: "12px 0",
				}}
			>
				{counter}
			</div>
			<button
				type="button"
				onClick={() => {
					setCounter((c) => c + 1);
					console.log("Counter:", counter + 1);
				}}
				style={{
					background: "#533483",
					color: "white",
					border: "none",
					padding: "8px 24px",
					borderRadius: "4px",
					cursor: "pointer",
					fontSize: "16px",
					display: "block",
					margin: "0 auto",
				}}
			>
				Click me
			</button>
		</div>
	);
}
