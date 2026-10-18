use dom_bindings::{html, js::JsValue, policy::Policy};
use web_api::{
    headers::Headers, http::Response, PageRuntime, RuntimeConfig, WebCommand, WebCompletion,
};
fn runtime() -> PageRuntime {
    let url = "https://example.test/".parse().unwrap();
    let doc = html::parse_full(b"<body><p id=result>initial</p>", &url, Default::default());
    PageRuntime::new(
        doc,
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
        RuntimeConfig::default(),
    )
    .unwrap()
}
#[test]
fn promises_mutation_observers_and_microtasks_share_fifo_order() {
    let mut r = runtime();
    r.eval("var log=[];var target=document.getElementById('result');var observer=new MutationObserver(()=>{log.push('observer');queueMicrotask(()=>log.push('observer microtask'));});observer.observe(target,{childList:true});Promise.resolve().then(()=>log.push('promise'));target.textContent='changed';queueMicrotask(()=>log.push('microtask'));log.push('sync');").unwrap();
    assert_eq!(
        r.eval("log.join(',');").unwrap(),
        JsValue::string("sync,promise,observer,microtask,observer microtask")
    );
}
#[test]
fn checkpoints_run_between_timers_and_animation_callbacks() {
    let mut r = runtime();
    r.eval("var log=[];setTimeout(()=>{log.push('timer1');queueMicrotask(()=>log.push('microtask1'));},0);setTimeout(()=>{log.push('timer2');Promise.resolve().then(()=>log.push('promise2'));},0);requestAnimationFrame(()=>{log.push('frame');queueMicrotask(()=>log.push('frame microtask'));});").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    r.poll_timers().unwrap();
    assert_eq!(
        r.eval("log.join(',');").unwrap(),
        JsValue::string("timer1,microtask1,timer2,promise2,frame,frame microtask")
    );
}
#[test]
fn awaiting_fetch_and_body_uses_native_promises_and_real_checkpoints() {
    let mut r = runtime();
    r.eval("var log=[];async function run(){var response=await fetch('/api');log.push(response instanceof Response);log.push(await response.text());queueMicrotask(()=>log.push('queued'));}var task=run();log.push(task instanceof Promise);task.then(()=>log.push('done'));").unwrap();
    let commands = r.commands();
    let WebCommand::Fetch { id, request, .. } = &commands[0] else {
        panic!("fetch")
    };
    r.gc_collect();
    r.complete(WebCompletion::Fetch {
        id: *id,
        result: Ok(Response {
            url: request.url.clone(),
            status: 200,
            status_text: "OK".into(),
            headers: Headers::default(),
            body: b"body".to_vec(),
            kind: "basic".into(),
            redirected: false,
        }),
    })
    .unwrap();
    assert_eq!(
        r.eval("log.join(',');").unwrap(),
        JsValue::string("true,true,body,queued,done")
    );
}
#[test]
fn unhandled_rejection_events_can_cancel_logging_and_report_late_handlers() {
    let mut r = runtime();
    r.eval("var log=[];var rejected;addEventListener('unhandledrejection',event=>{log.push(event.reason);log.push(event.promise===rejected);event.preventDefault();});addEventListener('rejectionhandled',event=>log.push('handled:'+event.reason));rejected=Promise.reject('failure');").unwrap();
    assert_eq!(
        r.eval("log.join(',');").unwrap(),
        JsValue::string("failure,true")
    );
    assert!(!r
        .console()
        .iter()
        .any(|line| line.contains("Unhandled promise rejection")));
    r.eval("rejected.catch(()=>{});").unwrap();
    assert_eq!(
        r.eval("log.join(',');").unwrap(),
        JsValue::string("failure,true,handled:failure")
    );
}
#[test]
fn listener_and_microtask_work_share_one_task_budget() {
    let mut r = runtime();
    r.eval("var work=0;var target=document.getElementById('result');function consume(){for(var i=0;i<500;i++)work++;}target.addEventListener('click',consume);target.addEventListener('click',()=>consume());").unwrap();
    r.vm.limits.max_steps = 12_000;
    let node = r.document().get_element_by_id("result").unwrap();
    let error = r.dispatch(node, "click", "").unwrap_err();
    assert!(error.is_resource_limit());
    assert!(r.is_stopped());
    let JsValue::Number(work) = r.vm.get_global("work").unwrap() else {
        panic!("work")
    };
    assert!(
        (500.0..1000.0).contains(&work),
        "first callback completes, second shares remaining budget: {work}"
    );
    let mut r = runtime();
    r.vm.limits.max_steps = 1000;
    let error = r
        .eval("function again(){queueMicrotask(again);}queueMicrotask(again);")
        .unwrap_err();
    assert!(error.is_resource_limit());
    assert!(r.is_stopped());
}
#[test]
fn ordinary_on_prefix_globals_and_event_handler_properties_do_not_hide_each_other() {
    let mut r = runtime();
    r.eval("var one=1;var only=2;var log=[];var target=document.getElementById('result');target.addEventListener('click',()=>log.push('listener'));target.onclick=()=>log.push('handler');").unwrap();
    let node = r.document().get_element_by_id("result").unwrap();
    r.dispatch(node, "click", "").unwrap();
    assert_eq!(
        r.eval("one+':'+only+':'+log.join(',');").unwrap(),
        JsValue::string("1:2:listener,handler")
    );
}
