use js::{JsValue, NullHost, Vm};
fn result(source: &str, expression: &str) -> JsValue {
    let mut vm = Vm::new(NullHost);
    vm.eval_source(source).unwrap();
    vm.gc_collect();
    vm.drain_microtasks().unwrap();
    vm.eval_source(expression).unwrap()
}
#[test]
fn async_await_yields_even_for_an_already_fulfilled_value() {
    assert_eq!(result("var order=[];async function f(){order.push('start');var value=await 42;order.push(value);return 'done';}f().then(v=>order.push(v));queueMicrotask(()=>order.push('micro'));order.push('sync');","order.join(',');"),JsValue::string("start,sync,42,micro,done"));
}
#[test]
fn async_rejection_catch_and_finally_preserve_pending_return() {
    assert_eq!(result("var value=0;async function f(){try{await Promise.reject(7);}catch(e){return e+1;}finally{await 1;value=2;}}f().then(v=>value+=v);","value;"),JsValue::Number(10.0));
}
#[test]
fn suspended_assignment_preserves_its_reference_and_does_not_replay_side_effects() {
    assert_eq!(result("var calls=0;var object={x:1};function key(){calls++;return 'x';}async function f(){object[key()]+=await Promise.resolve(4);}f();","object.x+':'+calls;"),JsValue::string("5:1"));
}
#[test]
fn generators_send_values_and_unwind_return_through_yielding_finally() {
    assert_eq!(result("var log=[];function* f(){try{var sent=yield 1;log.push(sent);return 3;}finally{yield 2;log.push('finally');}}var g=f();log.push(g.next().value);log.push(g.next('sent').value);var end=g.next();log.push(end.value);log.push(end.done);","log.join(',');"),JsValue::string("1,sent,2,finally,3,true"));
}
#[test]
fn generator_return_closes_delegates_and_for_of_break_runs_finally() {
    assert_eq!(result("var log=[];function* inner(){try{yield 1;yield 2;}finally{log.push('closed');}}function* outer(){yield* inner();}for(var x of outer()){log.push(x);break;}var g=inner();g.next();var end=g.return(9);log.push(end.value);log.push(end.done);","log.join(',');"),JsValue::string("1,closed,closed,9,true"));
}
#[test]
fn async_generators_queue_requests_and_await_yield_and_return_values() {
    assert_eq!(result("var log=[];async function* f(){yield Promise.resolve(1);yield await 2;return Promise.resolve(3);}var g=f();Promise.all([g.next(),g.next(),g.next(),g.next()]).then(values=>{for(var v of values)log.push(v.value+':'+v.done);});", "log.join(',');"),JsValue::string("1:false,2:false,3:true,undefined:true"));
}
#[test]
fn for_await_uses_async_iterators_and_awaits_sync_iterator_values() {
    assert_eq!(result("var log=[];async function* f(){yield 1;yield 2;}async function run(){for await(var x of f())log.push(x);for await(var x of [Promise.resolve(3),4])log.push(x);}run().then(()=>log.push('done'));", "log.join(',');"),JsValue::string("1,2,3,4,done"));
}
#[test]
fn async_iteration_awaits_close_and_preserves_original_throw() {
    let mut vm = Vm::new(NullHost);
    vm.eval_source("var log=[];var close;var iterator={[Symbol.asyncIterator](){return this;},next(){return Promise.resolve({value:1,done:false});},return(){log.push('closing');return new Promise(resolve=>close=resolve);}};async function run(){try{for await(var x of iterator){log.push(x);break;}log.push('after');}finally{log.push('finally');}}run();").unwrap();
    vm.drain_microtasks().unwrap();
    assert_eq!(
        vm.eval_source("log.join(',');").unwrap(),
        JsValue::string("1,closing")
    );
    vm.gc_collect();
    vm.eval_source("close({done:true});").unwrap();
    vm.drain_microtasks().unwrap();
    assert_eq!(
        vm.eval_source("log.join(',');").unwrap(),
        JsValue::string("1,closing,after,finally")
    );
    assert_eq!(result("var value;var iterator={[Symbol.asyncIterator](){return this;},next(){return {value:1};},return(){return Promise.reject('close');}};async function run(){try{for await(var x of iterator)throw 'body';}catch(e){value=e;}}run();","value;"),JsValue::string("body"));
}
#[test]
fn async_yield_star_delegates_to_both_async_and_sync_iterators() {
    assert_eq!(result("var log=[];async function* inner(){yield 1;return 2;}async function* outer(){var end=yield* inner();log.push(end);yield* [Promise.resolve(3),4];}async function run(){for await(var value of outer())log.push(value);}run().then(()=>log.push('done'));","log.join(',');"),JsValue::string("1,2,3,4,done"));
}
#[test]
fn bigint_operators_are_exact_and_reject_number_mixing() {
    assert_eq!(result("var x=(2n**100n)+7n;var values=[String(x/3n),String(x%3n),String(-9n>>2n),String(~0n),String(BigInt.asIntN(8,255n))];var mixed=false;try{1n+1;}catch(e){mixed=e.name==='TypeError';}values.push(mixed);","values.join(',');"),JsValue::string("422550200076076467165567735127,2,-3,-1,-1,true"));
}
#[test]
fn bytecode_migration_matches_the_owned_ast_oracle_on_control_flow() {
    let source =
        "var sum=0;for(let i=0;i<10;i++){try{if(i%2===0)continue;sum+=i;}finally{sum++;}}sum;";
    let program = js::parse(source, Default::default()).unwrap();
    let mut bytecode = Vm::new(NullHost);
    let mut ast = Vm::new(NullHost);
    assert_eq!(
        bytecode.eval(&program).unwrap(),
        ast.eval_ast_reference(&program).unwrap()
    );
    assert!(js::bytecode::compile(&program).unwrap().instruction_count() > 10);
}
#[test]
fn async_errors_keep_useful_source_function_line_and_column() {
    let mut vm = Vm::new(NullHost);
    vm.set_source_url("https://example.test/application.js");
    vm.eval_source("var stack='';\nasync function fail(){\n await 1;\n throw new Error('failure');\n}\nfail().catch(error=>stack=error.stack);").unwrap();
    vm.drain_microtasks().unwrap();
    let JsValue::String(stack) = vm.eval_source("stack;").unwrap() else {
        panic!("stack")
    };
    assert!(stack.contains("failure"));
    assert!(stack.contains("fail"));
    assert!(stack.contains("application.js:4:"), "{stack}");
}
#[test]
fn native_stack_headroom_is_checked_before_recursive_host_and_vm_frames_overflow() {
    let mut vm = Vm::new(NullHost);
    vm.limits.max_native_stack_bytes = 64 * 1024;
    let error = vm
        .eval_source("function recurse(){return recurse();}recurse();")
        .unwrap_err();
    assert_eq!(error.kind, js::ErrorKind::RangeError);
    assert!(error.message.contains("stack"));
    vm.limits.max_native_stack_bytes = 1024 * 1024;
    assert_eq!(vm.eval_source("1+2;").unwrap(), JsValue::Number(3.0));
}
