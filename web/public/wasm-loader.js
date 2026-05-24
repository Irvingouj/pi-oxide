/**
 * Browser WASM loader for pi-host-web.
 *
 * Loads the WASM module built by wasm-bindgen and provides the same
 * API as the Node CJS wrapper, but using fetch() + WebAssembly.
 */

let wasm;

const cachedTextEncoder = new TextEncoder();
const cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
  if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true ||
      (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
    cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
  }
  return cachedDataViewMemory0;
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
  if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
    cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
  }
  return cachedUint8ArrayMemory0;
}

let WASM_VECTOR_LEN = 0;

function passStringToWasm0(arg, malloc, realloc) {
  if (realloc === undefined) {
    const buf = cachedTextEncoder.encode(arg);
    const ptr = malloc(buf.length, 1) >>> 0;
    getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
    WASM_VECTOR_LEN = buf.length;
    return ptr;
  }
  let len = arg.length;
  let ptr = malloc(len, 1) >>> 0;
  const mem = getUint8ArrayMemory0();
  let offset = 0;
  for (; offset < len; offset++) {
    const code = arg.charCodeAt(offset);
    if (code > 0x7F) break;
    mem[ptr + offset] = code;
  }
  if (offset !== len) {
    if (offset !== 0) arg = arg.slice(offset);
    ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
    const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
    const ret = cachedTextEncoder.encodeInto(arg, view);
    offset += ret.written;
    ptr = realloc(ptr, len, offset, 1) >>> 0;
  }
  WASM_VECTOR_LEN = offset;
  return ptr;
}

function getStringFromWasm0(ptr, len) {
  return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

// WASM imports expected by the module
function __wbg_get_imports() {
  return {
    "./pi_host_web_bg.js": {
      __proto__: null,
      __wbg_error_a6fa202b58aa1cd3: function(arg0, arg1) {
        try {
          console.error(getStringFromWasm0(arg0, arg1));
        } finally {
          wasm.__wbindgen_free(arg0, arg1, 1);
        }
      },
      __wbg_new_227d7c05414eb861: function() {
        return new Error();
      },
      __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
        const ret = arg1.stack;
        const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
        getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
      },
      __wbindgen_init_externref_table: function() {
        const table = wasm.__wbindgen_externrefs;
        const offset = table.grow(4);
        table.set(0, undefined);
        table.set(offset + 0, undefined);
        table.set(offset + 1, null);
        table.set(offset + 2, true);
        table.set(offset + 3, false);
      },
    },
  };
}

// --- Public API (mirrors wasmBinding.ts) ---

async function init(wasmUrl) {
  const response = await fetch(wasmUrl);
  const bytes = await response.arrayBuffer();
  const { instance } = await WebAssembly.instantiate(bytes, __wbg_get_imports());
  wasm = instance.exports;
  if (wasm.__wbindgen_start) wasm.__wbindgen_start();
  return api;
}

const api = {
  init,

  createAgent(options_json) {
    const ptr0 = passStringToWasm0(options_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const ret = wasm.createAgent(ptr0, WASM_VECTOR_LEN);
    const s = getStringFromWasm0(ret[0], ret[1]);
    wasm.__wbindgen_free(ret[0], ret[1], 1);
    return s;
  },

  destroyAgent(handle) {
    const ret = wasm.destroyAgent(handle);
    const s = getStringFromWasm0(ret[0], ret[1]);
    wasm.__wbindgen_free(ret[0], ret[1], 1);
    return s;
  },

  prompt(handle, prompt_json) {
    const ptr0 = passStringToWasm0(prompt_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const ret = wasm.prompt(handle, ptr0, WASM_VECTOR_LEN);
    const s = getStringFromWasm0(ret[0], ret[1]);
    wasm.__wbindgen_free(ret[0], ret[1], 1);
    return s;
  },

  feedLlmChunk(handle, chunk_json) {
    const ptr0 = passStringToWasm0(chunk_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const ret = wasm.feedLlmChunk(handle, ptr0, WASM_VECTOR_LEN);
    const s = getStringFromWasm0(ret[0], ret[1]);
    wasm.__wbindgen_free(ret[0], ret[1], 1);
    return s;
  },

  onLlmDone(handle, result_json) {
    const ptr0 = passStringToWasm0(result_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const ret = wasm.onLlmDone(handle, ptr0, WASM_VECTOR_LEN);
    const s = getStringFromWasm0(ret[0], ret[1]);
    wasm.__wbindgen_free(ret[0], ret[1], 1);
    return s;
  },

  onToolDone(handle, tool_call_id, result_json) {
    const ptr0 = passStringToWasm0(tool_call_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passStringToWasm0(result_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const ret = wasm.onToolDone(handle, ptr0, len0, ptr1, WASM_VECTOR_LEN);
    const s = getStringFromWasm0(ret[0], ret[1]);
    wasm.__wbindgen_free(ret[0], ret[1], 1);
    return s;
  },

  onToolStarted(handle, tool_call_id) {
    const ptr0 = passStringToWasm0(tool_call_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const ret = wasm.onToolStarted(handle, ptr0, WASM_VECTOR_LEN);
    const s = getStringFromWasm0(ret[0], ret[1]);
    wasm.__wbindgen_free(ret[0], ret[1], 1);
    return s;
  },

  projectContext(input_json) {
    const ptr0 = passStringToWasm0(input_json, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const ret = wasm.projectContext(ptr0, WASM_VECTOR_LEN);
    const s = getStringFromWasm0(ret[0], ret[1]);
    wasm.__wbindgen_free(ret[0], ret[1], 1);
    return s;
  },
};

export default api;
