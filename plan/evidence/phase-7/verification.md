# Phase 7 quality verification

Commands ran in `browser/`, Linux aarch64 / stable Rust 1.99. Declared MSRV 1.85
was not separately exercised. All results below use the patched rustls 0.23.46
workspace lock and the final API/modal-wait code.

| Command / check | Result |
|---|---|
| `cargo test --workspace --all-targets` | 227 passed, 8 ignored; example/benchmark smoke passes |
| `cargo test --workspace --no-default-features` | 227 passed, 8 ignored; doc tests pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo fmt --all --check` | Passed |
| `cargo audit` | No vulnerabilities; six existing maintenance notices |
| `cargo deny --log-level error check` | Advisories/bans/licenses/sources pass |
| `cargo build --release -p browser` | Passed |
| Explicit GPU/software display-list parity | Passed on Apple M2 Pro Vulkan |
| Explicit JS/Node differential | 27 representative results match |
| `web_api` adapted WPT runner | 40 pass, 0 fail, 17 explicit skips; JSON archived |
| Final sanitized API fuzz run | 91,137 executions / 301 seconds; no failure artifact |
| Comparable layout Criterion suite | No >20% regression |
| `git diff --check` | Passed |

The eight normal ignores are the five existing online checks, optional WOFF2,
Node oracle and GPU adapter check. The last two were also run explicitly. WPT
case-level skips are separately inventoried in the archived JSON and are not
additional ignored Rust tests.

## Live fixture and restart

With the explicit loopback demo server running:

```sh
cargo run -p shell --example phase7_probe -- http://127.0.0.1:8097/ /tmp/opencode/phase7-probe-profile
cargo run -p shell --example phase7_probe -- http://127.0.0.1:8097/ /tmp/opencode/phase7-probe-profile
target/release/browser --headless-test http://127.0.0.1:8097/ --profile-dir /tmp/opencode/phase7-headless-final --perf
```

Both probes pass fetch, worker, WebSocket, SSE, IDB, mutation/resize/intersection,
canvas/image/text/paths, narrow card bounds, SPA history, login and another live
refresh. The repeated profile shows visits 2 then 3 and paint hash
`d96deeb477d835aa`. The final CLI reports status 200, 1 sheet, 34 article blocks,
0 warnings, 71 boxes, 52 lines, 1055 glyphs, 16,384 decoded image bytes and hash
`35af872c9d23ba9d`. Layout/preparation was 18.318 ms and software paint 8.751 ms;
the 2.474 s total includes the explicit bounded async test settlement.

GUI restart independently retains the authenticated HttpOnly session and local
visits while session storage remains window-local. Native permission rejection,
revocation and unavailable-provider behavior are captured in [gui.md](gui.md).

## Security/input-path review

* No new unsafe/assembly or input-path panic/expect/unwrap in `web_api` service,
  cookie, transport-exchange and API binding code. Mutex poison recovery uses
  unwrap_or_else rather than panicking. The internal clone decoder's object
  extraction follows only successful VM object allocations, not a page-provided
  value; reference indexes and graph sizes are validated before use.
* Cross-origin storage/history and worker construction are rejected. Fetch/XHR
  hide Set-Cookie/HttpOnly data, enforce CORS/preflight and check CSP/mixed content
  before redirect dialing. Canvas taint blocks pixel readback.
* Reused XHR instances have independent operation IDs; abort removes unsent
  commands and late completions/events cannot overwrite a newer request.
* Host accounting includes API payloads/commands/canvas/observer data. Detached
  host references and queued values participate in the existing tracing GC.
* Modal UI waiting is excluded from elapsed script CPU-task budget without
  resetting instruction/call caps or navigation cancellation.
* Profile/cookie writes use fixed Rust-owned paths, size limits, atomic rename
  and private Unix files. Session data is not serialized. Missing native
  capabilities report denial/unavailable/unsupported rather than invented data.
* Idle startup has no queued service command/task and does not connect to its
  document origin before an API invocation; there is no background updater.

The initial audit caught four newly published rustls advisories (-0346 through
-0349). Updating the compatible auxiliary to 0.23.46 cleared them; no security
exemption was introduced. Comprehensive final hardening remains the later sealed
roadmap phase, with the present compatibility boundaries explicitly recorded.
