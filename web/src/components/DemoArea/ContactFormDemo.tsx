import { useState } from "react";

export default function ContactFormDemo() {
	const [name, setName] = useState("");
	const [email, setEmail] = useState("");
	const [output, setOutput] = useState("");

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
				Contact Form
			</h3>
			<div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
				<label
					htmlFor="contact-name"
					style={{ fontSize: "13px", color: "#888" }}
				>
					Name
				</label>
				<input
					id="contact-name"
					type="text"
					placeholder="Enter your name"
					value={name}
					onChange={(e) => setName(e.target.value)}
					style={{
						padding: "6px 10px",
						background: "#0f3460",
						border: "1px solid #533483",
						color: "#e0e0e0",
						borderRadius: "4px",
						fontSize: "14px",
					}}
				/>
				<label
					htmlFor="contact-email"
					style={{ fontSize: "13px", color: "#888" }}
				>
					Email
				</label>
				<input
					id="contact-email"
					type="email"
					placeholder="Enter your email"
					value={email}
					onChange={(e) => setEmail(e.target.value)}
					style={{
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
						const msg = `Submitted: ${name} (${email})`;
						setOutput((prev) => (prev ? `${prev}\n${msg}` : msg));
						console.log("Form submitted:", { name, email });
					}}
					style={{
						background: "#533483",
						color: "white",
						border: "none",
						padding: "8px 16px",
						borderRadius: "4px",
						cursor: "pointer",
						marginTop: "4px",
						alignSelf: "flex-start",
					}}
				>
					Submit
				</button>
				{output && (
					<pre
						style={{
							background: "#0a0a1a",
							border: "1px solid #333",
							borderRadius: "4px",
							padding: "8px",
							fontFamily: "monospace",
							fontSize: "12px",
							whiteSpace: "pre-wrap",
							color: "#aaa",
							maxHeight: "120px",
							overflowY: "auto",
							marginTop: "8px",
						}}
					>
						{output}
					</pre>
				)}
			</div>
		</div>
	);
}
