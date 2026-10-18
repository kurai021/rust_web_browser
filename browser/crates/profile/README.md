# Local browser profile

Trusted shell persistence; this crate has no network, DOM or VM dependency.
The GUI selects `$XDG_CONFIG_HOME/navegador-rust` (otherwise
`~/.config/navegador-rust`) unless `--profile-dir` is given. Headless tests use
memory unless an explicit directory is supplied.

* `config.toml`: homepage, search URL with `{query}`, session restore, download
  directory/ask preference, appearance and origin-scoped JavaScript choices.
  Third-party cookies are blocked by default. DoH is opt-in to Quad9 HTTPS and
  takes effect after restart. Adblock preferences are consumed in Phase 10.
* `browser.json`: visits, bookmarks, session URLs/back-forward stacks/scroll,
  per-origin zoom and download metadata. Bytes remain in the chosen download
  directory. Interrupted transfers restore paused and require explicit resume.
* `web-data.json`, `cookies.json`, `hsts.json`: existing origin Web API, cookie
  and HSTS stores. SessionStorage is per-tab and memory-only.

The in-memory shell model submits coalesced snapshots to a background writer.
Atomic replacement and 0600 files in a 0700 profile directory protect browser
history/settings; ordinary UI edits do not perform disk writes on the UI thread.
History/bookmark/session URLs omit credentials. Opening validates capped input
and reports malformed data instead of silently overwriting it.

Caps: 16 MiB per browser/config file, 50 tabs, 200 back/forward entries per tab,
10,000 visits, 5,000 bookmarks, 1,000 download records and 1,000 zoom settings.
Sessions restore URLs into fresh isolated realms; this is not a live JS heap or
back-forward-cache checkpoint. Only the selected restored page loads immediately.

`cargo test -p profile` verifies persistence, file modes, URL cleaning, local
autocomplete, scoped deletion and search-query escaping.
