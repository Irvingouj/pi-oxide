// SnapshotSerializer: converts between internal PersistData and public AgentSnapshot.
// Validates version and rejects unknown versions (returns null).

import type { AgentSnapshot } from "./types.ts";

const CURRENT_VERSION = 1;

export class SnapshotSerializer {
  serialize(data: unknown): AgentSnapshot {
    return {
      version: CURRENT_VERSION,
      data,
    };
  }

  deserialize(snapshot: AgentSnapshot): unknown | null {
    if (snapshot.version !== CURRENT_VERSION) {
      console.warn(
        `Snapshot version mismatch: expected ${CURRENT_VERSION}, got ${snapshot.version}. Starting fresh.`,
      );
      return null;
    }
    return snapshot.data;
  }
}
