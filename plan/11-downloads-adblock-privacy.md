# 11 — Downloads, adblock and privacy

> `shell(profile/downloads)` + `adblock` crates. Part of "daily-driver + no telemetry".

## 11.1 Downloads

Requirements:

- Triggered by: navigation to a non-renderable MIME (`Content-Disposition: attachment` or unsupported type), `download` attribute, "Save as", fetch→blob saved via UI.
- Dialog: save to the configured path or always ask (default: ask when no path is set). Sanitized filename (no path traversal), deduplication (`file (1).ext`).
- Engine: streaming to disk (not RAM), progress via `Content-Length` or chunked, speed/ETA, pause (with `Range` where the server supports it, otherwise an announced restart), retry (3x with backoff), cancel + partial cleanup, size/hash check when the server provides `Content-MD5`/`Digest` (optional).
- Security: MIME sniffing (never execute), quarantine xattr on Linux where applicable, executable warning (`.sh`/`.AppImage`/`.deb` → "this may harm your computer"), no auto-open by default.
- List persisted in the profile (download history, not the bytes). Concurrent cap of 5, rest queued.

Not included: torrent/magnet, mirror/P2P download managers, downloader extensions.

## 11.2 Built-in blocker (native, not an extension)

Requirements:

- Public lists: **EasyList + EasyPrivacy** by default, **uBlock Filters** + **Peter Lowe** optional. Format: ABP (`||domain^`, `##cosmetic`, `@@exception`, …) + uBO extended (documented subset: cosmetic + basic scriptlet-blocking, no arbitrary JS injection from lists in v1 for safety).
- Own `adblock` engine or an audited crate (Brave's `adblock` as reference, but if used it must be an audited telemetry-free dependency; otherwise implement it here per this spec). Must block **before connecting** (hook in `net`), plus hide elements (`##`) via injected UA CSS and block scripts by URL.
- Cosmetic filtering: apply `##.ads` rules as an extra high-priority stylesheet, never exposed to the page.
- Per-page counter + "blocked" panel + per-site toggle (disabling → reload). The counter proves it works.
- Updater: fetches lists over HTTPS from official URLs, verification (hash/ETag), 48h interval by default, manual + disableable. **The only background connection allowed** besides what pages request, and it must be visible in settings + logs. No silent updater that cannot be turned off.
- Basic anti-circumvention: CNAME-cloaking detected via the local resolver when the internal DoH is active (late phase, document the limit).

Not included: advanced visual element picker, scriptlets with arbitrary JS, cloud sync of custom rules. User custom rules: yes (local `user.txt` list).

## 11.3 Anti-telemetry (hard rule)

Forbidden in the binary:

- Crash reports, metrics, experiments, "first run" pings, browser update checks except explicit opt-in, real-time remote safe-browsing (if phishing protection is ever wanted → a locally downloaded list like the adblock one, never a remote per-URL lookup in v1).
- Any connection at startup with `about:blank` = P0 bug. Automated test (doc 13) fails the build on an unexpected socket.
- Short fixed `User-Agent`, no unique build-id, no complex canvas-fingerprint mitigations in v1 except basic `privacy.resistFingerprinting` (disabling `canvas.readback` entropy? document as future).

## 11.4 Acceptance criteria

- [ ] 100 MB download with pause/resume + cancel and no FD leaks.
- [ ] With EasyList, a demo page with ads/trackers shows 0 requests to those hosts + hidden elements + counter > 0.
- [ ] Sniffer/test: 0 connections with a clean profile + `about:blank` + updater off.
- [ ] Per-site toggle + custom rules survive restarts.
