/**
 * Persistent host wrapper — wires session + artifact persistence into RealAgentHost.
 *
 * Records trace entries as session entries and persists artifact contents
 * when Rust projection replaces a tool result. Does not mutate the canonical
 * Rust transcript.
 *
 * Host-owned — no Rust/Node/process assumptions in pi-core.
 */

import type {
	ArtifactRecord,
	ArtifactStore,
} from "../context/rustProjection.ts";
import type { ToolRegistry } from "../fakeTools.ts";
import {
	type ContextProjectionConfig,
	RealAgentHost,
	type RealLlm,
	type TraceEntry,
} from "../providers/realLlm.ts";
import type { AgentOptions } from "../wasmBinding.ts";
import { FileArtifactStore } from "./fileArtifactStore.ts";
import { LocalSessionStore, type SessionEntryKind } from "./sessionStore.ts";
import type { ToolRuntime } from "./toolRuntime.ts";

/**
 * Trace phases that map to session entry kinds.
 * Not every trace entry needs to be persisted.
 */
function traceToSessionKind(
	phase: TraceEntry["phase"],
	type: string,
): SessionEntryKind | null {
	if (phase === "host") {
		switch (type) {
			case "prompt":
				return "user_prompt";
			case "tool_done":
				return "tool_result";
			case "llm_result":
				return "assistant_message";
			case "create_agent":
				return "lifecycle_event";
			case "destroy_agent":
				return "lifecycle_event";
			default:
				return null;
		}
	}
	if (phase === "event") {
		switch (type) {
			case "tool_execution_start":
				return "tool_call";
			case "tool_execution_update":
				return "tool_streaming_update";
			case "tool_execution_end":
				return "lifecycle_event";
			case "agent_start":
				return "lifecycle_event";
			case "agent_end":
				return "lifecycle_event";
			default:
				return null;
		}
	}
	return null;
}

export interface PersistentHostOptions {
	sessionDir: string;
	sessionId: string;
	cwd: string;
	model: string;
}

export class PersistentHost {
	readonly session: LocalSessionStore;
	readonly fileArtifacts: FileArtifactStore;
	readonly host: RealAgentHost;
	private projectionLogs: string[] = [];

	constructor(
		options: PersistentHostOptions,
		llm: RealLlm,
		tools: ToolRegistry,
		runtime?: ToolRuntime,
	) {
		this.session = new LocalSessionStore(options.sessionDir, {
			session_id: options.sessionId,
			cwd: options.cwd,
			model: options.model,
		});

		this.fileArtifacts = new FileArtifactStore(this.session.artifactsDir);

		this.host = new RealAgentHost(llm, tools, runtime);

		// If llm has context projection, hook into artifact storage
		const config = (
			llm as unknown as { contextProjection?: ContextProjectionConfig }
		).contextProjection;
		if (config) {
			// Wrap the artifact store to also write to filesystem
			const originalStore = config.artifacts;
			config.artifacts = new DualArtifactStore(
				originalStore,
				this.fileArtifacts,
			);
		}
	}

	async run(agentOptions: AgentOptions, userPrompt: string) {
		const result = await this.host.run(agentOptions, userPrompt);

		// Persist the entire trace
		for (const entry of result.trace) {
			const kind = traceToSessionKind(entry.phase, entry.type);
			if (kind !== null) {
				this.session.append(kind, {
					phase: entry.phase,
					type: entry.type,
					data: entry.data,
				});
			}
		}

		// Persist context projection logs
		const llm = this.host.llm;
		const projectionLogs = llm.log.filter((l) =>
			l.startsWith("context_projection:"),
		);
		for (const log of projectionLogs) {
			if (!this.projectionLogs.includes(log)) {
				this.session.append("context_projection_report", { log });
				this.projectionLogs.push(log);
			}
		}

		return result;
	}

	cleanup(handle: number): void {
		this.host.cleanup(handle);
		this.session.close();
	}
}

/**
 * Dual artifact store that writes to both an in-memory store (for tests)
 * and a filesystem store (for persistence).
 */
class DualArtifactStore implements ArtifactStore {
	private readonly memory: ArtifactStore;
	private readonly disk: FileArtifactStore;

	constructor(memory: ArtifactStore, disk: FileArtifactStore) {
		this.memory = memory;
		this.disk = disk;
	}

	put(record: ArtifactRecord): string {
		this.memory.put(record);
		this.disk.put(record);
		return record.id;
	}

	get(id: string): ArtifactRecord | undefined {
		return this.memory.get(id) ?? this.disk.get(id);
	}
}
