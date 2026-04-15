# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

**selfsync** — self-hosted Chrome sync solution. A Cargo workspace with two crates:

- **selfsync-payload** — LD_PRELOAD shared library (cdylib) that injects into Google Chrome. Hooks `__libc_start_main` to redirect sync traffic to a local server via `--sync-url`, identifies users by `cache_guid → email` mapping from Chrome Preferences.
- **selfsync-server** — Chrome sync server implementation (TODO).

## Build & Test

```bash
cargo build --release                        # Build all
cargo build --release -p selfsync-payload    # Payload .so only
cargo build --release -p selfsync-server     # Server only
cargo check                                  # Type check workspace
cargo clippy                                 # Lint check
```

Run payload with Chrome:
```bash
LD_PRELOAD=./target/release/libselfsync_payload.so google-chrome-stable
```

## Project Structure

```
selfsync/
├── crates/
│   ├── payload/         # LD_PRELOAD .so (cdylib)
│   │   └── src/
│   │       ├── lib.rs       # __libc_start_main hook, argv injection
│   │       ├── mapping.rs   # cache_guid → email mapping from Preferences
│   │       └── proxy.rs     # HTTP proxy, adds X-Sync-User-Email header
│   └── sync-server/     # Chrome sync server (TODO)
│       └── src/
│           └── main.rs
└── docs/
    └── account-mapping.md   # Mapping algorithm documentation
```

## Payload Architecture

- **lib.rs** — `__libc_start_main` hook. Detects Chrome browser process (checks `argv[0]` ends with `/chrome`; skips `--type=` subprocesses and non-Chrome binaries). Reads `--user-data-dir` from argv. Injects `--sync-url` pointing to embedded proxy. Starts proxy thread.

- **mapping.rs** — Builds `cache_guid -> email` mapping by scanning all Chrome profile directories. Algorithm: `account_info[].gaia` → `base64(sha256(gaia_id))` → match key in `sync.transport_data_per_account` → extract `sync.cache_guid`. See `docs/account-mapping.md`.

- **proxy.rs** — HTTP proxy on dynamic port (OS-assigned). Extracts `client_id` from URL query, looks up email, adds `X-Sync-User-Email` header, forwards to upstream.

## Key Chromium Source References

Relevant paths in `~/modous/chromium/src/`:

- `components/sync/base/sync_util.cc` — `GetSyncServiceURL()`, reads `--sync-url` (no branding guard)
- `components/sync/engine/sync_manager_impl.cc` — `MakeConnectionURL()`, appends `/command/` path
- `components/sync/engine/net/url_translator.cc` — `AppendSyncQueryString()`, adds `client` and `client_id` params
- `components/sync/engine/net/http_bridge.cc` — `MakeAsynchronousPost()`, HTTP request construction
- `components/sync/protocol/sync.proto` — `ClientToServerMessage`, `ClientToServerResponse`
- `components/sync/engine/loopback_server/loopback_server.cc` — Reference sync server implementation

## Important Constraints

- `LD_PRELOAD` affects ALL child processes. `is_chrome_browser_process()` must verify `argv[0]` to skip non-Chrome binaries.
- Chrome runs multiple profiles in a single browser process. Proxy differentiates via `client_id` (cache_guid).
- Chrome's BoringSSL is statically linked — cannot hook SSL functions via LD_PRELOAD.
- Proxy uses HTTP for local endpoint; Chrome's `--sync-url` accepts `http://`.
- `GURL::ReplaceComponents` with `SetPathStr` preserves existing query parameters.
