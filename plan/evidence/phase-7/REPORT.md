# Phase 7 — Web API verification

Implementation and verification against Phase 6 baseline `6f729b7`, on Linux
aarch64 / Apple M2 Pro / stable Rust 1.99, 2026-10-16.

The owned `PageRuntime` now supplies the committed Phase 7 service subsets:
fetch/XHR/CORS, location/same-document history, origin storage/cookies/IndexedDB,
dedicated workers, WebSocket/SSE, observers, Canvas2D and native permission/dialog
chrome. The shell keeps VM/DOM work on the JS thread and transport on cancellable
tasks. Data-only cloned messages never share arena pointers between realms.

The loopback workshop and executable probe exercise a real fetched SPA, persistent
login and visits, IDB commit, worker computation, socket echo, SSE, observers,
image/text/path chart rendering and repeated refresh in one live context. GUI
verification covers GPU/software, responsive resize, SPA back, restart, permission
denial/revocation, unavailable geolocation and a native prompt held beyond the JS
execution deadline. UI findings added control-height/password-mask and modal-wait
budget regressions before final verification.

Evidence index:

* [verification.md](verification.md): quality commands, dependency repair and audits.
* [compatibility.md](compatibility.md): measured WPT and explicit subset boundaries.
* [gui.md](gui.md): captures and native interaction observations.
* [performance.md](performance.md): comparable Criterion and local GUI measurements.
* [fuzz.md](fuzz.md): final 301-second sanitized page/API run.
* [kernel-registry.md](kernel-registry.md): scalar/SIMD decision.
* The three JSON files contain per-case fetch/XHR pass/fail/skip accounting.

This is an experimental phase completion, not full web-platform conformance or
a daily-driver declaration. Full modern JS/event-loop semantics are Phase 8;
browser/session completion and final hardening follow the sealed roadmap.
