/**
 * Background job table for tracking long-running tool executions.
 *
 * Host-owned — Rust core never touches this.
 */

export interface JobEntry {
	jobId: string;
	toolCallId: string;
	command: string;
	startedAt: number;
	stopped: boolean;
}

let nextJobId = 1;

export class JobTable {
	private readonly jobs = new Map<string, JobEntry>();

	add(toolCallId: string, command: string): string {
		const jobId = `job-${nextJobId++}`;
		const entry: JobEntry = {
			jobId,
			toolCallId,
			command,
			startedAt: Date.now(),
			stopped: false,
		};
		this.jobs.set(jobId, entry);
		return jobId;
	}

	get(jobId: string): JobEntry | undefined {
		return this.jobs.get(jobId);
	}

	getByToolCallId(toolCallId: string): JobEntry | undefined {
		for (const entry of this.jobs.values()) {
			if (entry.toolCallId === toolCallId && !entry.stopped) {
				return entry;
			}
		}
		return undefined;
	}

	stop(jobId: string): boolean {
		const entry = this.jobs.get(jobId);
		if (entry && !entry.stopped) {
			entry.stopped = true;
			return true;
		}
		return false;
	}

	active(): JobEntry[] {
		return [...this.jobs.values()].filter((j) => !j.stopped);
	}

	cleanup(): void {
		this.jobs.clear();
	}
}
