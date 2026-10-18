use js::{ErrorKind, JsValue, NullHost, Vm};
fn run(source: &str, expression: &str) -> JsValue {
    let mut vm = Vm::new(NullHost);
    vm.eval_source(source).unwrap();
    vm.drain_microtasks().unwrap();
    vm.eval_source(expression).unwrap()
}
#[test]
fn resolving_thenables_gets_then_now_but_calls_it_in_fifo_job_order() {
    assert_eq!(run("var order=[];var p=new Promise(resolve=>{order.push('executor');resolve({get then(){order.push('getter');return function(resolve,reject){order.push('thenable');resolve(42);reject('late');};}});});p.then(v=>order.push(v));queueMicrotask(()=>order.push('microtask'));order.push('sync');","order.join(',');"),JsValue::string("executor,getter,sync,thenable,microtask,42"));
}
#[test]
fn reactions_forward_values_errors_and_adopt_returned_promises() {
    assert_eq!(run("var result='';Promise.resolve(2).then(v=>new Promise(resolve=>resolve(v+1))).then(v=>{throw v*2;}).catch(e=>e+1).then(v=>result=v);","result;"),JsValue::Number(7.0));
}
#[test]
fn all_any_race_and_all_settled_preserve_input_order() {
    assert_eq!(run("var result=[];Promise.all([Promise.resolve(1),2]).then(v=>result.push(v.join('-')));Promise.any([Promise.reject('first'),Promise.resolve('ok')]).then(v=>result.push(v));Promise.allSettled([1,Promise.reject('bad')]).then(v=>result.push(v[0].status+':'+v[1].reason));Promise.any([]).catch(e=>result.push(e.name+':'+e.errors.length));","result.sort().join(',');"),JsValue::string("1-2,AggregateError:0,fulfilled:bad,ok"));
}
#[test]
fn finally_waits_and_rejection_from_finally_overrides_the_original() {
    assert_eq!(run("var result=[];Promise.resolve('original').finally(()=>Promise.resolve('ignored')).then(v=>result.push(v));Promise.resolve(1).finally(()=>Promise.reject('override')).catch(e=>result.push(e));","result.sort().join(',');"),JsValue::string("original,override"));
}
#[test]
fn gc_traces_pending_jobs_resolvers_and_reactions() {
    let mut vm = Vm::new(NullHost);
    vm.eval_source("var value='';var d=Promise.withResolvers();d.promise.then(v=>value=v.text);var resolve=d.resolve;d=null;").unwrap();
    vm.gc_collect();
    vm.eval_source("resolve({text:'retained'});resolve=null;")
        .unwrap();
    vm.gc_collect();
    vm.drain_microtasks().unwrap();
    assert_eq!(
        vm.eval_source("value;").unwrap(),
        JsValue::string("retained")
    );
}
#[test]
fn unhandled_rejections_notify_only_after_checkpoint_and_late_handlers_notify() {
    let mut vm = Vm::new(NullHost);
    vm.eval_source("var bad=Promise.reject('reason');Promise.reject('handled').catch(()=>{});")
        .unwrap();
    vm.drain_microtasks().unwrap();
    let notices = vm.take_promise_notifications();
    assert_eq!(notices.len(), 1);
    assert!(!notices[0].handled);
    assert_eq!(notices[0].reason, JsValue::string("reason"));
    vm.eval_source("bad.catch(()=>{});").unwrap();
    vm.drain_microtasks().unwrap();
    assert!(vm.take_promise_notifications()[0].handled);
}
#[test]
fn microtask_starvation_obeys_instruction_and_checkpoint_limits() {
    let mut vm = Vm::new(NullHost);
    vm.limits.max_steps = 5000;
    vm.eval_source("function again(){queueMicrotask(again);}queueMicrotask(again);")
        .unwrap();
    assert_eq!(vm.drain_microtasks().unwrap_err().kind, ErrorKind::Timeout);
}
