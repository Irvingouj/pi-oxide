// AgentError interface and factory.
// AgentError is an interface, NOT a class — it is a plain object.

import type { AgentError } from "./types.ts";

export type { AgentError } from "./types.ts";

export function createAgentError(
	code: AgentError["code"],
	message: string,
	options?: {
		cause?: unknown;
		recoverable?: boolean;
		metadata?: Record<string, unknown>;
	},
): AgentError {
	return {
		code,
		message,
		cause: options?.cause,
		recoverable: options?.recoverable ?? false,
		metadata: options?.metadata,
	};
}
