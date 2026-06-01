import assert from "node:assert";
import { describe, it } from "node:test";
import { z } from "zod";
import { tool, defineTools } from "../../pi-host-web/sdk/tools.ts";
import { ToolRegistryBuilder } from "../../pi-host-web/sdk/internal/tools/registry.ts";
import { artifactTools } from "../../pi-host-web/sdk/internal/tools/artifact.ts";
import type { AgentTools } from "../../pi-host-web/sdk/types.ts";

describe("Tool API", () => {
  describe("tool() and defineTools()", () => {
    it("TM-10: creates a tool definition with Zod schema", () => {
      const clickTool = tool({
        description: "Click an element",
        input: z.object({ selector: z.string() }),
        run: ({ selector }) => ({ clicked: true, selector }),
      });

      assert.equal(clickTool.description, "Click an element");
      assert.ok(clickTool.inputSchema instanceof z.ZodType);
      assert.equal(clickTool.name, ""); // filled by defineTools
    });

    it("TM-10: defineTools assigns names from record keys", () => {
      const tools = defineTools({
        click: tool({
          description: "Click",
          input: z.object({ selector: z.string() }),
          run: ({ selector }) => ({ clicked: true }),
        }),
        type: tool({
          description: "Type",
          input: z.object({ selector: z.string(), text: z.string() }),
          run: ({ selector, text }) => ({ selector, text }),
        }),
      });

      assert.equal(tools.definitions.length, 2);
      assert.equal(tools.definitions[0].name, "click");
      assert.equal(tools.definitions[1].name, "type");
    });

    it("TM-10: getHandler returns the correct run function", () => {
      const runFn = ({ selector }: { selector: string }) => ({ clicked: true });
      const tools = defineTools({
        click: tool({
          description: "Click",
          input: z.object({ selector: z.string() }),
          run: runFn,
        }),
      });

      const handler = tools.getHandler("click");
      assert.ok(handler);
      assert.equal(typeof handler, "function");
    });

    it("TM-10: getHandler returns null for unknown tool", () => {
      const tools = defineTools({
        click: tool({
          description: "Click",
          input: z.object({ selector: z.string() }),
          run: ({ selector }) => ({ clicked: true }),
        }),
      });

      const handler = tools.getHandler("nonexistent");
      assert.strictEqual(handler, null);
    });
  });

  describe("ToolRegistryBuilder", () => {
    it("TM-11: accepts array of tool packs", () => {
      const pack1 = defineTools({
        toolA: tool({
          description: "Tool A",
          input: z.object({}),
          run: () => "a",
        }),
      });

      const pack2 = defineTools({
        toolB: tool({
          description: "Tool B",
          input: z.object({}),
          run: () => "b",
        }),
      });

      const builder = new ToolRegistryBuilder();
      const toolMap = builder.build([pack1, pack2]);
      const llmTools = builder.getLlmTools([pack1, pack2]);

      assert.ok(typeof toolMap["toolA"] === "function");
      assert.ok(typeof toolMap["toolB"] === "function");
      assert.equal(llmTools.length, 2);
      assert.equal(llmTools[0].name, "toolA");
      assert.equal(llmTools[1].name, "toolB");
    });

    it("TM-12: throws on duplicate tool names within a pack", () => {
      // JavaScript objects can't truly have duplicate keys, but we can simulate
      // by passing two packs with the same name
      const pack1 = defineTools({
        sameName: tool({
          description: "First",
          input: z.object({}),
          run: () => "first",
        }),
      });

      const pack2 = defineTools({
        sameName: tool({
          description: "Second",
          input: z.object({}),
          run: () => "second",
        }),
      });

      const builder = new ToolRegistryBuilder();
      assert.throws(
        () => builder.build([pack1, pack2]),
        (err: any) => err.code === "tool_duplicate",
      );
    });

    it("TM-12: throws on duplicate tool names in getLlmTools", () => {
      const pack1 = defineTools({
        sameName: tool({
          description: "First",
          input: z.object({}),
          run: () => "first",
        }),
      });

      const pack2 = defineTools({
        sameName: tool({
          description: "Second",
          input: z.object({}),
          run: () => "second",
        }),
      });

      const builder = new ToolRegistryBuilder();
      assert.throws(
        () => builder.getLlmTools([pack1, pack2]),
        (err: any) => err.code === "tool_duplicate",
      );
    });

    it("TM-10: handler receives arguments from tool call", async () => {
      const testTool = defineTools({
        validateMe: tool({
          description: "Validate input",
          input: z.object({ selector: z.string() }),
          run: ({ selector }) => ({ selector }),
        }),
      });

      const builder = new ToolRegistryBuilder();
      const toolMap = builder.build([testTool]);
      const handler = toolMap["validateMe"];

      // Handler receives arguments and returns result
      const result = await handler({ id: "1", name: "validateMe", arguments: { selector: "#btn" } });
      assert.equal(result.content[0].text, '{\n  "selector": "#btn"\n}');
    });

    it("TM-31: preserves details field in ToolResult", async () => {
      const testTool = defineTools({
        detailed: tool({
          description: "Detailed tool",
          input: z.object({}),
          run: () => ({ result: "ok" }),
          details: (output) => ({ strategy: "compact", output }),
        }),
      });

      const builder = new ToolRegistryBuilder();
      const toolMap = builder.build([testTool]);
      const handler = toolMap["detailed"];

      const result = await handler({ id: "1", name: "detailed", arguments: {} });
      assert.deepStrictEqual(result.details, { strategy: "compact", output: { result: "ok" } });
    });

    it("converts Zod schema to JSON Schema for LLM tools", () => {
      const testTool = defineTools({
        schemaTest: tool({
          description: "Schema test",
          input: z.object({ name: z.string(), count: z.number().optional() }),
          run: () => "ok",
        }),
      });

      const builder = new ToolRegistryBuilder();
      const llmTools = builder.getLlmTools([testTool]);

      assert.equal(llmTools.length, 1);
      assert.equal(llmTools[0].name, "schemaTest");
      assert.ok(typeof llmTools[0].parameters === "object");
      // zodToJsonSchema with { name } wraps in definitions
      const params = llmTools[0].parameters as any;
      assert.ok(params.$ref || params.type === "object" || params.definitions?.schemaTest?.type === "object");
    });
  });

  describe("artifactTools", () => {
    it("returns artifact tool definitions", () => {
      const tools = artifactTools();

      assert.equal(tools.definitions.length, 2);
      assert.ok(tools.definitions.some((d) => d.name === "artifact_read"));
      assert.ok(tools.definitions.some((d) => d.name === "artifact_search"));
    });

    it("getHandler returns null (handlers wired at build time)", () => {
      const tools = artifactTools();
      assert.strictEqual(tools.getHandler("artifact_read"), null);
    });
  });
});
