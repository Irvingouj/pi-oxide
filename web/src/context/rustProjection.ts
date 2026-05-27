/**
 * Thin JS wrapper around the Rust/WASM context projection engine.
 *
 * This module does NOT implement any projection policy.
 * All projection logic lives in Rust (pi-core).
 * JS only calls the WASM export and manages artifact storage.
 */

import type { AgentMessageShape } from "../providers/types.ts";
import { raw } from "../rawBinding.ts";

// --- Types mirroring Rust structs ---

export interface ApiUsageSnapshot {
	estimated_tokens: number;
	actual_input_tokens: number;
}

export interface ContextProjectionBudget {
	max_tool_result_chars: number;
	max_context_tokens: number;
	default_preview_chars: number;
	microcompact_after_turns?: number;
	compaction_threshold?: number;
}

export interface ContextReplacement {
	tool_call_id: string;
	tool_name: string;
	artifact_id: string;
	original_chars: number;
	preview_chars: number;
	strategy: {
		type: string;
		max_chars?: number;
		head_chars?: number;
		tail_chars?: number;
	};
}

export interface ContextProjectionState {
	replacements: Record<string, ContextReplacement>;
	last_api_usage?: ApiUsageSnapshot | null;
	turns_since_compaction?: number;
}

export interface ContextProjectionReport {
	estimated_tokens: number;
	replacements: ContextReplacement[];
	dropped_messages: number;
	needs_compaction: boolean;
	cache_breakpoints: number[];
}

export interface ProjectionResult {
	projected_messages: AgentMessageShape[];
	updated_state: ContextProjectionState;
	report: ContextProjectionReport;
}

// --- WASM call ---

/**
 * Call the Rust context projection engine through WASM.
 *
 * Returns a ProjectionResult with projected messages, updated state,
 * and a report for observability and host artifact storage.
 */
export function callProjectContext(
	systemPrompt: string,
	messages: AgentMessageShape[],
	budget: ContextProjectionBudget,
	state: ContextProjectionState,
): ProjectionResult {
	const input = {
		system_prompt: systemPrompt,
		messages,
		budget,
		state,
	};

	let result: {
		ok: boolean;
		data?: ProjectionResult;
		error?: { code: string; message: string };
	};
	try {
		result = raw.projectContext(input) as typeof result;
	} catch (e) {
		throw new Error(
			`projectContext failed: ${e instanceof Error ? e.message : String(e)}`,
		);
	}

	if (!result.ok || !result.data) {
		throw new Error(
			`projectContext failed: ${result.error?.code ?? "unknown"}: ${result.error?.message ?? "no data"}`,
		);
	}

	return result.data;
}

// --- Artifact store interface ---

export interface ArtifactRecord {
	id: string;
	toolName: string;
	toolCallId: string;
	content: string;
	storedAt: number;
}

export interface ArtifactStore {
	put(record: ArtifactRecord): string;
	get(id: string): ArtifactRecord | undefined;
}

/**
 * In-memory artifact store for tests and single-session use.
 */
export class MemoryArtifactStore implements ArtifactStore {
	private readonly store = new Map<string, ArtifactRecord>();

	put(record: ArtifactRecord): string {
		this.store.set(record.id, record);
		return record.id;
	}

	get(id: string): ArtifactRecord | undefined {
		return this.store.get(id);
	}
}
