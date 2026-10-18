# Phase 9 GUI observations

The loopback workshop, async board, Web API demo, reading sample and JSON document
were exercised in one profile. Release used the Apple M2 Pro Vulkan adapter;
debug software rendering was checked earlier. Display-list parity passed
separately. Captures contain only the test browser, with its compositor opacity
overridden during recording.

* [phase9-tabs-zoom.png](phase9-tabs-zoom.png): dragging the async board to the final
  slot preserves its state. Ctrl++ raises per-site zoom to 121%; Ctrl+0 resets it.
* [phase9-gpu-find.png](phase9-gpu-find.png): `café` and `CAFÉ` both match, with
  current/all-match highlights and native navigation.
* [phase9-download-paused.png](phase9-download-paused.png): a real 100 MiB fixture
  paused around 5 MiB, then resumed to completion. A separate GUI transfer was
  cancelled and its partial removed.
* Fetch→Blob→object URL→native save produced a 125-byte JSON report without page
  filesystem access. Completed files did not open automatically.
* Ctrl+P saved a 572815-byte PDF from the reading page. `pdfinfo` verified two
  A4 pages, PDF 1.7, no encryption, forms or embedded JavaScript.
* [phase9-bookmarks.png](phase9-bookmarks.png) and
  [phase9-history.png](phase9-history.png): native filtering, range/site deletion,
  bookmark bar and Netscape export. GUI export created the selected file; inert
  import and round-trip are covered by tests.
* [phase9-preferences.png](phase9-preferences.png) and
  [phase9-narrow-dark.png](phase9-narrow-dark.png): native preferences at wide and
  620px widths; appearance changes persist. DNS explicitly requires restart.
  Phase 10 list controls are labelled as the next integration step.
* [phase9-network.png](phase9-network.png): bounded local request inspection, no
  remote debugging service. DOM/styles and console use trusted native snapshots.

Closing and reopening the same temporary profile restored five slots, loading
the selected page first. Selecting dormant tabs loaded their saved URLs.
Network-error content is generated locally with retry/back advice; certificate
metadata is displayed from strictly verified TLS responses.

GUI review corrected Unicode canonical-equivalence matching and a stale
pause/retry message after resumed completion. The machine's existing XCompose
include warning was left untouched; no compositor or OS configuration was edited.
