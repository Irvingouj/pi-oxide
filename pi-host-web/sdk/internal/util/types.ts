/** Type-safe runtime helpers for narrowing `unknown` values. */

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function getString(value: unknown, key: string): string | undefined {
  if (isRecord(value)) {
    const v = value[key];
    return typeof v === "string" ? v : undefined;
  }
  return undefined;
}

export function getNumber(value: unknown, key: string): number | undefined {
  if (isRecord(value)) {
    const v = value[key];
    return typeof v === "number" ? v : undefined;
  }
  return undefined;
}

export function getBoolean(value: unknown, key: string): boolean | undefined {
  if (isRecord(value)) {
    const v = value[key];
    return typeof v === "boolean" ? v : undefined;
  }
  return undefined;
}
