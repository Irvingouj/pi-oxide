import { useState, useEffect, useCallback, useRef } from "react";
import { Agent } from "../agent.ts";
import type {
  AgentConfig,
  AgentInput,
  AgentRunOptions,
  AgentRunResult,
  AgentMessage,
  AgentToolRun,
  AgentArtifactRef,
  AgentStatus,
  AgentError,
  UseAgentResult,
} from "../types.ts";
import { createAgentError } from "../errors.ts";

// ---------------------------------------------------------------------------
// Shallow comparison helpers
// ---------------------------------------------------------------------------

function shallowEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object") return false;
  if (a === null || b === null) return false;

  const aRecord = a as Record<string, unknown>;
  const bRecord = b as Record<string, unknown>;
  const aKeys = Object.keys(aRecord);
  const bKeys = Object.keys(bRecord);
  if (aKeys.length !== bKeys.length) return false;

  for (const key of aKeys) {
    if (aRecord[key] !== bRecord[key]) return false;
  }
  return true;
}

function shallowArrayEqual(a: unknown[], b: unknown[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

function toolsEqual(
  a: AgentConfig["tools"],
  b: AgentConfig["tools"],
): boolean {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) {
    return shallowArrayEqual(a, b);
  }
  if (!Array.isArray(a) && !Array.isArray(b)) {
    return shallowEqual(a, b);
  }
  return false;
}

// ---------------------------------------------------------------------------
// useStableConfig
// ---------------------------------------------------------------------------

/**
 * Stabilises an `AgentConfig` reference so that the hook only sees a new
 * object when one of the config fields actually changes (by value).
 * Object fields (`context`, `artifacts`, `telemetry`) are compared shallowly.
 * `tools` is compared shallowly (array or single object).
 */
function useStableConfig(config: AgentConfig): AgentConfig {
  const ref = useRef<AgentConfig>(config);

  const prev = ref.current;
  const changed =
    prev.sessionId !== config.sessionId ||
    prev.model !== config.model ||
    prev.instructions !== config.instructions ||
    prev.store !== config.store ||
    !shallowEqual(prev.context, config.context) ||
    !shallowEqual(prev.artifacts, config.artifacts) ||
    !shallowEqual(prev.telemetry, config.telemetry) ||
    !toolsEqual(prev.tools, config.tools);

  if (changed) {
    ref.current = config;
  }

  return ref.current;
}

// ---------------------------------------------------------------------------
// useAgent
// ---------------------------------------------------------------------------

export function useAgent(config: AgentConfig): UseAgentResult {
  const stableConfig = useStableConfig(config);
  const agentRef = useRef<Agent | null>(null);

  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [toolCalls, setToolCalls] = useState<AgentToolRun[]>([]);
  const [artifacts, setArtifacts] = useState<AgentArtifactRef[]>([]);
  const [status, setStatus] = useState<AgentStatus>({ state: "idle" });
  const [error, setError] = useState<AgentError | null>(null);

  // Create / re-create Agent whenever the stable config changes.
  useEffect(() => {
    const agent = new Agent(stableConfig);
    agentRef.current = agent;

    const unsubscribers: (() => void)[] = [];

    unsubscribers.push(
      agent.on("messageStart", (message) => {
        setMessages((prev) => [...prev, message]);
      }),
    );

    unsubscribers.push(
      agent.on("text", (delta) => {
        setMessages((prev) => {
          const lastIndex = prev.length - 1;
          const last = prev[lastIndex];
          if (!last || last.role !== "assistant") return prev;

          const content = [...last.content];
          const textIndex = content.findIndex((b) => b.type === "text");
          if (textIndex >= 0) {
            const block = content[textIndex] as { type: "text"; text: string };
            content[textIndex] = { ...block, text: block.text + delta };
          } else {
            content.push({ type: "text", text: delta });
          }

          return [...prev.slice(0, lastIndex), { ...last, content }];
        });
      }),
    );

    unsubscribers.push(
      agent.on("messageEnd", (message) => {
        setMessages((prev) => {
          const lastIndex = prev.length - 1;
          if (lastIndex >= 0 && prev[lastIndex].role === "assistant") {
            return [...prev.slice(0, lastIndex), message];
          }
          return [...prev, message];
        });
      }),
    );

    unsubscribers.push(
      agent.on("toolStart", (tool) => {
        setToolCalls((prev) => [...prev, tool]);
      }),
    );

    unsubscribers.push(
      agent.on("toolUpdate", (tool) => {
        setToolCalls((prev) => prev.map((t) => (t.id === tool.id ? tool : t)));
      }),
    );

    unsubscribers.push(
      agent.on("toolEnd", (tool) => {
        setToolCalls((prev) => prev.map((t) => (t.id === tool.id ? tool : t)));
      }),
    );

    unsubscribers.push(
      agent.on("artifact", (artifact) => {
        setArtifacts((prev) => [...prev, artifact]);
      }),
    );

    unsubscribers.push(
      agent.on("status", (newStatus) => {
        setStatus(newStatus);
      }),
    );

    unsubscribers.push(
      agent.on("error", (err) => {
        setError(err);
      }),
    );

    unsubscribers.push(
      agent.on("done", (result) => {
        if (result.status === "completed") {
          setStatus({ state: "completed" });
        } else if (result.status === "aborted") {
          setStatus({ state: "aborted" });
        } else if (result.status === "failed") {
          setStatus({ state: "failed", message: result.error?.message });
        }
      }),
    );

    return () => {
      for (const unsub of unsubscribers) {
        unsub();
      }
      agent.dispose();
      agentRef.current = null;
    };
  }, [stableConfig]);

  const send = useCallback(
    async (
      input: string | AgentInput,
      options?: AgentRunOptions,
    ): Promise<AgentRunResult> => {
      const agent = agentRef.current;
      if (!agent) {
        const err = createAgentError(
          "agent_disposed",
          "Agent is not available",
          { recoverable: false },
        );
        setError(err);
        return {
          status: "failed",
          text: "",
          toolCalls: [],
          artifacts: [],
          error: err,
        };
      }

      // Prepend user message to messages state before calling run()
      const userMessage: AgentMessage = {
        id: `user-${Date.now()}`,
        role: "user",
        content: [
          { type: "text", text: typeof input === "string" ? input : input.text },
        ],
        timestamp: Date.now(),
      };
      setMessages((prev) => [userMessage, ...prev]);

      try {
        return await agent.run(input, options);
      } catch (e) {
        const err = createAgentError(
          "internal_error",
          e instanceof Error ? e.message : String(e),
          { cause: e, recoverable: false },
        );
        setError(err);
        return {
          status: "failed",
          text: "",
          toolCalls: [],
          artifacts: [],
          error: err,
        };
      }
    },
    [],
  );

  const stop = useCallback((reason?: string) => {
    agentRef.current?.stop(reason);
  }, []);

  const steer = useCallback(
    async (input: string | AgentInput): Promise<void> => {
      const agent = agentRef.current;
      if (!agent) {
        const err = createAgentError(
          "agent_disposed",
          "Agent is not available",
          { recoverable: false },
        );
        setError(err);
        throw err;
      }
      try {
        await agent.steer(input);
      } catch (e) {
        const err = createAgentError(
          "internal_error",
          e instanceof Error ? e.message : String(e),
          { cause: e, recoverable: false },
        );
        setError(err);
        throw err;
      }
    },
    [],
  );

  const reset = useCallback(async (): Promise<void> => {
    const agent = agentRef.current;
    if (!agent) {
      const err = createAgentError(
        "agent_disposed",
        "Agent is not available",
        { recoverable: false },
      );
      setError(err);
      throw err;
    }
    try {
      setError(null); // Clear error BEFORE await
      await agent.reset();
      setMessages([]);
      setToolCalls([]);
      setArtifacts([]);
    } catch (e) {
      const err = createAgentError(
        "internal_error",
        e instanceof Error ? e.message : String(e),
        { cause: e, recoverable: false },
      );
      setError(err);
      // Error is NOT cleared after failure — it reflects the failure
      throw err;
    }
  }, []);

  return {
    send,
    stop,
    steer,
    reset,
    status,
    messages,
    toolCalls,
    artifacts,
    error,
  };
}
