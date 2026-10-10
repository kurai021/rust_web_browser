//! Measured upstream assertions on the Phase 5 single-document host surface.
use dom_bindings::{policy::Policy, DomRuntime};
use js::JsValue;
const HARNESS: &str = r#"
var cases=0, assertions=0;
function test(fn){cases++;fn();}
function assert_equals(a,b,message){assertions++;if(a!==b)throw new Error(message||'assert_equals');}
function assert_true(value,message){assert_equals(value,true,message);}
function assert_false(value,message){assert_equals(value,false,message);}
function assert_unreached(message){throw new Error(message||'unreachable');}
function assert_array_equals(a,b,message){assert_equals(a.join('|'),b.join('|'),message);}
"#;
fn run(source: &str) -> (f64, f64) {
    let url = "https://example.test/".parse().unwrap();
    let document = html::parse_full(b"<body><div id=target></div>", &url, Default::default());
    let mut runtime = DomRuntime::new(
        document,
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
    )
    .unwrap();
    runtime.eval(HARNESS).unwrap();
    runtime.eval(source).unwrap();
    assert!(
        runtime.console().is_empty(),
        "callback assertion failures: {:?}",
        runtime.console()
    );
    let JsValue::Number(cases) = runtime.vm.get_global("cases").unwrap() else {
        panic!("case count")
    };
    let JsValue::Number(assertions) = runtime.vm.get_global("assertions").unwrap() else {
        panic!("assertion count")
    };
    (cases, assertions)
}
#[test]
fn wpt_events_once_passive_target_order_and_immediate_stop() {
    let result = run(include_str!("wpt/events.js"));
    assert_eq!(result, (11.0, 77.0));
    println!("WPT events-adapted: 11/11 cases, 77/77 assertions");
}
#[test]
fn wpt_node_contains_single_document_matrix() {
    let result = run(include_str!("wpt/contains.js"));
    assert_eq!(result, (56.0, 56.0));
    println!("WPT DOM-adapted: 56/56 cases, 56/56 assertions");
}
