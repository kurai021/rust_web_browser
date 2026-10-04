# 15 — Aggressive optimization: Rust-first + SIMD/asm/GPU

> Doctrine: Rust first, asm last, features intact. No optimization may change web semantics or cut compatibility.

## 15.1 Principle (non-negotiable)

1. **Correct → portable-fast → SIMD → asm.** Every kernel first exists as correct, tested pure Rust. It moves one level down only when `criterion` + `perf` prove a real bottleneck.
2. **No feature sacrifices:** the optimized path and the pure fallback pass exactly the same tests (WPT/fuzz/golden). On divergence, the optimized path is disabled by flag.
3. **No broken CPUs or portability:** primary targets `x86_64` + `aarch64` Linux desktop. All SIMD/asm with runtime dispatch and fallback. Compiling the release binary with `-C target-cpu=native` is forbidden.
4. **Isolated and auditable:** each kernel lives in its own `*_simd.rs` / `*_arch.rs` module, minimal `unsafe`, documented, listed in §15.4. Scattered asm forbidden.

Technical preference order:

1. Portable Rust + algorithms (a better algorithm beats any asm).
2. Already-optimized `std` (`memchr`, `str::`, `slice::`, `image`, `brotli`, …).
3. Portable `core::arch` intrinsics (`SSE4.2`/`AVX2`, `NEON`) with dispatch.
4. Inline `core::arch::asm!` only with proof that LLVM cannot generate the wanted code (e.g. VM dispatch, carry-less; `rdrand` excluded — forbidden for fingerprinting).
5. GPU (`wgpu` WGSL/compute) for paint/composite, not CPU asm.

## 15.2 Where it IS allowed (closed list)

Only these kernels may grow a SIMD/asm/GPU path. Anything else requires editing this doc:

| # | Crate | Kernel | Allowed technique | Why it pays off |
|---|-------|--------|-------------------|-----------------|
| K1 | `html` | tokenizer scan (`<`, `&`, `-->`, whitespace, entities) | SIMD `memchr`-style AVX2/NEON, `memchr` crate fallback | O(n) over every HTML byte |
| K2 | `css` + `adblock` | substring matching / Aho-Corasick, host hashing, `||domain^` | SIMD + hash (SipHash/`ahash` local only, no fingerprinting) | thousands of rules × every request |
| K3 | `js` | bytecode dispatch, number/string parsing, property-lookup IC, GC mark/sweep scan | `asm!` computed-goto only if the VM justifies it, else switch + intrinsics | interpreter hot loop |
| K4 | `paint` | alpha blending, fillRect, blur/shadow, sRGB↔linear, bilinear scaling | WGSL GPU first; CPU SIMD for the `softbuffer` fallback | per pixel, per frame |
| K5 | images/text | PNG filters, JPEG upsampling, WebP decode, glyph raster, no bidi (bidi stays in Rust) | reuse SIMD crates (`image`, `libjpeg-turbo` bindings, `rustybuzz`/`swash`; GPL allowed only as a registered exception per doc 02) | heavy decode off the UI |
| K6 | `net` | gzip/br/zstd, cache hashing, base64/data-url | reuse crates (they already ship asm). Reimplementing forbidden | network throughput |
| K7 | `layout` | flex/grid numeric reductions (sum/clamp) | Rust autovectorization, intrinsics only with a bench | marginal; algorithms rule |

Explicitly forbidden: own TLS/crypto, own DNS in asm, `rdrand`/`rdtsc` for fingerprinting, JIT with asm codegen in v1 (doc 08 excludes it).

## 15.3 How it is implemented (mandatory per-kernel template)

Fixed layout:

```
crates/<crate>/src/
  k1_scan.rs          # pure-Rust public API + dispatch
  k1_scan_scalar.rs   # portable fallback (always compiles)
  k1_scan_simd.rs     # #[cfg(target_arch="x86_64"/"aarch64")] + minimal unsafe
  k1_bench.rs         # criterion bench comparing scalar vs simd
```

```rust
// Mandatory dispatch, example:
pub fn scan_lt_amp(haystack: &[u8]) -> Option<usize> {
  #[cfg(target_arch = "x86_64")] {
    if std::arch::is_x86_feature_detected!("avx2") {
      unsafe { return k1_avx2(haystack); }
    }
  }
  #[cfg(target_arch = "aarch64")] {
    if std::arch::is_aarch64_feature_detected!("neon") {
      unsafe { return k1_neon(haystack); }
    }
  }
  k1_scalar(haystack)
}
```

Rules:

- `unsafe` only inside `*_simd.rs`, minimal functions, documented `# Safety:` (alignment, length, aliasing).
- No `asm!` without a `// asm-justification: <perf report + objdump diff>` comment + a bench showing >15% over intrinsics.
- Release `RUSTFLAGS`: `-C opt-level=3 -C lto=thin -C codegen-units=16 -C panic=unwind` (pinned in Phase 0). No `native`.
- Cargo feature flags: `simd` (default-on), `no-simd` (force the fallback for tests/audits). `cargo test --no-default-features` must pass.
- WGSL shaders versioned under `crates/paint/shaders/*`, with identical GPU-vs-CPU display-list snapshots.

## 15.4 Kernel registry (initially empty, filled in as we optimize)

| Kernel | File | Archs | Measured gain | Bench | Status |
|--------|------|-------|---------------|-------|--------|
| (ex) K1 | `crates/html/src/scan_simd.rs` | x86_64 AVX2, aarch64 NEON | pending Phase 2 bench | `k1_bench` | proposed |
| … | | | | | |

Rule: no row here + bench + green tests means the kernel does not exist.

## 15.5 Quality gates (per-kernel DoD)

A kernel merges only when:

- [ ] A pure scalar fallback exists, compiles under `no-simd`, and passes all crate tests.
- [ ] `cargo test -p <crate>` + 10-min fuzz + affected WPT subset identical scalar vs simd (`test_utils` comparator).
- [ ] `cargo bench` shows ≥15% improvement or −15% memory/energy on average hardware, with no >5% regression in other crate benches. Report in `plan/evidence/<phase>/simd-<kernel>.md`.
- [ ] `cargo clippy`, `miri` on scalar paths (miri cannot run asm; the scalar path is tested under miri), green `cargo audit`/`deny`.
- [ ] `SAFETY` + `asm-justification` documented. Explicit review of alignment/endianness/panics on hostile input (fuzz with forced CPU features).
- [ ] Kill-switch flag: `SIMD=0` env or `--no-simd` disables the kernel at runtime for debugging.

## 15.6 When it happens (no dates, tied to the roadmap)

- Phases 0–3: asm forbidden except reused K6 (crates already ship it). Correctness focus.
- Phase 4 on: K1/K4/K5 if benches demand it.
- Phase 8 on: K3 (hot bytecode VM).
- Phase 10: K2 (adblock at real-list scale).
- Phase 11: final perf audit + `perf stat` + first published "scalar vs simd" report.

No phase is blocked by missing SIMD; SIMD accelerates, never enables features.
