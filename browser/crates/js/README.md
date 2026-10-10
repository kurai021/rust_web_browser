# In-house JavaScript — Phase 5 / Level 1 MVP

The lexer, recursive-descent parser, AST evaluator, lexical environments,
abstract operations, object/prototype model and tracing heap are owned Rust code.
`regex`, `serde_json` and `time` supply bounded auxiliary functionality, not a JS
engine. Node appears only in the explicit differential test.

## Implemented surface

- ES5-style expressions/statements, functions, hoisted var/function declarations,
  exceptions and strict mode; lexical let/const, closures and per-iteration let.
- Arrow/template syntax, basic classes/inheritance/super, rest/spread and basic
  destructuring/default patterns.
- Primitive values, objects, arrays, functions, prototypes, accessors/descriptors,
  basic Symbol; Object/Array/String/Number/Boolean/Math/JSON/Date/RegExp/Error,
  parsing globals and host-local console methods.
- CSP-gated direct/indirect eval and Function construction. Strict eval has its
  own variable scope; indirect eval uses the global context. Cached host policy
  remains enforced during host-side argument getters.
- Object identities contain arena index/generation. GC follows properties,
  lexical closures, bound arguments, pins and host roots/references; DOM↔JS
  detached cycles are collectable. Collection happens between host tasks, so
  interpreter temporaries never become untraced roots during a native call.

## Default limits

| Resource | Limit |
|---|---|
| Source / AST | 8 MiB source, 500,000 tokens, 100,000 parser work nodes |
| Parsing | 128 nesting/chain depth; nested templates inherit remaining budget |
| Execution | 2,000,000 ticks, 5 seconds, 128 calls |
| Arenas | 500,000 JS objects and 500,000 environments |
| Strings | 8 MiB per result; bounded join/split/match/replacement/JSON temporaries |
| Page accounting | 512 MiB conservative JS + DOM retained-allocation budget |

Accounting covers properties/keys, retained strings, environments/bindings,
function ASTs and host-owned DOM/control/listener/timer data. Shared string/AST
holders can be charged more than once; regexes are charged for their compilation
and cache caps. Arena capacity can remain reserved after collection. This is a
conservative allocator budget, not a promise about total process/GPU RSS.
Resource-limit failures are not catchable by JS. Cancellation is checked against
an atomic flag/navigation token; native iterative work participates in the ticks.

## Compatibility and verification

The published initial Test262-adapted subset contains eight sources, executed
in sloppy and strict modes (16 case-mode executions). The custom language suite
adds 18 stateful scenarios, and Node checks 27 representative results. These
are measured subsets, not complete ES5 or Test262 conformance.

Known edge coverage remains incomplete for property enumeration/descriptors,
sparse arrays, early errors/hoisting, class and legacy strict corner cases. Strings
are UTF-8 internally with UTF-16 indexing helpers; isolated surrogate behavior
is incomplete. The regex auxiliary does not implement ECMAScript lookaround or
backreferences. JSON hooks/replacers/revivers are incomplete. Date parsing is
ISO-oriented, local accessors currently use UTC, and calendar construction is
limited by the auxiliary calendar range. These compatibility gaps need growing
conformance coverage before a daily-driver claim.

Promises, async/await, generators, modules and modern collection APIs belong to
Level 2 / Phase 8. Bytecode dispatch and performance migration follow that work;
this MVP has no own SIMD/asm kernel.

```sh
cargo test -p js
cargo test -p js --test differential -- --ignored --exact representative_language_results_match_node
cargo bench -p js --bench interpreter
# From crates/js, with cargo-fuzz and nightly installed:
ASAN_OPTIONS=quarantine_size_mb=16 cargo +nightly fuzz run parse_eval fuzz/corpus/parse_eval fuzz/seeds -- -max_total_time=300 -max_len=8192 -timeout=3 -rss_limit_mb=1536
```
