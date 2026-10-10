//! Stateful language tests, not implementation-shape assertions.
use js::{ErrorKind, JsValue, Limits, NullHost, Vm};
fn run(src: &str) -> JsValue {
    Vm::new(NullHost)
        .eval_source(src)
        .unwrap_or_else(|e| panic!("{e}\n{src}"))
}
fn number(src: &str, n: f64) {
    assert_eq!(run(src), JsValue::Number(n));
}
fn string(src: &str, s: &str) {
    assert_eq!(run(src), JsValue::string(s));
}

#[test]
fn hoisting_and_lexical_scopes() {
    number(
        "var result = f(3); function f(x) { return x + 2; } result;",
        5.0,
    );
    number("var x=1; {let x=8; x++;} x;", 1.0);
    assert_eq!(
        run("function f(){return typeof x;var x=3;}f();"),
        JsValue::string("undefined")
    );
}
#[test]
fn closures_and_per_iteration_let() {
    number(
        "function outer(x){return function(y){x += y;return x;};}var f=outer(2);f(3)+f(4);",
        14.0,
    );
    number(
        "var fs=[];for(let i=0;i<3;i++){fs.push(()=>i);}fs[0]()+fs[1]()+fs[2]();",
        3.0,
    );
}
#[test]
fn strict_this_arrow_and_apply_bind() {
    assert_eq!(
        run("function f(){'use strict';return this===undefined;}f();"),
        JsValue::Bool(true)
    );
    number(
        "var o={x:7,f:function(){return (()=>this.x)();}};o.f();",
        7.0,
    );
    number(
        "function f(a,b){return this.x+a+b;}var g=f.bind({x:2},3);g(4);",
        9.0,
    );
}
#[test]
fn coercions_equality_and_bitwise() {
    string("'2'+3;", "23");
    assert_eq!(
        run("null==undefined && null!==undefined && 1=='1' && !(1==='1');"),
        JsValue::Bool(true)
    );
    number("(-1 >>> 0) + (1 << 4);", 4294967311.0);
}
#[test]
fn prototype_new_and_property_accessors() {
    number("function C(x){this.x=x;}C.prototype.get=function(){return this.x;};var c=new C(9);c.get();",9.0);
    number(
        "var o={get x(){return 4;},set x(v){this.y=v;}};o.x=7;o.x+o.y;",
        11.0,
    );
    assert_eq!(
        run("var a=Object.create({x:1});a.x===1&&!a.hasOwnProperty('x');"),
        JsValue::Bool(true)
    );
}
#[test]
fn descriptors_and_strict_writes() {
    assert_eq!(run("var o={};Object.defineProperty(o,'x',{value:3});o.x=9;o.x===3&&Object.keys(o).length===0;"),JsValue::Bool(true));
    assert!(Vm::new(NullHost)
        .eval_source("'use strict';var o={};Object.defineProperty(o,'x',{value:3});o.x=4;")
        .is_err());
}
#[test]
fn destructuring_default_rest_spread_and_templates() {
    string(
        "let [a,b=4,...rest]=[2,undefined,6,7];let {x:y}={x:3};`${a+b+y}:${rest.join('-')}`;",
        "9:6-7",
    );
    number(
        "function sum(a,...xs){return xs.reduce((v,x)=>v+x,a);}sum(...[1,2,3]);",
        6.0,
    );
    number("var a={x:2};var b={...a,y:3};b.x+b.y;", 5.0);
}
#[test]
fn classes_inheritance_static_and_super() {
    number("class A{constructor(x){this.x=x;}get(){return this.x;}}class B extends A{constructor(x){super(x+1);}static value(){return 3;}}var b=new B(4);b.get()+B.value();",8.0);
    assert_eq!(run("class A{};new A() instanceof A;"), JsValue::Bool(true));
    number("class A{constructor(){this.x=4;}get value(){return this.x;}get(){return this.x;}static value(){return this.x;}}class B extends A{get(){return super.get()+super.value;}static value(){return super.value();}}B.x=3;new B().get()+B.value();", 11.0);
}
#[test]
fn loops_switch_and_finally_completion() {
    number(
        "var x=0;for(var i=0;i<8;i++){if(i==2)continue;if(i==5)break;x+=i;}x;",
        8.0,
    );
    number("function f(){try{return 1;}finally{return 2;}}f();", 2.0);
    number(
        "var x=0;switch(2){case 1:x=1;break;case 2:x=2;default:x+=3;}x;",
        5.0,
    );
}
#[test]
fn exceptions_are_catchable_and_stack_is_useful() {
    string(
        "try{throw new TypeError('bad');}catch(e){e.name+':'+e.message;}",
        "TypeError:bad",
    );
    let e = Vm::new(NullHost)
        .eval_source("function broken(){return absent.x;}broken();")
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::ReferenceError);
    assert!(!e.stack.is_empty());
}
#[test]
fn arrays_callbacks_and_mutation() {
    string("[1,2,3,4].filter(x=>x%2===0).map(x=>x*2).join(',');", "4,8");
    string(
        "var a=[1,2,3];var r=a.splice(1,1,8,9);a.join('-')+':'+r;",
        "1-8-9-3:2",
    );
    number("[3,1,2].sort((a,b)=>a-b).reduce((a,b)=>a+b,0);", 6.0);
}
#[test]
fn basic_regexp_string_json_date_symbol() {
    assert_eq!(run("/^[a-z]+$/i.test('Abc');"), JsValue::Bool(true));
    string("'ab ab'.replace(/ab/g,'X');", "X X");
    string(r#"'abc'.replace(/(b)/, "$`-$1-$&-$$-$'");"#, "aa-b-b-$-cc");
    string(
        "JSON.stringify(JSON.parse('{\"x\":[1,true,null]}'));",
        "{\"x\":[1,true,null]}",
    );
    string(
        "new Date(Date.UTC(2020,0,2)).toISOString();",
        "2020-01-02T00:00:00.000Z",
    );
    assert_eq!(
        run("isNaN(Date.UTC(1e30,1e30))&&isNaN(Date.UTC(NaN,0));"),
        JsValue::Bool(true)
    );
    assert_eq!(run("let a=Symbol('x'),b=Symbol('x');let o={};o[a]=3;a!==b&&o[a]===3&&Object.keys(o).length===0;"),JsValue::Bool(true));
}
#[test]
fn dynamic_eval_and_function_constructor() {
    number("function f(){var x=2;eval('x+=3');return x;}f();", 5.0);
    number("var f=new Function('a','return a+1;');f(3);", 4.0);
    string(
        "function f(){'use strict';eval('var hidden=2');return typeof hidden;}f();",
        "undefined",
    );
    number(
        "function f(){'use strict';var indirect=eval;return indirect('leaked=3');}f();leaked;",
        3.0,
    );
    number("function f(){var eval=x=>x+2;return eval(3);}f();", 5.0);
    string(
        "function f(){var local=1;var indirect=eval;return indirect('typeof local');}f();",
        "undefined",
    );
}
#[test]
fn tdz_const_and_syntax_limits() {
    assert_eq!(
        Vm::new(NullHost)
            .eval_source("typeof x;let x;")
            .unwrap_err()
            .kind,
        ErrorKind::ReferenceError
    );
    assert_eq!(
        Vm::new(NullHost).eval_source("let x=x;").unwrap_err().kind,
        ErrorKind::ReferenceError
    );
    assert_eq!(
        Vm::new(NullHost)
            .eval_source("const x=1;x=2;")
            .unwrap_err()
            .kind,
        ErrorKind::TypeError
    );
    assert!(js::parse(&"(".repeat(1000), Default::default()).is_err());
    for source in [
        format!("{}1;", "1+".repeat(10000)),
        format!("x{};", ".x".repeat(10000)),
        format!("{}X;", "new ".repeat(10000)),
    ] {
        assert!(js::parse(&source, Default::default()).is_err());
    }
}
#[test]
fn instruction_budget_is_not_catchable() {
    let mut vm = Vm::new(NullHost);
    vm.limits = Limits {
        max_steps: 1000,
        ..Default::default()
    };
    assert_eq!(
        vm.eval_source("try{while(true){}}catch(e){1;}")
            .unwrap_err()
            .kind,
        ErrorKind::Timeout
    );
}
#[test]
fn gc_preserves_closures_and_reclaims_cycles() {
    let mut vm = Vm::new(NullHost);
    vm.eval_source("function f(){let x=3;return ()=>x;}var keep=f();")
        .unwrap();
    let initial = vm.heap_objects();
    vm.eval_source("for(var i=0;i<1000;i++){var a={};var b={};a.b=b;b.a=a;}a=null;b=null;")
        .unwrap();
    vm.gc_collect();
    assert!(vm.heap_objects() < initial + 20);
    assert_eq!(vm.eval_source("keep();").unwrap(), JsValue::Number(3.0));
}

#[test]
fn evaluation_order_and_member_reference_side_effects() {
    number("var n=0;var a=[2];a[n++]+=3;n*10+a[0];", 15.0);
    string("var order=[];function get(){order.push('callee');return function(x){return order.join(',');};}get()((order.push('arg'),1));","callee,arg");
    number("var n=0;var a=[2];a[n++]++;n*10+a[0];", 13.0);
    number("var n=0;switch(2){case ++n:break;case ++n:break;}n;", 2.0);
}
#[test]
fn memory_caps_cover_properties_strings_and_environments() {
    for source in [
        "var o={};for(var i=0;i<1000;i++)o[i]='value';",
        "for(let i=0;i<1000;i++){let x=i;}",
    ] {
        let mut vm = Vm::new(NullHost);
        vm.limits.max_memory_bytes = vm.accounted_bytes() + 16 * 1024;
        assert_eq!(
            vm.eval_source(source).unwrap_err().kind,
            ErrorKind::MemoryLimit
        );
    }
    let mut vm = Vm::new(NullHost);
    vm.limits.max_string_bytes = 1024;
    assert_eq!(
        vm.eval_source("new Array(1000000).join('xxxxxxxx');")
            .unwrap_err()
            .kind,
        ErrorKind::MemoryLimit
    );
    vm.gc_collect();
    assert_eq!(
        vm.eval_source("parseFloat('1.25e2suffix');").unwrap(),
        JsValue::Number(125.0)
    );
    vm.gc_collect();
    vm.limits.max_memory_bytes = vm.accounted_bytes() + 4096;
    assert_eq!(
        vm.eval_source("'x'.repeat(1000).split('');")
            .unwrap_err()
            .kind,
        ErrorKind::MemoryLimit
    );
    assert_eq!(
        vm.eval_source("var x='payload';for(var i=0;i<20;i++)x={a:x,b:x};JSON.stringify(x);")
            .unwrap_err()
            .kind,
        ErrorKind::MemoryLimit
    );
}
