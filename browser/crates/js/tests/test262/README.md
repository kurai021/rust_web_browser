# Initial Level 1 Test262-adapted subset

Source: tc39/test262 `main`, retrieved 2026-10-08. BSD-3-Clause; see LICENSE.
Eight upstream inputs/assertions are retained with paths in each file. Error
messages/formatting are condensed, assertions unchanged. An in-house Rust
harness installs Test262Error/assert.throws and runs each input in both sloppy
and strict mode: 16 case-mode executions, measured independently of custom tests.

This is an initial measured subset, not complete Test262 conformance. The custom
language suite additionally exercises lexical scopes, classes, closures,
destructuring, coercions, builtins, GC and execution caps. No external JS engine
is linked; Node is only an optional differential oracle.
