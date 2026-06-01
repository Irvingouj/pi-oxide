import assert from "node:assert";
import { describe, it } from "node:test";
import { memoryStore, localStorageStore } from "../../pi-host-web/sdk/stores.ts";
import { SnapshotSerializer } from "../../pi-host-web/sdk/snapshot.ts";

// Mock localStorage for Node environment
const storage = new Map<string, string>();
global.localStorage = {
  getItem(key: string) {
    return storage.get(key) ?? null;
  },
  setItem(key: string, value: string) {
    storage.set(key, value);
  },
  removeItem(key: string) {
    storage.delete(key);
  },
  clear() {
    storage.clear();
  },
  get length() {
    return storage.size;
  },
  key(index: number) {
    return Array.from(storage.keys())[index] ?? null;
  },
} as any;

describe("Store API", () => {
  describe("memoryStore", () => {
    it("TM-13: roundtrip save and load", async () => {
      const store = memoryStore();
      const snapshot = { version: 1, data: { foo: "bar" } };

      await store.saveSession("s1", snapshot);
      const loaded = await store.loadSession("s1");

      assert.deepStrictEqual(loaded, snapshot);
    });

    it("returns null for unknown session", async () => {
      const store = memoryStore();
      const loaded = await store.loadSession("unknown");

      assert.strictEqual(loaded, null);
    });

    it("overwrites existing session", async () => {
      const store = memoryStore();
      await store.saveSession("s1", { version: 1, data: { v: 1 } });
      await store.saveSession("s1", { version: 1, data: { v: 2 } });

      const loaded = await store.loadSession("s1");
      assert.deepStrictEqual(loaded, { version: 1, data: { v: 2 } });
    });

    it("stores multiple sessions independently", async () => {
      const store = memoryStore();
      await store.saveSession("s1", { version: 1, data: { id: "s1" } });
      await store.saveSession("s2", { version: 1, data: { id: "s2" } });

      const loaded1 = await store.loadSession("s1");
      const loaded2 = await store.loadSession("s2");

      assert.deepStrictEqual(loaded1, { version: 1, data: { id: "s1" } });
      assert.deepStrictEqual(loaded2, { version: 1, data: { id: "s2" } });
    });
  });

  describe("localStorageStore", () => {
    it("TM-13: roundtrip save and load", async () => {
      const store = localStorageStore();
      const snapshot = { version: 1, data: { foo: "bar" } };

      await store.saveSession("s1", snapshot);
      const loaded = await store.loadSession("s1");

      assert.deepStrictEqual(loaded, snapshot);
    });

    it("returns null for unknown session", async () => {
      const store = localStorageStore();
      const loaded = await store.loadSession("unknown");

      assert.strictEqual(loaded, null);
    });

    it("overwrites existing session", async () => {
      const store = localStorageStore();
      await store.saveSession("s1", { version: 1, data: { v: 1 } });
      await store.saveSession("s1", { version: 1, data: { v: 2 } });

      const loaded = await store.loadSession("s1");
      assert.deepStrictEqual(loaded, { version: 1, data: { v: 2 } });
    });
  });

  describe("SnapshotSerializer integration", () => {
    it("TM-20: SnapshotSerializer roundtrip with memoryStore", async () => {
      const serializer = new SnapshotSerializer();
      const store = memoryStore();
      const data = { messages: [{ role: "user", content: "hi" }] };

      const snapshot = serializer.serialize(data);
      await store.saveSession("s1", snapshot);
      const loaded = await store.loadSession("s1");

      assert.equal(loaded!.version, 1);
      const restored = serializer.deserialize(loaded!);
      assert.deepStrictEqual(restored, data);
    });

    it("TM-20: rejects unknown version from store", async () => {
      const store = memoryStore();
      const badSnapshot = { version: 999, data: { foo: "bar" } };

      await store.saveSession("s1", badSnapshot);
      const loaded = await store.loadSession("s1");

      const serializer = new SnapshotSerializer();
      const restored = serializer.deserialize(loaded!);

      assert.strictEqual(restored, null);
    });
  });
});
