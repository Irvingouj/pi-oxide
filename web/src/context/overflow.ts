/**
 * Overflow error detection for LLM API responses.
 *
 * Pattern-matches error messages from providers to detect context window
 * overflow, enabling reactive compaction and retry.
 */

const OVERFLOW_PATTERNS = [
	/prompt is too long/i,
	/too many tokens/i,
	/request too large/i,
	/exceeds.*context.*window/i,
	/token.?limit/i,
	/maximum.*context.*length/i,
	/input length.*exceed/i,
	/context_length_exceeded/i,
];

export function isOverflowError(error: unknown): boolean {
	const msg = error instanceof Error ? error.message : String(error);
	return OVERFLOW_PATTERNS.some((p) => p.test(msg));
}
