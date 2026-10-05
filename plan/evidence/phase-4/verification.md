# Phase 4 verification record

Run from `browser/` on 2026-10-05 unless noted.

Final closure: the full/default and no-default-feature suites, explicit GPU
parity, fmt, clippy, audit and deny were rerun after the interrupted task resumed.
All passed. The completed ASan campaign logs were rechecked before closing.

```text
cargo test --workspace
  131 passed; 0 failed; 7 ignored
cargo test --workspace --no-default-features
  131 passed; 0 failed; 7 ignored
cargo clippy --workspace --all-targets -- -D warnings
  Finished dev profile; no warnings
cargo fmt --all --check
  pass
cargo build --release
  Finished release profile [optimized]
cargo deny --log-level error check
  advisories ok, bans ok, licenses ok, sources ok
cargo audit
  492 dependencies scanned; no vulnerability failures
  existing maintenance notices: paste, rustybuzz (2 versions), ttf-parser (3)
```

The same documented maintenance exceptions cover transitive versions; no
vulnerability exemption was added. Direct licensing additions are recorded in
`../phase-0/licenses.md`.

Explicit hardware check:

```text
cargo test -p paint --test backends gpu_and_software_render_the_same_display_list -- --ignored --exact --nocapture
  GPU parity adapter: Apple M2 Pro (G14S B1) (Vulkan)
  1 passed; 0 failed
```

Offline headless image-bearing document:

```text
cargo run --release -- --headless-test http://127.0.0.1:8764/ --perf
HEADLESS OK status=200 url=http://127.0.0.1:8764/ bytes=2650 elapsed=52.542961ms
HEADLESS CSS sheets=1 blocks=30 warnings=0 title=Some("Flow notebook — Phase 4")
HEADLESS LAYOUT boxes=59 lines=38 glyphs=1386 image_bytes=633604 paint_hash=d8af28fc06299e8e layout_ms=5.748 paint_ms=4.051
```

Image bytes = the 720×220 SVG raster plus the valid 1×1 PNG; the missing image
uses clipped alt text. No fixture resource points outside the local server.

## Conformance subset

| Suite | Result | Meaning |
|---|---:|---|
| html5lib/WPT-derived entities.test | 80/80 | retained real tokenizer inputs |
| local.dat tree-format suite | 45/45 | retained hand-authored tree cases |
| WPT-adapted CSS subset | 52/52 | assertions ported into 7 Rust tests |
| Flow layout custom cases | 17/17 | model/geometry/replaced/overflow/bidi/caps |
| Progressive HTML regression cases | 4/4 | snapshots/chunk boundaries/recovery/adoption |
| Deferred resource/cancellation integration | 2/2 | first actual paint before held EOF; stop |

Rates did not regress. No JS WPT harness or full CSS layout conformance claim.

## Artifacts

- `software-1000.png`, `software-420.png`: offline scalar snapshots, created by
  the explicit `phase4_probe` example.
- `gpu-wide.png`, `gpu-scroll.png`: cropped real GPU window.
- `software-window-wide.png`, `software-window-narrow.png`: forced
  `--software-render`, including the 420px resize.
- `performance.md`: Criterion/stage timings and actual retained-scroll counters.
- `fuzz.md`: final campaigns and repaired/minimized findings.

Only fixture/window content is captured. Compositor opacity can show a faint
background in desktop crops; offline snapshots are the clean pixel oracle.
The system's existing XCompose include warning did not prevent keyboard input;
no system configuration was modified for verification.
