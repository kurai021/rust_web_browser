# Phase 8 async project board

A small complete SPA using static and dynamic source-text modules, top-level
await, async fetch/POST/PATCH, async-generator hydration and live module state.
Map/Set store state and subscribers, Proxy/Reflect route ordinary writes, Symbol
identifies updates, generator iteration renders tasks, WeakMap attaches ephemeral
DOM metadata, and BigInt revisions cross a structured-clone worker boundary.

The visible FIFO check mixes Promise/MutationObserver/queueMicrotask/timer jobs;
the animation callback also queues a microtask. Route changes use history and
reuse a dynamically imported module. The server holds task data in memory, so
refresh/reload exercises real HTTP rather than replacing fetch with a test stub.
No external assets, telemetry or third-party scripts are requested.

```sh
cargo run -p web_api --example demo_server -- 8097
cargo run -- http://127.0.0.1:8097/modern/
cargo run -p shell --example phase8_probe -- http://127.0.0.1:8097/modern/
```
