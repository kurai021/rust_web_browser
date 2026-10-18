use dom_bindings::{html, js::JsValue, policy::Policy};
use web_api::{PageRuntime, WebCommand};
fn page() -> PageRuntime {
    let url = "https://example.test/".parse().unwrap();
    let document=html::parse_full(b"<body><a id=file href=/report download=report.txt>Save</a><a id=other href=https://other.test/report download=ignored>Other</a></body>",&url,Default::default());
    let mut page = PageRuntime::new(
        document,
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
        Default::default(),
    )
    .unwrap();
    page.set_ready("complete");
    page
}
#[test]
fn synthetic_blob_click_resolves_before_revoke_and_hands_only_bytes_to_native_ui() {
    let mut p = page();
    assert_eq!(p.eval("var a=document.createElement('a');var blob=new Blob(['local report'],{type:'text/plain'});var uri=URL.createObjectURL(blob);a.href=uri;a.download='report.txt';document.body.appendChild(a);var result=a.click();URL.revokeObjectURL(uri);result===undefined;").unwrap(),JsValue::Bool(true));
    let commands = p.commands();
    let download = commands
        .into_iter()
        .find_map(|c| {
            if let WebCommand::Download {
                filename,
                bytes,
                gesture,
                ..
            } = c
            {
                Some((filename, bytes, gesture))
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(download.0, "report.txt");
    assert_eq!(download.1.unwrap().as_slice(), b"local report");
    assert!(!download.2);
    p.gc_collect();
}
#[test]
fn cancelled_activation_and_cross_origin_download_attributes_do_not_save_files() {
    let mut p = page();
    p.eval("var link=document.getElementById('file');link.addEventListener('click',e=>e.preventDefault());link.click();").unwrap();
    assert!(!p
        .commands()
        .iter()
        .any(|c| matches!(c, WebCommand::Download { .. })));
    p.begin_gesture();
    p.eval("document.getElementById('other').click();").unwrap();
    let commands = p.commands();
    assert!(!commands
        .iter()
        .any(|c| matches!(c, WebCommand::Download { .. })));
    assert!(commands
        .iter()
        .any(|c| matches!(c,WebCommand::Navigate{url,..}if url.host_str()==Some("other.test"))));
}
#[test]
fn object_urls_are_realm_local_and_survive_gc_until_revoked() {
    let mut first = page();
    let url=first.eval("var link=document.createElement('a');link.download='x.txt';link.href=URL.createObjectURL(new Blob(['x']));link.href;").unwrap();
    first.gc_collect();
    first.eval("link.click();").unwrap();
    assert!(first
        .commands()
        .iter()
        .any(|c| matches!(c, WebCommand::Download { .. })));
    let mut second = page();
    second.vm.set_global("foreign", url).unwrap();
    second.eval("var a=document.createElement('a');a.download='x.txt';a.href=foreign;try{a.click();}catch(e){};").unwrap();
    assert!(!second
        .commands()
        .iter()
        .any(|c| matches!(c, WebCommand::Download { .. })));
}
