# 12 — Security

## 12.1 Threat model (fixed)

Attacker: any hostile remote site/content trying to read another origin's data, execute code outside the sandbox, or hang/exfiltrate.

Assets: user cookies/storage/history, local files, local network, other tabs.

Assumptions: OS and Rust toolchain uncompromised; TLS and audited crates trustworthy; everything else (HTML/CSS/JS/images) is hostile.

## 12.2 Mandatory rules

1. **Strict SOP + CORS** (doc 09). No `disable-web-security` except a test flag that brands the window "UNSAFE".
2. **Per-tab/context isolation:** no DOM pointers shared across tabs. `postMessage` only through an origin-validated channel. Closing a tab frees the JS heap + in-memory storage.
3. **CSP enforcement** (`script-src`/`style-src`/`connect-src`/`img-src`/`object-src:none` by default when the page sends it). `eval`/inline blocked when CSP says so.
4. **Mixed content** blocked (active) / warned (passive) — doc 04.
5. **Top-level navigation:** gesture or popup-blocked API only. Gestureless `window.open` → blocked + notice.
6. **Local files:** an http/https page never reads `file://` and vice versa. Downloads never executable without confirmation. Never any access to `~/.ssh` and the like.
7. **Anti-DoS caps:** max nodes, CSS, JS time/memory, decompressed images, redirects, workers. Everything fails to an error page, never a panic.
8. **Audited `unsafe`/asm** + fuzz + `cargo audit/deny`. Licenses per the doc 02 policy (permissive by default, GPL/AGPL only as a registered exception).
9. **Future sandbox:** keep context IDs and message-passing ready for multiprocessing. In v1, per-tab `catch_unwind` + optional minimal seccomp documented as best-effort, not a guarantee.

## 12.3 Permissions (closed list)

Permissions that ask per origin and are revocable in settings: `geolocation`, `notifications`, `clipboard-read`, `camera`/`mic` (these two: deny by default with a message when there is no v1 backend, don't even ask), `fullscreen`, `persistent-storage`.

No silent grants. No "remember forever" by default (options: once / always / never).

## 12.4 v1 security non-goals

- No remote Safe Browsing, no full certificate-transparency enforcement, no FIDO2/WebAuthn, no strong per-tab OS sandbox (future). Never promise what is not implemented.

## 12.5 Acceptance criteria

- [ ] Basic SOP/CORS/XSS tests (cross-origin script cannot read another origin's DOM, cross fetch without CORS fails).
- [ ] `eval` blocked under CSP `script-src 'self'`.
- [ ] Fuzz + `cargo audit` clean in CI. `unsafe` listed and justified in this doc once implemented.
