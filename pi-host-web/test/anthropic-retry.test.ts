import assert from "node:assert";
import { describe, it } from "node:test";
import { callAnthropic } from "../sdk/internal/providers/anthropic.ts";
import type { AnthropicConfig, RetryInfo } from "../sdk/internal/providers/anthropic.ts";

function mockHeaders(retryAfter?: string): Headers {
	const h = new Headers();
	if (retryAfter !== undefined) h.set("retry-after", retryAfter);
	return h;
}

function makeConfig(overrides: Partial<AnthropicConfig> = {}): AnthropicConfig {
	return {
		apiKey: "test-key",
		baseUrl: "https://api.test.com",
		model: "claude-test",
		...overrides,
	};
}

const noopRequest = {
	system_prompt: "",
	messages: [],
	tools: [],
};

describe("callAnthropic retry", () => {
	it("does not retry on 401", async () => {
		let calls = 0;
		const originalFetch = globalThis.fetch;
		globalThis.fetch = (async () => {
			calls++;
			return new Response("Unauthorized", {
				status: 401,
				headers: mockHeaders(),
			});
		}) as typeof globalThis.fetch;

		try {
			const result = await callAnthropic(noopRequest, makeConfig());
			assert.ok("Err" in result.llmResult);
			assert.strictEqual(calls, 1);
		} finally {
			globalThis.fetch = originalFetch;
		}
	});

	it("retries on 429 then exhausts", async () => {
		let calls = 0;
		const originalFetch = globalThis.fetch;
		globalThis.fetch = (async () => {
			calls++;
			return new Response("Rate limited", {
				status: 429,
				headers: mockHeaders(),
			});
		}) as typeof globalThis.fetch;

		try {
			const result = await callAnthropic(
				noopRequest,
				makeConfig({ maxRetries: 2 }),
			);
			assert.ok("Err" in result.llmResult);
			// 1 initial + 2 retries = 3 calls
			assert.strictEqual(calls, 3);
		} finally {
			globalThis.fetch = originalFetch;
		}
	});

	it("retries on 529 overload", async () => {
		let calls = 0;
		const originalFetch = globalThis.fetch;
		globalThis.fetch = (async () => {
			calls++;
			return new Response("Overloaded", {
				status: 529,
				headers: mockHeaders(),
			});
		}) as typeof globalThis.fetch;

		try {
			const result = await callAnthropic(
				noopRequest,
				makeConfig({ maxRetries: 1 }),
			);
			assert.ok("Err" in result.llmResult);
			assert.strictEqual(calls, 2);
		} finally {
			globalThis.fetch = originalFetch;
		}
	});

	it("retries on network error (TypeError) then exhausts", async () => {
		let calls = 0;
		const originalFetch = globalThis.fetch;
		globalThis.fetch = (async () => {
			calls++;
			throw new TypeError("fetch failed");
		}) as typeof globalThis.fetch;

		try {
			const result = await callAnthropic(
				noopRequest,
				makeConfig({ maxRetries: 2 }),
			);
			assert.ok("Err" in result.llmResult);
			assert.strictEqual(calls, 3);
		} finally {
			globalThis.fetch = originalFetch;
		}
	});

	it("does not retry on non-retryable network error", async () => {
		let calls = 0;
		const originalFetch = globalThis.fetch;
		globalThis.fetch = (async () => {
			calls++;
			throw new Error("Network failure");
		}) as typeof globalThis.fetch;

		try {
			const result = await callAnthropic(noopRequest, makeConfig());
			assert.ok("Err" in result.llmResult);
			assert.strictEqual(calls, 1);
		} finally {
			globalThis.fetch = originalFetch;
		}
	});

	it("emits onRetry callback on each retry", async () => {
		const retries: RetryInfo[] = [];
		let calls = 0;
		const originalFetch = globalThis.fetch;
		globalThis.fetch = (async () => {
			calls++;
			return new Response("Error", {
				status: calls < 2 ? 429 : 400,
				headers: mockHeaders(),
			});
		}) as typeof globalThis.fetch;

		try {
			await callAnthropic(noopRequest, makeConfig({ maxRetries: 3, onRetry: (info) => retries.push(info) }));
			assert.strictEqual(retries.length, 1);
			assert.strictEqual(retries[0].attempt, 1);
			assert.strictEqual(retries[0].status, 429);
			assert.strictEqual(retries[0].recoverable, true);
		} finally {
			globalThis.fetch = originalFetch;
		}
	});

	it("returns success on retry after transient 429", async () => {
		let calls = 0;
		const originalFetch = globalThis.fetch;
		globalThis.fetch = (async () => {
			calls++;
			if (calls === 1) {
				return new Response("Rate limited", {
					status: 429,
					headers: mockHeaders(),
				});
			}
			return new Response(
				JSON.stringify({
					id: "msg_1",
					type: "message",
					role: "assistant",
					model: "claude-test",
					stop_reason: "end_turn",
					content: [{ type: "text", text: "Hello" }],
					usage: {
						input_tokens: 10,
						output_tokens: 5,
					},
				}),
				{
					status: 200,
					headers: { "content-type": "application/json" },
				},
			);
		}) as typeof globalThis.fetch;

		try {
			const result = await callAnthropic(
				noopRequest,
				makeConfig({ maxRetries: 3 }),
			);
			assert.ok("Ok" in result.llmResult);
			assert.strictEqual(calls, 2);
		} finally {
			globalThis.fetch = originalFetch;
		}
	});
	it("honors retry-after header for delay", async () => {
		const retries: RetryInfo[] = [];
		let calls = 0;
		const originalFetch = globalThis.fetch;
		globalThis.fetch = (async () => {
			calls++;
			return new Response("Rate limited", {
				status: 429,
				headers: mockHeaders("3"),
			});
		}) as typeof globalThis.fetch;

		try {
			await callAnthropic(
				noopRequest,
				makeConfig({ maxRetries: 1, onRetry: (info) => retries.push(info) }),
			);
			assert.strictEqual(retries.length, 1);
			assert.strictEqual(retries[0].delayMs, 3000);
		} finally {
			globalThis.fetch = originalFetch;
		}
	});
});