# Phase 9 browser workshop

Serve through `cargo run -p web_api --example demo_server -- 8099`, then open
`http://127.0.0.1:8099/browser/`. This is a synthetic loopback fixture for the
owned browser, with no external assets or network targets.

Use `/browser/article`, `/modern/`, `/` and `/api/data` for a five-tab session.
The workshop exercises page-local state, find, per-site zoom, bookmarks/history,
preferences and basic PDF export. Its 100 MiB endpoint implements validated
Range/If-Range resumption and generates only a 64 KiB chunk at a time. The JSON
button exercises fetch → Blob → object URL → native save, without page filesystem
access. Use a temporary profile/download directory for acceptance recordings.
