# WPT-adapted Phase 3 subset

Source: https://github.com/web-platform-tests/wpt (retrieved 2026-10-05).
License: BSD-3-Clause; see `LICENSE.md`. All tests run offline.

| Upstream path | Assertions ported | Rust test |
|---|---:|---|
| `css/css-syntax/ident-three-code-points.html` | 8 | `syntax_ident_three_code_points_eight_assertions` |
| `css/selectors/not-specificity.html` | 8 | `selectors_not_specificity_eight_assertions` |
| `css/selectors/not-complex.html` | 20 | `selectors_not_complex_twenty_assertions` |
| `css/selectors/attribute-selector-escape.html` | 1 | `selectors_escaped_attribute_space_one_assertion` |
| `css/selectors/nth-child-and-nth-last-child.html` | 3 selected Success cases | `selectors_nth_of_list_three_assertions` |
| `css/css-cascade/important-vs-inline-001.html` | 4 | `cascade_important_vs_inline_eight_assertions` |
| `css/css-cascade/important-vs-inline-002.html` | 4 | `cascade_important_vs_inline_eight_assertions` |
| `css/css-cascade/inherit-initial.html` | 4 | `cascade_root_inherit_initial_four_assertions` |

Total: **52 assertions in 7 Rust tests**. The five vendored HTML files retain
their source CSS and JS assertions. The other three cases copy the upstream
inputs/assertions directly into the Rust adapter, with source paths in comments.

Adaptation: call `Cascade::compute`/`Selector::matches` directly. JS-assigned
inline styles become successive static HTML inputs. Layout-forcing reads and
the JS harness are not executed (JS arrives in Phase 5). This is a measured
subset, not a claim that the full WPT suites pass. General WPT execution remains
the separate harness work in the roadmap.

Run: `cargo test -p css --test wpt_adapted` from `browser/`.
