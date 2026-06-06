import type { ToolCall, ToolCallPreparation, ToolError } from "../../pi_host_web.js";
import type { Logger } from "../types.ts";
import { HostError } from "./init.ts";
import type { AgentRunConfig } from "./types.ts";

type Result<T> = { ok: true; value: T } | { ok: false; error: Error };

function safeHook<T>(fn: () => T | Promise<T>): Promise<Result<T>> {
	try {
		return Promise.resolve(fn()).then(
			(value) => ({ ok: true, value }),
			(error) => ({ ok: false, error: error instanceof Error ? error : new Error(String(error)) }),
		);
	} catch (error) {
		return Promise.resolve({
			ok: false,
			error: error instanceof Error ? error : new Error(String(error)),
		});
	}
}

function matchResult<T, U>(result: Result<T>, arms: { ok: (value: T) => U; err: (error: Error) => U }): U {
	return result.ok ? arms.ok(result.value) : arms.err(result.error);
}

export function toolErrorFromUnknown(e: unknown): ToolError {
	if (e instanceof HostError) {
		return { code: e.code, message: e.message };
	}
	return {
		code: "tool_failed",
		message: e instanceof Error ? e.message : String(e),
	};
}

interface ToolCallPrepResult {
	items: ToolCallPreparation[];
	hadError: boolean;
	errorMessage?: string;
}

export async function buildToolCallPreparations(
	calls: ToolCall[],
	hooks: AgentRunConfig["prepareToolCalls"],
	logger: Logger,
): Promise<ToolCallPrepResult> {
	const items: ToolCallPreparation[] = [];
	let hadError = false;
	let errorMessage: string | undefined;

	const blocked = (id: string): ToolCallPreparation => ({
		tool_call_id: id,
		transform: { type: "none" as const },
		permission: { type: "block" as const, reason: "host preparation failed" },
	});

	for (const call of calls) {
		if (hadError) {
			items.push(blocked(call.id));
			continue;
		}

		const result = await safeHook(async () => {
			const rawTransform = await (hooks?.transform?.(call) ?? { type: "none" as const });
			const transformedCall =
				rawTransform.type === "rewrite_args" ? { ...call, arguments: rawTransform.arguments } : call;
			const rawPermission = await (hooks?.permission?.(transformedCall) ?? { type: "allow" as const });
			return { rawTransform, rawPermission };
		});

		matchResult(result, {
			ok: ({ rawTransform, rawPermission }) => {
				items.push({
					tool_call_id: call.id,
					transform: rawTransform,
					permission: rawPermission,
				});
			},
			err: (error) => {
				hadError = true;
				errorMessage = error.message;
				logger.error("Hook error", { toolCallId: call.id, error: error.message });
				items.push(blocked(call.id));
			},
		});
	}

	return { items, hadError, errorMessage };
}
