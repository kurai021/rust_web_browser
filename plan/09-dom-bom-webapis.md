# 09 — DOM, BOM and Web APIs

> `dom_bindings` + `web_api` crates. Defines what a page may touch. Closed list: if it is not here, JS does not expose it.

## 9.1 Principle

Expose the minimum that makes "virtually any everyday page" work, with real-web semantics. No exotic APIs in v1.

## 9.2 DOM Core (Level 1)

- `Node`: `nodeType`/`nodeName`/`childNodes`/`parentNode`/`firstChild`/`lastChild`/`nextSibling`/`previousSibling`/`appendChild`/`removeChild`/`insertBefore`/`replaceChild`/`cloneNode`/`contains`/`textContent`.
- `Document`: `createElement`/`createTextNode`/`createDocumentFragment`/`getElementById`/`getElementsByClassName`/`getElementsByTagName`/`querySelector`/`querySelectorAll`/`documentElement`/`head`/`body`/`title`/`referrer`/`domain`/`readyState`/`visibilityState`.
- `Element`: `tagName`/`id`/`className`/`classList`/`attributes`/`getAttribute`/`setAttribute`/`removeAttribute`/`hasAttribute`/`matches`/`closest`/`innerHTML`/`outerHTML`/`textContent`/`insertAdjacentHTML`/`querySelector*`, `addEventListener`/`removeEventListener`/`dispatchEvent`.
- Basic `HTML*Element`: `HTMLAnchorElement.href`, `HTMLFormElement.submit`/`reset`, `HTMLInputElement.value`/`checked`/`type`, `HTMLScriptElement.src`/`async`/`defer`, `HTMLImageElement.src`/`alt`, `HTMLLinkElement`, `HTMLStyleElement`, `HTMLTemplateElement`, `HTMLVideo`/`AudioElement` (play/pause/currentTime in a middle phase).
- `Text`/`Comment`/`DocumentFragment`, basic `Range` (for `innerHTML` tests), `DOMParser` (`text/html` subset).
- Basic `MutationObserver`, simplified `NodeIterator`/`TreeWalker`.
- CSSOM: `getComputedStyle`, `CSSStyleDeclaration` (set/get/removeProperty), `matchMedia`, `CSSStyleSheet` (limited insertRule/deleteRule).

## 9.3 Events

- `Event`/`EventTarget`: capture/bubble, `stopPropagation`/`preventDefault`, `CustomEvent`, `MouseEvent`/`KeyboardEvent`/`FocusEvent`/`InputEvent`/`SubmitEvent`/`HashChangeEvent`/`PopStateEvent`.
- Delegation, `once`/`passive`/`capture` options. `click`/`focus`/`blur`/`input`/`change`/`submit`/`keydown`/`keypress`/`keyup`/`mousemove`/`DOMContentLoaded`/`load`/`unload`/`beforeunload`/`hashchange`/`popstate`.
- `requestAnimationFrame` synced to the compositor VSync.

## 9.4 Committed BOM + Web APIs

- `window`: `location`/`history`/`navigator`/`document`/`localStorage`/`sessionStorage`/`fetch`/`setTimeout`/`setInterval`/`open`? (`window.open` → new tab, with popup blocker), `alert`/`confirm`/`prompt` (per-tab blocking native dialogs, never a global freeze), `getSelection`, `devicePixelRatio`/`innerWidth`/`innerHeight`.
- `location`: `href`/`assign`/`replace`/`reload`/`hash`/`search`. `history`: `pushState`/`replaceState`/`back`/`forward`/`go`.
- `navigator`: `userAgent`/`language`/`languages`/`onLine`/`cookieEnabled` (minimal, no extra fingerprint).
- `fetch` + `XMLHttpRequest` (same CORS model), `Headers`/`Request`/`Response`, `FormData`/`URLSearchParams`/`Blob`/basic `FileReader` (for uploads), `WebSocket` (ws/wss), basic `EventSource` (SSE).
- Storage: `localStorage`/`sessionStorage` (5 MB/origin, synced to the profile), `document.cookie` (via the net jar), `IndexedDB`? **Yes, committed but late phase** (subset: object stores, get/put, basic indexes). No daily-driver is complete without IndexedDB.
- Workers: basic dedicated `Worker` (no full SharedWorker/ServiceWorker in v1; ServiceWorker only as optional future minimal offline cache).
- Basic `IntersectionObserver`/`ResizeObserver`/`MutationObserver` (needed for modern lazy-loading).
- `Canvas2D`? **Yes, committed subset** (basic `fillRect`/`drawImage`/`fillText`/paths) because many sites use it for captchas/charts. WebGL/WebGPU out of v1.
- Clipboard (`readText`/`writeText` gated on user gesture), `Notifications`? Permission-gated only, no push in v1. Geolocation? Permission-gated only with a local provider (no Google MLS by default; with no provider → "unavailable" error, no hidden call).

## 9.5 Explicitly NOT in v1

- Full ServiceWorkers, Push API, Background Sync, WebRTC, WebGL/WebGPU, WebBluetooth/USB/NFC/HID/Serial, WebAuthn (except future OS passthrough), Payment Request, advanced Credential Management, full File System Access, EME/DRM, WebXR, Speech, MIDI, advanced Gamepad (basic optional).
- WebExtensions. `chrome.*`/`browser.*` do not exist.

## 9.6 Security/CORS/SOP (summary, details in doc 12)

- Strict Same-Origin Policy. CORS: `Origin`, `OPTIONS` preflight, `Access-Control-Allow-*`. `file://` = opaque origin.
- Basic CSP (`default-src`/`script-src`/`style-src`/`connect-src`/`img-src`, simple `nonce`/`hash`). `SameSite` cookies + 3rd-party partitioning.

## 9.7 Acceptance criteria

- [ ] Forms, login, JSON `fetch`, `pushState` SPA demo and `localStorage` survive restarts.
- [ ] WPT `dom`, `fetch`, `xhr`, `events` subsets with measured pass rates.
- [ ] Popup blocker + permissions (clipboard/geo/notifications) always ask, never silent.
