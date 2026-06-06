import assert from "node:assert";
import { describe, it } from "node:test";
import { SnapshotSerializer } from "../sdk/snapshot.ts";

describe("SnapshotSerializer", () => {
  const serializer = new SnapshotSerializer();

  it("serializes data into AgentSnapshot with version 1", () => {
    const data = { foo: "bar", nested: { arr: [1, 2, 3] } };
    const snapshot = serializer.serialize(data);

    assert.equal(snapshot.version, 1);
    assert.deepStrictEqual(snapshot.data, data);
  });

  it("deserializes valid version 1 snapshot", () => {
    const data = { T: [], A: {}, system_prompt: "test" };
    const snapshot = { version: 1, data };
    const result = serializer.deserialize(snapshot);

    assert.deepStrictEqual(result, data);
  });

  it("rejects unknown version (returns null)", () => {
    const snapshot = { version: 999, data: { foo: "bar" } };
    const result = serializer.deserialize(snapshot);

    assert.strictEqual(result, null);
  });

  it("roundtrip: serialize then deserialize preserves data", () => {
    const data = { messages: [{ role: "user", content: "hi" }], meta: 42 };
    const snapshot = serializer.serialize(data);
    const restored = serializer.deserialize(snapshot);

    assert.deepStrictEqual(restored, data);
  });

  it("handles null data gracefully", () => {
    const snapshot = serializer.serialize(null);
    assert.equal(snapshot.version, 1);
    assert.strictEqual(snapshot.data, null);

    const restored = serializer.deserialize(snapshot);
    assert.strictEqual(restored, null);
  });
});
