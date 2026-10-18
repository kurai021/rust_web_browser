# Phase 8 — defensive testing scope

This note records the intent and the boundaries of the robustness testing used
across Phases 7 and 8 (fuzzing, native-stack limits, GC/weak-reference stress and
differential oracles). It exists so the purpose of these tests is unambiguous.

## Purpose

All of this work is **defensive robustness testing of the project's own
in-house engine**. The goal is that the owned JavaScript VM, DOM, HTML/CSS,
layout and Web API subsets fail safely (bounded error instead of crash, OOM,
hang or corruption) when they receive malformed, adversarial or deeply nested
input. It is not vulnerability research against third-party software, and it is
not exploit development.

## Boundaries

* **Own code only.** Targets exercise this repository's parser, interpreter,
  bytecode VM, module linker, structured clone, DOM/host bridge and page runtime.
  No target fuzzes an external process, library or service.
* **No external network targets.** Fixtures, seeds and servers use loopback
  (`127.0.0.1`) and reserved non-routable names (`fuzz.invalid`, `fuzz.test`).
* **No arbitrary command execution.** `node` is spawned only inside `#[ignore]`d
  differential *tests* as a conformance oracle; it is never linked into or
  shipped with the product. Product code only launches the browser's own binary
  (popup window) and desktop helpers (`wl-copy`/`wl-paste`/`notify-send`).
* **No secrets or arbitrary filesystem access.** Product filesystem use is the
  browser's own storage/cookies/HSTS with `0600` permissions.
* **No `unsafe`.** The workspace contains no `unsafe` blocks; all memory
  management is safe Rust plus the owned garbage collector.
* **Bounded runs.** Fuzzing runs are time-bounded (`-max_total_time`), memory-
  bounded (`-rss_limit_mb`), and single-target. Fuzzing is never run concurrently
  with heavy builds.

## Resource discipline (learned the hard way)

Running two sanitized fuzzers concurrently *and* then a full
`cargo test --workspace` compile on a 15 GiB machine exhausts RAM and CPU, because
Rust/LLVM compilation runs many memory-hungry jobs in parallel. Builds/tests must
be serialized with a small job count (`CARGO_BUILD_JOBS=2`), ideally under `nice`,
and never overlap with fuzzers. This is an operational constraint, not a code
defect.

## Typical defensive techniques in use

Carry-forward clarification: the earlier resource explanation was not a proof
that all code was free of resource defects. Phase 9 GUI measurements found and
fixed idle-GC bugs in both page and dedicated workers. See
[`../phase-9/testing-scope.md`](../phase-9/testing-scope.md) and the measured
[`../phase-9/performance.md`](../phase-9/performance.md).

* Sanitized (ASan) libFuzzer targets for the JS parser/evaluator and the page API
  surface, seeded with modern-JS constructs (async generators, `for await`,
  Proxy, WeakMap, BigInt, structured clone, modules).
* Explicit native-stack budget (`Limits::max_native_stack_bytes`) so deep
  recursion returns a `RangeError` instead of overflowing the OS stack.
* GC/weak-reference stress: WeakMap/WeakRef ephemeron fixed points, detached DOM
  metadata reclamation.
* Differential testing against Node as an oracle to confirm semantics, not to
  attack it.
