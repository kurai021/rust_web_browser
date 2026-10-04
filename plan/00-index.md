# Plan — Theoretical framework for the Rust browser

> Folder `plan/`. Holds the complete theoretical framework. It is the source of truth before writing code.
> Rule: if it is not here, it does not exist as scope. No assuming, no adding.

## Documents

| # | File | What it defines |
|---|------|-----------------|
| 01 | `01-vision-scope.md` | What this browser is and is NOT. Definition of "non-experimental" and daily-driver. |
| 02 | `02-principles-constraints.md` | Why Rust, hard constraints, blocked non-goals. |
| 03 | `03-general-architecture.md` | Per-crate workspace, threading, load/render pipeline, dependency decisions. |
| 04 | `04-network-https.md` | Networking: what is reused (TCP/TLS) and what is built (URL, HTTP/HTTPS, fetch, HSTS). |
| 05 | `05-html-parser.md` | Own HTML5 engine: tokenizer, tree, incremental parsing, errors. |
| 06 | `06-css.md` | Own CSS engine: parsing, cascade, selectors, values, media queries. |
| 07 | `07-layout-render-paint.md` | Layout (block/inline/flex/grid), paint, compositing, text and images. |
| 08 | `08-javascript-engine.md` | Own JS engine from scratch: lexer/parser/AST/interpreter/VM, event loop. |
| 09 | `09-dom-bom-webapis.md` | Minimal DOM, BOM and Web APIs for daily-driver use. |
| 10 | `10-graphical-shell.md` | Graphical window: tabs, omnibox, navigation, history. No console mode. |
| 11 | `11-downloads-adblock-privacy.md` | Downloads, blocker with public lists, anti-telemetry. |
| 12 | `12-security.md` | Security model, sandbox, per-origin/site isolation. |
| 13 | `13-testing-compatibility.md` | How "functional" is measured: WPT, Acid, benchmarks, acceptance criteria. |
| 14 | `14-roadmap-phases.md` | Bridge-style phases, in order, each with entry/exit (DoD). No dates. |
| 15 | `15-asm-simd-optimization.md` | Rust-first + SIMD/asm/GPU doctrine: allowed kernels, dispatch, DoD. No feature cuts. |

## How to use this framework

1. Read in order `01` → `15`.
2. Each phase in `14-roadmap-phases.md` references its specs (`04`–`12` + `15` for perf).
3. Scope change = edit the corresponding doc + update the roadmap. No implicit scope.
4. Future code must cite the doc/section it implements.

## Status

Theoretical framework v1.2 (full English translation; added doc 15 SIMD/asm in v1.1). No name, no logo. Only code and specs.

## Language policy

- Working conversation with the author happens in Spanish.
- Everything produced (docs, code, comments, identifiers, CLI strings, commit messages) is written in English for universality.
