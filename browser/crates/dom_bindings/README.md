# Minimal DOM/BOM bindings — Phase 5

`DomRuntime` owns a persistent JS realm and one document on the JS worker thread.
Bindings reuse `html::Document` and the CSS selector/cascade/parser APIs.

Implemented: document/id/tag/class queries, querySelector(All), basic node tree
mutation/fragments/cloning, attributes/text/HTML, classList, inline style and
getComputedStyle/matchMedia; event listeners, capture/bubble/once/passive,
cancelation/propagation, basic Event subclasses and standalone EventTarget;
form value/checked state, focus/validation helpers, timers and rAF-style callbacks.
The shell routes pointer/keyboard input and Tab navigation, then rebuilds the
retained Stage A scene from mutated snapshots off the UI thread.

Connected listeners and timers are roots; reachable detached nodes retain their
listeners. Events trace their targets. Unreachable DOM↔JS cycles are swept and
arena slots reused, including the 10,000 node/listener regression. Removed focus
is cleared before the next page snapshot. Callbacks run after host borrows are
restored. Resource exhaustion disables subsequent script callbacks/timers.

Basic CSP intersects header/meta policies, handles script-src/default-src,
unsafe-eval, unsafe-inline and nonce/SHA hashes, and checks every external script
redirect before connecting. SHA hashing uses the approved sha2 auxiliary.

This is the partial Phase 9-document surface scheduled by Phase 5, not the full
daily-driver DOM/BOM/Web API implementation. Collections are snapshots, controls
have a basic append/Backspace editor, and there is no full selection/caret,
IME/composition, WebIDL/prototype or form-submission implementation. Geometry,
dynamic stylesheet/script insertion and comprehensive lifecycle/resource/event
semantics need further work. Window listener routing currently shares the
document target. Full fetch/storage/history/worker/observer APIs are Phase 7;
microtasks/Promises and the complete event loop are Phase 8.

The classic loader resumes parser boundaries for blocking scripts and runs defer
after parsing; the current async attribute takes the defer-compatible path.
Execution begins after the document response is received. Phase 4 HTML previews
still paint before EOF, and fonts/images remain deferred worker resources.

Compatibility evidence: 11 adapted WPT event cases / 77 assertions and a
single-document Node.contains matrix of 56 cases / 56 assertions. Sources,
adaptations and BSD license are retained under `tests/wpt/`.
