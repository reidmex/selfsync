# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

LD_PRELOAD shared library (Rust cdylib) that injects into Google Chrome to intercept Chrome Sync requests and tag them with user email. It hooks `__libc_start_main` to inject `--sync-url` pointing to an embedded HTTP proxy, which adds `X-Sync-User-Email` header before forwarding to Google's sync servers.

## Build & Test

```bash
cargo build --release          # Output: target/release/liblzc_chrome_sync.so
cargo check                    # Type check without building
```

Run with Chrome:
```bash
LD_PRELOAD=./target/release/liblzc_chrome_sync.so google-chrome-stable
```

No test suite yet. Manual verification by checking stderr log output.

## Architecture

Three modules in a single cdylib .so:

- **lib.rs** — `__libc_start_main` hook. Detects Chrome browser process (checks `argv[0]` ends with `/chrome`; skips `--type=` subprocesses and non-Chrome binaries like `grep`, `readlink`). Reads `--user-data-dir` from argv (defaults to `~/.config/google-chrome`). Injects `--sync-url=http://127.0.0.1:18643/chrome-sync` into argv. Starts proxy thread.

- **mapping.rs** — Builds `cache_guid -> email` mapping table by scanning all Chrome profile directories. The mapping algorithm: read `account_info[].gaia` and `account_info[].email` from each profile's `Preferences` JSON, compute `base64(sha256(gaia_id))` to match keys in `sync.transport_data_per_account`, extract `sync.cache_guid` from the matched entry. See `docs/account-mapping.md` for full details.

- **proxy.rs** — HTTP proxy on `127.0.0.1:18643` using `tiny_http`. Receives Chrome sync requests, extracts `client_id` query parameter (which is the `cache_guid`), looks up email in the mapping table, adds `X-Sync-User-Email` header, forwards via HTTPS (`reqwest`) to `https://clients4.google.com/chrome-sync`.

## Key Chromium Source References

When working with this project, relevant Chromium source paths (in `~/modous/chromium/src/`):

- `components/sync/base/sync_util.cc` — `GetSyncServiceURL()`, reads `--sync-url` flag (no branding/official-build guard)
- `components/sync/engine/sync_manager_impl.cc` — `MakeConnectionURL()`, appends `/command/` to sync URL path
- `components/sync/engine/net/url_translator.cc` — `AppendSyncQueryString()`, adds `client` and `client_id` query params
- `components/sync/engine/net/http_bridge.cc` — `MakeAsynchronousPost()`, where HTTP headers are set and request is sent
- `google_apis/gaia/gaia_urls.cc` — GAIA URL configuration, all overridable via command-line switches
- `google_apis/gaia/gaia_switches.cc` — `--gaia-url`, `--lso-url`, `--google-apis-url` switch definitions
- `chrome/browser/ui/startup/bad_flags_prompt.cc` — `kBadFlags` list (only `--gaia-url` shows warning; `--sync-url` is not in this list)

## Important Constraints

- `LD_PRELOAD` affects ALL child processes (Chrome forks `grep`, `readlink`, `mkdir` etc via startup scripts). The `is_chrome_browser_process()` check in `lib.rs` is critical — it must verify `argv[0]` before doing anything.
- Chrome runs multiple profiles in a single browser process. The proxy must handle requests from different profiles via `client_id` differentiation.
- Chrome's BoringSSL is statically linked — cannot hook SSL functions via LD_PRELOAD.
- The proxy uses HTTP (not HTTPS) for the local sync endpoint because Chrome's `--sync-url` accepts `http://` and this avoids TLS complexity for localhost.
- The `GURL::ReplaceComponents` with `SetPathStr` only preserves query parameters if they were not explicitly set in the replacement — verified in Chromium's `url_canon_internal.cc`.
