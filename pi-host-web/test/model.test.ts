import assert from "node:assert";
import { describe, it, mock } from "node:test";
import { anthropic, openai, openaiCompatible, defineModel } from "../sdk/index.ts";
import type { ModelRequest, AgentMessage } from "../sdk/types.ts";

describe("Provider factories", () => {
  describe("TM-35: defineModel()", () => {
    it("returns an AgentModel with working generate()", async () => {
      const model = defineModel({
        id: "test-model",
        contextWindow: 50000,
        capabilities: { vision: true },
        generate: async (req) => ({
          content: [{ type: "text", text: "ok" }],
          stopReason: "end" as const,
        }),
      });

      assert.equal(model.id, "test-model");
      assert.equal(model.contextWindow, 50000);
      assert.equal(model.capabilities?.vision, true);

      const result = await model.generate({
        instructions: "test",
        messages: [],
        tools: [],
      });

      assert.equal(result.content[0].text, "ok");
      assert.equal(result.stopReason, "end");
    });

    it("can be passed to Agent constructor", () => {
      const model = defineModel({
        generate: async () => ({
          content: [{ type: "text", text: "hi" }],
          stopReason: "end" as const,
        }),
      });

      assert.equal(typeof model.generate, "function");
      assert.equal(model.id, "custom-model");
    });
  });

  describe("TM-7: anthropic()", () => {
    it("returns AgentModel with correct metadata", () => {
      const model = anthropic({ apiKey: "test-key", model: "claude-3-5-sonnet" });

      assert.equal(model.id, "claude-3-5-sonnet");
      assert.equal(model.contextWindow, 200000);
      assert.equal(model.maxTokens, 4096);
      assert.equal(model.capabilities?.vision, true);
      assert.equal(model.capabilities?.functionCalling, true);
    });

    it("returns AgentModel with generate method", () => {
      const model = anthropic({ apiKey: "test-key", model: "claude-3" });
      assert.equal(typeof model.generate, "function");
    });
  });

  describe("TM-8: openaiCompatible()", () => {
    it("returns AgentModel with correct metadata", () => {
      const model = openaiCompatible({
        apiKey: "test-key",
        baseUrl: "https://api.fireworks.ai",
        model: "llama-v3",
      });

      assert.equal(model.id, "llama-v3");
      assert.equal(model.contextWindow, 128000);
      assert.equal(model.maxTokens, 4096);
    });

    it("TM-30: parses tool_calls from OpenAI response", async () => {
      const mockFetch = mock.fn(async () => ({
        ok: true,
        status: 200,
        json: async () => ({
          choices: [
            {
              message: {
                content: null,
                tool_calls: [
                  {
                    id: "tc1",
                    type: "function",
                    function: {
                      name: "browser_click",
                      arguments: '{"selector":"#btn"}',
                    },
                  },
                ],
              },
              finish_reason: "tool_calls",
            },
          ],
          usage: {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
          },
        }),
        text: async () => "",
      }));

      global.fetch = mockFetch as any;

      const model = openaiCompatible({
        apiKey: "test-key",
        baseUrl: "https://api.fireworks.ai",
        model: "llama-v3",
      });

      const request: ModelRequest = {
        instructions: "test",
        messages: [],
        tools: [
          {
            name: "browser_click",
            description: "Click",
            inputSchema: { type: "object", properties: {} },
            run: () => null,
          },
        ],
      };

      const result = await model.generate(request);

      assert.equal(result.content.length, 1);
      assert.equal(result.content[0].type, "tool_call");
      assert.equal((result.content[0] as any).id, "tc1");
      assert.equal((result.content[0] as any).name, "browser_click");
      assert.deepStrictEqual((result.content[0] as any).arguments, { selector: "#btn" });
      assert.equal(result.stopReason, "tool_call");

      // Verify usage
      assert.equal(result.usage?.input, 10);
      assert.equal(result.usage?.output, 5);
      assert.equal(result.usage?.total_tokens, 15);

      // Verify URL
      const callArgs = mockFetch.mock.calls[0].arguments;
      assert.ok((callArgs[0] as string).includes("/v1/chat/completions"));

      global.fetch = fetch; // restore
    });

    it("converts messages to OpenAI format", async () => {
      const mockFetch = mock.fn(async () => ({
        ok: true,
        status: 200,
        json: async () => ({
          choices: [{ message: { content: "Hello" }, finish_reason: "stop" }],
          usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
        }),
        text: async () => "",
      }));

      global.fetch = mockFetch as any;

      const model = openaiCompatible({
        apiKey: "test-key",
        baseUrl: "https://api.test.com",
        model: "gpt-4",
      });

      const messages: AgentMessage[] = [
        { id: "1", role: "user", content: [{ type: "text", text: "Hi" }] },
        {
          id: "2",
          role: "assistant",
          content: [
            { type: "text", text: "Let me" },
            { type: "tool_call", id: "tc1", name: "tool", arguments: {} },
          ],
        },
        { id: "3", role: "tool_result", content: [{ type: "text", text: "result" }], tool_call_id: "tc1" },
      ];

      await model.generate({
        instructions: "test",
        messages,
        tools: [],
      });

      const body = JSON.parse(mockFetch.mock.calls[0].arguments[1].body);

      // User message
      assert.equal(body.messages[0].role, "user");
      assert.equal(body.messages[0].content, "Hi");

      // Assistant message with tool_calls
      assert.equal(body.messages[1].role, "assistant");
      assert.equal(body.messages[1].content, "Let me");
      assert.ok(Array.isArray(body.messages[1].tool_calls));
      assert.equal(body.messages[1].tool_calls[0].id, "tc1");

      // Tool result message
      assert.equal(body.messages[2].role, "tool");
      assert.equal(body.messages[2].tool_call_id, "tc1");
      assert.equal(body.messages[2].content, "result");

      global.fetch = fetch;
    });

    it("throws structured error on HTTP failure", async () => {
      const mockFetch = mock.fn(async () => ({
        ok: false,
        status: 401,
        text: async () => "Unauthorized",
      }));

      global.fetch = mockFetch as any;

      const model = openaiCompatible({
        apiKey: "bad-key",
        baseUrl: "https://api.test.com",
        model: "gpt-4",
      });

      try {
        await model.generate({ instructions: "test", messages: [], tools: [] });
        assert.fail("should have thrown");
      } catch (err: any) {
        assert.equal(err.code, "model_auth_failed");
        assert.ok(err.message.includes("401"));
      }

      global.fetch = fetch;
    });
  });

  describe("TM-9: openai() URL correctness", () => {
    it("uses correct URL without double /v1", async () => {
      const mockFetch = mock.fn(async () => ({
        ok: true,
        status: 200,
        json: async () => ({
          choices: [{ message: { content: "Hello" }, finish_reason: "stop" }],
          usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
        }),
        text: async () => "",
      }));

      global.fetch = mockFetch as any;

      const model = openai({ apiKey: "test-key", model: "gpt-4o" });

      await model.generate({ instructions: "test", messages: [], tools: [] });

      const url = mockFetch.mock.calls[0].arguments[0] as string;
      assert.equal(url, "https://api.openai.com/v1/chat/completions");

      global.fetch = fetch;
    });
  });
});
