//! Pinned, attributed Test262 inputs run by the owned VM, including real jobs
//! and module records. This reports a selected subset, not whole-suite coverage.
use js::{JsValue, NullHost, Vm};
use serde_json::{json, Value};
const UPSTREAM: &str = "tc39/test262@d53ebc8b1931d9f1541e43d59650776e4b766248";
const HARNESS: &str = r#"
var Test262Error=Error;
var __test262_assertions=0, __test262_done=0, __test262_error;
var assert=function(value,message){__test262_assertions++;if(!value)throw new Test262Error(message||'assertion failed');};
assert.sameValue=function(a,b,message){__test262_assertions++;if(!Object.is(a,b))throw new Test262Error(message||'SameValue failed');};
assert.notSameValue=function(a,b,message){__test262_assertions++;if(Object.is(a,b))throw new Test262Error(message||'notSameValue failed');};
assert.throws=function(C,fn,message){__test262_assertions++;var threw=false;try{fn();}catch(e){threw=true;if(e.name!==C.name)throw new Test262Error('wrong exception: '+e.name);}if(!threw)throw new Test262Error(message||'expected exception');};
function checkSequence(sequence,message){for(var i=0;i<sequence.length;i++)assert.sameValue(sequence[i],i+1,message);}
function $DONE(error){__test262_done++;if(error!==undefined)__test262_error=error;}
"#;
struct Case {
    source: &'static str,
    asynchronous: bool,
    module: bool,
}
macro_rules! script {
    ($file:literal,$asynchronous:literal) => {
        Case {
            source: include_str!(concat!("test262/level2/", $file, ".js")),
            asynchronous: $asynchronous,
            module: false,
        }
    };
}
macro_rules! module {
    ($file:literal,$asynchronous:literal) => {
        Case {
            source: include_str!(concat!("test262/level2/", $file, ".js")),
            asynchronous: $asynchronous,
            module: true,
        }
    };
}
fn count(vm: &mut Vm, name: &str) -> u64 {
    match vm.get_global(name).unwrap() {
        JsValue::Number(value) => value as u64,
        _ => 0,
    }
}
fn reason(vm: &mut Vm, value: JsValue) -> String {
    vm.to_string(value)
        .unwrap_or_else(|_| "unprintable exception".into())
}
fn execute(case: &Case, strict: bool) -> Result<u64, String> {
    let mut vm = Vm::new(NullHost);
    vm.limits.max_steps = 100_000;
    vm.limits.max_duration = std::time::Duration::from_secs(1);
    vm.eval_source(HARNESS).map_err(|e| e.to_string())?;
    let evaluation = if case.module {
        for (name, source) in [
            (
                "distinct-for-each-module_FIXTURE.js",
                include_str!("test262/level2/distinct-for-each-module_FIXTURE.js"),
            ),
            (
                "module-import-resolution_FIXTURE.js",
                include_str!("test262/level2/module-import-resolution_FIXTURE.js"),
            ),
        ] {
            vm.define_module(&format!("https://test262.example/{name}"), source)
                .map_err(|e| e.to_string())?;
        }
        vm.define_module("https://test262.example/main.js", case.source)
            .map_err(|e| e.to_string())?;
        Some(
            vm.eval_module("https://test262.example/main.js")
                .map_err(|e| e.to_string())?,
        )
    } else {
        let source = format!(
            "{}\n{}",
            if strict { "'use strict';" } else { "" },
            case.source
        );
        vm.eval_source(&source).map_err(|e| e.to_string())?;
        None
    };
    vm.gc_collect();
    vm.drain_microtasks().map_err(|e| e.to_string())?;
    if let Some(promise) = evaluation {
        match vm.promise_result(&promise) {
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(reason(&mut vm, error)),
            None => return Err("module evaluation did not finish".into()),
        }
    }
    let error = vm
        .get_global("__test262_error")
        .map_err(|e| e.to_string())?;
    if error != JsValue::Undefined {
        return Err(reason(&mut vm, error));
    }
    if case.asynchronous && count(&mut vm, "__test262_done") != 1 {
        return Err("async case must call $DONE exactly once".into());
    }
    let errors = vm.take_async_errors();
    if let Some(error) = errors.first() {
        return Err(error.to_string());
    }
    for notice in vm.take_promise_notifications() {
        if !notice.handled {
            return Err(reason(&mut vm, notice.reason));
        }
    }
    Ok(count(&mut vm, "__test262_assertions"))
}
#[test]
fn level2_pinned_subset_reports_scripts_jobs_modules_and_explicit_exclusions() {
    let cases = [
        script!("promise-resolve-thenable", true),
        script!("promise-poisoned-then", true),
        script!("promise-all-late-rejection", true),
        script!("promise-all-string", true),
        script!("promise-any-empty", true),
        script!("promise-finally-throw", true),
        script!("async-return-after-await", true),
        script!("async-throw-after-await", true),
        script!("async-parameter-throw", true),
        script!("async-finally-throw", true),
        script!("generator-no-yield", false),
        script!("generator-not-constructor", false),
        script!("map-bigint-number", false),
        script!("map-live-for-each", false),
        script!("map-zero-key", false),
        script!("set-bigint-number", false),
        script!("weakmap-object-keys", false),
        script!("weakmap-symbol-keys", false),
        script!("symbol-uniqueness", false),
        script!("symbol-registry", false),
        script!("bigint-coercion", false),
        script!("bigint-empty-string", false),
        script!("bigint-hex-string", false),
        script!("bigint-invalid-strings", false),
        script!("proxy-get-invariant", false),
        script!("reflect-get-receiver", false),
        script!("reflect-set-primitive", false),
        module!("import-meta-same", false),
        module!("import-meta-distinct", false),
        module!("module-await-exports", true),
    ];
    let mut records: Vec<Value> = Vec::new();
    let (mut pass, mut fail, mut assertions) = (0, 0, 0);
    for case in &cases {
        let path = case
            .source
            .lines()
            .find_map(|line| line.split_once("Upstream: ").map(|(_, path)| path))
            .expect("attributed upstream path");
        let modes: &[bool] = if case.module { &[true] } else { &[false, true] };
        for &strict in modes {
            let mode = if case.module {
                "module"
            } else if strict {
                "strict"
            } else {
                "sloppy"
            };
            let result = execute(case, strict);
            let (status, detail, count) = match result {
                Ok(count) => {
                    pass += 1;
                    assertions += count;
                    ("pass", None, count)
                }
                Err(error) => {
                    fail += 1;
                    eprintln!("{path} [{mode}]: {error}");
                    ("fail", Some(error), 0)
                }
            };
            records.push(
                json!({"path":path,"mode":mode,"status":status,"assertions":count,"detail":detail}),
            );
        }
    }
    let excluded = [
        (
            "built-ins/WeakRef/constructor.js",
            "WeakRef is optional in Level 2 and is not installed",
        ),
        (
            "built-ins/Symbol/keyFor/cross-realm.js",
            "the test harness does not expose $262.createRealm",
        ),
        (
            "built-ins/Map/proto-from-ctor-realm.js",
            "cross-realm newTarget needs the multi-realm $262 harness",
        ),
    ];
    for (path, detail) in excluded {
        records.push(
            json!({"path":path,"mode":"not-run","status":"skip","assertions":0,"detail":detail}),
        );
    }
    let report = json!({"suite":"test262-level2-adapted","upstream":UPSTREAM,"source_cases":cases.len(),"pass":pass,"fail":fail,"skip_sources":excluded.len(),"assertions":assertions,"cases":records});
    println!("Test262 Level 2: {pass} passed case-mode executions; {fail} failures; {} excluded sources; {assertions} assertions",excluded.len());
    if let Some(directory) = std::env::var_os("BROWSER_TEST262_REPORT_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("test262-level2.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(fail, 0, "see per-case diagnostics and the JSON report");
}
