use dom_bindings::{policy::Policy, DomRuntime};
use js::JsValue;
fn runtime(markup: &str) -> DomRuntime {
    let url = "https://example.test/page".parse().unwrap();
    let doc = html::parse_full(markup.as_bytes(), &url, Default::default());
    DomRuntime::new(doc, url, Vec::new(), Default::default(), Policy::default()).unwrap()
}
#[test]
fn identity_queries_mutation_and_stale_id_indexes() {
    let mut rt = runtime("<main id=m><p id=p class='old'>before</p></main>");
    assert_eq!(
        rt.eval("let p=document.getElementById('p');p===document.querySelector('#p');")
            .unwrap(),
        JsValue::Bool(true)
    );
    rt.eval(
        "p.id='new';p.classList.remove('old');p.classList.add('active');p.textContent='after';",
    )
    .unwrap();
    assert_eq!(rt.eval("document.getElementById('p')===null&&document.querySelector('.active').textContent==='after';").unwrap(),JsValue::Bool(true));
}
#[test]
fn fragment_clone_and_hierarchy() {
    let mut rt = runtime("<ul id=list></ul>");
    rt.eval("let list=document.querySelector('#list');let frag=document.createDocumentFragment();let li=document.createElement('li');li.textContent='one';frag.appendChild(li);list.appendChild(frag);let copy=li.cloneNode(true);list.appendChild(copy);").unwrap();
    assert_eq!(
        rt.eval("list.children.length===2&&copy.textContent==='one'&&frag.childNodes.length===0;")
            .unwrap(),
        JsValue::Bool(true)
    );
    assert!(rt.eval("li.appendChild(list);").is_err());
}
#[test]
fn inner_html_and_cssom_use_existing_parsers() {
    let mut rt = runtime("<div id=x></div>");
    rt.eval("let x=document.querySelector('#x');x.innerHTML='<p class=hello>Hello <b>world</b></p>';x.style.color='red';x.style.setProperty('margin-left','12px');").unwrap();
    assert_eq!(
        rt.eval("x.querySelector('b').textContent+'|'+getComputedStyle(x).color;")
            .unwrap(),
        JsValue::string("world|rgb(255, 0, 0)")
    );
}
#[test]
fn capture_bubble_once_passive_prevent_and_delegation() {
    let mut rt = runtime("<main id=m><button id=b>go</button></main>");
    rt.eval("let order=[];let m=document.getElementById('m');let b=document.getElementById('b');m.addEventListener('click',e=>order.push('capture'),true);b.addEventListener('click',function(e){order.push(this.id);e.preventDefault();},{once:true});m.addEventListener('click',e=>order.push(e.target.closest('button').id));").unwrap();
    let id = rt.document().get_element_by_id("b").unwrap();
    assert!(!rt.dispatch(id, "click", "").unwrap());
    assert_eq!(
        rt.eval("order.join(',');").unwrap(),
        JsValue::string("capture,b,b")
    );
    assert!(rt.dispatch(id, "click", "").unwrap());
}
#[test]
fn stop_propagation_keeps_same_target_listeners() {
    let mut rt = runtime("<div id=p><button id=b>go</button></div>");
    rt.eval("let n=0;let b=document.getElementById('b');b.addEventListener('click',e=>{n++;e.stopPropagation();});b.addEventListener('click',e=>n++);document.getElementById('p').addEventListener('click',e=>n+=10);").unwrap();
    let id = rt.document().get_element_by_id("b").unwrap();
    rt.dispatch(id, "click", "").unwrap();
    assert_eq!(rt.eval("n;").unwrap(), JsValue::Number(2.0));
}
#[test]
fn input_properties_are_not_attribute_mutations() {
    let mut rt = runtime("<input id=x value=default><input id=c type=checkbox>");
    rt.eval("let x=document.getElementById('x');x.value='current';")
        .unwrap();
    assert_eq!(
        rt.eval("x.value+':'+x.getAttribute('value');").unwrap(),
        JsValue::string("current:default")
    );
    let id = rt.document().get_element_by_id("c").unwrap();
    rt.dispatch(id, "click", "").unwrap();
    assert!(rt.document().control_checked(id));
}
#[test]
fn timers_and_csp_dynamic_code() {
    let mut rt = runtime("<p></p>");
    rt.eval("var n=0;setTimeout((x)=>n=x,0,7);").unwrap();
    rt.poll_timers().unwrap();
    assert_eq!(rt.eval("n;").unwrap(), JsValue::Number(7.0));
    let url = "https://example.test/".parse().unwrap();
    let doc = html::parse_full(b"<p></p>", &url, Default::default());
    let policy = Policy::parse(&["script-src 'self'".into()], &doc);
    let mut rt = DomRuntime::new(doc, url, Vec::new(), Default::default(), policy).unwrap();
    assert!(rt.eval("eval('1+2');").is_err());
    assert_eq!(rt.eval("(()=>{const indirect=eval;let blocked=false;const options={get capture(){try{indirect('1+2');}catch(e){blocked=true;}return false;}};document.body.addEventListener('test',()=>{},options);return blocked;})();").unwrap(), JsValue::Bool(true));
}
#[test]
fn timer_failure_stops_the_realm_and_interval_can_clear_itself() {
    let mut rt = runtime("<p></p>");
    rt.eval("var n=0;var timer=setInterval(()=>{n++;clearInterval(timer);},0);")
        .unwrap();
    rt.poll_timers().unwrap();
    rt.poll_timers().unwrap();
    assert_eq!(rt.eval("n;").unwrap(), JsValue::Number(1.0));
    rt.eval("setTimeout(()=>{while(true){}},0);setTimeout(()=>n++,0);")
        .unwrap();
    rt.vm.limits.max_steps = 2000;
    assert_eq!(rt.poll_timers().unwrap_err().kind, js::ErrorKind::Timeout);
    assert!(rt.is_stopped());
    rt.poll_timers().unwrap();
    assert_eq!(rt.vm.get_global("n").unwrap(), JsValue::Number(1.0));
}
#[test]
fn dom_owned_strings_participate_in_the_page_memory_cap() {
    let mut rt = runtime("<div id=root></div>");
    rt.vm.limits.max_memory_bytes = rt.vm.accounted_bytes() + 16 * 1024;
    let error = rt.eval("var root=document.getElementById('root');for(var i=0;i<100;i++){var p=document.createElement('p');p.textContent='x'.repeat(1024);root.appendChild(p);}").unwrap_err();
    assert_eq!(error.kind, js::ErrorKind::MemoryLimit);
    assert!(rt.is_stopped());
}
#[test]
fn large_native_dom_allocations_and_queries_check_before_ast_boundaries() {
    let mut rt = runtime("<body></body>");
    rt.vm.limits.max_memory_bytes = rt.vm.accounted_bytes() + 64 * 1024;
    assert_eq!(
        rt.eval("var big='x'.repeat(10000);for(var i=0;i<1000;i++)document.createTextNode(big);")
            .unwrap_err()
            .kind,
        js::ErrorKind::MemoryLimit
    );
    assert!(
        rt.document().node_count() < 25,
        "cap must be checked per large host allocation"
    );
    let mut rt = runtime("<p></p>");
    rt.vm.limits.max_duration = std::time::Duration::ZERO;
    assert_eq!(
        rt.eval("document.querySelectorAll('p');").unwrap_err().kind,
        js::ErrorKind::Timeout
    );
}
#[test]
fn dom_js_listener_cycles_are_collectable() {
    let mut rt = runtime("<div id=root></div>");
    let initial = rt.vm.heap_objects();
    rt.eval("let root=document.getElementById('root');for(let i=0;i<10000;i++){let node=document.createElement('p');root.appendChild(node);node.addEventListener('click',()=>node.textContent);root.removeChild(node);}").unwrap();
    rt.set_ready("complete");
    rt.eval("undefined;").unwrap();
    rt.gc_collect();
    assert_eq!(rt.listener_count(), 0);
    assert!(rt.vm.heap_objects() < initial + 30);
    let nodes = rt.document().node_count();
    rt.eval("for(let i=0;i<100;i++){let n=document.createElement('p');}")
        .unwrap();
    assert_eq!(rt.document().node_count(), nodes);
}
#[test]
fn retained_event_traces_its_detached_target_until_released() {
    let mut rt = runtime("<body></body>");
    rt.eval("var saved=null;var node=document.createElement('p');node.id='kept';node.addEventListener('test',e=>saved=e);document.body.appendChild(node);node.dispatchEvent(new Event('test'));node.remove();node=null;undefined;").unwrap();
    rt.set_ready("complete");
    rt.gc_collect();
    assert_eq!(
        rt.eval("saved.target.id;").unwrap(),
        JsValue::string("kept")
    );
    assert_eq!(rt.listener_count(), 1);
    rt.eval("saved=null;undefined;").unwrap();
    rt.gc_collect();
    assert_eq!(rt.listener_count(), 0);
}
#[test]
fn standalone_event_targets_are_not_node_arguments_and_survive_gc() {
    let mut rt = runtime("<body></body>");
    rt.eval("var target=new EventTarget();var n=0;target.addEventListener('test',()=>n++);")
        .unwrap();
    rt.set_ready("complete");
    rt.gc_collect();
    rt.eval("target.dispatchEvent(new Event('test'));").unwrap();
    assert_eq!(rt.vm.get_global("n").unwrap(), JsValue::Number(1.0));
    assert_eq!(
        rt.eval("document.body.removeChild(target);")
            .unwrap_err()
            .kind,
        js::ErrorKind::TypeError
    );
    assert_eq!(
        rt.eval("document.body.appendChild(target);")
            .unwrap_err()
            .kind,
        js::ErrorKind::TypeError
    );
}
#[test]
fn csp_intersection_nonces_hashes_and_mixed_content() {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let url: url::Url = "https://example.test/".parse().unwrap();
    let doc = html::parse_full(b"<p></p>", &url, Default::default());
    let source = "var x=1;";
    let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(source));
    let p = Policy::parse(
        &[format!("script-src 'self' 'nonce-ok' 'sha256-{hash}'")],
        &doc,
    );
    assert!(p.inline_allowed(source, None));
    assert!(p.inline_allowed("other", Some("ok")));
    assert!(!p.inline_allowed("other", None));
    assert!(p.external_allowed(&url, &url.join("x.js").unwrap(), None));
    assert!(!p.external_allowed(&url, &"http://example.test/x.js".parse().unwrap(), None));
}
