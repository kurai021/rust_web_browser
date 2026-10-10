# Phase 5 local verification

Linux aarch64, stable Rust 1.99, Apple M2 Pro (G14S B1), 2026-10-09.
MSRV is declared as 1.85; this local verification uses stable, not an MSRV run.
Cargo commands run in `browser/`.

| Command | Result |
|---|---|
| `cargo test --workspace --all-targets` | 173 passed, 8 ignored; benchmark smoke/examples compile and pass |
| `cargo test --workspace --no-default-features` | 173 passed, 8 ignored; doc tests pass |
| `cargo test -p js --test differential -- --ignored --exact representative_language_results_match_node --nocapture` | 27/27 Node results |
| `cargo test -p paint --test backends gpu_and_software_render_the_same_display_list -- --ignored --exact --nocapture` | Vulkan adapter parity passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo fmt --all --check` | Passed |
| `cargo build --release -p browser` | Passed |
| `cargo audit` | No vulnerabilities; six previously documented maintenance notices |
| `cargo deny --log-level error check` | Advisories/bans/licenses/sources pass |
| `git diff --check` | Passed |

The eight default ignores are five online TLS/network checks, optional WOFF2,
GPU parity and the Node oracle. GPU parity and Node were run explicitly.

## Meaningful hardening regressions

- Resource exhaustion cannot be caught by JS and disables subsequent callbacks
  and timers, including a timer that fails before another due callback.
- A self-clearing interval is not reinserted; event queue draining is bounded.
- Properties/strings/environments and host DOM-owned strings participate in the
  conservative memory quota; oversized join/split/JSON expansion is bounded.
- Flat member/operator/new chains and nested template parsing are bounded.
  Large/NaN Date inputs no longer overflow integer calendar normalization.
- Large native DOM allocations and long selector queries check cancellation and
  the memory quota as they run, not only at the next interpreter tick.
- GC reclaims 10,000 detached nodes/listeners/closures, preserves live closures,
  traces retained event targets, and preserves standalone EventTarget listeners.
- Synthetic EventTarget identities cannot be used as Node mutation arguments.
- CSP survives argument-getter dynamic-code attempts; redirected scripts are
  rejected before a connection to a different, disallowed origin is opened.
- Canceling a pending stylesheet leaves the next worker navigation responsive
  (asserted under 500 ms against a two-second stalled server).
- Timers publish new image resource requests. Initial focus/console are delivered
  to the UI, and Tab/typing/keyup/button defaults use the persistent realm.
- Controls paint their current value/placeholder and use the same click geometry
  as the retained scene. Phase 4 pre-EOF/stop/loading tests continue to pass.

Input-path audit (`rg --count-matches` for unwrap/expect/panic/todo/unimplemented
over js/dom_bindings sources and script loader/worker) found no such calls.
No own unsafe, assembly, SIMD or `target-cpu=native` change was added.
