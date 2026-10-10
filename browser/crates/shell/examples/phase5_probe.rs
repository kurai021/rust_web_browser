//! Explicit local performance/GC evidence, without product console output.
use dom_bindings::{policy::Policy, DomRuntime};
use std::time::Instant;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_str!("../tests/fixtures/js-demo/app.js");
    let start = Instant::now();
    let program = js::parse(source, Default::default())?;
    let parse_ms = start.elapsed().as_secs_f64() * 1000.0;
    let url = "http://localhost/".parse()?;
    let document = html::parse_full(
        include_bytes!("../tests/fixtures/js-demo/index.html"),
        &url,
        Default::default(),
    );
    let mut runtime = DomRuntime::new(
        document,
        url,
        Vec::new(),
        Default::default(),
        Policy::default(),
    )?;
    let start = Instant::now();
    runtime.vm.eval(&program)?;
    let eval_ms = start.elapsed().as_secs_f64() * 1000.0;
    runtime.set_ready("complete");
    runtime.eval("undefined;")?;
    runtime.gc_collect();
    let baseline_objects = runtime.vm.heap_objects();
    let baseline_bytes = runtime.vm.accounted_bytes();
    let start = Instant::now();
    runtime.eval("{let root=document.body;for(let i=0;i<10000;i++){let node=document.createElement('p');root.appendChild(node);node.addEventListener('click',()=>node.textContent);root.removeChild(node);}}undefined;")?;
    let churn_ms = start.elapsed().as_secs_f64() * 1000.0;
    let peak_objects = runtime.vm.heap_objects();
    let peak_bytes = runtime.vm.accounted_bytes();
    let start = Instant::now();
    runtime.gc_collect();
    let gc_ms = start.elapsed().as_secs_f64() * 1000.0;
    let after_objects = runtime.vm.heap_objects();
    let after_bytes = runtime.vm.accounted_bytes();
    let button = runtime
        .document()
        .get_element_by_id("increment")
        .ok_or("counter missing")?;
    let start = Instant::now();
    runtime.dispatch(button, "click", "")?;
    let event_ms = start.elapsed().as_secs_f64() * 1000.0;
    runtime.vm.limits.max_steps = 2000;
    let spin = runtime
        .document()
        .get_element_by_id("spin")
        .ok_or("spin missing")?;
    let start = Instant::now();
    let killed = runtime.dispatch(spin, "click", "").is_err();
    let kill_ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("{{\"parse_ms\":{parse_ms:.3},\"eval_ms\":{eval_ms:.3},\"churn_10000_ms\":{churn_ms:.3},\"gc_ms\":{gc_ms:.3},\"event_ms\":{event_ms:.3},\"kill_ms\":{kill_ms:.3},\"killed\":{killed},\"objects\":[{baseline_objects},{peak_objects},{after_objects}],\"accounted_bytes\":[{baseline_bytes},{peak_bytes},{after_bytes}],\"remaining_listeners\":{}}}", runtime.listener_count());
    Ok(())
}
