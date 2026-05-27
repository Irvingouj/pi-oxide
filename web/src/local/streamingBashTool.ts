/**
 * Streaming bash tool — async shell execution with stdout/stderr callbacks.
 *
 * Replaces the synchronous execFileSync-based bashTool for the async runtime.
 * Host-owned — no Rust/Node/process assumptions in pi-core.
 */

import { type ChildProcess, spawn } from "node:child_process";

export interface StreamingBashCallbacks {
	onStdout: (chunk: string) => void;
	onStderr: (chunk: string) => void;
}

export interface StreamingBashResult {
	stdout: string;
	stderr: string;
	exitCode: number | null;
	cancelled: boolean;
}

export interface StreamingBashHandle {
	process: ChildProcess;
	result: Promise<StreamingBashResult>;
	cancel: () => void;
}

/**
 * Start a bash command with streaming output.
 *
 * Returns a handle with the child process, a promise for the final result,
 * and a cancel function.
 */
export function startStreamingBash(
	command: string,
	cwd: string,
	callbacks: StreamingBashCallbacks,
	timeout?: number,
): StreamingBashHandle {
	let cancelled = false;
	let settled = false;
	let stdout = "";
	let stderr = "";
	let timeoutHandle: NodeJS.Timeout | undefined;

	const proc = spawn("sh", ["-c", command], {
		cwd,
		stdio: ["pipe", "pipe", "pipe"],
		detached: process.platform !== "win32",
	});

	const signalProcess = (signal: NodeJS.Signals): void => {
		if (settled) return;
		try {
			if (process.platform !== "win32" && proc.pid !== undefined) {
				process.kill(-proc.pid, signal);
			} else {
				proc.kill(signal);
			}
		} catch {
			try {
				proc.kill(signal);
			} catch {
				// Process may have already exited.
			}
		}
	};

	const cancel = () => {
		if (settled) return;
		cancelled = true;
		signalProcess("SIGTERM");
		setTimeout(() => {
			if (!settled) {
				signalProcess("SIGKILL");
			}
		}, 2000);
	};

	const result = new Promise<StreamingBashResult>((resolve) => {
		proc.stdout.on("data", (data: Buffer) => {
			const chunk = data.toString("utf-8");
			stdout += chunk;
			callbacks.onStdout(chunk);
		});

		proc.stderr.on("data", (data: Buffer) => {
			const chunk = data.toString("utf-8");
			stderr += chunk;
			callbacks.onStderr(chunk);
		});

		proc.on("close", (code) => {
			settled = true;
			if (timeoutHandle !== undefined) {
				clearTimeout(timeoutHandle);
			}
			resolve({
				stdout,
				stderr,
				exitCode: code,
				cancelled,
			});
		});

		proc.on("error", (err) => {
			settled = true;
			if (timeoutHandle !== undefined) {
				clearTimeout(timeoutHandle);
			}
			stderr += (stderr.length > 0 ? "\n" : "") + err.message;
			resolve({
				stdout,
				stderr,
				exitCode: 1,
				cancelled,
			});
		});

		if (timeout) {
			timeoutHandle = setTimeout(() => {
				cancel();
			}, timeout);
		}
	});

	return { process: proc, result, cancel };
}
