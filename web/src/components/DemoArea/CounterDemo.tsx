import { useState } from "react";
import { useUIStore } from "../../stores/uiStore.ts";

export default function CounterDemo() {
	const [counter, setCounter] = useState(0);
	const addConsoleLine = useUIStore((state) => state.addConsoleLine);

	return (
		<div className="bg-surface border border-border rounded-2xl shadow-sm p-5 mb-5 backdrop-blur-[22px]">
			<h3 className="text-text-muted text-sm font-semibold mb-2">Counter</h3>
			<div className="text-5xl font-bold text-center text-text my-3">
				{counter}
			</div>
			<button
				type="button"
				onClick={() => {
					const next = counter + 1;
					setCounter(next);
					addConsoleLine(`Counter: ${next}`);
				}}
				className="bg-text text-bg rounded-full px-6 py-2.5 text-base font-medium block mx-auto hover:text-text-muted transition-colors duration-150 ease-out"
			>
				Click me
			</button>
		</div>
	);
}
