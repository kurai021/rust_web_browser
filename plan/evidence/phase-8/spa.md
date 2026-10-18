# Phase 8 async SPA acceptance

A small complete SPA (`browser/crates/shell/tests/fixtures/modern-demo`, served
by `web_api/examples/demo_server` with in-memory task data) exercises static and
dynamic source-text modules, top-level await, async fetch/POST/PATCH, async-
generator hydration and live module bindings. Map/Set store state with
subscribers, Proxy/Reflect route stores, Symbol tags updates, generator
iteration renders tasks, WeakMap attaches ephemeral DOM metadata, and BigInt
revisions cross a structured-clone worker boundary. All JavaScript runs in the
owned VM; transport and the worker use explicit page services.

## In-process probe — `shell/examples/phase8_probe.rs`

Automation drives ordinary DOM events (`input`, `submit`, `click`, history) over
real loopback HTTP. It asserts initial hydrate, POST create, PATCH toggle,
route/completed filter, dynamic `import()` caching (`detailsLoads === 1`),
history back, refresh, FIFO order and rAF→microtask checkpoints, an empty
console, and a fresh GC pass. It also renders the page wide (1000×900) and narrow
(620×900) and checks no `.card` overflows the narrow viewport.

```text
PHASE8 PROBE PASS static-modules top-level-await async-fetch POST PATCH generators
Proxy Map WeakMap Symbol BigInt structured-clone-worker FIFO rAF
dynamic-import-cache history refresh wide narrow paint_hash=e4a02fcb449b43a0
```

## Upstream TodoMVC acceptance — `shell/tests/upstream_todomvc.rs`

Runs the **unmodified** upstream `tastejs/todomvc` vanilla example
(`examples/javascript-es5`, pinned `ff43b02`) on 8 MiB stacks. Adds an
HTML-escaped title, toggles, edits, filters completed/all, clears completed and
deletes, asserting the application HTML and its seven source files are unchanged
on disk.

```text
UPSTREAM TODOMVC PASS add escaped-title toggle edit completed-filter
clear-completed all-filter delete; application HTML and seven source files unchanged
```

## Finding fixed during this acceptance

The SPA toggle initially no-op'd: the injected `WeakMap` metadata was keyed by a
DOM node wrapper, and the GC did not trace native DOM parent/child edges to
already-created host wrappers, so the wrapper could be collected and
`action(event.target)` returned `undefined`. The fix traces those native edges
(`HostHooks::trace_native_edges`) during collection; a DOM `WeakMap` metadata
regression test now covers both survival while attached and reclamation after
detach.
