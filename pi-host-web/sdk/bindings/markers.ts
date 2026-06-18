import { hostReadArtifact } from "../../pi_host_web.js";
import type { HostAgent } from "./host-agent.ts";
import type { AgentRunConfig } from "./types.ts";

export async function processStepMarkers(
	step: { markers?: Array<{ type: string; entry_ids?: string[] }> },
	hostAgent: HostAgent,
	config: AgentRunConfig,
): Promise<void> {
	if (!step.markers) return;
	for (const marker of step.markers) {
		if (marker.type === "new_artifacts") {
			for (const artifactId of marker.entry_ids ?? []) {
				const content = hostReadArtifact(hostAgent.handle, artifactId);
				await config.artifactStore?.save(
					hostAgent.sessionId ?? "default",
					artifactId,
					content,
				);
			}
		}
	}
	config.onMarkers?.(step.markers);
}
