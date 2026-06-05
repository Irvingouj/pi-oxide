// Artifact types for the pi-oxide SDK.
// These are public objects, not leaked host internals.

export interface AgentArtifact {
	id: string;
	kind: "text" | "json" | "binary";
	content: string | Uint8Array | unknown;
	mimeType?: string;
	title?: string;
	metadata?: Record<string, unknown>;
	createdAt: number;
}

export interface AgentArtifactRef {
	id: string;
	kind: AgentArtifact["kind"];
	title?: string;
	mimeType?: string;
}

export interface ArtifactPolicy {
	mode: "inline" | "external";
}

export interface ArtifactSearchQuery {
	text: string;
	limit?: number;
}

export interface ArtifactSearchResult {
	artifact: AgentArtifactRef;
	snippet?: string;
	score?: number;
	matchCount?: number;
}
