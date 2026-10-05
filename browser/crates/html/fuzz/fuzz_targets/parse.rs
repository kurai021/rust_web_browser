#![no_main]

//! Fuzz the full HTML pipeline: arbitrary bytes must never panic, hang
//! (input is capped), or exceed budgets. Run with:
//! `cargo +nightly fuzz run parse -- -max_total_time=300`

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let input = if data.len() > 8192 { &data[..8192] } else { data };
    let url = url::Url::parse("https://example.com/").expect("static url parses");
    let opts = html::ParseOpts {
        max_nodes: 20_000,
        max_depth: 128,
        ..html::ParseOpts::default()
    };
    // Single push plus split pushes exercise both pipeline shapes.
    let mut parser = html::Parser::new(url.clone(), Box::new(html::NullSink), opts);
    let mid = input.len() / 2;
    parser.push(&input[..mid]);
    parser.push(&input[mid..]);
    let doc = parser.finish();
    assert!(doc.node_count() <= 20_001, "node budget respected");
});
