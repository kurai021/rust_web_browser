# 04 — Networking and HTTPS

> Scope: reuse TCP/TLS/DNS. Build URL/fetch/HTTP-semantics/cache/cookies/HSTS.

## 4.1 Goal

Load `http://` and `https://` correctly, safely and without blocking the UI — enough for daily-driver use. Everything else (HTML/CSS/JS) consumes bytes from this stack.

## 4.2 What is NOT implemented

- No hand-rolled TCP, IP, TLS, crypto or DNS. `tokio::net`, `rustls` and `hickory-resolver` / the system resolver are used.
- No FTP/Gopher/etc. A non-http/https URL shows an "unsupported scheme" error. No exceptions.
- No HTTP/3 QUIC in v1 (design so it can be added later, but v1 = HTTP/1.1 + HTTP/2 + TLS 1.2/1.3).

## 4.3 Functional spec (`net` crate)

### 4.3.1 URL

- Use the `url` crate (WHATWG URL). Support: parsing, normalization, punycode/IDNA, default ports, userinfo (never sent to the UI without warning), fragments (never sent to the server), `file://` for local tests only.
- Omnibox: tell URL apart from search query. Rule: if it parses as a URL with a dotted host or `localhost` or an IP → navigate; otherwise → search (configured engine, DuckDuckGo by default or configurable, no telemetry).

### 4.3.2 Fetch / HTTP

- Methods: GET, POST (forms), HEAD. PUT/DELETE/PATCH only via the `fetch()` API, never navigation.
- Versions: HTTP/1.1 mandatory, HTTP/2 mandatory via ALPN (`h2`). Correct negotiation and fallback.
- Headers: `Host`, `User-Agent` (fixed documented value, no extra fingerprint), `Accept`, `Accept-Encoding: gzip, br, zstd` (if decoders are enabled), `Accept-Language` (configurable), `Referer`/`Origin` per policy (strict-origin-when-cross-origin by default), `Cookie`, `If-None-Match`/`If-Modified-Since` (cache), `Range` (resumable downloads).
- Redirects: follow 301/302/303/307/308 up to 10 hops. 303 → GET. Preserve the method on 307/308. Block http→https downgrade redirects? No: allow but flag "not secure" if it lands on http. Ask before re-sending POST on redirect (or re-send only on 307/308 with confirmation in advanced UI; v1: never re-send POST automatically except 303→GET).
- Compression: `gzip`, `br`, `zstd`, `deflate`. Streaming decompression with limits (zip bombs: cap decompressed size, e.g. 10x or 100 MB max per document).
- Timeouts: connect 10s, TLS 10s, TTFB 15s, 60s total per request (configurable). Cancellation when the tab closes.
- Concurrency: per-origin connection pool (6 http/1.1 connections per host, multiplexed on h2), global cap (e.g. 30). Priority queue (document > css > sync js > images).
- Size caps: HTML document 10 MB (then a visible truncation error), resource 50 MB, downloads uncapped except by disk (see doc 11).

### 4.3.3 TLS / HTTPS

- `rustls` + `webpki` only, system roots + `webpki-roots` bundle. TLS 1.2+ (1.0/1.1 forbidden). rustls default secure cipher suites.
- Strict verification: hostname, dates, chain. Visible error with a code (`CERT_EXPIRED`, `SELF_SIGNED`, …) and a "go back" button, no one-click bypass (bypass only via advanced `about:config` + per-site confirmation, documented).
- HSTS: cached in the profile, honor `max-age`, `includeSubDomains`, minimal embedded preload list (small top list, updatable with the adblock lists if decided, but no hidden fetch).
- Mixed content: block active (scripts/fetch) http on https pages; warn on passive (img/video) with a per-site "load anyway" option.
- Upgrade: if the user types `example.com` → try `https://` first, fall back to `http://` only if https fails at connect (not on cert errors).

### 4.3.4 Cookies, cache, network storage

- Cookies: RFC 6265 + `SameSite=Lax` by default, `Secure` required on https for `__Secure-`, per-site partitioning for 3rd-party (block 3rd-party by default except an allowlist; see doc 11).
- HTTP cache: RFC 9111 memory (50–200 MB) + disk (500 MB) with `Cache-Control`, `ETag`, `Last-Modified`, `Vary`. "Reload" mode (Ctrl+R) = revalidate; "force" (Ctrl+Shift+R) = bypass.
- HSTS + no HPKP (HPKP discarded), no Expect-CT.

### 4.3.5 Adblock and privacy integration

- Before issuing a request, ask `crates/adblock`: if blocked → cancel with no DNS/TCP (zero bytes). Per-page counter.
- DNS: system resolver by default; DoH as explicit opt-in (configurable Cloudflare/Quad9), never by default, to avoid "connections behind your back".
- Minimized `User-Agent` and fingerprinting headers. No `Client-Hints` in v1 except a disabled-by-default minimal `Sec-CH-UA`.

## 4.4 v1 network non-goals

- No auto proxy (PAC/WPAD) except a user-configured manual HTTP/SOCKS5 proxy.
- No WebSockets? They ARE needed for minimal daily-driver use: WS/WSS (RFC 6455) lands in the Web APIs phase (doc 09), not in this doc. Dependency noted.
- No HTTP/3, no DoQ, no ECH in v1 (leave hooks).
- No full offline-first service workers in v1 (manual cache is enough).

## 4.5 Acceptance criteria

- [ ] `https://example.com`, redirects, gzip/br, cookies, HSTS pass tests with a local server + badssl.com cases (expired/wrong-host/self-signed shown, no crashes).
- [ ] Closing a tab cancels sockets. No leaks (test with 100 aborted requests).
- [ ] Adblock blocks before connecting (verified via counter + no socket).
- [ ] URL parser fuzzed with no panics. Decompression limits verified.
