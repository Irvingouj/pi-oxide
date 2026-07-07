# pi-record-server

HTTP recording/replay proxy server for LLM API streams. Capture real DeepSeek
SSE streams and replay them for deterministic TUI tests — no API key needed
during replay, byte-for-byte identical to the original.

## Quick Start

### Record a session

```sh
# Start the recording proxy (blocks until Ctrl+C)
cargo run -p pi-record-server -- record --target https://api.deepseek.com

# In another terminal, run the TUI pointed at the proxy:
PI_BASE_URL=http://localhost:9999 PI_PROVIDER=deepseek PI_API_KEY=sk-... pio

# Press Ctrl+C to stop recording → cassette.json saved
```

### Replay for tests

```sh
# Start the replay server
cargo run -p pi-record-server -- replay --cassette cassette.json

# Run the TUI against replay (no real API key needed):
PI_BASE_URL=http://localhost:9999 PI_PROVIDER=deepseek PI_API_KEY=any pio
```

### With chunk delay (simulate network latency)

```sh
pi-record-server replay --cassette cassette.json --delay-ms 50
```

## Cassette Format

```json
{
  "version": 2,
  "target": "https://api.deepseek.com",
  "entries": [
    {
      "request": {
        "method": "POST",
        "url": "/v1/chat/completions",
        "body_json": "{\"model\":\"deepseek-chat\",...}"
      },
      "response": {
        "status": 200,
        "body_base64": "ZGF0YToge..."
      }
    }
  ]
}
```

The response body is raw SSE bytes, base64-encoded. This preserves exact
byte-level fidelity — whitespace, newlines, chunk boundaries, everything.

## CLI Reference

```
pi-record-server <COMMAND>

Commands:
  record   Proxy requests to a real API and capture SSE streams
  replay   Serve recorded SSE streams from a cassette file
  help     Print this message or the help of the given subcommand(s)

Record options:
  --target <URL>     Upstream API base URL [default: https://api.deepseek.com]
  --port <PORT>      Port to listen on [default: 9999]
  --output <PATH>    Output cassette file path [default: cassette.json]

Replay options:
  --cassette <PATH>  Cassette file to replay from [default: cassette.json]
  --port <PORT>      Port to listen on [default: 9999]
  --delay-ms <MS>    Delay between SSE chunks in ms [default: 0]
```

## Architecture

- **Record mode**: Axum HTTP server proxies `POST /v1/chat/completions` to the
  upstream API. Uses an mpsc channel to tee the SSE response stream — bytes flow
  to both the client and a capture buffer. On SIGINT/SIGTERM, the cassette is
  serialized to JSON.

- **Replay mode**: Loads a cassette file and serves entries sequentially via
  `POST /v1/chat/completions`. Returns 503 when the cassette is exhausted.
  Optional `--delay-ms` splits responses into 256-byte chunks with inter-chunk
  delays to simulate real network streaming.

- **No pi-core dependency**: The cassette records raw HTTP, not typed
  `pi_core::LlmChunk` values. This means the recording is provider-agnostic and
  doesn't require recompilation of the TUI with feature flags.
