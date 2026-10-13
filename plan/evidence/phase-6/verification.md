# Phase 6 quality verification

Commands run in `browser/`, Linux aarch64 / stable Rust 1.99. Declared MSRV 1.85
is not separately exercised by this local stable run.

| Command / check | Result |
|---|---|
| `cargo test --workspace --all-targets` | 193 passed, 8 ignored; benchmark smoke/examples pass |
| `cargo test --workspace --no-default-features` | 193 passed, 8 ignored; doc tests pass |
| `cargo fmt --all --check` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo audit` | No vulnerabilities; six previously documented maintenance notices |
| `cargo deny --log-level error check` | Advisories/bans/licenses/sources pass |
| `cargo build --release -p browser` | Passed |
| Explicit GPU/software adapter parity | Passed on Apple M2 Pro Vulkan |
| `git diff --check` | Passed |
| Input-path unwrap/expect/panic/todo/unimplemented/unsafe audit | No matches in layout, layout grammars or display-list code |

The default ignores remain five online checks, optional WOFF2, Node oracle and
GPU adapter parity. The latter was run explicitly. No new phase-specific ignore
or dependency vulnerability exemption was introduced.

Workshop headless load also succeeds: initial scripting/CSS/tree/layout/paint
use the same path, with no warnings. GUI/fuzz/case measurements are in companion
reports. The framework core files remain sealed; changes are in implementation
documentation and evidence.

Meaningful regressions include deferred positioned flow, context isolation,
fixed coordinates under scroll/ancestor clipping, float exclusions/clear,
track/shorthand rejection, spans, native font/glyph coordinate saturation and
full-versus-incremental equivalence across dependency and structure changes.
