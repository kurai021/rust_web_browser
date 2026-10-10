# Phase 5 — In-house JavaScript MVP and minimal DOM bindings

Local verification, 2026-10-09. Base commit: `46a4d1e` (completed Phase 4).
The sealed v1.2 framework is preserved. Scope is roadmap Phase 5 / JS Level 1
and partial DOM bindings; modern JS and daily-driver Web APIs remain later phases.

## Delivered

- Owned lexer/parser/AST interpreter, lexical environments, Level 1 builtins,
  strict/direct/indirect eval, bounded execution and tracing arena GC.
- Persistent per-navigation realm on a dedicated JS worker; blocking/defer
  classic loader, basic CSP/nonce/hash/mixed-content and pre-connection redirect
  checks, atomic cancellation, local console and visible script-stop notice.
- DOM queries/mutation/events/inline CSSOM, basic controls and real keyboard/pointer
  routing, timers, scene invalidation and deferred resource publication.
- Offline counter, menu toggle, form validation and simple vanilla-JS TodoMVC,
  using the generic bindings and existing Stage A renderers.
- Compatibility harnesses, parser/runtime fuzzing, Criterion baselines and
  a 10,000 DOM-node/listener/closure cycle collection probe.

## Evidence index

- [verification.md](verification.md): commands, case counts and hardening regressions.
- [compatibility.md](compatibility.md): Test262/WPT/differential measurements and boundaries.
- [performance.md](performance.md): parser/interpreter/GC and GUI timings.
- [fuzz.md](fuzz.md): bounded instrumented parser/runtime campaigns.
- [gui.md](gui.md): interactive acceptance and screenshot descriptions.
- [kernel-registry.md](kernel-registry.md): scalar implementation decision.

The measured subsets support the Phase 5 MVP demos; they do not establish full
language/WebIDL conformance or daily-driver readiness. Detailed current coverage
is documented in `browser/crates/js/README.md` and `dom_bindings/README.md`.
