/**
 * High-level JS SDK for @pi-oxide/pi-host-web.
 *
 * Re-exports all raw types so consumers never need to import from ./raw.
 */

export * from "../pi_host_web";

export declare function ensureInit(): Promise<void>;

export declare function toolResult(
	text: string,
	opts?: { terminate?: boolean; details?: object },
): {
	content: Array<{ type: "text"; text: string }>;
	terminate?: boolean;
	details?: object;
};

export declare class HostError extends Error {
	code: string;
	constructor(code: string, message: string);
}

export declare function unwrap<T>(result: {
	ok: boolean;
	data?: T;
	error?: { code: string; message: string };
}): T;
