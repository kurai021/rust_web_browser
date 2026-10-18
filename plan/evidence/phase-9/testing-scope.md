# Defensive robustness testing scope

The purpose of these tests is to make this repository's own browser more robust:
detect incorrect behavior, resource waste, data loss and failure-isolation bugs.
The testing agreement carries forward from Phase 8 into the browser UX work.

## Concrete boundaries

* HTTP fixtures bind loopback. Range/If-Range, cancellation and invalid-response
  cases exercise the owned download actor against a synthetic local server.
* Profiles, imported/exported files and downloads used for acceptance live in
  temporary directories. Filename cases check that generated names remain in the
  selected directory; reservation tests preserve preexisting fixture files.
* The fault-injection command is compiled only under `cfg(test)`. It verifies that
  an unwind in one owned worker leaves the other realm interactive.
* The page fuzz harness creates no transport/platform. Page commands are data
  only and are dropped by the harness. The reserved `fuzz.test` origin is not a
  network destination. The object-URL seed cannot write a file.
* Conformance oracles and the Wayland pointer are explicit test-only programs.
  No external engine or automation service is embedded into the product.
* The dependency gates consult vulnerability/license metadata. Their purpose is
  dependency verification, separate from browser startup traffic.

## Resource controls

Builds use `CARGO_BUILD_JOBS=2`, tests use two test threads, and local heavy commands
run under `nice`. Sanitized fuzzing is single-target, time/input/timeout/RSS
bounded, and serialized with builds. The final page run used 300 seconds, 8 KiB
inputs, a 10-second per-input timeout and a 1536 MiB RSS limit.

The five-tab probe checks startup traffic, resident memory and process CPU during
an idle interval. Streaming download tests verify the complete 100 MiB output in
small buffers rather than loading the whole file into RAM. GUI automation targets
the test browser window and uses an isolated profile.

## Findings remain possible

Defensive intent and safe Rust do not establish that code is error-free. GUI
measurement found real idle-GC waste in both the page worker and dedicated workers.
Those loops now collect after actual work and sleep while dormant. The probe
records the improvement and gates idle CPU; functional ordering tests remain green.
Further failures must likewise be investigated on their evidence.
