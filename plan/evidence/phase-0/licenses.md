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

## Copyleft exceptions (GPL/AGPL/LGPL)

| Crate | Version | License | Anti-overengineering justification |
|-------|---------|---------|------------------------------------|
| *(none)* | — | — | No exceptions in Phase 0–1. |

## Verification

- `cargo deny` config in `browser/deny.toml` (blocked: `cargo-deny`/`cargo-audit` binaries unavailable in this environment; first CI run pending).
- Manual substitute: direct licenses read from registry manifests; transitive scan over the resolved tree.
