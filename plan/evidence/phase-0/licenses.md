# Dependency license registry (living document)

Policy: `plan/02-principles-constraints.md` §2.3.

## Direct workspace dependencies (Phase 0)

| Crate | Version | License | Reason / note |
|-------|---------|---------|---------------|
| *(none)* | — | — | Phase 0 uses `std` only. Zero external dependencies. |

## Direct workspace dependencies added in Phase 1 (2026-10-04)

All permissive; no copyleft exception needed.

| Crate | Version | License |
|-------|---------|---------|
| url | 2.5.8 | MIT OR Apache-2.0 |
| tokio | 1.53.2 | MIT |
| reqwest | 0.12.28 | MIT OR Apache-2.0 |
| futures | 0.3.34 | MIT OR Apache-2.0 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.151 | MIT OR Apache-2.0 |
| flate2 (dev) | 1.x | MIT OR Apache-2.0 |
| rustls (dev, tests) | 0.23 | Apache-2.0 OR ISC OR MIT |
| winit | 0.30.13 | Apache-2.0 |
| softbuffer | 0.4.8 | MIT OR Apache-2.0 |
| cosmic-text | 0.12.1 | MIT OR Apache-2.0 |

Transitive note: `self_cell` 1.3.0 (via cosmic-text) is dual `Apache-2.0 OR GPL-2.0-only` — used under Apache-2.0, no exception required. Full-tree scan on 2026-10-04 found no GPL-only crates in `browser/Cargo.lock`.

## Direct workspace dependencies added in Phase 2 (2026-10-04)

| Crate | Version | License | Note |
|-------|---------|---------|------|
| encoding_rs | 0.8 | MIT OR Apache-2.0 | Mandated by plan/05 §5.2 |
| criterion | 0.5 | MIT OR Apache-2.0 | dev-dependency (benches only) |
| libfuzzer-sys | 0.4 | MIT OR Apache-2.0 | fuzz harness only, nightly, never in the binary |

## Phase 3 dependency additions and first actual deny run (2026-10-05)

| Dependency / data | License | Use |
|---|---|---|
| woff2-patched 0.4.0 | Apache-2.0 | Rust WOFF2 auxiliary decoder, not a web engine |
| brotli 7.0.0 | BSD-3-Clause OR MIT | Bounded preflight before WOFF2 reconstruction |
| base64 0.22.1 | MIT OR Apache-2.0 | Font data-URL decode |
| percent-encoding 2.3.2 | MIT OR Apache-2.0 | Font data-URL decode |
| slotmap 1.1.1 (existing transitive) | Zlib | Font database arena |
| webpki-roots 1.0.9 (existing transitive data) | CDLA-Permissive-2.0 | Mozilla certificate-root data |
| WPT test inputs | BSD-3-Clause | Adapted CSS conformance cases; retained license under css/tests/wpt |

Other additions reuse existing approved crates (`url`, `encoding_rs`, `flate2`,
`libfuzzer-sys`). No GPL/AGPL-only product dependency was introduced.

The first actual cargo-deny run required allowing the two existing permissive
licenses above and adding explicit local path dependency versions. Maintenance
notices `RUSTSEC-2024-0436`, `RUSTSEC-2026-0206`, `RUSTSEC-2026-0192` have narrowly
documented maintenance exceptions, with replacement review tracked for Phase 11.
Vulnerability advisories remain enforced. cargo-audit and cargo-deny now run
locally and pass; maintenance-only cargo-audit warnings are recorded in Phase 3.

## Phase 4 auxiliary dependencies (2026-10-05)

| Dependency | License | Use |
|---|---|---|
| wgpu 24.0.5 | MIT OR Apache-2.0 | GPU window/compositing backend |
| image 0.25.8 | MIT OR Apache-2.0 | Bounded PNG/JPEG/GIF/WebP decoding and test PNG encoding |
| resvg/usvg 0.45.1 | MIT OR Apache-2.0 | SVG-as-image auxiliary renderer, not a web engine |
| roxmltree 0.20.0 | MIT OR Apache-2.0 | SVG resource-budget preflight before geometry expansion |
| bytemuck 1.25.2 | MIT OR Apache-2.0 OR Zlib | Typed GPU vertex uploads |
| pollster 0.4.0 | MIT OR Apache-2.0 | GPU initialization on the dedicated setup thread |
| unicode-bidi 0.3.18 | MIT OR Apache-2.0 | Rust bidi ordering of inline flow |

No copyleft-only addition. Other direct dependencies reuse approved crates.
SVG font fallback adds maintained-policy notices for the newer transitive
rustybuzz/ttf-parser versions under the same documented maintenance exceptions.
No vulnerability exemption was added; cargo-audit/cargo-deny remain enforced.

## Phase 5 language/policy auxiliaries and test data (2026-10-09)

| Dependency / data | License | Use |
|---|---|---|
| regex 1.13.1 | MIT OR Apache-2.0 | Bounded regex auxiliary; JS parsing/evaluation remain owned |
| time 0.3.55 | MIT OR Apache-2.0 | Date/calendar/ISO auxiliary, promoted from existing dependency tree |
| serde_json 1.0.151 | MIT OR Apache-2.0 | Approved JSON auxiliary reused in the own interpreter |
| sha2 0.10.9 | MIT OR Apache-2.0 | CSP SHA-256/384/512 hashing; no own cryptography |
| wayland-client 0.31.15 (dev) | MIT | Explicit GUI-test pointer; existing winit dependency reused |
| wayland-protocols-wlr 0.3.12 (dev) | MIT | Virtual-pointer test bindings; not a product input service |
| Test262 adapted inputs | BSD-3-Clause | Eight sources; notices/license under js/tests/test262 |
| WPT DOM/events adapted inputs | BSD-3-Clause | Notices/license/provenance under dom_bindings/tests/wpt |

Other additions reuse approved `thiserror`, `url`, `base64`, Criterion and
libfuzzer-sys. No external JS engine or copyleft-only dependency was added.
The resolved workspace has 499 dependencies; cargo-audit reports no vulnerabilities
and cargo-deny passes, with the existing maintenance-only exceptions retained.

## Copyleft exception registry

| Crate | Version | License | Anti-overengineering justification |
|-------|---------|---------|------------------------------------|
| *(none)* | — | — | No exceptions in Phase 0–1. |

## Verification

- `cargo deny` config in `browser/deny.toml`; Phase 3 actual check passes. Earlier phases had only the manual scan below.
- Manual substitute: direct licenses read from registry manifests; transitive scan over the resolved tree.
