use js::{ErrorKind, JsValue, NullHost, Vm};
const A: &str = "https://example.test/a.js";
const B: &str = "https://example.test/b.js";
#[test]
fn named_imports_are_live_and_module_bindings_do_not_leak_into_global() {
    let mut vm = Vm::new(NullHost);
    vm.define_module(
        A,
        "export let value=1;export function increment(){value++;}",
    )
    .unwrap();
    vm.define_module(
        B,
        "import {value,increment} from './a.js';export function read(){return value;}increment();",
    )
    .unwrap();
    let promise = vm.eval_module(B).unwrap();
    vm.drain_microtasks().unwrap();
    let namespace = vm.promise_result(&promise).unwrap().unwrap();
    vm.set_global("ns", namespace).unwrap();
    assert_eq!(
        vm.eval_source("ns.read()+':'+typeof value;").unwrap(),
        JsValue::string("2:undefined")
    );
}
#[test]
fn module_cycles_link_hoisted_functions_and_star_reexports() {
    let mut vm = Vm::new(NullHost);
    vm.define_module(
        A,
        "import {read} from './b.js';export function base(){return 3;}export const value=read();",
    )
    .unwrap();
    vm.define_module(
        B,
        "import {base} from './a.js';export function read(){return base()+1;}",
    )
    .unwrap();
    vm.define_module(
        "https://example.test/main.js",
        "export * from './a.js';export {read as indirect} from './b.js';",
    )
    .unwrap();
    let promise = vm.eval_module("https://example.test/main.js").unwrap();
    vm.drain_microtasks().unwrap();
    vm.set_global("ns", vm.promise_result(&promise).unwrap().unwrap())
        .unwrap();
    assert_eq!(
        vm.eval_source("ns.value+':'+ns.indirect()+':'+Object.keys(ns).join(',');")
            .unwrap(),
        JsValue::string("4:4:base,indirect,value")
    );
}
#[test]
fn top_level_await_delays_dependents_and_dynamic_import_reuses_namespace() {
    let mut vm = Vm::new(NullHost);
    vm.define_module(
        A,
        "export const value=await Promise.resolve(7);export default import.meta.url;",
    )
    .unwrap();
    vm.define_module(
        B,
        "import url,{value} from './a.js';export const result=value+1;export {url};",
    )
    .unwrap();
    let promise = vm.eval_module(B).unwrap();
    assert!(vm.promise_result(&promise).is_none());
    vm.gc_collect();
    vm.drain_microtasks().unwrap();
    let namespace = vm.promise_result(&promise).unwrap().unwrap();
    vm.set_global("ns", namespace).unwrap();
    vm.set_source_url(B);
    vm.eval_source("var same=false;import('./b.js').then(value=>same=value===ns);")
        .unwrap();
    vm.drain_microtasks().unwrap();
    assert_eq!(
        vm.eval_source("ns.result+':'+ns.url+':'+same;").unwrap(),
        JsValue::string("8:https://example.test/a.js:true")
    );
}
#[test]
fn import_assignments_fail_and_namespace_is_read_only() {
    let mut vm = Vm::new(NullHost);
    vm.define_module(A, "export let value=1;").unwrap();
    vm.define_module(B, "import {value} from './a.js';value=2;")
        .unwrap();
    let promise = vm.eval_module(B).unwrap();
    vm.drain_microtasks().unwrap();
    assert!(vm.promise_result(&promise).unwrap().is_err());
    let promise = vm.eval_module(A).unwrap();
    let ns = vm.promise_result(&promise).unwrap().unwrap();
    vm.set_global("ns", ns).unwrap();
    assert_eq!(vm.eval_source("('value' in ns)+':'+Reflect.set(ns,'value',9)+':'+Reflect.deleteProperty(ns,'value')+':'+ns.value;").unwrap(),JsValue::string("true:false:false:1"));
}
#[test]
fn invalid_module_syntax_and_missing_exports_are_rejected() {
    assert!(js::parse_module("export const value=1;export {value};", Default::default()).is_err());
    let mut vm = Vm::new(NullHost);
    vm.define_module(A, "export const value=1;").unwrap();
    vm.define_module(B, "import {missing} from './a.js';")
        .unwrap();
    assert_eq!(vm.eval_module(B).unwrap_err().kind, ErrorKind::SyntaxError);
    assert_eq!(vm.eval_module(B).unwrap_err().kind, ErrorKind::SyntaxError);
}
#[test]
fn import_meta_is_per_module_stable_and_captured_by_exported_functions() {
    let mut vm = Vm::new(NullHost);
    vm.define_module(A,"var first=import.meta;export function meta(){return import.meta;}export const stable=first===import.meta;export const prototype=Object.getPrototypeOf(import.meta);").unwrap();
    let promise = vm.eval_module(A).unwrap();
    let ns = vm.promise_result(&promise).unwrap().unwrap();
    vm.set_global("ns", ns).unwrap();
    vm.gc_collect();
    assert_eq!(vm.eval_source("ns.stable && ns.prototype===null && ns.meta()===ns.meta() && ns.meta().url==='https://example.test/a.js';").unwrap(),JsValue::Bool(true));
    assert_eq!(vm.eval_source("Reflect.defineProperty(ns,'stable',{value:true})+':'+Reflect.defineProperty(ns,'stable',{value:false})+':'+Reflect.set(ns,'stable',false,{});").unwrap(),JsValue::string("true:false:false"));
}
