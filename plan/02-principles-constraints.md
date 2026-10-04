# 02 — Principles and constraints

## 2.1 Why Rust

Author's decision, non-negotiable:

- Near-hardware control (memory, threads, SIMD/GPU) without the risk of "breaking the CPU writing asm blind".
- Memory and concurrency safety (`ownership`, `Send`/`Sync`) fit for a multithreaded program parsing hostile content.
- Abstract enough to avoid hand-rolling TCP/TLS, yet low-level enough for in-house engines.
- Per the original motivation: Rust is verbose and "ugly", but very powerful, and AI-assisted development + phases absorb that verbosity. Therefore: scope is never cut because "this is a lot of code in Rust". It is sliced into phases instead.

Implications:

- All product code is Rust (edition 2021 or newer). Built with Cargo workspaces.
- `unsafe` allowed only at justified points (FFI, SIMD, rendering), isolated in small modules, audited and tested. Scattered `unsafe` forbidden.
- ASM/SIMD allowed only as surgical aggressive optimization under the Rust-first policy (see `15-asm-simd-optimization.md`): first correct portable Rust, then `core::arch` intrinsics, only at the end `asm!` if LLVM falls short. Always with pure-Rust fallback, runtime dispatch and identical semantics. No feature sacrifices.
- `clippy` + `rustfmt` mandatory since Phase 0. Warnings = error in CI.

## 2.2 Engineering principles

1. **Bridge-phases:** each phase delivers something that compiles, passes tests and integrates. No dead branches lasting months. No dates, but strict order (see doc 14).
2. **Specs first:** `plan/` rules. If code and docs disagree, docs win until the doc is updated.
3. **Own engines, borrowed network:** HTML/CSS/JS/DOM/layout are written here. TCP/DNS/TLS/HTTP are reused. No reinventing cryptography or sockets.
4. **Hostile content by default:** everything from the network is treated as potentially malicious. Defensive parsers, memory/time limits, fuzzing.
5. **UI never blocked:** I/O and parsing off the UI thread. Hard rule.
6. **No network magic:** zero unsolicited connections. Every connection must trace back to: a user action, a page request, or an explicit documented opt-in/opt-out update.
7. **Performance by design:** streaming/incremental wherever it applies (incremental HTML parsing, incremental layout, layered painting). Measure from Phase 4 on.
8. **Pragmatic portability:** primary target 64-bit Linux desktop. Portable design for Windows/macOS, but v1 is not blocked on them.

## 2.3 Hard constraints

- Language: Rust-first for the product (100% Rust except SIMD/asm kernels declared in doc 15). Utility scripts may be Python/shell for tooling only, never part of the binary. WGSL/compute shaders for the GPU count as permitted render code, versioned alongside `paint`.
- Licenses (pragmatic, anti-overengineering): prefer MIT / Apache-2.0 / BSD / ISC. **GPL/AGPL allowed as an exception** when the library is clearly better implemented and reimplementing it would be overengineering. Conditions: (1) no reasonable permissive alternative, (2) registered in `plan/evidence/phase-0/licenses.md` with crate, version, license and why it pays off, (3) `cargo deny` passes with the exception declared explicitly, never silently. Accepted implication: linking GPL/AGPL into the binary subjects that part of the binary to copyleft (must distribute source + notices; with AGPL also a source offer if served over a network). The project license is set in Phase 0 to "MIT/Apache-2.0 by default, GPL-3.0-or-later where a copyleft dependency applies". No dependencies with telemetry or hidden network calls, whatever their license.
- No dependencies doing telemetry or hidden network calls. Audit `Cargo.lock`.
- MSRV defined in Phase 0 (suggested: recent stable, e.g. 1.75+; pin the exact one at kickoff).
- Native GUI (see doc 10). Embedding Chromium/Electron/CEF/WKWebView/Gecko as a shortcut is forbidden. That would betray the "own engines" goal.

## 2.4 What is explicitly not optimized in v1

So no scope gets assumed:

- Beating Chrome/Firefox on benchmarks is not a v1 goal. Target: usable and correct first, fast later.
- No world-class JS JIT is promised in v1. A tree-walking interpreter + bytecode VM is enough to leave "experimental"; JIT is future optimization.
- No pixel-perfect parity with Chrome everywhere is promised. Target: progressive WPT conformance (see doc 13).
- Aggressive optimization (SIMD/asm/GPU) never cuts features or compatibility: every optimized kernel must pass the same tests as the pure fallback (see doc 15).
