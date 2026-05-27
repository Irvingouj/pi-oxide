/**
 * Filesystem-backed artifact store.
 *
 * Stores full tool outputs by artifact ID emitted from Rust projection reports.
 * Each artifact is a file in the artifacts directory. A metadata JSON sidecar
 * records tool name, tool call ID, byte length, and creation time.
 *
 * Layout:
 *   <artifactsDir>/
 *     <artifact-id>.txt        — full content
 *     <artifact-id>.meta.json  — metadata
 *
 * Implements the ArtifactStore interface from rustProjection.ts so it can be
 * used as a drop-in replacement for MemoryArtifactStore.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import type {
	ArtifactRecord,
	ArtifactStore,
} from "../context/rustProjection.ts";

export interface FileArtifactMeta {
	id: string;
	toolName: string;
	toolCallId: string;
	byteLength: number;
	createdAt: number;
}

function assertSafeArtifactId(id: string): void {
	if (!/^[A-Za-z0-9._-]+$/.test(id)) {
		throw new Error(`invalid artifact id: ${id}`);
	}
}

export class FileArtifactStore implements ArtifactStore {
	readonly artifactsDir: string;

	constructor(artifactsDir: string) {
		this.artifactsDir = artifactsDir;
		fs.mkdirSync(artifactsDir, { recursive: true });
	}

	/** Store an artifact. Writes content to a file and metadata to a sidecar. */
	put(record: ArtifactRecord): string {
		assertSafeArtifactId(record.id);
		const contentPath = path.join(this.artifactsDir, `${record.id}.txt`);
		const metaPath = path.join(this.artifactsDir, `${record.id}.meta.json`);

		fs.writeFileSync(contentPath, record.content, "utf-8");

		const meta: FileArtifactMeta = {
			id: record.id,
			toolName: record.toolName,
			toolCallId: record.toolCallId,
			byteLength: Buffer.byteLength(record.content, "utf-8"),
			createdAt: record.storedAt,
		};
		fs.writeFileSync(metaPath, JSON.stringify(meta, null, 2), "utf-8");

		return record.id;
	}

	/** Retrieve an artifact by ID. Returns undefined if not found. */
	get(id: string): ArtifactRecord | undefined {
		assertSafeArtifactId(id);
		const contentPath = path.join(this.artifactsDir, `${id}.txt`);

		if (!fs.existsSync(contentPath)) {
			return undefined;
		}

		const content = fs.readFileSync(contentPath, "utf-8");
		const meta = this.readMeta(id);

		return {
			id,
			toolName: meta?.toolName ?? "unknown",
			toolCallId: meta?.toolCallId ?? "",
			content,
			storedAt: meta?.createdAt ?? 0,
		};
	}

	/** Read just the metadata for an artifact. */
	readMeta(id: string): FileArtifactMeta | undefined {
		assertSafeArtifactId(id);
		const metaPath = path.join(this.artifactsDir, `${id}.meta.json`);
		if (!fs.existsSync(metaPath)) {
			return undefined;
		}
		return JSON.parse(fs.readFileSync(metaPath, "utf-8")) as FileArtifactMeta;
	}

	/** Check if an artifact exists. */
	has(id: string): boolean {
		assertSafeArtifactId(id);
		return fs.existsSync(path.join(this.artifactsDir, `${id}.txt`));
	}

	/** List all artifact IDs in the store. */
	list(): string[] {
		if (!fs.existsSync(this.artifactsDir)) return [];
		return fs
			.readdirSync(this.artifactsDir)
			.filter((f) => f.endsWith(".txt"))
			.map((f) => f.slice(0, -".txt".length));
	}
}
