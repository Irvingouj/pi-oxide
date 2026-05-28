/**
 * Fake LLM server for manual replay testing.
 *
 * Serves the built web app and intercepts LLM API calls,
 * replaying recorded responses in order so you can
 * interact with the UI and see the recorded conversation.
 *
 * Usage:
 *   npx tsx scripts/fake-server.ts
 *   Then open http://localhost:3456 in your browser.
 */

import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { join } from "node:path";
import {
	DIST_DIR,
	extractPromptContext,
	loadRecording,
	type Recording,
} from "./lib/testHarness.ts";

let recording: Recording;
try {
	recording = loadRecording("./test/fixtures/multi-turn-recording.json");
} catch {
	console.error(
		"Recording not found. Run: npx tsx scripts/record-multi-turn.ts",
	);
	process.exit(1);
}

let replayIndex = 0;

const server = createServer((req, res) => {
	const urlPath = (req.url || "/").split("?")[0];

	// Intercept LLM API calls
	if (req.url?.includes("/v1/messages") && req.method === "POST") {
		let body = "";
		req.on("data", (chunk) => (body += chunk));
		req.on("end", () => {
			const promptContext = extractPromptContext(body);

			if (replayIndex >= recording.responses.length) {
				console.log(`[FAKE] No more recordings. Replayed all ${replayIndex}.`);
				res.writeHead(200, { "Content-Type": "application/json" });
				res.end(
					JSON.stringify({
						id: "fake-end",
						type: "message",
						role: "assistant",
						content: [{ type: "text", text: "(No more recorded responses.)" }],
						model: recording.meta.model,
						stop_reason: "end_turn",
					}),
				);
				return;
			}

			const recorded = recording.responses[replayIndex];
			console.log(`[FAKE ${replayIndex}] ${promptContext.slice(0, 80)}...`);
			replayIndex++;

			res.writeHead(200, { "Content-Type": "application/json" });
			res.end(JSON.stringify(recorded.response));
		});
		return;
	}

	// Reset endpoint
	if (urlPath === "/__reset") {
		replayIndex = 0;
		console.log("[FAKE] Replay index reset to 0");
		res.writeHead(200, { "Content-Type": "application/json" });
		res.end(JSON.stringify({ ok: true }));
		return;
	}

	// Serve static files
	const filePath = join(DIST_DIR, urlPath === "/" ? "index.html" : urlPath);
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

const PORT = 3456;
server.listen(PORT, () => {
	console.log(`Fake LLM server running at http://localhost:${PORT}`);
	console.log(`  Recording: ${recording.meta.model}`);
	console.log(`  Responses: ${recording.responses.length}`);
	console.log(`  Date: ${recording.meta.date}`);
	console.log("");
	console.log(
		"  Type anything in the chat and the next recorded response will play.",
	);
	console.log("  POST to /__reset to restart from the first response.");
	console.log("  Ctrl+C to stop.");
});
