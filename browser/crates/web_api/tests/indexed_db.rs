use dom_bindings::{html, js::JsValue, policy::Policy};
use std::sync::Arc;
use web_api::{storage::Profile, PageRuntime, RuntimeConfig};
fn runtime(profile: Arc<Profile>) -> PageRuntime {
    let url = "https://example.test/".parse().unwrap();
    PageRuntime::new(
        html::parse_full(b"<body></body>", &url, Default::default()),
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
        RuntimeConfig {
            profile,
            ..Default::default()
        },
    )
    .unwrap()
}
const OPEN:&str="var db;var opened=indexedDB.open('app',1);opened.onupgradeneeded=e=>{db=e.target.result;var store=db.createObjectStore('people',{keyPath:'id',autoIncrement:true});store.createIndex('email','email',{unique:true});};opened.onsuccess=e=>db=e.target.result;";
#[test]
fn upgrade_put_index_get_and_transaction_commit_are_deferred() {
    let mut r = runtime(Arc::new(Profile::default()));
    r.eval(OPEN).unwrap();
    assert_eq!(
        r.eval("db.name+':'+db.version+':'+db.objectStoreNames.contains('people');")
            .unwrap(),
        JsValue::string("app:1:true")
    );
    r.eval("var committed=false;var key=0;var original={email:'a@example.test',name:'Ada'};var tx=db.transaction('people','readwrite');tx.oncomplete=()=>committed=true;var put=tx.objectStore('people').put(original);put.onsuccess=()=>key=put.result;").unwrap();
    assert_eq!(
        r.eval("committed+':'+key+':'+(original.id===undefined);")
            .unwrap(),
        JsValue::string("true:1:true")
    );
    r.gc_collect();
    r.eval("var found=null;var get=db.transaction('people').objectStore('people').index('email').get('a@example.test');get.onsuccess=()=>found=get.result;").unwrap();
    assert_eq!(
        r.eval("found.name+':'+found.id;").unwrap(),
        JsValue::string("Ada:1")
    );
}
#[test]
fn uniqueness_failure_aborts_all_writes_unless_error_is_prevented() {
    let profile = Arc::new(Profile::default());
    let mut r = runtime(profile.clone());
    r.eval(OPEN).unwrap();
    r.eval("var aborted=false;var tx=db.transaction('people','readwrite');tx.onabort=()=>aborted=true;tx.objectStore('people').add({email:'same'});tx.objectStore('people').add({email:'same'});").unwrap();
    assert_eq!(r.eval("aborted;").unwrap(), JsValue::Bool(true));
    let db = profile
        .database(&"https://example.test/".parse().unwrap(), "app")
        .unwrap();
    assert!(db.stores["people"].records.is_empty());
    r.eval("var complete=false;var tx=db.transaction('people','readwrite');tx.oncomplete=()=>complete=true;tx.objectStore('people').put({email:'same'});var duplicate=tx.objectStore('people').add({email:'same'});duplicate.onerror=e=>e.preventDefault();").unwrap();
    assert_eq!(
        r.eval("complete+':'+duplicate.error.name;").unwrap(),
        JsValue::string("true:ConstraintError")
    );
}
#[test]
fn aborted_transaction_is_atomic_and_scopes_remain_origin_isolated() {
    let profile = Arc::new(Profile::default());
    let mut r = runtime(profile.clone());
    r.eval(OPEN).unwrap();
    r.eval("var tx=db.transaction('people','readwrite');tx.objectStore('people').put({email:'never'});tx.abort();").unwrap();
    assert!(profile
        .database(&"https://example.test/".parse().unwrap(), "app")
        .unwrap()
        .stores["people"]
        .records
        .is_empty());
    assert_eq!(
        profile
            .database(&"https://other.test/".parse().unwrap(), "app")
            .unwrap()
            .version,
        0
    );
    assert!(r
        .eval("var tx=db.transaction('people');tx.objectStore('people').put({email:'bad'});")
        .is_err());
}
#[test]
fn overlapping_transactions_observe_prior_commits_and_blocked_upgrade_can_close() {
    let mut r = runtime(Arc::new(Profile::default()));
    r.eval(OPEN).unwrap();
    r.eval("var found=null;var write=db.transaction('people','readwrite');write.objectStore('people').put({email:'first'});var get=db.transaction('people').objectStore('people').get(1);get.onsuccess=()=>found=get.result;").unwrap();
    assert_eq!(r.eval("found.email;").unwrap(), JsValue::string("first"));
    r.eval("var upgraded=false;db.onversionchange=()=>db.close();var second=indexedDB.open('app',2);second.onupgradeneeded=e=>e.target.result.createObjectStore('notes');second.onsuccess=()=>upgraded=second.result.objectStoreNames.contains('notes');").unwrap();
    assert_eq!(r.eval("upgraded;").unwrap(), JsValue::Bool(true));
}
