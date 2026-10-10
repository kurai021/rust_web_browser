use js::{NullHost, Vm};
const HARNESS: &str = r#"
var Test262Error=Error;
var assert=function(v,message){if(!v)throw new Test262Error(message||'assertion failed');};
assert.sameValue=function(a,b){if(!Object.is(a,b))throw new Test262Error('SameValue failed');};
assert.throws=function(C,fn){var threw=false;try{fn();}catch(e){threw=true;if(e.name!==C.name)throw new Test262Error('wrong error type '+e.name);}if(!threw)throw new Test262Error('expected exception');};
"#;
#[test]
fn initial_test262_subset_sloppy_and_strict() {
    let cases = [
        ("addition", include_str!("test262/addition.js")),
        ("subtraction", include_str!("test262/subtraction.js")),
        ("multiplication", include_str!("test262/multiplication.js")),
        ("division", include_str!("test262/division.js")),
        (
            "strict-equality-whitespace",
            include_str!("test262/strict-equality-whitespace.js"),
        ),
        ("map-undefined", include_str!("test262/map-undefined.js")),
        (
            "filter-undefined",
            include_str!("test262/filter-undefined.js"),
        ),
        (
            "foreach-undefined",
            include_str!("test262/foreach-undefined.js"),
        ),
    ];
    for (name, source) in cases {
        for strict in [false, true] {
            let mut vm = Vm::new(NullHost);
            vm.eval_source(HARNESS).unwrap();
            let source = format!("{}\n{source}", if strict { "'use strict';" } else { "" });
            vm.eval_source(&source)
                .unwrap_or_else(|e| panic!("{name} strict={strict}: {e}"));
        }
    }
    println!("Test262-adapted: 16/16 case-mode executions (8 source cases)");
}
