import assert from "node:assert";
import { describe, it } from "node:test";
import { modelStreamToLlmStream } from "../sdk/orchestration/model-adapter.ts";
import type { ModelEvent } from "../sdk/types.ts";

describe("modelStreamToLlmStream", () => {
	it("turns error stop reasons into Err without partial tool calls", async () => {
		async function* events(): AsyncGenerator<ModelEvent> {
			yield {
				type: "tool_call_delta",
				payload: { id: "tc-orphan", name: "run_js", arguments: "{}" },
			};
			yield {
				type: "done",
				payload: {
					content: [],
					stopReason: "error",
				},
			};
		}

		const stream = modelStreamToLlmStream(events(), new AbortController().signal, {});

		for await (const _chunk of stream.chunks) {
			// drain stream
		}
		const result = await stream.result;

		assert.ok("Err" in result);
		assert.equal(result.Err.error.code, "model_error");
	});
});
