# 10 — Graphical shell (window, tabs, navigation)

> `shell` crate. The visible product. No console as a product.

## 10.1 Goal

Daily-driver graphical window: tabs, omnibox, navigation, history, bookmarks, downloads UI, page viewport (`paint` viewport), dialogs and errors.

## 10.2 Functional requirements (closed list)

1. `winit` window: viewport + chrome (toolbar, tab strip, omnibox, back/forward/reload/stop/home buttons, menu, TLS state/lock, load progress).
2. Tabs: create/close/reorder/duplicate, Ctrl+T/W/Tab, restore last session, isolated "tab crashed", favicon (fetch `link[rel=icon]` with a cap) + title.
3. Omnibox: URL vs search (doc 04 rule), autocomplete from local history/bookmarks only (no remote suggestions in v1), http="Not secure" / https=lock indicator + cert details on click.
4. Navigation: per-tab back/forward, reload (normal/forced), stop, configurable home, local error pages (`about:net-error`, `about:cert-error`) with no remote assets.
5. History: per-tab + global in the profile, local search, delete by range/site, no upload.
6. Bookmarks: bar + minimal manager, Netscape-HTML import (export too).
7. Downloads UI: panel/list with progress, pause/resume (Range), cancel, open folder, configurable "always ask where to save".
8. Per-site zoom (80%–200%), find-in-page (Ctrl+F), basic print-to-PDF (render to PDF via a `print` crate, no complex OS dialog in v1).
9. Local `about:preferences` settings: homepage, search engine, privacy (adblock, 3rd-party cookies, DoH, per-site JS on/off), appearance (light/dark, follows system), downloads path.
10. Minimal DevTools (late phase): Elements (DOM+styles), Console (JS), Network (request list with adblock blocks), no full profiler in v1.
11. Minimal CLI: `<bin> [url] [--profile-dir p] [--software-render] [--perf] [--headless-test url]` (headless for tests only, not a product).

## 10.3 v1 shell non-goals

- No third-party extensions/themes, no cloud sync, no complex multi-profile UI (only `--profile-dir`), no inline PDF reader (download), no email/RSS reader, no voice commands.

## 10.4 Fixed technical stack

- `winit` for window/events/keyboard/clipboard. `wgpu` for the viewport + self-drawn chrome (minimal coherent style, no GTK/Qt/Electron dependency). `softbuffer` fallback.
- Style: self-drawn 2D chrome (buttons/tabs/omnibox) in the early phase to avoid toolkit lock-in; evaluate `egui` for DevTools/settings only if it speeds things up, never touching the web viewport.
- Minimal accessibility: keyboard focus, tab order in chrome, documented shortcuts. Full ATK/screen-reader support is future work.

## 10.5 Acceptance criteria

- [ ] Starts in < 2s, opens 5 typical tabs with no freeze (UI stays responsive while loading).
- [ ] Back/forward/reload/stop correct, incl. SPA `pushState`.
- [ ] Closing/crashing one tab never affects the others. Session restore works.
- [ ] Zero connections at startup with a clean profile and `about:blank` homepage (verified with a sniffer/`net` test).
