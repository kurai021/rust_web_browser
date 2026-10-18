#![no_main]
use dom_bindings::{
    html,
    js::{JsValue, Limits},
    policy::Policy,
};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    if data.len() > 8192 {
        return;
    }
    let mut parser = web_api::streams::SseParser::default();
    for chunk in data.chunks(17) {
        if parser.push(chunk).is_err() {
            break;
        }
    }
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(url) = url::Url::parse("https://fuzz.test/") else {
        return;
    };
    let document = html::parse_full(
        b"<body><div id=root></div><canvas id=chart width=32 height=32></canvas></body>",
        &url,
        Default::default(),
    );
    let Ok(mut r) = web_api::PageRuntime::new(
        document,
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
        Default::default(),
    ) else {
        return;
    };
    r.vm.limits = Limits {
        // ASan fake-stack addresses are not a native stack-depth measurement;
        // this harness keeps the independent call-depth cap at sixteen.
        max_native_stack_bytes: usize::MAX,
        max_steps: 10000,
        max_duration: std::time::Duration::from_millis(50),
        max_call_depth: 16,
        max_objects: 20000,
        max_environments: 10000,
        max_string_bytes: 65536,
        max_memory_bytes: 64 * 1024 * 1024,
        ..Default::default()
    };
    r.set_ready("complete");
    let _=r.eval("var root=document.getElementById('root');var c=document.getElementById('chart').getContext('2d');");
    let _ = r.vm.set_global("input", JsValue::string(source));
    match data.first().copied().unwrap_or(0) % 4 {
        0 => {
            let _ = r.eval(source);
        }
        1 => {
            let _=r.eval("var h=new Headers({'x-test':input});var p=new URLSearchParams(input);var b=new Blob([input]);new Response(b).text().then(s=>root.textContent=s);");
        }
        2 => {
            let _=r.eval("var o=new MutationObserver(function(records){records.length;});o.observe(root,{subtree:true,childList:true,attributes:true,characterData:true});root.innerHTML=input;o.takeRecords();c.fillStyle=input;c.fillRect(0,0,16,16);");
        }
        _ => {
            let _=r.eval("var open=indexedDB.open(input,1);open.onupgradeneeded=e=>e.target.result.createObjectStore('s');open.onsuccess=()=>open.result.transaction('s','readwrite').objectStore('s').put({text:input},input);");
        }
    }
    // Commands are data-only here: fuzzing never creates a transport or platform.
    let _ = r.commands();
    r.gc_collect();
});
