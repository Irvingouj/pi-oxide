import assert from "node:assert";
import { describe, it } from "node:test";
import { EventEmitter } from "../sdk/events.ts";

describe("EventEmitter", () => {
  it("emits events to registered handlers", () => {
    const emitter = new EventEmitter();
    const payloads: any[] = [];

    const unsubscribe = emitter.on("text", (payload) => {
      payloads.push(payload);
    });

    emitter.emit("text", "hello");
    emitter.emit("text", "world");

    assert.deepStrictEqual(payloads, ["hello", "world"]);
    unsubscribe();
  });

  it("does not emit to unregistered handlers", () => {
    const emitter = new EventEmitter();
    const payloads: any[] = [];

    emitter.on("text", (payload) => {
      payloads.push(payload);
    });

    const unsubscribe = emitter.on("text", (payload) => {
      payloads.push(payload);
    });

    unsubscribe();
    emitter.emit("text", "hello");

    assert.equal(payloads.length, 1);
    assert.equal(payloads[0], "hello");
  });

  it("off removes a specific handler", () => {
    const emitter = new EventEmitter();
    const payloads: any[] = [];

    const handler = (payload: any) => payloads.push(payload);
    emitter.on("text", handler);
    emitter.off("text", handler);
    emitter.emit("text", "hello");

    assert.equal(payloads.length, 0);
  });

  it("clear removes all handlers", () => {
    const emitter = new EventEmitter();
    const payloads: any[] = [];

    emitter.on("text", (payload) => payloads.push(payload));
    emitter.on("messageStart", (payload) => payloads.push(payload));
    emitter.clear();
    emitter.emit("text", "hello");
    emitter.emit("messageStart", {});

    assert.equal(payloads.length, 0);
  });

  it("supports multiple event types independently", () => {
    const emitter = new EventEmitter();
    const texts: any[] = [];
    const messages: any[] = [];

    emitter.on("text", (payload) => texts.push(payload));
    emitter.on("messageStart", (payload) => messages.push(payload));

    emitter.emit("text", "a");
    emitter.emit("messageStart", { id: "1" });
    emitter.emit("text", "b");

    assert.deepStrictEqual(texts, ["a", "b"]);
    assert.equal(messages.length, 1);
    assert.equal(messages[0].id, "1");
  });

  it("unsubscribe returns a function that removes the handler", () => {
    const emitter = new EventEmitter();
    const payloads: any[] = [];

    const unsubscribe = emitter.on("text", (payload) => {
      payloads.push(payload);
    });

    emitter.emit("text", "before");
    unsubscribe();
    emitter.emit("text", "after");

    assert.deepStrictEqual(payloads, ["before"]);
  });
});
