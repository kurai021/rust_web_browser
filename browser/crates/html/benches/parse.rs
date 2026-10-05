//! Phase 2 benchmark: parse 1 MB of synthetic HTML (plan/05 §5.6 goal:
//! under 200 ms on average hardware).

use criterion::{criterion_group, criterion_main, Criterion};
use html::ParseOpts;

fn megabyte_document() -> Vec<u8> {
    let mut doc = b"<!DOCTYPE html><html><head><title>bench</title></head><body>".to_vec();
    // ~132 B per row; 8192 rows ≈ 1 MB.
    let row = "<div class=\"row\" id=\"r\"><p>Hello <b>world</b> &amp; friends</p><ul><li>a</li><li>b</li></ul><table><tr><td>x</td></tr></table></div>";
    for _ in 0..8192 {
        doc.extend_from_slice(row.as_bytes());
    }
    doc.extend_from_slice(b"</body></html>");
    assert!(doc.len() > 1024 * 1024, "fixture is {:?} bytes", doc.len());
    doc
}

fn bench_parse(criterion: &mut Criterion) {
    let input = megabyte_document();
    let url = url::Url::parse("https://example.com/").expect("static url parses");
    criterion.bench_function("parse_1mb", |bencher| {
        bencher.iter(|| html::parse_full(&input, &url, ParseOpts::default()));
    });
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
