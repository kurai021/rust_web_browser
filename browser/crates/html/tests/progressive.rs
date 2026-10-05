#[test]
fn snapshots_do_not_inject_eof_or_consume_pending_input() {
    let url: url::Url = "http://localhost/".parse().unwrap();
    let mut parser = html::Parser::new(url.clone(), Box::new(html::NullSink), Default::default());
    parser.push(b"<title>Streaming</title>");
    assert!(parser.snapshot().is_none());
    let prefix = format!("{}<p id='first'>first</p><p id='last'>", " ".repeat(1100));
    parser.push(prefix.as_bytes());
    let snapshot = parser.snapshot().unwrap();
    assert_eq!(snapshot.title().as_deref(), Some("Streaming"));
    assert_eq!(
        snapshot.text_content(snapshot.get_element_by_id("first").unwrap()),
        "first"
    );
    parser.push(b"last &am");
    parser.push(b"p; \xf0\x9f");
    parser.push(b"\x8c\x8d</p>");
    let finished = parser.finish();
    assert_eq!(
        finished.text_content(finished.get_element_by_id("last").unwrap()),
        "last & 🌍"
    );
    assert_eq!(
        snapshot.text_content(snapshot.get_element_by_id("last").unwrap()),
        ""
    );
}

#[test]
fn network_chunk_boundaries_preserve_tags_attributes_comments_rawtext_and_crlf() {
    let url = "http://localhost/".parse().unwrap();
    let input = format!("{}<!doctype html><title>Title &amp; Ω</title><!-- comment --><style>p{{color:red}}</style><p data-value='a>b &amp; c'>Text &#x1f30d; &amp;!</p><pre>one\r\n\ntwo\rthree</pre><script><!--<script>ignored</script>tail--></script><p>end</p>", " ".repeat(1024));
    let full = html::parse_full(input.as_bytes(), &url, Default::default());
    for size in [1, 2, 3, 7, 31, 1024] {
        let mut parser =
            html::Parser::new(url.clone(), Box::new(html::NullSink), Default::default());
        for chunk in input.as_bytes().chunks(size) {
            parser.push(chunk);
        }
        let streamed = parser.finish();
        assert_eq!(streamed.title(), full.title(), "chunk {size}");
        assert_eq!(
            streamed.text_content(streamed.root()),
            full.text_content(full.root()),
            "chunk {size}"
        );
        for (a, b) in streamed
            .get_elements_by_tag_name("p")
            .iter()
            .zip(full.get_elements_by_tag_name("p"))
        {
            assert_eq!(
                streamed.get_attribute(*a, "data-value"),
                full.get_attribute(b, "data-value")
            );
        }
    }
}

#[test]
fn unmatched_p_end_tags_and_malformed_streaming_attributes_terminate() {
    let url: url::Url = "http://localhost/".parse().unwrap();
    let input = b"<!-- hi --><<F\xff  \"\"F\xff  \"\0\0  F \xa0<8\r\r\r\r \rp>x</p>";
    for bytes in [
        input.as_slice(),
        b"</p></p><p>valid</p>",
        b"<script>if; }</script; }</script><p>end</p>",
        b"<style>x</style /bad/attribute><p>end</p>",
    ] {
        let mut parser =
            html::Parser::new(url.clone(), Box::new(html::NullSink), Default::default());
        for chunk in bytes.chunks(bytes.len() / 2) {
            parser.push(chunk);
        }
        let doc = parser.finish();
        assert!(doc.node_count() < 30);
    }
}

#[test]
fn adoption_agency_tracks_identities_after_inner_loop_removals() {
    let bytes = b"rb><i>b><i><\xff\x0c>\n<b><ul><b><i><C\0\0++\xff++><ivxl<><i>UUUU</i>><h3/ui><>c</i/U\xff";
    let url = "http://localhost/".parse().unwrap();
    let mut parser = html::Parser::new(url, Box::new(html::NullSink), Default::default());
    let mid = bytes.len() / 2;
    parser.push(&bytes[..mid]);
    parser.push(&bytes[mid..]);
    let doc = parser.finish();
    assert!(doc.node_count() < 100);
    assert!(doc.text_content(doc.root()).contains("UUUU"));
}
