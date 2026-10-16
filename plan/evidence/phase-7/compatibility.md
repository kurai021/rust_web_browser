# Phase 7 compatibility accounting

The adapter runs the own interpreter, actual API objects and loopback HTTP
transport. Upstream source: WPT `4aa56d3beafc5655fa55491169eb14845b44eab3`.
Sources, BSD notices and exact scheduling/harness adaptations are recorded in
`browser/crates/web_api/tests/wpt/PROVENANCE.md`.

| Adapted subset | Pass | Fail | Explicit skip | Assertions |
|---|---:|---:|---:|---:|
| fetch Headers constructors/record/sequence/CRUD | 13 | 0 | 10 | 68 |
| fetch request body/header/origin wire behavior | 18 | 0 | 7 | 101 |
| XHR method case preservation, async transport | 9 | 0 | 0 | 9 |
| Retained DOM single-document contains matrix | 56 | 0 | 0 | 56 |
| Retained DOM events capture/once/passive/order | 11 | 0 | 0 | 77 |

New fetch/XHR total: 40 pass, 0 fail, 17 explicit skips / 57 inventoried cases,
178 assertions. Executed cases pass 100%; including skips, coverage/pass is
70.18%. Per-case JSON is archived alongside this file. Skipped iterable/generator
and typed-binary cases are counted rather than silently removed. The old DOM and
events rates remain 100% on their measured subsets; these are not full WPT rates.

Custom regressions additionally cover CORS/preflight credentials, forbidden and
hidden headers, CSP/mixed-content redirect gating, strict origin storage,
reentrant coercions, deferred body/finally ordering, XHR stale operations and
abort, cyclic data-only messages, no-DOM workers, socket/SSE lifecycle, atomic IDB
transactions/unique indexes, profile restart, session isolation, canvas taint and
resize reset, mutation records and real layout/scroll observations. Idle-service
startup is checked for empty commands/tasks and no connection to its document
origin before an API is invoked.

## Explicit boundaries

The inventory and quotas are in `browser/crates/web_api/README.md`. In particular:

* Native then/catch/finally results are the Phase 7 bridge, not full Promise,
  thenable, async/await, generator/module or microtask conformance.
* Headers and DOM collections are snapshots; full iterable/WebIDL/prototype edge
  semantics remain incomplete. XHR is async and buffered, not a streaming/sync
  implementation. Binary typed-array bodies are not yet exposed.
* IDB is the object-store/get/put/basic-index transaction subset, with no cursor,
  key-range or multiEntry interface. Cross-context conflicts abort rather than
  providing a complete cross-process scheduler.
* Workers have isolated messages/basic timers; importScripts/worker fetch and
  transferable binary buffers are not part of this subset. SSE retries an opened
  stream after EOF but does not claim all browser reconnection edge semantics.
* Canvas uses bounded simplified CPU rasterization and system shaping. Full
  clipping/composites/font metrics, CORS image opt-in and typed ImageData need
  further conformance. Cross-origin readback is rejected.
* Observer delivery follows bounded layout/task checkpoints. Window event routing
  shares the document target. Same-document SPA history works; complete
  cross-document state restoration, tabs, WindowProxy and popup profile sharing
  belong to the browser/session work. Fullscreen currently changes the native
  window rather than implementing element-specific fullscreen layout.
* Third-party cookies are blocked, with no unpartitioned fallback. Complete
  initiator-sensitive top-level SameSite distinctions and partitioned jars are
  still hardening work. Existing Phase 5 DOM/JS and Phase 6 layout limits remain
  explicitly documented; dynamic resource/stylesheet semantics need more coverage.

No service returns invented geolocation or silently grants a permission. Missing
native notification/clipboard/dialog backends fail explicitly; memory-only
profiles cannot claim persistent-storage success.
