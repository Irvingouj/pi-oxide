/* tslint:disable */
/* eslint-disable */
export const memory: WebAssembly.Memory;
export const createAgent: (a: number, b: number) => [number, number];
export const destroyAgent: (a: number) => [number, number];
export const feedLlmChunk: (a: number, b: number, c: number) => [number, number];
export const followUp: (a: number, b: number, c: number) => [number, number];
export const onLlmDone: (a: number, b: number, c: number) => [number, number];
export const onToolDone: (a: number, b: number, c: number, d: number, e: number) => [number, number];
export const projectContext: (a: number, b: number) => [number, number];
export const prompt: (a: number, b: number, c: number) => [number, number];
export const reset: (a: number) => [number, number];
export const state: (a: number) => [number, number];
export const steer: (a: number, b: number, c: number) => [number, number];
export const __wbindgen_free: (a: number, b: number, c: number) => void;
export const __wbindgen_malloc: (a: number, b: number) => number;
export const __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
export const __wbindgen_externrefs: WebAssembly.Table;
export const __wbindgen_start: () => void;
