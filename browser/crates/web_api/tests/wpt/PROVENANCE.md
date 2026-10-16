# Phase 7 adapted WPT inputs

Upstream: <https://github.com/web-platform-tests/wpt>, reviewed 2026-10-16,
HEAD `4aa56d3beafc5655fa55491169eb14845b44eab3` (BSD-3-Clause).

* `headers.js`: `fetch/api/headers/headers-basic.any.js`; retains the 13
  constructor/record/sequence/CRUD cases and assertions, with 10 explicitly
  reported skips for generators, live iteration, iterator prototypes and forEach.
* `fetch.js`: `fetch/api/basic/request-headers.any.js`; 18 request/body/origin
  cases use the real loopback transport and retain the wire-header assertions.
  Seven typed-binary cases are reported as skips. The body-type classifier drops
  references to the unavailable ArrayBuffer constructor; the seven cases using
  it are not executed. `/inspect` replaces the upstream Python echo resource.
* `xhr.js`: `xhr/open-method-case-sensitive.htm`; all nine original method tokens
  and case-preservation assertions are retained. Network requests run async,
  because synchronous XHR is an explicit unsupported API, not a fake success.

The restricted testharness adapter lives in `test_utils/src/wpt.js`. It counts
assertions per callback/case and records failures and skips in JSON. Its error
type assertion checks the error name, reflecting the existing Level 1 Error
prototype boundary. No Test262/JS implementation or third-party engine ships in
the browser. These are measured adapted subsets, not a claim to pass full WPT.
