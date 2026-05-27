import {
	RealAgentHost,
	type RealLlm,
	type SessionBackend,
} from "../providers/realLlm.ts";
import type { AgentOptions } from "../wasmBinding.ts";
import type { BrowserRuntime } from "./browserRuntime.ts";
import { BROWSER_TOOLS, BrowserToolRegistry } from "./browserTools.ts";

export interface BrowserHostOptions {
	runtime: BrowserRuntime;
	/** If true, browser tools will also be available as model tools. Default: true. */
	includeTools?: boolean;
	/** Optional session persistence backend. */
	sessionBackend?: SessionBackend;
}

export class BrowserHost {
	readonly host: RealAgentHost;
	readonly tools: BrowserToolRegistry;
	readonly llm: RealLlm;
	private readonly sessionBackend?: SessionBackend;

	constructor(options: BrowserHostOptions, llm: RealLlm) {
		this.tools = new BrowserToolRegistry(options.runtime);
		this.llm = llm;
		this.host = new RealAgentHost(llm, this.tools);
		this.sessionBackend = options.sessionBackend;
	}

	async run(options: AgentOptions, userPrompt: string) {
		return this.host.run(options, userPrompt, this.sessionBackend);
	}

	cleanup(handle: number): void {
		this.host.cleanup(handle);
	}
}

/**
 * Build browser AgentOptions with browser tool definitions.
 */
export function browserAgentOptions(
	overrides: Partial<AgentOptions> &
		Pick<AgentOptions, "system_prompt" | "model">,
): AgentOptions {
	return {
		thinking_level: "off",
		tools: BROWSER_TOOLS,
		...overrides,
	};
}
