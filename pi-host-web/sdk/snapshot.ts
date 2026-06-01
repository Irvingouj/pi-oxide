// SnapshotSerializer: converts between internal PersistData and public AgentSnapshot.
// Validates version and rejects unknown versions (returns null).

import type { AgentSnapshot } from "./types.ts";
import { getLogger } from "./internal/logger.ts";

const CURRENT_VERSION = 1;

export class SnapshotSerializer {
  private logger = getLogger("snapshot");

  serialize(data: unknown): AgentSnapshot {
    return {
      version: CURRENT_VERSION,
      data,
    };
  }

  deserialize(snapshot: AgentSnapshot): unknown | null {
    if (snapshot.version !== CURRENT_VERSION) {
      this.logger.warn("Snapshot version mismatch, starting fresh", {
        expected: CURRENT_VERSION,
        got: snapshot.version,
      });
      return null;
    }
    return snapshot.data;
  }
}
