# Phase 5 WPT-adapted DOM/events subset

Retrieved from web-platform-tests/wpt `master` on 2026-10-09:

- `dom/events/AddEventListenerOptions-once.any.js` (all four tests).
- `dom/events/AddEventListenerOptions-passive.any.js` (five synchronous equivalents;
  the returnValue async wrapper becomes synchronous, with the same assertions).
- `dom/events/Event-dispatch-order-at-target.html` (complete script assertion set).
- `dom/events/Event-stopImmediatePropagation.html` (complete script assertion set).
- `dom/nodes/Node-contains.html` (upstream ancestor oracle, seven single-document
  nodes instead of the cross-document/shadow/other nodes in `common.js`).

Formatting and assertion messages are condensed. The Rust harness supplies
test/assert helpers, counts cases/assertions, and fails on callback console errors.
There are 11 event cases / 77 assertions and 56 DOM cases / 56 assertions.
These adapted subsets do not claim full upstream suite or WebIDL conformance.
Source license: BSD-3-Clause, retained in LICENSE.md.
