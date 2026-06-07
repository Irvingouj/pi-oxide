import { useUIStore } from "../../stores/uiStore.ts";

export default function ConsoleOutput() {
	const lines = useUIStore((state) => state.consoleLines);

	return (
		<div className="bg-surface border border-border rounded-2xl shadow-sm p-5 mb-5 backdrop-blur-[22px]">
			<h3 className="text-text-muted text-sm font-semibold mb-2">
				Console Output
			</h3>
			<div className="bg-surface-solid border border-border rounded-xl p-2 font-mono text-xs max-h-[120px] overflow-y-auto whitespace-pre-wrap text-text-muted">
				{lines.join("\n")}
			</div>
		</div>
	);
}
