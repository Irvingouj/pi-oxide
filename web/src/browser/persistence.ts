/**
 * Session persistence backend for browser agents.
 *
 * Replaces the old flat message/artifact storage with a typed SessionState
 * tree that mirrors the Rust core session model.
 */

import type { SessionState } from "../wasmBinding.ts";

export interface SessionBackend {
  save(sessionId: string, state: SessionState): Promise<void>;
  load(sessionId: string): Promise<SessionState | null>;
}

const DB_NAME = "pi-oxide-browser-agent";
const DB_VERSION = 2;
const STORE_NAME = "sessions";

export class IndexedDBSessionBackend implements SessionBackend {
  private dbPromise: Promise<IDBDatabase>;

  constructor() {
    this.dbPromise = this.openDB();
  }

  private openDB(): Promise<IDBDatabase> {
    return new Promise((resolve, reject) => {
      const req = indexedDB.open(DB_NAME, DB_VERSION);
      req.onupgradeneeded = () => {
        const db = req.result;
        // Drop legacy v1 stores
        if (db.objectStoreNames.contains("session")) {
          db.deleteObjectStore("session");
        }
        if (db.objectStoreNames.contains("artifacts")) {
          db.deleteObjectStore("artifacts");
        }
        if (!db.objectStoreNames.contains(STORE_NAME)) {
          db.createObjectStore(STORE_NAME, { keyPath: "sessionId" });
        }
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
  }

  async save(sessionId: string, state: SessionState): Promise<void> {
    const db = await this.dbPromise;
    return new Promise((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readwrite");
      tx.objectStore(STORE_NAME).put({
        sessionId,
        state,
        updatedAt: Date.now(),
      });
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  }

  async load(sessionId: string): Promise<SessionState | null> {
    const db = await this.dbPromise;
    return new Promise((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readonly");
      const req = tx.objectStore(STORE_NAME).get(sessionId);
      req.onsuccess = () => {
        const result = req.result as { state: SessionState } | undefined;
        resolve(result?.state ?? null);
      };
      req.onerror = () => reject(req.error);
    });
  }
}
