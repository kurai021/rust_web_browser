use dom_bindings::{html, js::JsValue, policy::Policy};
use std::sync::Arc;
use web_api::{
    headers::Headers, http::Response, storage::Profile, PageRuntime, RuntimeConfig, WebCommand,
    WebCompletion,
};
fn runtime(config: RuntimeConfig) -> PageRuntime {
    let url = "https://example.test/".parse().unwrap();
    PageRuntime::new(
        html::parse_full(
            b"<body><output id=result></output></body>",
            &url,
            Default::default(),
        ),
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
        config,
    )
    .unwrap()
}
fn response(body: &[u8]) -> Response {
    Response {
        url: "https://example.test/api".parse().unwrap(),
        status: 200,
        status_text: "OK".into(),
        headers: Headers(vec![
            ("content-type".into(), "application/json".into()),
            ("content-length".into(), body.len().to_string()),
        ]),
        body: body.to_vec(),
        kind: "basic".into(),
        redirected: false,
    }
}
#[test]
fn xhr_reuse_ignores_stale_completions_and_abort_has_no_late_load() {
    let mut r = runtime(Default::default());
    assert_eq!(r.eval("var states=[];var x=new XMLHttpRequest();x.onreadystatechange=()=>states.push(x.readyState);x.open('GET','/old');states.join(',');").unwrap(),JsValue::string("1"));
    r.eval("x.send();").unwrap();
    let WebCommand::Fetch { id: old, .. } = r.commands().remove(0) else {
        panic!("request")
    };
    r.eval("x.open('GET','/new');x.responseType='json';x.onload=()=>document.getElementById('result').textContent=x.response.value;x.send();").unwrap();
    let commands = r.commands();
    let new = commands
        .iter()
        .find_map(|c| match c {
            WebCommand::Fetch { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    assert_ne!(old, new);
    r.complete(WebCompletion::Fetch {
        id: old,
        result: Ok(response(b"{\"value\":1}")),
    })
    .unwrap();
    assert_eq!(r.eval("x.readyState;").unwrap(), JsValue::Number(1.0));
    r.complete(WebCompletion::Fetch {
        id: new,
        result: Ok(response(b"{\"value\":2}")),
    })
    .unwrap();
    let doc = r.document();
    assert_eq!(
        doc.text_content(doc.get_element_by_id("result").unwrap()),
        "2"
    );
    r.eval("var aborted=[];x.open('GET','/cancel');x.onabort=()=>aborted.push(x.readyState);x.send();x.abort();").unwrap();
    assert_eq!(
        r.eval("aborted.join(',')+':'+x.readyState;").unwrap(),
        JsValue::string("4:0")
    );
    assert!(!r
        .commands()
        .iter()
        .any(|c| matches!(c, WebCommand::Fetch { .. })));
}
#[test]
fn headers_are_live_branded_and_fetch_responses_immutable() {
    let mut r = runtime(Default::default());
    r.eval("var request=new Request('/api');var headers=request.headers;headers.set('x-test','value');headers.set('Cookie','blocked');fetch(request);").unwrap();
    let WebCommand::Fetch { id, request, .. } = r.commands().remove(0) else {
        panic!("request")
    };
    assert_eq!(request.headers.get("x-test").as_deref(), Some("value"));
    assert!(request.headers.get("cookie").is_none());
    assert_eq!(r.eval("headers===request.headers && headers instanceof Headers && request instanceof Request;").unwrap(),JsValue::Bool(true));
    r.complete(WebCompletion::Fetch {
        id,
        result: Ok(response(b"{}")),
    })
    .unwrap();
    r.eval("var immutable=false;fetch('/next').then(response=>{try{response.headers.set('x','y');}catch(e){immutable=e instanceof TypeError;}});").unwrap();
    let WebCommand::Fetch { id, .. } = r.commands().remove(0) else {
        panic!("request")
    };
    r.complete(WebCompletion::Fetch {
        id,
        result: Ok(response(b"{}")),
    })
    .unwrap();
    assert_eq!(r.eval("immutable;").unwrap(), JsValue::Bool(true));
    assert_eq!(r.eval("var url=new URL('https://example.test/?x=1');var params=url.searchParams;url.search='?y=2';params.get('y')+':'+params.size;").unwrap(),JsValue::string("2:1"));
}
#[test]
fn finally_waits_for_native_async_work_and_preserves_the_original_result() {
    let mut r = runtime(Default::default());
    r.eval("var order=[];var value='';new Response('original').text().finally(()=>fetch('/later').then(()=>order.push('later'))).then(text=>{order.push('done');value=text;});").unwrap();
    assert_eq!(r.eval("order.length;").unwrap(), JsValue::Number(0.0));
    let WebCommand::Fetch { id, .. } = r.commands().remove(0) else {
        panic!("request")
    };
    r.gc_collect();
    r.complete(WebCompletion::Fetch {
        id,
        result: Ok(response(b"{}")),
    })
    .unwrap();
    assert_eq!(
        r.eval("order.join(',')+':'+value;").unwrap(),
        JsValue::string("later,done:original")
    );
}
struct NativeWindow;
struct DelayedWindow;
impl web_api::runtime::Platform for DelayedWindow {
    fn dialog(&self, _: web_api::runtime::Dialog) -> web_api::Result<Option<String>> {
        std::thread::sleep(std::time::Duration::from_millis(250));
        Ok(Some("answered".into()))
    }
}
#[test]
fn native_dialog_wait_does_not_consume_the_script_execution_budget() {
    let mut r = runtime(RuntimeConfig {
        platform: Some(Arc::new(DelayedWindow)),
        ..Default::default()
    });
    r.vm.limits.max_duration = std::time::Duration::from_millis(150);
    assert_eq!(
        r.eval("prompt('Question','default');").unwrap(),
        JsValue::string("answered")
    );
    assert!(!r.is_stopped());
}
impl web_api::runtime::Platform for NativeWindow {
    fn dialog(&self, _: web_api::runtime::Dialog) -> web_api::Result<Option<String>> {
        Ok(None)
    }
}
#[test]
fn permissions_require_a_gesture_persist_only_explicit_choices_and_revoke() {
    let mut r = runtime(RuntimeConfig {
        platform: Some(Arc::new(NativeWindow)),
        ..Default::default()
    });
    r.eval("var clipboardDenied=false;navigator.clipboard.readText().catch(e=>clipboardDenied=e.name==='NotAllowedError');").unwrap();
    assert_eq!(r.eval("clipboardDenied;").unwrap(), JsValue::Bool(true));
    assert!(r.commands().is_empty());
    r.eval("var persisted=true;navigator.storage.persist().then(value=>persisted=value);")
        .unwrap();
    assert_eq!(r.eval("persisted;").unwrap(), JsValue::Bool(false));
    assert!(r.commands().is_empty());
    r.begin_gesture();
    r.eval("var full=false;document.body.requestFullscreen().then(()=>full=true);")
        .unwrap();
    r.end_gesture();
    let WebCommand::Permission { id, name, gesture } = r.commands().remove(0) else {
        panic!("permission")
    };
    assert!(gesture);
    r.complete(WebCompletion::Permission {
        id,
        name,
        decision: "once".into(),
    })
    .unwrap();
    assert_eq!(
        r.eval("full && document.fullscreenElement===document.body;")
            .unwrap(),
        JsValue::Bool(true)
    );
    assert!(r
        .config()
        .profile
        .permission(&r.url(), "fullscreen")
        .is_none());
    r.revoke_origin_permissions().unwrap();
    assert_eq!(
        r.eval("document.fullscreenElement===null;").unwrap(),
        JsValue::Bool(true)
    );
    r.eval(
        "var geoCode=0;navigator.geolocation.getCurrentPosition(()=>geoCode=99,e=>geoCode=e.code);",
    )
    .unwrap();
    let commands = r.commands();
    let (id, name) = commands
        .into_iter()
        .find_map(|c| match c {
            WebCommand::Permission { id, name, .. } => Some((id, name)),
            _ => None,
        })
        .unwrap();
    r.complete(WebCompletion::Permission {
        id,
        name,
        decision: "always".into(),
    })
    .unwrap();
    assert_eq!(r.eval("geoCode;").unwrap(), JsValue::Number(2.0));
    assert_eq!(
        r.config()
            .profile
            .permission(&r.url(), "geolocation")
            .as_deref(),
        Some("granted")
    );
}
#[test]
fn persistent_profile_and_cookies_survive_restart_but_session_data_does_not() {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "browser-web-profile-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let profile = Arc::new(Profile::open(Some(path.clone())).unwrap());
    let mut r = runtime(RuntimeConfig {
        profile,
        session: "one".into(),
        ..Default::default()
    });
    r.eval("localStorage.setItem('persistent','yes');sessionStorage.setItem('temporary','yes');var open=indexedDB.open('persistent',1);open.onupgradeneeded=e=>e.target.result.createObjectStore('items');open.onsuccess=()=>open.result.transaction('items','readwrite').objectStore('items').put({value:42},'key');").unwrap();
    let url = r.url();
    let jar = r.config().cookies;
    assert!(jar.set(&url, &url, "persistent=yes; Secure; Max-Age=3600", false));
    assert!(jar.set(&url, &url, "session=private; Secure", false));
    let cookie_path = path.join("cookies.json");
    jar.save(&cookie_path).unwrap();
    assert!(!std::fs::read_to_string(&cookie_path)
        .unwrap()
        .contains("private"));
    drop(r);
    let profile = Arc::new(Profile::open(Some(path.clone())).unwrap());
    assert!(profile.storage(&url, Some("one")).unwrap().is_empty());
    assert_eq!(
        profile
            .storage(&url, None)
            .unwrap()
            .get("persistent")
            .map(String::as_str),
        Some("yes")
    );
    assert_eq!(
        profile.database(&url, "persistent").unwrap().stores["items"]
            .records
            .len(),
        1
    );
    let restored = net::cookies::CookieJar::default();
    restored.load(&cookie_path).unwrap();
    assert_eq!(restored.header(&url, &url, false), "persistent=yes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&cookie_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.join("web-data.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(path).unwrap();
}
