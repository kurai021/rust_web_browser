# Phase 9 verification

Host: Linux aarch64 / Apple M2 Pro, stable Rust 1.99. Heavy commands were serialized
under `nice`, with `CARGO_BUILD_JOBS=2`; test harnesses used two threads.

| Gate | Result |
|---|---|
| `cargo test --workspace -- --test-threads=2` | 302 passed, 0 failed, 10 explicitly ignored |
| `cargo test --workspace --no-default-features -- --test-threads=2` | 302 passed, 0 failed, 10 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo fmt --all --check` | pass |
| `cargo audit` | 0 vulnerabilities; six existing maintenance notices |
| `cargo deny check` | advisories, bans, licenses and sources pass |
| `cargo +1.89.0 check --workspace` | pass on the declared minimum |
| `cargo build --release -p browser -p shell --bin browser --example phase9_probe` | pass, portable release flags |
| `js/modern_differential --include-ignored` | 2 passed; Node remains a test oracle |
| ignored upstream TodoMVC acceptance | pass; upstream HTML and seven source files unchanged |
| ignored GPU/software display-list parity | pass on Vulkan |
| `phase9_probe` / `--headless-test about:blank` | five contexts and zero blank-startup connections pass |

The default ignored cases retain explicit requirements (online integration,
GPU/font fixtures, Node oracle and the separately pinned TodoMVC checkout). They
are not counted as ordinary suite passes. Relevant explicit runs are above.

## Browser/session/download regressions

* Five live pages retain separate realms and sessionStorage; click effects remain
  in their owner. Reorder/close preserve the selected context and other tabs.
* Settings, private file modes, history/bookmarks and lazy sessions survive restart.
  Credential-bearing URLs are cleaned before browser-profile persistence.
* A clean blank profile, five blank tabs, native preferences and construction of
  the opt-in lazy DNS adapter issue no HTTP/DNS requests.
* Same-document history survives a subsequent document navigation. A stopped
  committed preview stays inert rather than dispatching into the previous VM.
* A `cfg(test)` worker fault is caught; another realm remains interactive.
* The 100 MiB download streams through small buffers. Pause/resume preserves bytes,
  cancellation removes partials, invalid ranges do not append, missing range support
  restarts visibly, and a sixth transfer stays queued behind five active ones.
* Failed reservations preserve existing fixture files; complete partials finish via
  validated `416` metadata. Output is private and has no execute bits.
* Blob activation resolves before revocation, stays realm-local and hands only
  bytes/metadata to the native UI. Prevented/cross-origin `download` cases retain policy.
* Rendered find covers Unicode case/canonical equivalence, inline boundaries and
  hidden-text exclusion. PDF export is paginated; Netscape import is inert and
  exports preserve existing files. Web-font aliases remain tab-local.

## Conformance carried forward

`wpt/*.json`: fetch/XHR adapted subsets retain **40 pass / 0 fail / 17 skip /
178 assertions**. Existing DOM and events matrices also pass (56/56 and 11/11
cases, 133 assertions combined). Provenance retains scope exclusions.

`test262/test262-level2.json`: **57 case-mode passes / 0 failures**, 30 source
cases, 3 excluded sources and 206 assertions. This is the pinned adapted subset,
not the full upstream suite.

## Dependency and toolchain repair

The first audit rejected hickory-proto 0.25.2 (RUSTSEC-2026-0118/-0119). The DoH
adapter now uses hickory-resolver/net/proto 0.26.3, propagating lazy-builder errors;
no vulnerability exemption was added. Public DoH traffic was not a fixture
target. The explicit opt-in resolver stays dormant at startup.

The previous Rust 1.85 declaration did not match resolved dependency minima.
The workspace now declares **1.89**, including uuid's requirement, and this
minimum was checked explicitly. Stable 1.99 is the primary test toolchain.
Equivalent standard slice/integer APIs satisfy newly enabled Clippy lints.

## Reproduce

```sh
export CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2
cargo run -p web_api --example demo_server -- 8099
# A second terminal, with an isolated profile/download directory:
cargo run --release -- --profile-dir /tmp/browser-workshop http://127.0.0.1:8099/browser/
cargo run --release -p shell --example phase9_probe -- http://127.0.0.1:8099/
cargo test --workspace
cargo test --workspace --no-default-features
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo audit
cargo deny check
```
