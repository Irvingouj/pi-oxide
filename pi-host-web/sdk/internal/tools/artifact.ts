// artifactTools() pack — wraps createArtifactToolRegistry from service.ts.
// Hides host handles; the actual handle is wired at build time by ToolRegistryBuilder.

import type { AgentToolDefinition, AgentTools } from "../../types.ts";
import { ARTIFACT_TOOLS } from "./service.ts";

export function artifactTools(): AgentTools {
	const definitions: AgentToolDefinition[] = ARTIFACT_TOOLS.map((t) => ({
		name: t.name,
		description: t.description,
		inputSchema: t.parameters,
		run: () => {
			throw new Error(
				"artifactTools handlers are wired at build time by ToolRegistryBuilder",
			);
		},
	}));

	return {
		definitions,
		getHandler() {
			// Handlers are provided by createArtifactToolRegistry at build time.
			return null;
		},
	};
}
