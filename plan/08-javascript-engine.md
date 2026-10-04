# 08 — In-house JavaScript engine

> `js` crate. In-house engine from scratch. Embedding V8/SpiderMonkey/JavaScriptCore/QuickJS/Boa in the product is forbidden. They may be used as differential oracles in tests.

## 8.1 Goal

Run real modern-web JavaScript well enough for daily-driver use: DOM scripting, fetch, events, timers, async. No shortcuts.

## 8.2 Scope by levels (all committed, in order)

**Level 1 — MVP (escape static):**

- Lexer + Parser + AST for ES5 + `let`/`const`, arrow functions, template literals, basic classes, spread/rest, basic destructuring.
- Tree-walking interpreter with lexical scopes, hoisting (`var`, function), closures, `this` (sloppy/strict), prototypes (`Object.create`, `new`, prototype chain).
- Types: `undefined`/`null`/`boolean`/`number`/`string`/`object`/`array`/`function`/`regexp`/`date`/`error`, basic `Symbol` (`bigint` lands in Level 2).
- `"use strict"` strict mode. Spec coercions (abstract ops).
- Level 1 builtins: `Object`/`Array`/`String`/`Number`/`Boolean`/`Math`/`JSON`/`Date`/`RegExp`/`Error`/*Error types, `parseInt`/`parseFloat`/`isNaN`/`isFinite`, `console.*` (to the DevTools console, not product stdout).

**Level 2 — Usable modern:**

- `Promise`, `async`/`await`, generators, modules (`import`/`export` with fetch + same-origin CORS), `Map`/`Set`/`WeakMap`/`WeakRef`? (WeakRef optional), `Symbol.*`, `BigInt`, basic `Proxy`/`Reflect`, `structuredClone`.
- Per-page event loop: task queue + microtask queue, `queueMicrotask`, `setTimeout`/`setInterval`/`requestAnimationFrame`, `fetch` integration.
- Exceptions with useful stack traces (line/column/function).

**Level 3 — Performance (after functional daily-driver):**

- Own bytecode VM + simple mark-and-sweep/generational GC. The tree-walking interpreter migrates to bytecode with no semantics change.
- Optimizations: inline caches (property lookups), hidden classes/shapes, string interning, number fast-paths.
- JIT: explicitly **out of v1**. Leave hooks (stable IR/bytecode) but promise nothing.

Never included as a shortcut: transpiling to another engine, WASM-for-JS (web WASM support is a separate future phase, not a replacement for the JS engine).

## 8.3 DOM integration (summary, details in doc 09)

- `dom_bindings` exposes host objects (`document`, `window`, `Node`, `EventTarget`) as JS objects with Rust trampolines. The GC must trace DOM↔JS references (cycles). No leaks: create/destroy 10k nodes + listeners test.
- JS-driven DOM mutation invalidates style/layout (dirty). Forced synchronous layout (`offsetHeight`) performs a synchronous re-layout of only the needed subtree (document as an exception to "never block").
- Caps: stack depth, loop detector (long-running script → "stop script" dialog after 5s), per-page memory (e.g. 512 MB), recursion.

## 8.4 Fixed API

```rust
// crates/js
pub fn parse(src: &str, opts: ParseOpts) -> Result<Program, ParseError>;
pub struct Vm { /* ... */ }
impl Vm {
  pub fn new(host: HostHooks) -> Self;
  pub fn eval(&mut self, prog: &Program) -> Result<JsValue, JsError>;
  pub fn run_event_loop(&mut self); // tasks + microtasks
  pub fn gc_collect(&mut self);
}
pub trait HostHooks { fn print(&self, s: &str); /* fetch/dom callbacks */ }
```

- Errors carry `line:col`, `stack`, `kind` (SyntaxError/TypeError/…). Never `panic` on arbitrary JS.
- `eval`/`Function` allowed (that is the real web), but gated by simple CSP (see doc 12): if CSP `script-src` blocks inline → `eval` fails like on the web.

## 8.5 JS-specific testing

- Test262 subset (levels 1–2) with an in-house harness, published and growing pass rate. v1 daily-driver goal: pass core language + common builtins, not 100% of Test262.
- Parser + runtime fuzzing (random inputs, no panics, timeout-bounded termination).
- Differential runs vs Boa/Node in tests only (oracle), never in the binary.

## 8.6 Acceptance criteria

- [ ] Vanilla TodoMVC, sample counter/fetch/form-login work with no per-site patches.
- [ ] `async/await + fetch + DOM` demo passes. Promises/microtasks in the right order.
- [ ] A long-running script never freezes the UI forever (dialog + kill).
- [ ] Test262 subset reported in `13-testing`.
