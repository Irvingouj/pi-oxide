/**
 * Shared test harness for record, replay, and fake-server scripts.
 */

import { mkdirSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

export const DIST_DIR = join(__dirname, "../../dist");
export const FIXTURES_DIR = join(__dirname, "../../test/fixtures");

export interface Recording {
	meta: {
		model: string;
		baseUrl: string;
		date: string;
		prompts: string[];
	};
	responses: Array<{
		requestIndex: number;
		promptContext: string;
		response: unknown;
	}>;
}

export function ensureFixturesDir(): void {
	mkdirSync(FIXTURES_DIR, { recursive: true });
}

export function loadRecording(path: string): Recording {
	return JSON.parse(readFileSync(path, "utf-8"));
}

export function createStaticServer(
	distDir: string = DIST_DIR,
): import("http").Server {
	return createServer((req, res) => {
		const urlPath = (req.url || "/").split("?")[0];
		const filePath = join(distDir, urlPath === "/" ? "index.html" : urlPath);
		try {
			const data = readFileSync(filePath);
			const ext = filePath.endsWith(".wasm")
				? "application/wasm"
				: filePath.endsWith(".js")
					? "application/javascript"
					: filePath.endsWith(".html")
						? "text/html"
						: filePath.endsWith(".css")
							? "text/css"
							: "application/octet-stream";
			res.writeHead(200, { "Content-Type": ext });
			res.end(data);
		} catch {
			res.writeHead(404);
			res.end(`Not found: ${urlPath}`);
		}
	});
}

export function getServerBaseUrl(
	server: import("http").Server,
): Promise<string> {
	return new Promise((resolve) => {
		server.listen(0, () => {
			const addr = server.address() as { port: number };
			resolve(`http://localhost:${addr.port}`);
		});
	});
}

export function extractPromptContext(postData: string | null): string {
	try {
		const body = JSON.parse(postData || "{}");
		const lastMsg = body.messages?.at(-1);
		if (lastMsg?.role === "user") {
			return typeof lastMsg.content === "string"
				? lastMsg.content
				: JSON.stringify(lastMsg.content).slice(0, 200);
		}
	} catch {
		/* ignore */
	}
	return "";
}
