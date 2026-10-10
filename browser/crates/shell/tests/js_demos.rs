use dom_bindings::{policy::Policy, DomRuntime};
fn runtime() -> DomRuntime {
    let url = "http://localhost/".parse().unwrap();
    let doc = html::parse_full(
        include_bytes!("fixtures/js-demo/index.html"),
        &url,
        Default::default(),
    );
    let sheets = vec![css::parse_stylesheet(
        include_str!("fixtures/js-demo/site.css"),
        Some(&url),
    )];
    let mut rt = DomRuntime::new(doc, url, sheets, Default::default(), Policy::default()).unwrap();
    rt.eval(include_str!("fixtures/js-demo/app.js")).unwrap();
    rt.set_ready("complete");
    rt
}
fn click(rt: &mut DomRuntime, id: &str) {
    let node = rt.document().get_element_by_id(id).unwrap();
    rt.dispatch(node, "click", "").unwrap();
}
fn content(rt: &DomRuntime, id: &str) -> String {
    let doc = rt.document();
    doc.text_content(doc.get_element_by_id(id).unwrap())
}
#[test]
fn counter_menu_validation_and_todo_work_without_site_patches() {
    let mut rt = runtime();
    click(&mut rt, "increment");
    click(&mut rt, "increment");
    assert_eq!(content(&rt, "count"), "2");
    click(&mut rt, "reset-count");
    assert_eq!(content(&rt, "count"), "0");
    click(&mut rt, "toggle-menu");
    let doc = rt.document();
    assert!(doc
        .get_attribute(doc.get_element_by_id("menu").unwrap(), "hidden")
        .is_none());
    rt.eval("document.getElementById('name').value='Ada';document.getElementById('email').value='ada@example.test';").unwrap();
    let form = rt.document().get_element_by_id("validation").unwrap();
    assert!(!rt.dispatch(form, "submit", "").unwrap());
    assert_eq!(content(&rt, "validation-result"), "Valid form for Ada");
    rt.eval("document.getElementById('new-todo').value='First';")
        .unwrap();
    click(&mut rt, "add-todo");
    rt.eval("document.getElementById('new-todo').value='Second';")
        .unwrap();
    click(&mut rt, "add-todo");
    assert_eq!(content(&rt, "todo-count"), "2 active");
    let doc = rt.document();
    let toggle = doc.get_elements_by_class_name("toggle")[0];
    rt.dispatch(toggle, "click", "").unwrap();
    assert_eq!(content(&rt, "todo-count"), "1 active");
    click(&mut rt, "filter-completed");
    assert!(content(&rt, "todo-list").contains("First"));
    assert!(!content(&rt, "todo-list").contains("Second"));
    click(&mut rt, "clear-completed");
    click(&mut rt, "filter-all");
    assert!(!content(&rt, "todo-list").contains("First"));
    assert!(content(&rt, "todo-list").contains("Second"));
}
#[test]
fn long_script_is_terminated_and_console_stays_local() {
    let mut rt = runtime();
    rt.vm.limits.max_steps = 2000;
    let node = rt.document().get_element_by_id("spin").unwrap();
    let error = rt.dispatch(node, "click", "").unwrap_err();
    assert_eq!(error.kind, js::ErrorKind::Timeout);
    assert!(rt.is_stopped());
    let count = content(&rt, "count");
    click(&mut rt, "increment");
    rt.poll_timers().unwrap();
    assert_eq!(content(&rt, "count"), count);
    assert!(rt.console().is_empty());
}
