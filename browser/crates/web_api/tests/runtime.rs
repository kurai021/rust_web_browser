use dom_bindings::{html, js::JsValue, policy::Policy};
use web_api::{
    headers::Headers, http::Response, PageRuntime, RuntimeConfig, WebCommand, WebCompletion,
};
fn runtime() -> PageRuntime {
    let url = "https://example.test/index".parse().unwrap();
    let doc = html::parse_full(
        b"<body><p id=result>before</p><form id=f><input name=x value=one></form>",
        &url,
        Default::default(),
    );
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
fn fetch_chain_adopts_body_result_and_preserves_task_order() {
    let mut r = runtime();
    r.eval("var order=['sync'];fetch('/api',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({x:1})}).then(response=>{order.push('response');return response.json();}).then(data=>{document.getElementById('result').textContent=data.value;order.push('data');});order.push('end');").unwrap();
    let commands = r.commands();
    let WebCommand::Fetch { id, request, .. } = &commands[0] else {
        panic!("expected request")
    };
    assert_eq!(request.method, "POST");
    assert_eq!(request.body, b"{\"x\":1}");
    r.gc_collect();
    r.complete(WebCompletion::Fetch {
        id: *id,
        result: Ok(Response {
            url: request.url.clone(),
            status: 200,
            status_text: "OK".into(),
            headers: Headers::default(),
            body: b"{\"value\":\"after\"}".to_vec(),
            kind: "basic".into(),
            redirected: false,
        }),
    })
    .unwrap();
    assert_eq!(
        r.eval("order.join(',');").unwrap(),
        JsValue::string("sync,end,response,data")
    );
    let d = r.document();
    assert_eq!(
        d.text_content(d.get_element_by_id("result").unwrap()),
        "after"
    );
}
#[test]
fn data_helpers_storage_and_same_document_history() {
    let mut r = runtime();
    assert_eq!(
        r.eval("var p=new URLSearchParams('x=a+b&x=c');p.set('x','hello world');p.toString();")
            .unwrap(),
        JsValue::string("x=hello+world")
    );
    assert_eq!(r.eval("var u=new URL('/next?x=1','https://example.test/');u.searchParams.set('x','two');u.href;").unwrap(),JsValue::string("https://example.test/next?x=two"));
    assert_eq!(r.eval("localStorage.setItem('answer',42);sessionStorage.setItem('temp','one');localStorage.answer+':'+sessionStorage.getItem('temp');").unwrap(),JsValue::string("42:one"));
    r.eval("history.pushState({page:2},'', '/view?q=1');")
        .unwrap();
    assert_eq!(
        r.eval("location.pathname+':'+history.state.page;").unwrap(),
        JsValue::string("/view:2")
    );
    assert!(r
        .eval("history.pushState({},'', 'https://other.test/');")
        .is_err());
}
#[test]
fn blob_reader_and_form_data_do_not_use_filesystem() {
    let mut r = runtime();
    r.eval("var b=new Blob(['hello ', 'world'],{type:'text/plain'});var reader=new FileReader();reader.onload=()=>document.getElementById('result').textContent=reader.result;reader.readAsText(b);").unwrap();
    assert_eq!(
        r.eval("reader.result;").unwrap(),
        JsValue::string("hello world")
    );
    assert_eq!(r.eval("var f=new FormData(document.getElementById('f'));f.append('y','two');f.get('x')+':'+f.get('y');").unwrap(),JsValue::string("one:two"));
}
#[test]
fn body_reuse_rejects_and_dictionary_getters_can_reenter_dom() {
    let mut r = runtime();
    r.eval("var response=new Response('hello');var text='';response.text().then(value=>text=value);var rejected=false;response.text().catch(e=>rejected=e.name==='TypeError');").unwrap();
    assert_eq!(
        r.eval("text+':'+rejected;").unwrap(),
        JsValue::string("hello:true")
    );
    r.eval("fetch('/api',{get method(){document.getElementById('result').textContent='getter';return 'GET';}});").unwrap();
    let d = r.document();
    assert_eq!(
        d.text_content(d.get_element_by_id("result").unwrap()),
        "getter"
    );
}
