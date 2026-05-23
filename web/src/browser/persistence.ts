/**
 * IndexedDB persistence for browser sessions and tool artifacts.
 */

const DB_NAME = "pi-oxide-browser-agent";
const DB_VERSION = 1;

const sessionId = "browser-session-" + Date.now();
let messageSeq = 0;

function openDB(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains("session")) db.createObjectStore("session", { keyPath: "id" });
      if (!db.objectStoreNames.contains("artifacts")) db.createObjectStore("artifacts", { keyPath: "id" });
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function dbPut(storeName: string, value: unknown): Promise<void> {
  return openDB().then(
    (db) =>
      new Promise((resolve, reject) => {
        const tx = db.transaction(storeName, "readwrite");
        tx.objectStore(storeName).put(value);
        tx.oncomplete = () => resolve();
        tx.onerror = () => reject(tx.error);
      }),
  );
}

function dbGetAll(storeName: string): Promise<unknown[]> {
  return openDB().then(
    (db) =>
      new Promise((resolve, reject) => {
        const tx = db.transaction(storeName, "readonly");
        const req = tx.objectStore(storeName).getAll();
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
      }),
  );
}

export async function persistMessage(role: string, content: string): Promise<void> {
  await dbPut("session", {
    id: `${sessionId}-${messageSeq++}`,
    sessionId,
    role,
    content,
    timestamp: Date.now(),
  });
}

export async function persistArtifact(
  id: string,
  toolName: string,
  content: string,
): Promise<void> {
  await dbPut("artifacts", { id, toolName, content, storedAt: Date.now() });
}

export async function loadSession(): Promise<unknown[]> {
  const all = (await dbGetAll("session")) as Array<{ sessionId: string; timestamp: number }>;
  return all
    .filter((e) => e.sessionId === sessionId)
    .sort((a, b) => a.timestamp - b.timestamp);
}
