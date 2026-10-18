use super::*;
fn source(markup: &str) -> Fetched {
    Fetched {
        url: "http://127.0.0.1/".parse().unwrap(),
        status: 200,
        content_type: Some("text/html".into()),
        bytes: markup.as_bytes().to_vec(),
        script_policies: Vec::new(),
    }
}
fn receive(
    receiver: &mpsc::Receiver<FetchEvent>,
    test: impl Fn(&FetchEvent) -> bool,
) -> FetchEvent {
    let start = std::time::Instant::now();
    loop {
        let event = receiver
            .recv_timeout(Duration::from_secs(3).saturating_sub(start.elapsed()))
            .unwrap();
        if test(&event) {
            return event;
        }
    }
}
#[test]
fn a_caught_tab_fault_leaves_the_other_realm_interactive() {
    let client = Arc::new(Client::new(Default::default()).unwrap());
    let (a_tx, a_rx) = mpsc::channel();
    let (b_tx, b_rx) = mpsc::channel();
    let mut first = ScriptWorker::spawn(
        client.clone(),
        a_tx,
        Arc::new(|| {}),
        Arc::new(AtomicU64::new(1)),
    )
    .unwrap();
    let mut second =
        ScriptWorker::spawn(client, b_tx, Arc::new(|| {}), Arc::new(AtomicU64::new(2))).unwrap();
    first.initialize(1, source("<p>first</p>"), Default::default());
    second.initialize(2,source("<button id=inc>Increment</button><output id=n>0</output><script>var n=0;document.getElementById('inc').onclick=()=>document.getElementById('n').textContent=++n;</script>"),Default::default());
    receive(&a_rx, |e| matches!(e, FetchEvent::Done { .. }));
    let FetchEvent::ScriptPage { page, .. } =
        receive(&b_rx, |e| matches!(e, FetchEvent::ScriptPage { .. }))
    else {
        unreachable!()
    };
    first.commands.send(Command::Fault).unwrap();
    receive(&a_rx, |e| matches!(e, FetchEvent::Crashed { id: 1, .. }));
    second.event(
        2,
        DomCommand::Click(page.document.get_element_by_id("inc").unwrap()),
    );
    receive(
        &b_rx,
        |e| matches!(e,FetchEvent::ScriptPage{page,..}if page.document.get_element_by_id("n").is_some_and(|n|page.document.text_content(n)=="1")),
    );
    first.shutdown();
    second.shutdown();
}
