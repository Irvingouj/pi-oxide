import { useConfigStore } from "../stores/configStore.ts";

interface HeaderProps {
	status: string;
}

export default function Header({ status }: HeaderProps) {
	const config = useConfigStore();

	return (
		<div className="bg-surface border-b border-border backdrop-blur-[22px] px-4 py-2 flex items-center gap-3 flex-shrink-0">
			<h1 className="text-lg text-text font-semibold tracking-tight">pi-oxide</h1>
			<span className="text-xs text-text-muted font-medium">{status}</span>
			<input
				id="api-key-input"
				type="password"
				placeholder="API Key (e.g. fpk_...)"
				value={config.apiKey}
				onChange={(e) => config.setApiKey(e.target.value)}
				className="bg-surface-solid text-text border border-border rounded-xl px-3 py-2 text-sm outline-none focus:border-accent focus:ring-4 focus:ring-accent-soft transition-all duration-150 ease-out w-[280px]"
			/>
			<input
				id="base-url-input"
				type="text"
				placeholder="Base URL (e.g. https://api.fireworks.ai/inference)"
				value={config.baseUrl}
				onChange={(e) => config.setBaseUrl(e.target.value)}
				className="bg-surface-solid text-text border border-border rounded-xl px-3 py-2 text-sm outline-none focus:border-accent focus:ring-4 focus:ring-accent-soft transition-all duration-150 ease-out w-[200px]"
			/>
			<input
				id="model-input"
				type="text"
				placeholder="Model (e.g. claude-sonnet-4-20250514)"
				value={config.model}
				onChange={(e) => config.setModel(e.target.value)}
				className="bg-surface-solid text-text border border-border rounded-xl px-3 py-2 text-sm outline-none focus:border-accent focus:ring-4 focus:ring-accent-soft transition-all duration-150 ease-out w-[180px]"
			/>
		</div>
	);
}
