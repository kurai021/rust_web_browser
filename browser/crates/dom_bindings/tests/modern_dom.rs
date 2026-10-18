use dom_bindings::{html, js::JsValue, policy::Policy, DomRuntime};
fn runtime() -> DomRuntime {
    let url = "https://example.test/".parse().unwrap();
    let document = html::parse_full(
        b"<body><main id=target><p>old</p></main><p id=keep>main</p>",
        &url,
        Default::default(),
    );
    let mut runtime = DomRuntime::new(
        document,
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
    )
    .unwrap();
    runtime.set_ready("complete");
    runtime
}
#[test]
fn static_node_lists_are_iterable_branded_and_do_not_become_arrays() {
    let mut r = runtime();
    assert_eq!(r.eval("var list=document.querySelectorAll('p');(list instanceof NodeList)+':'+Array.isArray(list)+':'+list.length+':'+Array.from(list).length+':'+list.item(0).textContent+':'+list.item(99)+':'+Object.prototype.toString.call(list);").unwrap(),JsValue::string("true:false:2:2:old:null:[object NodeList]"));
    r.eval("document.getElementById('target').replaceChildren('new');")
        .unwrap();
    r.gc_collect();
    assert_eq!(
        r.eval("list.length+':'+list[0].textContent+':'+document.querySelectorAll('p').length;")
            .unwrap(),
        JsValue::string("2:old:1")
    );
}
#[test]
fn inert_html_documents_have_separate_identity_queries_and_safe_node_adoption() {
    let mut r = runtime();
    r.eval("var ran=false;var parser=new DOMParser();var parsed=parser.parseFromString('<title>inert</title><body><p id=keep>parsed</p><script>ran=true;</script></body>','text/html');var original=parsed.getElementById('keep');").unwrap();
    assert_eq!(r.eval("(parser instanceof DOMParser)+':'+(parsed!==document)+':'+(original.ownerDocument===parsed)+':'+parsed.getElementById('keep').textContent+':'+document.getElementById('keep').textContent+':'+parsed.location+':'+ran;").unwrap(),JsValue::string("true:true:true:parsed:main:null:false"));
    r.eval("document.getElementById('target').replaceChildren(...parsed.body.childNodes);")
        .unwrap();
    r.gc_collect();
    assert_eq!(r.eval("(original.ownerDocument===document)+':'+parsed.body.childNodes.length+':'+document.getElementById('target').textContent.includes('parsed')+':'+ran;").unwrap(),JsValue::string("true:0:true:false"));
}
#[test]
fn dataset_reflects_camel_case_attributes_and_reentrant_value_conversion() {
    let mut r = runtime();
    assert_eq!(r.eval("var target=document.getElementById('target');target.dataset.taskId={toString(){target.setAttribute('data-other','yes');return '42';}};target.dataset.taskId+':'+target.getAttribute('data-task-id')+':'+target.dataset.other;").unwrap(),JsValue::string("42:42:yes"));
}
#[test]
fn weak_dom_metadata_survives_while_the_native_node_is_reachable_but_not_after_detach() {
    let mut r = runtime();
    r.eval("var metadata=new WeakMap();metadata.set(document.getElementById('target'),{value:42});for(var i=0;i<1000;i++){var node=document.createElement('p');document.body.appendChild(node);metadata.set(node,{payload:i});}node=null;undefined;").unwrap();
    r.gc_collect();
    assert_eq!(
        r.eval("metadata.get(document.getElementById('target')).value;")
            .unwrap(),
        JsValue::Number(42.0)
    );
    let before = r.vm.heap_objects();
    r.eval("document.body.replaceChildren();undefined;")
        .unwrap();
    r.gc_collect();
    assert!(r.vm.heap_objects() + 1000 < before);
}
