# Phase 4 final fuzz campaigns

Linux aarch64, nightly toolchain, libFuzzer/ASan. Inputs are generated into the
ignored corpus directory; source seeds and standalone locks are retained.
`ASAN_OPTIONS=quarantine_size_mb=16`, 3-second per-input deadline and 1536 MiB
RSS guard. No generated corpus/artifact/build file belongs in the commit.

## Final HTML streaming campaign

```sh
ASAN_OPTIONS=quarantine_size_mb=16 cargo +nightly fuzz run parse fuzz/corpus/parse fuzz/seeds -- -max_total_time=300 -max_len=8192 -timeout=3 -rss_limit_mb=1536
```

```text
#2198718 DONE cov: 6015 ft: 30237 corp: 8344/2361Kb lim: 1041 exec/s: 7304 rss: 215Mb
Done 2198718 runs in 301 second(s)
```

**2,198,718 executions**, five minutes complete, **215 MiB peak**, no final-run
panic/crash/timeout/OOM. The existing target splits each input into two pushes;
explicit regression tests additionally cover 1/2/3/7/31/1024-byte chunk sizes.

Earlier campaigns exposed two non-progress recovery loops and a stale
adoption-agency index. They were fixed, replayed and regression-tested before
the clean full campaign. Adoption input was minimized to 74 bytes; its byte
literal is retained in `html/tests/progressive.rs`. Recovery seeds are under
`html/fuzz/seeds/`.

## Final bounded image/SVG campaign

```sh
ASAN_OPTIONS=quarantine_size_mb=16 cargo +nightly fuzz run image_decode fuzz/corpus/image_decode fuzz/seeds -- -max_total_time=300 -max_len=4096 -timeout=3 -rss_limit_mb=1536
```

```text
#1331805 DONE cov: 12086 ft: 46271 corp: 4163/1623Kb lim: 4096 exec/s: 4424 rss: 313Mb
Done 1331805 runs in 301 second(s)
```

**1,331,805 executions**, five minutes complete, **313 MiB peak**, no final-run
panic/crash/timeout/OOM. Inputs exercise raw decoding or a `base64:` seed wrapper
for binary raster fixtures; mutations of valid PNG/GIF and SVG/budget inputs
are included. PNG/JPEG/GIF/WebP format roundtrip/decode tests separately pass.

An earlier SVG input with extreme arc radii caused a timeout in the auxiliary
`svgtypes::SimplifyingPathParser` cubic expansion. Numeric geometry/depth/node
preflight now rejects it before usvg/resvg expansion. `arc-budget.svg` and a
decoder regression retain the failure class. An intermediate repaired campaign
also completed 3,463,738 executions at 225 MiB; the final run above covers the
latest source, including UTF-8-independent SVG sniffing.

CI runs HTML/CSS/image five-minute smoke campaigns on push/PR and one-hour
campaigns nightly. Decoder work remains bounded and isolated from the UI thread.
