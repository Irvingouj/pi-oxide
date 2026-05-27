import { useConfig } from "../hooks/useConfig.ts";

interface HeaderProps {
	status: string;
}

export default function Header({ status }: HeaderProps) {
	const config = useConfig();

	return (
		<div
			style={{
				background: "#16213e",
				padding: "8px 16px",
				display: "flex",
				alignItems: "center",
				gap: "12px",
				borderBottom: "1px solid #0f3460",
				flexShrink: 0,
			}}
		>
			<h1 style={{ fontSize: "16px", color: "#e94560", margin: 0 }}>
				pi-oxide
			</h1>
			<span style={{ fontSize: "12px", color: "#888" }}>{status}</span>
			<input
				id="api-key-input"
				type="password"
				placeholder="API Key (e.g. fpk_...)"
				value={config.apiKey}
				onChange={(e) => config.setApiKey(e.target.value)}
				style={{
					background: "#0f3460",
					border: "1px solid #533483",
					color: "#e0e0e0",
					padding: "4px 8px",
					borderRadius: "4px",
					fontSize: "13px",
					width: "280px",
				}}
			/>
			<input
				id="base-url-input"
				type="text"
				placeholder="Base URL (e.g. https://api.fireworks.ai/inference)"
				value={config.baseUrl}
				onChange={(e) => config.setBaseUrl(e.target.value)}
				style={{
					background: "#0f3460",
					border: "1px solid #533483",
					color: "#e0e0e0",
					padding: "4px 8px",
					borderRadius: "4px",
					fontSize: "12px",
					width: "200px",
				}}
			/>
			<input
				id="model-input"
				type="text"
				placeholder="Model (e.g. claude-sonnet-4-20250514)"
				value={config.model}
				onChange={(e) => config.setModel(e.target.value)}
				style={{
					background: "#0f3460",
					border: "1px solid #533483",
					color: "#e0e0e0",
					padding: "4px 8px",
					borderRadius: "4px",
					fontSize: "12px",
					width: "180px",
				}}
			/>
		</div>
	);
}
