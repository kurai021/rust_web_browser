# Origin-scoped Web APIs

`PageRuntime` extends the existing `DomRuntime`/owned JS interpreter. VM objects
and DOM handles remain thread-local. Bounded `WebCommand`/`WebCompletion` messages
connect the realm to Tokio HTTP/socket tasks, isolated dedicated-worker VMs and
trusted shell permission/dialog adapters. Navigation and shutdown cancel services.
Workers exchange bounded data-only graphs, including cycles; they receive no DOM,
profile, filesystem or other-context pointers.

Phase 9 connects ordinary link activation and realm-local object URLs to the
native download UI. `URL.createObjectURL` / `revokeObjectURL` use capped owned Blob
buffers (32 URLs / 32 MiB); activation captures data before revocation. Native save
paths are selected by the shell, never by page strings. Cancelled clicks and
cross-origin `download` attributes retain navigation policy. Idle page/dedicated
workers sleep and collect after actual work instead of polling GC continuously.

## Implemented subsets

* fetch and async XMLHttpRequest share strict origin/CORS/preflight processing,
  credentials, header filtering, body/redirect/deadline caps and cancellation.
  Policy is checked before every redirect connection. Request/Response/Headers,
  text/JSON/blob bodies, FormData multipart, URL/URLSearchParams, Blob and basic
  FileReader operate on owned capped buffers. XHR supports text/JSON/blob,
  MIME/charset override, ready-state/progress/load/error/timeout/abort events and
  independent operation IDs when an instance is reused.
* Location and same-document history implement push/replace/go and
  popstate/hashchange. State uses data-only cloning rather than sharing heap IDs.
  Full document navigation requires a trusted gesture; blocked actions surface a
  notice. The shell supplies browser-wide history and URL-based session restore.
* Profile-backed localStorage, IndexedDB and explicit saved permissions use fixed
  profile paths, capped JSON, atomic replacement and private Unix files. Session
  storage stays in memory, isolated by window/session plus origin, and is cleared
  on close. Standalone runtimes default to memory; the GUI opens its configured
  persistent profile (`--profile-dir` overrides the platform default).
  `navigator.storage.estimate()` reports real retained origin data and a fixed
  quota; `persist()` returns false when there is no persistent profile backend.
* document.cookie shares the net jar used by fetch/XHR and subresources. Domain,
  path, public suffix, Secure, HttpOnly, expiry and cookie prefixes are checked.
   Third-party cookies are blocked by default; the native preference can allow
   eligible SameSite=None/Secure cookies. Only persistent, unexpired cookies are saved;
  HttpOnly values are never returned to document.cookie or response headers.
* IndexedDB supports version upgrades, object stores, inline/out-of-line keys,
  auto-increment, get/put/add/delete/clear/getAll/count, basic unique/non-unique
  indexes, deferred request events and atomic transactions. Overlapping local
  transactions serialize. A conflicting cross-context commit aborts through a
  generation check. Error preventDefault can preserve the rest of a transaction;
  aborted work never changes persisted records.
* Dedicated Worker supports data-only messages, events, close/terminate and basic
  function timers. WebSocket supports ws/wss, Origin/subprotocol validation,
  bounded text/blob messages, bufferedAmount and close. EventSource streams named
  or default events, multiline data, last-event IDs, CR/LF/BOM framing and bounded
  retry after an opened stream ends.
* MutationObserver journals actual tree/attribute/character mutations, batches
  callbacks and exposes takeRecords/disconnect. ResizeObserver and
  IntersectionObserver consume real layout boxes, scroll clips and viewport
  updates. Delivery is bounded and does not perform layout on the UI thread.
* Canvas2D supports rect fill/clear/stroke, basic paths/curves/arcs, transforms,
  save/restore, drawImage, system-shaped fillText/measureText and bounded pixel
  read/write. Immutable RGBA snapshots use the existing retained image painter
  for GPU/software. Cross-origin images taint the canvas; readback then fails
  until dimensions reset it. Canvas resizing also resets drawing state.
* Origin-labelled native dialogs block only that realm. Permission chrome offers
  Once (current page), Always and Never; `about:permissions` revokes grants.
  Clipboard requires a secure context and gesture and uses Linux Wayland desktop
  adapters. Notifications use the desktop service after permission. Fullscreen
  requires approval and Escape exits. Camera/microphone report denied; approved
  geolocation reports unavailable without a local provider, with no hidden call.
  Gestureless popups are blocked; approved popups open isolated windows.

## Bounds

| Resource | Bound |
|---|---|
| Request/response/blob/form payload | 8 MiB |
| Pending requests / service tasks | 64 |
| Host objects / queued commands | 50,000 / 1,024; commands also capped at 32 MiB |
| Local and session storage | 5 MiB UTF-16 bytes per origin and store |
| IndexedDB / profile file | 32 MiB per origin / 64 MiB total |
| Active IDB transactions / stores / indexes | 16 / 256 per DB / 64 per store |
| Dedicated workers | 4, each with a 64 MiB interpreter budget |
| Socket message and buffered sends | 1 MiB |
| Observers / targets / records | 256 / 1,024 per observer / 1,024 per batch |
| Mutation journal / observer records | 16 MiB / 32 MiB per page |
| Canvas | 64 canvases, 4,096 per dimension, 16 MiB per bitmap, 32 MiB total |
| Canvas paths / saved states / text | 1,024 points / 128 states / 512 characters |
| Pixel readback | 262,144 channel values per call |
| Deferred checkpoint | 2,048 jobs, plus the existing VM time/memory/call limits |

The VM conservatively accounts host-owned payloads as well as JS/DOM data; quotas
are allocator/work limits, not a guarantee about process, sanitizer or GPU RSS.

## Compatibility boundaries

This is an experimental, measured subset. Native deferred results provide
then/catch/finally chaining needed by fetch and body helpers; Promise constructors,
thenables, async/await, the complete microtask/event loop, generators, modules and
modern collection/typed-array semantics belong to Phase 8. Header/DOM collections
currently expose snapshots; full iterable/WebIDL conformance is not claimed.
Synchronous XHR, response streams, AbortController, IDB cursors/key ranges/
multiEntry indexes, transferable binary objects, worker importScripts/fetch,
full Canvas path/clip/composite/font metrics and typed ImageData are outside this
subset. Canvas rasterization is bounded and simplified, with system font fallback.

Resize/intersection observations are frame/layout checkpoints, not a complete
rendering-opportunity implementation. Window event routing still shares the
document target inherited from Phase 5. Cross-document history state, tabs,
WindowProxy/opener communication and shared popup profiles need Phase 9 work.
Fullscreen currently changes the native window rather than implementing
element-specific fullscreen layout/pseudo-classes.
Cookie third-party blocking is stricter than partitioning; initiator-sensitive
top-level SameSite distinctions and partitioned jars remain hardening work.
Canvas CORS image opt-in and dynamic stylesheet/script resource semantics also
need broader conformance. Existing DOM/JS limitations remain documented in their
crate READMEs. These limits are not hidden as successful API implementations.

## Verification

```sh
cargo test -p web_api
cargo test -p web_api --test wpt_adapted -- --nocapture
cargo run -p web_api --example demo_server -- 8097
cargo run -p shell --example phase7_probe -- http://127.0.0.1:8097/ /tmp/browser-probe
```

WPT source notices, explicit adaptations/skips and the license are under
`tests/wpt/`; report accounting is shared through `test_utils`. The dedicated fuzz
target under `fuzz/` never creates a transport or native platform. Evidence and
remaining boundaries are recorded under `plan/evidence/phase-7/`.
