import { HostDirective, getHostAgentPersistData } from "@pi-oxide/pi-host-web";
const d: HostDirective = { type: "stream_llm", context: { messages: [], tools: [] } };
const r = getHostAgentPersistData(0);
