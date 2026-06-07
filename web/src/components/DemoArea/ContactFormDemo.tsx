import { useState } from "react";
import { useUIStore } from "../../stores/uiStore.ts";

export default function ContactFormDemo() {
	const [name, setName] = useState("");
	const [email, setEmail] = useState("");
	const [output, setOutput] = useState("");
	const addConsoleLine = useUIStore((state) => state.addConsoleLine);

	return (
		<div className="bg-surface border border-border rounded-2xl shadow-sm p-5 mb-5 backdrop-blur-[22px]">
			<h3 className="text-text-muted text-sm font-semibold mb-2">
				Contact Form
			</h3>
			<div className="flex flex-col gap-2">
				<label
					htmlFor="contact-name"
					className="text-xs text-text-muted font-medium"
				>
					Name
				</label>
				<input
					id="contact-name"
					type="text"
					placeholder="Enter your name"
					value={name}
					onChange={(e) => setName(e.target.value)}
					className="bg-surface-solid text-text border border-border rounded-xl px-3 py-2 text-sm outline-none focus:border-accent focus:ring-4 focus:ring-accent-soft transition-all duration-150 ease-out"
				/>
				<label
					htmlFor="contact-email"
					className="text-xs text-text-muted font-medium"
				>
					Email
				</label>
				<input
					id="contact-email"
					type="email"
					placeholder="Enter your email"
					value={email}
					onChange={(e) => setEmail(e.target.value)}
					className="bg-surface-solid text-text border border-border rounded-xl px-3 py-2 text-sm outline-none focus:border-accent focus:ring-4 focus:ring-accent-soft transition-all duration-150 ease-out"
				/>
				<button
					type="button"
					onClick={() => {
						const msg = `Submitted: ${name} (${email})`;
						setOutput((prev) => (prev ? `${prev}\n${msg}` : msg));
						addConsoleLine(`Form submitted: ${name} (${email})`);
					}}
					className="bg-text text-bg rounded-full px-4 py-2.5 text-sm font-medium self-start mt-1 hover:text-text-muted transition-colors duration-150 ease-out"
				>
					Submit
				</button>
				{output && (
					<pre className="bg-surface-solid border border-border rounded-xl p-2 font-mono text-xs whitespace-pre-wrap text-text-muted max-h-[120px] overflow-y-auto mt-2">
						{output}
					</pre>
				)}
			</div>
		</div>
	);
}
