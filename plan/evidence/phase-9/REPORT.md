# Phase 9 — browser shell, sessions and downloads

Implementation against Phase 8 baseline `4b30112`, Linux aarch64 / Apple M2 Pro,
stable Rust 1.99. The sealed roadmap and engine boundaries remain the reference.

The graphical shell now owns independent tabs with create/close/reorder/duplicate,
keyboard navigation, capped favicons and URL-based lazy session restore. Native
local managers provide history, bookmarks with Netscape HTML import/export,
preferences and downloads. Omnibox suggestions are local, find uses rendered
Unicode text and shaped glyph clusters, zoom is per-site (80–200%), and basic A4
PDF export uses the owned display list. Read-only Elements/Console and a bounded
in-memory Network log are available through F12.

A background profile writer coalesces private atomic snapshots. The window-wide
download actor streams to disk, limits active transfers to five, validates
Range/If-Range resumption, announces restarts, cleans cancelled partials and
persists metadata for explicit resumption. Save paths belong to native UI;
fetch→Blob→object URL activation hands only data to that UI. Downloaded content is
not executed or auto-opened. Linux output has no execute bits and receives
best-effort quarantine/origin attributes.

Evidence:

* [verification.md](verification.md): gates, conformance and reproduction.
* [performance.md](performance.md): startup, five-tab memory/idle CPU and fixes.
* [gui.md](gui.md): GPU/software, managers, find, download and PDF observations.
* [testing-scope.md](testing-scope.md): defensive purpose and resource boundaries.
* [fuzz.md](fuzz.md): bounded sanitized page/runtime run.
* [summary.json](summary.json): machine-readable gate and conformance summary.
* `wpt/` and `test262/`: machine-readable conformance reports.

This phase remains experimental. Session restoration recreates URL/history/scroll
state in fresh realms; it does not serialize live JS heaps or implement a BFCache.
PDF export is raster A4, capped at 50 pages / 64 MiB. Developer tools are bounded,
local read-only inspection. Adblock/list/update integration is Phase 10; the
compatibility thresholds and final hardening that precede a beta remain Phase 11.
