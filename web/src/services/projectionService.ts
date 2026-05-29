import {
	type AgentMessage,
	type ContextProjectionBudget,
	type ContextProjectionReport,
	type ContextProjectionState,
	projectContext,
	type TextContent,
} from "@pi-oxide/pi-host-web";

const budget: ContextProjectionBudget = {
	max_tool_result_chars: 50000,
	max_context_tokens: 100000,
	microcompact_after_turns: 5,
	compaction_threshold: 0.75,
};

// NOTE: Keep in sync with pi-core/src/context_projection.rs MAX_DEFERRED_ENTRIES.
const MAX_ARTIFACTS = 1000;

function buildOriginalTextMap(messages: AgentMessage[]): Map<string, string> {
	const map = new Map<string, string>();
	for (const msg of messages) {
		if (msg.role === "tool_result" && msg.tool_call_id) {
			const textBlocks = msg.content
				.filter((c): c is { type: "text" } & TextContent => c.type === "text")
				.map((c) => c.text);
			if (textBlocks.length > 0) {
				map.set(msg.tool_call_id, textBlocks.join("\n"));
			} else {
				console.warn(
					`buildOriginalTextMap: skipped tool_result with no text blocks (tool_call_id=${msg.tool_call_id})`,
				);
			}
		}
	}
	return map;
}

export interface ProjectionService {
	runProjection(
		systemPrompt: string,
		messages: AgentMessage[],
		overrideBudget?: Partial<ContextProjectionBudget>,
	): AgentMessage[];
	readArtifact(artifactId: string): string | undefined;
	searchArtifacts(
		pattern: string,
	): Array<{ id: string; snippet: string; matchCount: number }>;
	clearArtifacts(): void;
	getState(): ContextProjectionState;
	restoreState(state: ContextProjectionState): void;
	incrementTurnsSinceCompaction(): void;
	resetTurnsSinceCompaction(): void;
	getLastReport(): ContextProjectionReport | null;
	snapshotArtifacts(): Array<{ id: string; text: string }>;
	loadArtifacts(artifacts: Array<{ id: string; text: string }>): void;
}

export interface TestProjectionService extends ProjectionService {
	__seedArtifactForTest(id: string, text: string): void;
}

function createProjectionServiceInternal(
	projectCtx: typeof projectContext = projectContext,
): {
	service: ProjectionService;
	seedArtifact: (id: string, text: string) => void;
} {
	let state: ContextProjectionState = {
		tools: {},
		current_turn: 0,
		last_api_usage: null,
		turns_since_compaction: 0,
	};
	let lastReport: ContextProjectionReport | null = null;
	const artifactStore: Map<string, string> = new Map();

	// NOTE: Map insertion order is relied on for FIFO eviction.
	// Replacements are pre-sorted by `inserted_at_turn` before insertion,
	// which matches Rust's `evict_oldest_if_over_limit` semantics.
	function capArtifactStore() {
		while (artifactStore.size > MAX_ARTIFACTS) {
			const firstKey = artifactStore.keys().next().value;
			if (firstKey) {
				artifactStore.delete(firstKey);
			}
		}
	}

	const service: ProjectionService = {
		runProjection(
			systemPrompt: string,
			messages: AgentMessage[],
			overrideBudget?: Partial<ContextProjectionBudget>,
		): AgentMessage[] {
			const activeBudget = { ...budget, ...overrideBudget };
			try {
				const result = projectCtx({
					system_prompt: systemPrompt,
					messages,
					budget: activeBudget,
					state,
				});
				if (!result.ok || !result.data) {
					console.warn("projection error:", result.error);
					return messages;
				}
				const output = result.data;
				state = output.updated_state;
				lastReport = output.report;

				const report = output.report;
				if (report.replacements) {
					const originalTextMap = buildOriginalTextMap(messages);

					const sortedReplacements = [...report.replacements].sort((a, b) => {
						const aState = state.tools?.[a.tool_call_id];
						const bState = state.tools?.[b.tool_call_id];
						const aTurn =
							aState?.type === "replaced" ? (aState.inserted_at_turn ?? 0) : 0;
						const bTurn =
							bState?.type === "replaced" ? (bState.inserted_at_turn ?? 0) : 0;
						return aTurn - bTurn;
					});

					for (const r of sortedReplacements) {
						const originalText = originalTextMap.get(r.tool_call_id);
						if (originalText !== undefined) {
							artifactStore.set(r.artifact_id, originalText);
							capArtifactStore();
						}
					}
				}

				return output.projected_messages;
			} catch (e) {
				console.warn("projection error:", e);
				return messages;
			}
		},

		readArtifact(artifactId: string): string | undefined {
			return artifactStore.get(artifactId);
		},

		searchArtifacts(
			pattern: string,
		): Array<{ id: string; snippet: string; matchCount: number }> {
			const results: Array<{
				id: string;
				snippet: string;
				matchCount: number;
			}> = [];
			const SNIPPET_CONTEXT = 100;
			for (const [id, text] of artifactStore.entries()) {
				const matchIdx = text.indexOf(pattern);
				if (matchIdx !== -1) {
					const matchCount = text.split(pattern).length - 1;
					const start = Math.max(0, matchIdx - SNIPPET_CONTEXT);
					const end = Math.min(
						text.length,
						matchIdx + pattern.length + SNIPPET_CONTEXT,
					);
					let snippet = text.slice(start, end);
					if (start > 0) snippet = `...${snippet}`;
					if (end < text.length) snippet = `${snippet}...`;
					results.push({ id, snippet, matchCount });
				}
			}
			return results;
		},

		clearArtifacts(): void {
			artifactStore.clear();
		},

		getState(): ContextProjectionState {
			return state;
		},

		restoreState(newState: ContextProjectionState): void {
			state = newState;
		},

		incrementTurnsSinceCompaction(): void {
			state = {
				...state,
				turns_since_compaction: (state.turns_since_compaction ?? 0) + 1,
			};
		},

		resetTurnsSinceCompaction(): void {
			state = {
				...state,
				turns_since_compaction: 0,
			};
		},

		getLastReport(): ContextProjectionReport | null {
			return lastReport;
		},

		snapshotArtifacts(): Array<{ id: string; text: string }> {
			return Array.from(artifactStore.entries()).map(([id, text]) => ({
				id,
				text,
			}));
		},

		loadArtifacts(artifacts: Array<{ id: string; text: string }>): void {
			artifactStore.clear();
			for (const { id, text } of artifacts) {
				artifactStore.set(id, text);
			}
			capArtifactStore();
		},
	};

	return {
		service,
		seedArtifact: (id: string, text: string) => {
			artifactStore.set(id, text);
			capArtifactStore();
		},
	};
}

export function createProjectionService(): ProjectionService {
	return createProjectionServiceInternal().service;
}

export function createTestProjectionService(
	projectCtx?: typeof projectContext,
): TestProjectionService {
	const { service, seedArtifact } = createProjectionServiceInternal(projectCtx);
	return { ...service, __seedArtifactForTest: seedArtifact };
}
