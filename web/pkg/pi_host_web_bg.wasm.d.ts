/* tslint:disable */
/* eslint-disable */
export const memory: WebAssembly.Memory;
export const createAgent: (a: any) => any;
export const destroyAgent: (a: number) => any;
export const drainTraceLog: () => [number, number];
export const feedLlmChunk: (a: number, b: any) => any;
export const followUp: (a: number, b: any) => any;
export const onLlmDone: (a: number, b: any) => any;
export const onToolCancelled: (a: number, b: number, c: number, d: any) => any;
export const onToolDone: (a: number, b: number, c: number, d: any) => any;
export const onToolStarted: (a: number, b: number, c: number) => any;
export const onToolUpdate: (a: number, b: any) => any;
export const projectContext: (a: any) => any;
export const prompt: (a: number, b: any) => any;
export const reset: (a: number) => any;
export const state: (a: number) => any;
export const steer: (a: number, b: any) => any;
export const __wbindgen_malloc: (a: number, b: number) => number;
export const __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
export const __wbindgen_free: (a: number, b: number, c: number) => void;
export const __wbindgen_exn_store: (a: number) => void;
export const __externref_table_alloc: () => number;
export const __wbindgen_externrefs: WebAssembly.Table;
export const __externref_drop_slice: (a: number, b: number) => void;
export const __wbindgen_start: () => void;
