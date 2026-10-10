# Phase 5 compatibility measurements

| Suite | Passed | Failed | Skipped | Meaning |
|---|---:|---:|---:|---|
| Test262-adapted | 16 | 0 | 0 | Eight upstream inputs, each sloppy + strict |
| WPT events-adapted | 11 | 0 | 0 | Once/passive/default prevention/target order/immediate stop; 77 assertions |
| WPT DOM-adapted | 56 | 0 | 0 | Seven-node single-document contains matrix; 56 assertions |
| Node differential | 27 | 0 | 0 | Representative Level 1 results, explicit test oracle |
| Custom language | 18 | 0 | 0 | Stateful scenarios with multiple assertions |
| Custom DOM | 14 | 0 | 0 | Mutation/events/policy/limits and tracing regressions |
| Script loader/worker/demos | 8 | 0 | 0 | Three loader, three worker, two acceptance tests |

Each adapted suite is 100% of its published subset, not 100% of the upstream
suite. No whole-Test262/WPT denominator or daily-driver percentage is implied.
Existing Phase 2/3 HTML/CSS adapted suites also pass without regression.

Test262 data/provenance are in `browser/crates/js/tests/test262/`; WPT data,
assertion preservation and subset reductions are in `dom_bindings/tests/wpt/`.
Node is launched only by the ignored-by-default differential test; CI runs it
explicitly with a test-only Node installation. The product embeds no JS oracle.

Notable corrected semantic cases: callee-before-argument evaluation, one-time
member reference evaluation, switch case evaluation once, typeof in the TDZ,
strict eval scope versus global indirect eval, primitive sloppy this boxing,
super method/getter receivers, passive cancellation, nested once dispatch,
removed-listener dispatch and event-target tracing.

Remaining Level 1 edge coverage includes property ordering/descriptors, sparse
arrays, early-error/hoisting/legacy strict/class corners, UTF-16 surrogates, full
ECMAScript regex behavior, JSON hooks and full Date calendar/local semantics.
DOM collections/editing/WebIDL, dynamic insertion and complete lifecycle/event
semantics are partial. Async scripts currently use the defer-compatible path.
See crate READMEs for precise current boundaries and later-phase APIs.
