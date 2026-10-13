# Phase 6 WPT-adapted reftests

Retrieved 2026-10-13 from web-platform-tests/wpt `master`:
`css/css-flexbox/flex-shrink-001.html` and `flex-shrink-002.html`.
Authors: Intel, Shiyou Tan (shiyoux.tan@intel.com); WPT contributors.
Reference: `css/reference/ref-filled-green-100px-square.xht`.

Keep the test body/style geometry; remove the instruction paragraph and reset
UA margins to put the reference at (0,0). The own Rust harness checks the expected
60/40 or 50/50 shrink split and all 10,000 green reference pixels. Two cases,
not the full CSS flexbox suite. BSD-3-Clause license retained below.
