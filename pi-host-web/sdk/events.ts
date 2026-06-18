// Lightweight, type-safe EventEmitter (not Node's).
// Maps event names to typed handlers using the SDK's AgentEventName union.

import type { AgentEventHandler, AgentEventName } from "./types.ts";

type AnyHandler = (payload: unknown) => void;

export class EventEmitter {
	private handlers: Partial<Record<AgentEventName, Set<AnyHandler>>> = {};

	on<E extends AgentEventName>(
		event: E,
		handler: AgentEventHandler<E>,
	): () => void {
		if (!this.handlers[event]) {
			this.handlers[event] = new Set();
		}
		const set = this.handlers[event] as Set<AnyHandler>;
		set.add(handler as AnyHandler);

		return () => {
			set.delete(handler as AnyHandler);
			if (set.size === 0) {
				delete this.handlers[event];
			}
		};
	}

	off<E extends AgentEventName>(event: E, handler: AgentEventHandler<E>): void {
		const set = this.handlers[event];
		if (!set) return;
		set.delete(handler as AnyHandler);
		if (set.size === 0) {
			delete this.handlers[event];
		}
	}

	emit<E extends AgentEventName>(
		event: E,
		payload: Parameters<AgentEventHandler<E>>[0],
	): void {
		const set = this.handlers[event];
		if (!set) return;
		for (const handler of set) {
			handler(payload);
		}
	}

	clear(): void {
		this.handlers = {};
	}
}
