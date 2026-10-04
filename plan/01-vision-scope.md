# 01 — Vision and scope

## 1.1 What will be built

A general-purpose graphical web browser, written entirely in Rust, with in-house engines for:

1. **HTML5** (parsing and DOM tree).
2. **CSS** (parsing, cascade, layout and rendering).
3. **JavaScript** (in-house engine: parsing and execution).

TCP/IP and TLS are NOT built from scratch. Existing Rust libraries are reused for sockets/TCP/TLS/DNS (see `04-network-https.md`).

Phased project, no committed dates. Each phase is a span of the bridge: it must leave something functional and integrable, not a throwaway prototype.

## 1.2 Definition of "non-experimental"

The project is successful only if it meets **all** of these criteria:

- [ ] Loads "virtually any everyday web page" with no manual intervention: news, blogs, docs, basic e-commerce, simple SPAs, forms, login, progressive HTML5 `<video>`.
- [ ] Does not require being handed "a specific link or a concrete use case" to work. The user can paste any HTTPS URL and browse, click, go back/forward, reload, open tabs.
- [ ] Supports the inseparable trio: HTML5 + CSS (incl. flex/grid in an advanced phase) + JS (DOM + fetch + events + timers).
- [ ] HTTPS works, including redirects, basic HSTS, and TLS errors shown to the user.
- [ ] File downloads work (see doc 11).
- [ ] Built-in ad blocker enabled by default with public lists (see doc 11).
- [ ] Graphical. Window with tabs and address bar. No console, no TUI, no headless-as-product (headless only as a test tool).

If any of them fails, it is still experimental.

## 1.3 Included scope (closed list)

Only this. Nothing else is assumed included:

1. `http://` and `https://` schemes. Redirects between them. No other navigable schemes.
2. HTML5: WHATWG-conformant parsing, JS-manipulable DOM.
3. CSS: external sheets, `<style>`, inline `style=""`. Cascade, inheritance, specificity. Box model, flow (block/inline), basic positioning, flexbox, grid (these two in an advanced phase, but committed scope).
4. JS + minimal DOM + BOM + `fetch`/`XMLHttpRequest`, timers, events, `localStorage`/`sessionStorage`, `history`, `location`, minimal `navigator`.
5. Media: text (ttf/woff/woff2), images `png/jpeg/gif/webp/svg` (svg as image, not necessarily full SVG DOM in v1), progressive audio/video via `<audio>/<video>` with OS decoders or Rust crates (no DRM).
6. Navigation: tabs/processes, session history, minimal bookmarks, omnibox (URL + search; configurable search engine, no hardcoded telemetry).
7. Downloads: list, pause/cancel/retry, save-to-disk chosen by the user.
8. Integrated adblock with EasyList / EasyPrivacy / uBlock format (see doc 11).
9. No telemetry, no background connections to external servers except: (a) what the page requests, (b) explicit block-list updates, (c) self-updates if configured. All disableable.
10. Minimal DevTools in a late phase: DOM/style inspector + JS console. Not an early priority.

## 1.4 Explicit non-scope (blocked list)

To keep the project from bloating or the AI from "assuming" features, this is **forbidden** unless this doc is explicitly edited:

1. **No** FTP, Gopher, Gemini, NNTP, Finger or other legacy/alternative protocols. Only http/https. `file://` for local tests only, not as a general browsing feature in v1.
2. **No** BitTorrent, IPFS, magnet links or P2P.
3. **No** DRM (EME/CDM/Widevine/PlayReady/FairPlay). DRM-gated Netflix/Spotify are not a target. Non-DRM video only.
4. **No** third-party plugin / Chrome/Firefox-style extension support in v1. No NPAPI/PPAPI/WebExtensions. The adblocker is native, not an extension.
5. **No** crypto stuff: no wallet, no Web3, no dApps, no blockchain protocols.
6. **No** telemetry, automatic crash reports, remote experiments, cloud accounts/sync, or "services connecting behind your back".
7. **No** email client, full RSS reader, or IDE. At most, RSS/Atom renders as readable XML, not as a reader.
8. **No** console mode as a product. Headless for tests only.
9. **No** in-house PDF viewer in v1 (downloaded as a file; inline viewing is an optional future phase).
10. **No** WebRTC, advanced WebGL, WebGPU, Bluetooth, USB, NFC, sensors in v1. Documented as "discarded for now".

Any PR/issue asking for something on this list is rejected by default citing this file.

## 1.5 Daily-driver criterion

"The browser you would choose for your day to day" is operationalized as:

- Startup < 2s on average hardware, contained memory (goal: < 500 MB with 5 typical tabs when mature).
- Navigation never blocks the UI: network, parsing, layout and JS never freeze the window.
- Isolated failures: one broken tab does not take down the browser.
- Privacy by default: block known trackers/ads, no background calls.
