// Structured logging for the pi-oxide SDK.
// Works in browser, extension, and Node.js contexts.

export type LogLevel = "trace" | "debug" | "info" | "warn" | "error";

export interface LogEntry {
	level: LogLevel;
	scope: string;
	message: string;
	context?: Record<string, unknown>;
	timestamp: number;
}

export interface Logger {
	trace(message: string, context?: Record<string, unknown>): void;
	debug(message: string, context?: Record<string, unknown>): void;
	info(message: string, context?: Record<string, unknown>): void;
	warn(message: string, context?: Record<string, unknown>): void;
	error(message: string, context?: Record<string, unknown>): void;
}

const LEVEL_ORDER: LogLevel[] = ["trace", "debug", "info", "warn", "error"];

function levelIndex(level: LogLevel): number {
	return LEVEL_ORDER.indexOf(level);
}

let globalLevel: LogLevel = "warn";

export function getGlobalLogLevel(): LogLevel {
	return globalLevel;
}

export function setGlobalLogLevel(level: LogLevel): void {
	globalLevel = level;
}

export class ConsoleLogger implements Logger {
	private scope: string;
	private minLevel: LogLevel;

	constructor(scope: string, minLevel?: LogLevel) {
		this.scope = scope;
		this.minLevel = minLevel ?? globalLevel;
	}

	private shouldLog(level: LogLevel): boolean {
		return levelIndex(level) >= levelIndex(this.minLevel);
	}

	private log(level: LogLevel, message: string, context?: Record<string, unknown>): void {
		if (!this.shouldLog(level)) return;

		const prefix = `[${this.scope}]`;
		const timestamp = new Date().toISOString();

		if (context && Object.keys(context).length > 0) {
			switch (level) {
				case "trace":
					console.trace(`${timestamp} ${prefix} ${message}`, context);
					break;
				case "debug":
					console.debug(`${timestamp} ${prefix} ${message}`, context);
					break;
				case "info":
					console.info(`${timestamp} ${prefix} ${message}`, context);
					break;
				case "warn":
					console.warn(`${timestamp} ${prefix} ${message}`, context);
					break;
				case "error":
					console.error(`${timestamp} ${prefix} ${message}`, context);
					break;
			}
		} else {
			switch (level) {
				case "trace":
					console.trace(`${timestamp} ${prefix} ${message}`);
					break;
				case "debug":
					console.debug(`${timestamp} ${prefix} ${message}`);
					break;
				case "info":
					console.info(`${timestamp} ${prefix} ${message}`);
					break;
				case "warn":
					console.warn(`${timestamp} ${prefix} ${message}`);
					break;
				case "error":
					console.error(`${timestamp} ${prefix} ${message}`);
					break;
			}
		}
	}

	trace(message: string, context?: Record<string, unknown>): void {
		this.log("trace", message, context);
	}
	debug(message: string, context?: Record<string, unknown>): void {
		this.log("debug", message, context);
	}
	info(message: string, context?: Record<string, unknown>): void {
		this.log("info", message, context);
	}
	warn(message: string, context?: Record<string, unknown>): void {
		this.log("warn", message, context);
	}
	error(message: string, context?: Record<string, unknown>): void {
		this.log("error", message, context);
	}
}

export class NoopLogger implements Logger {
	trace(): void {}
	debug(): void {}
	info(): void {}
	warn(): void {}
	error(): void {}
}

export class CallbackLogger implements Logger {
	private callback: (entry: LogEntry) => void;
	private scope: string;

	constructor(callback: (entry: LogEntry) => void, scope: string) {
		this.callback = callback;
		this.scope = scope;
	}

	private emit(level: LogLevel, message: string, context?: Record<string, unknown>): void {
		this.callback({
			level,
			scope: this.scope,
			message,
			context,
			timestamp: Date.now(),
		});
	}

	trace(message: string, context?: Record<string, unknown>): void {
		this.emit("trace", message, context);
	}
	debug(message: string, context?: Record<string, unknown>): void {
		this.emit("debug", message, context);
	}
	info(message: string, context?: Record<string, unknown>): void {
		this.emit("info", message, context);
	}
	warn(message: string, context?: Record<string, unknown>): void {
		this.emit("warn", message, context);
	}
	error(message: string, context?: Record<string, unknown>): void {
		this.emit("error", message, context);
	}
}

const loggers = new Map<string, Logger>();

export function getLogger(scope: string): Logger {
	if (!loggers.has(scope)) {
		loggers.set(scope, new ConsoleLogger(scope));
	}
	const logger = loggers.get(scope);
	if (!logger) {
		throw new Error(`Logger for scope "${scope}" not found`);
	}
	return logger;
}

export function setLogger(scope: string, logger: Logger): void {
	loggers.set(scope, logger);
}

export function clearLoggers(): void {
	loggers.clear();
}
