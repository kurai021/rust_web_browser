# Phase 7 kernel decision

No new owned SIMD/assembly kernel. HTTP/TLS/socket framing, charset conversion,
font shaping and image decoding use the already permitted auxiliary crates.
Canvas rasterization and API/clone/observer logic remain bounded owned scalar
Rust. Existing render paths pass explicit GPU/software parity; default and
no-default-features suites pass. Criterion shows no >20% regression or measured
bottleneck warranting a new low-level kernel at this phase.
