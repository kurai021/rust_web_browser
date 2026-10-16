# Phase 7 workshop

Start the loopback API fixture from `browser/`:

```sh
cargo run -p web_api --example demo_server -- 8097
cargo run --release -- http://127.0.0.1:8097/ --profile-dir /tmp/browser-workshop --perf
```

Use `--software-render` for the matching CPU display list. A static file server
does not implement the JSON/login, WebSocket or EventSource routes. The server
binds loopback, caps requests and uses fixed synthetic credentials `demo` / `rust`;
it is a test fixture, not a production authentication service.

* Initial load fetches four rows, renders a chart and image badge, sends data to
  a dedicated worker, receives a socket echo/SSE update and stores an IDB record.
* Route links use pushState; the back button/Alt+Left runs same-document popstate.
* Sign in, restart with the same profile directory and observe the persisted
  HttpOnly session and increased localStorage visit count. Session storage is
  window-local and is cleared on close.
* Scroll/resize to observe geometry callbacks. Refresh data to exercise mutations
  and repeated worker messages in the same live realm.
* Permission buttons exercise origin-labelled native chrome. Once lasts for this
  page. `about:permissions` revokes current-origin or all saved decisions.
  Clipboard/notification backends are explicit desktop adapters; geolocation with
  no local provider returns unavailable after consent. Escape exits fullscreen.
* The native prompt suspends only the page's script thread and remains cancelable
  by navigation. Approved popups currently open an isolated window.

`cargo run -p shell --example phase7_probe -- http://127.0.0.1:8097/ /tmp/browser-probe`
asserts the live network/worker/canvas/login/history/IDB/observer path and emits a
software paint hash. It can be repeated to verify profile persistence.
