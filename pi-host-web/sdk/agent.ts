// Public Agent class — the primary SDK surface.
// Thin facade over the internal engine. No WASM imports here.

import { createAgentError } from "./errors.ts";
import { EventEmitter } from "./events.ts";
import type { HostAgent } from "./bindings/host-agent.ts";
import { createEngineAgent, destroyEngineAgent, resetAgentState, runAgentTurn, steerAgent } from "./orchestration/agent-engine.ts";
import { getLogger } from "./internal/logger.ts";
import type {
	AgentConfig,
	AgentError,
	AgentEventHandler,
	AgentEventName,
	AgentInput,
	AgentRunOptions,
	AgentRunResult,
	AgentStatus,
	Logger,
	Unsubscribe,
} from "./types.ts";

export class Agent {
	private config: AgentConfig;
	private emitter: EventEmitter;
	private status: AgentStatus = { state: "idle" };
	private currentRun: Promise<AgentRunResult> | null = null;
	private currentAbortController: AbortController | null = null;
	private disposed = false;
	private engineAgent: HostAgent | null = null;
	private logger: Logger;

	constructor(config: AgentConfig) {
		this.config = config;
		this.emitter = new EventEmitter();
		this.logger = config.logger ?? getLogger("agent");
	}

	on<E extends AgentEventName>(event: E, handler: AgentEventHandler<E>): Unsubscribe {
		if (this.disposed) {
			return () => {};
		}
		return this.emitter.on(event, handler);
	}

	async run(input: string | AgentInput, options?: AgentRunOptions): Promise<AgentRunResult> {
		if (this.disposed) {
			this.logger.warn("Run called on disposed agent");
			const error = createAgentError("agent_disposed", "Agent has been disposed", { recoverable: false });
			const result: AgentRunResult = {
				status: "failed",
				text: "",
				toolCalls: [],
				artifacts: [],
				error,
			};
			this.emitter.emit("error", error);
			this.emitter.emit("status", { state: "failed", message: error.message });
			this.emitter.emit("done", result);
			return result;
		}

		if (this.currentRun) {
			this.logger.warn("Run called while agent is busy");
			const error = createAgentError("agent_busy", "Agent is already running a turn", { recoverable: true });
			const result: AgentRunResult = {
				status: "failed",
				text: "",
				toolCalls: [],
				artifacts: [],
				error,
			};
			this.emitter.emit("error", error);
			this.emitter.emit("status", { state: "failed", message: error.message });
			this.emitter.emit("done", result);
			return result;
		}

		const abortController = new AbortController();
		this.currentAbortController = abortController;
		this.logger.info("Starting run", { sessionId: this.config.sessionId });

		// Merge external signal if provided
		if (options?.signal) {
			if (options.signal.aborted) {
				abortController.abort(options.signal.reason);
			} else {
				options.signal.addEventListener(
					"abort",
					() => {
						abortController.abort(options.signal?.reason);
					},
					{ once: true },
				);
			}
		}

		const runPromise = this._doRun(input, options, abortController.signal);
		this.currentRun = runPromise;

		try {
			const result = await runPromise;
			this.logger.info("Run completed", { status: result.status });
			this.emitter.emit("done", result);
			return result;
		} catch (e) {
			// Safety net: convert any unexpected throw to a failed result
			this.logger.error("Run failed", { error: e instanceof Error ? e.message : String(e) });
			const error = createAgentError("internal_error", e instanceof Error ? e.message : String(e), {
				cause: e,
				recoverable: false,
			});
			const failedResult: AgentRunResult = {
				status: "failed",
				text: "",
				toolCalls: [],
				artifacts: [],
				error,
			};
			this.emitter.emit("error", error);
			this.emitter.emit("status", { state: "failed", message: error.message });
			this.emitter.emit("done", failedResult);
			return failedResult;
		} finally {
			this.currentRun = null;
			this.currentAbortController = null;
		}
	}

	private async _doRun(
		input: string | AgentInput,
		options: AgentRunOptions | undefined,
		signal: AbortSignal,
	): Promise<AgentRunResult> {
		// Lazy initialization on first run
		if (!this.engineAgent) {
			this.engineAgent = await createEngineAgent(this.config, {
				onEvent: (event) => this.emitter.emit(event.type as AgentEventName, event.payload),
				onStatus: (status) => {
					this.status = status;
					this.emitter.emit("status", status);
				},
			});
		}

		try {
			return await runAgentTurn(this.engineAgent, this.config, input, options, signal, {
				onEvent: (event) => this.emitter.emit(event.type as AgentEventName, event.payload),
				onStatus: (status) => {
					this.status = status;
					this.emitter.emit("status", status);
				},
			});
		} catch (e) {
			const isAbort =
				signal.aborted ||
				(e instanceof Error && e.name === "AbortError") ||
				(e instanceof Error && e.message.includes("user_aborted"));

			if (isAbort) {
				const abortedResult: AgentRunResult = {
					status: "aborted",
					text: "",
					toolCalls: [],
					artifacts: [],
				};
				this.emitter.emit("status", {
					state: "aborted",
					message: "Stopped by user",
				});
				return abortedResult;
			}

			const code =
				e instanceof Error && "code" in e && typeof (e as { code: unknown }).code === "string"
					? ((e as { code: string }).code as AgentError["code"])
					: "internal_error";
			const error = createAgentError(code, e instanceof Error ? e.message : String(e), {
				cause: e,
				recoverable: false,
			});
			const failedResult: AgentRunResult = {
				status: "failed",
				text: "",
				toolCalls: [],
				artifacts: [],
				error,
			};
			this.emitter.emit("error", error);
			this.emitter.emit("status", { state: "failed", message: error.message });
			return failedResult;
		}
	}

	stop(reason?: string): void {
		if (this.disposed || !this.currentAbortController) return;
		this.logger.info("Stopping agent", { reason: reason ?? "user-requested" });
		this.currentAbortController.abort(reason ?? "user-requested");
	}

	async steer(input: string | AgentInput): Promise<void> {
		if (this.disposed) {
			this.logger.warn("Steer called on disposed agent");
			throw createAgentError("agent_disposed", "Agent has been disposed", { recoverable: false });
		}
		if (!this.engineAgent) {
			this.logger.warn("Steer called on uninitialized agent");
			throw createAgentError("agent_not_initialized", "Agent has not been run yet", { recoverable: true });
		}
		this.logger.info("Steering agent", { sessionId: this.config.sessionId });
		return steerAgent(this.engineAgent, input);
	}

	async reset(): Promise<void> {
		if (this.disposed) {
			this.logger.warn("Reset called on disposed agent");
			throw createAgentError("agent_disposed", "Agent has been disposed", { recoverable: false });
		}
		this.logger.info("Resetting agent", { sessionId: this.config.sessionId });
		if (this.engineAgent) {
			await resetAgentState(this.engineAgent);
			this.engineAgent = null;
		}
		this.currentRun = null;
		this.currentAbortController = null;
		this.status = { state: "idle" };
		this.emitter.emit("status", this.status);
	}

	dispose(): void {
		if (this.disposed) return;
		this.logger.info("Disposing agent", { sessionId: this.config.sessionId });
		this.disposed = true;
		if (this.currentAbortController) {
			this.currentAbortController.abort("disposed");
			this.currentAbortController = null;
		}
		if (this.engineAgent) {
			destroyEngineAgent(this.engineAgent);
			this.engineAgent = null;
		}
		this.emitter.clear();
		this.currentRun = null;
	}

	getStatus(): AgentStatus {
		return this.status;
	}
}
