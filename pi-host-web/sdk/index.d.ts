/**
 * High-level JS SDK for @pi-oxide/pi-host-web.
 *
 * Re-exports all raw types so consumers never need to import from ./raw.
 */

export * from "../pi_host_web";

export declare function ensureInit(): Promise<void>;

export declare function continueTurn(handle: number): StepResult;

export declare function createHostState(
	budget: ContextProjectionBudget,
): CreateHostStateResult;
export declare function destroyHostState(handle: number): EmptyResult;
export declare function createHostAgent(
	options: AgentOptions,
	budget: ContextProjectionBudget,
): CreateHostAgentResult;
export declare function destroyHostAgent(handle: number): EmptyResult;
export declare function startTurn(
	handle: number,
	input: StartTurnInput,
): TurnResultResult;
export declare function hostFeedLlmChunk(
	handle: number,
	chunk: LlmChunk,
): TurnResultResult;
export declare function hostLlmDone(
	handle: number,
	result: LlmResult,
): TurnResultResult;
export declare function hostToolDone(
	handle: number,
	id: ToolCallId,
	result: ToolResult,
): TurnResultResult;
export declare function hostAcceptCompaction(
	handle: number,
	summary: string,
	compactedEntryIds: string[],
): TurnResultResult;
export declare function hostContinueTurn(handle: number): TurnResultResult;
export declare function getHostStatePersistData(
	handle: number,
): HostStatePersistDataResult;
export declare function restoreHostState(
	data: PersistData,
): CreateHostStateResult;
export declare function restoreHostStateFromJson(
	json: string,
): CreateHostStateResult;
export declare function hostReadArtifact(
	handle: number,
	artifactId: string,
): string;
export declare function hostSearchArtifacts(
	handle: number,
	query: string,
): ArtifactSearchResults;
export declare function hostToolCancelled(
	handle: number,
	toolCallId: string,
	reason: CancelReason,
): TurnResultResult;
export declare function hostAbort(handle: number): TurnResultResult;
export declare function getHostAgentPersistData(
	handle: number,
): HostStatePersistDataResult;
export declare function restoreHostAgent(
	options: AgentOptions,
	data: PersistData,
): CreateHostAgentResult;

export declare function toolResult(
	text: string,
	opts?: { terminate?: boolean; details?: object },
): {
	content: Array<{ type: "text"; text: string }>;
	terminate?: boolean;
	details?: object;
};

export declare function toolError(
	code: string,
	message: string,
): { error: { code: string; message: string } };

export interface LlmStream {
	chunks: AsyncIterable<LlmChunk>;
	result: Promise<LlmResult>;
}

export interface LlmProvider {
	call(
		context: LlmContext,
		signal?: AbortSignal,
	): Promise<LlmStream> | LlmStream;
	summarize?(messages: AgentMessage[], signal?: AbortSignal): Promise<string>;
}

export type ToolMap = Record<
	string,
	(call: ToolCall) => Promise<ToolResult> | ToolResult
>;

export interface AgentRunConfig {
	llm: LlmProvider;
	tools: ToolMap;
	llmTools?: ToolDefinition[];
	onEvent?: (event: AgentEvent) => void;
	signal?: AbortSignal;
	onPersist?: (data: PersistData) => Promise<void>;
}

export declare class Agent {
	static create(options: AgentOptions): Promise<Agent>;
	run(promptText: string, config: AgentRunConfig): Promise<AgentAction>;
	stop(): void;
	reset(): void;
	state(): AgentState;
	getSessionState(): SessionState;
	setSessionState(sessionState: SessionState): void;
	steer(message: AgentMessage): AgentEvent[];
	followUp(message: AgentMessage): void;
	destroy(): void;
}
