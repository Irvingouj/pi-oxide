import { projectContext } from "@pi-oxide/pi-host-web";

export interface ProjectionBudget {
	max_tool_result_chars: number;
	max_context_tokens: number;
	default_preview_chars: number;
}

export interface ProjectionState {
	replacements: Record<string, unknown>;
}

export interface ProjectionResult {
	projected_messages: unknown[];
	updated_state: ProjectionState;
}

let state: ProjectionState = { replacements: {} };

const budget: ProjectionBudget = {
	max_tool_result_chars: 50000,
	max_context_tokens: 100000,
	default_preview_chars: 2000,
};

export function runProjection(
	systemPrompt: string,
	messages: unknown[],
): unknown[] {
	try {
		const result = projectContext({
			system_prompt: systemPrompt,
			messages,
			budget,
			state,
		});
		if (!result.ok) {
			console.warn("projection error:", result.error);
			return messages;
		}
		state = result.data.updated_state;
		return result.data.projected_messages;
	} catch (e) {
		console.warn("projection error:", e);
		return messages;
	}
}
