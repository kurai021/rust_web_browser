pub mod support;
use profile::{Download, DownloadState as Status};
use shell::downloads::{Manager, Source};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
fn source(server: &support::Server, path: &str) -> Source {
    Source {
        force_prompt: false,
        url: server.url(path),
        top: server.base.clone(),
        filename: "file.bin".into(),
        bytes: None,
    }
}
async fn wait(
    manager: &Manager,
    rows: &mut BTreeMap<u64, Download>,
    id: u64,
    test: impl Fn(&Download) -> bool,
) -> Download {
    let start = Instant::now();
    loop {
        for update in manager.drain() {
            rows.insert(update.download.id, update.download);
        }
        if let Some(row) = rows.get(&id).filter(|r| test(r)) {
            return row.clone();
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "download did not reach state: {:?}",
            rows.get(&id)
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
fn verify(path: &std::path::Path, size: u64) {
    use std::io::Read;
    let mut file = std::fs::File::open(path).unwrap();
    let mut buffer = [0u8; 65536];
    let mut position = 0u64;
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        for (i, byte) in buffer[..n].iter().enumerate() {
            assert_eq!(*byte, (((position + i as u64) % 65536) % 251) as u8);
        }
        position += n as u64;
    }
    assert_eq!(position, size);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hundred_megabytes_stream_to_disk_pause_resume_and_cancel_clean_partials() {
    let directory = support::Directory::new();
    let server = support::Server::start().await;
    let client = Arc::new(net::Client::new(Default::default()).unwrap());
    let mut manager = Manager::spawn(client, Vec::new(), Arc::new(|| {})).unwrap();
    let mut rows = BTreeMap::new();
    let id = manager
        .enqueue(source(&server, "/large.bin"), directory.0.join("large.bin"))
        .unwrap();
    wait(&manager, &mut rows, id, |r| {
        r.received > 0 && r.state == Status::Running
    })
    .await;
    manager.pause(id).unwrap();
    let paused = wait(&manager, &mut rows, id, |r| r.state == Status::Paused).await;
    assert_eq!(
        std::fs::metadata(&paused.partial).unwrap().len(),
        paused.received
    );
    assert!(paused.received < support::LARGE);
    manager.resume(id).unwrap();
    let complete = wait(&manager, &mut rows, id, |r| r.state == Status::Complete).await;
    assert_eq!(complete.received, support::LARGE);
    assert!(!complete.message.contains("Paused"));
    assert!(!complete.partial.exists());
    verify(&complete.destination, support::LARGE);
    assert!(server.requests.lock().unwrap().iter().any(|r| r
        .to_ascii_lowercase()
        .contains("\r\nrange: bytes=")
        && r.to_ascii_lowercase()
            .contains("\r\nif-range: \"fixture-v1\"")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&complete.destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let id = manager
        .enqueue(
            source(&server, "/cancel.bin"),
            directory.0.join("cancel.bin"),
        )
        .unwrap();
    wait(&manager, &mut rows, id, |r| {
        r.received > 0 && r.state == Status::Running
    })
    .await;
    manager.cancel(id).unwrap();
    let cancelled = wait(&manager, &mut rows, id, |r| r.state == Status::Cancelled).await;
    assert!(!cancelled.partial.exists());
    assert!(!cancelled.destination.exists());
    manager.shutdown();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ignored_ranges_restart_and_invalid_ranges_never_append_and_queue_is_capped_at_five() {
    let directory = support::Directory::new();
    let server = support::Server::start().await;
    let mut manager = Manager::spawn(
        Arc::new(net::Client::new(Default::default()).unwrap()),
        Vec::new(),
        Arc::new(|| {}),
    )
    .unwrap();
    let mut rows = BTreeMap::new();
    for path in ["/restart.bin", "/wrong.bin"] {
        let id = manager
            .enqueue(
                source(&server, path),
                directory.0.join(path.trim_start_matches('/')),
            )
            .unwrap();
        wait(&manager, &mut rows, id, |r| {
            r.state == Status::Running && r.received > 0
        })
        .await;
        manager.pause(id).unwrap();
        let before = wait(&manager, &mut rows, id, |r| r.state == Status::Paused).await;
        manager.resume(id).unwrap();
        if path == "/restart.bin" {
            let done = wait(&manager, &mut rows, id, |r| r.state == Status::Complete).await;
            verify(&done.destination, 8 * 1024 * 1024);
            assert!(done.message.contains("restarted"));
        } else {
            let failed = wait(&manager, &mut rows, id, |r| r.state == Status::Failed).await;
            assert_eq!(
                std::fs::metadata(&failed.partial).unwrap().len(),
                before.received
            );
            manager.cancel(id).unwrap();
            wait(&manager, &mut rows, id, |r| r.state == Status::Cancelled).await;
        }
    }
    let mut ids = Vec::new();
    for i in 0..6 {
        ids.push(
            manager
                .enqueue(
                    source(&server, "/held.bin"),
                    directory.0.join(format!("held-{i}.bin")),
                )
                .unwrap(),
        );
    }
    wait(&manager, &mut rows, ids[5], |r| r.state == Status::Queued).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(server.peak.load(std::sync::atomic::Ordering::Relaxed) <= 5);
    for id in &ids {
        manager.cancel(*id).unwrap();
    }
    for id in ids {
        let cancelled = wait(&manager, &mut rows, id, |r| r.state == Status::Cancelled).await;
        assert!(!cancelled.partial.exists());
    }
    manager.shutdown();
}
#[test]
fn server_names_cannot_escape_the_chosen_directory_or_hide_an_executable() {
    assert_eq!(
        shell::downloads::sanitize_filename("../../report.txt"),
        "report.txt"
    );
    assert!(!shell::downloads::sanitize_filename("a\\b/../c").contains('/'));
    let url = "https://example.test/x".parse().unwrap();
    let name = shell::downloads::suggested_filename(
        &url,
        Some("attachment; filename*=UTF-8''..%2F..%2Freport.txt"),
    );
    assert_eq!(name, "report.txt");
    assert!(shell::downloads::executable("innocent.txt", b"\x7fELFdata"));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_reservations_preserve_existing_files_and_a_finished_partial_can_commit() {
    let directory = support::Directory::new();
    let server = support::Server::start().await;
    let client = Arc::new(net::Client::new(Default::default()).unwrap());
    let original = directory.0.join("original.bin");
    let part = directory.0.join(".browser-download-1.part");
    std::fs::write(&original, b"").unwrap();
    std::fs::write(&part, b"preexisting partial").unwrap();
    let mut manager = Manager::spawn(client.clone(), Vec::new(), Arc::new(|| {})).unwrap();
    let id = manager
        .enqueue(source(&server, "/held.bin"), original.clone())
        .unwrap();
    let mut rows = BTreeMap::new();
    wait(&manager, &mut rows, id, |r| r.state == Status::Failed).await;
    manager.cancel(id).unwrap();
    wait(&manager, &mut rows, id, |r| r.state == Status::Cancelled).await;
    assert!(original.exists());
    assert_eq!(std::fs::read(&part).unwrap(), b"preexisting partial");
    manager.shutdown();
    let partial = directory.0.join(".browser-download-42.part");
    let destination = directory.0.join("finished.bin");
    let pattern: Vec<_> = (0..65536).map(|i| (i % 251) as u8).collect();
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&partial).unwrap();
        for _ in 0..128 {
            file.write_all(&pattern).unwrap();
        }
    }
    std::fs::write(&destination, b"").unwrap();
    let row = Download {
        id: 42,
        url: server.url("/done.bin").to_string(),
        top: server.base.to_string(),
        destination: destination.clone(),
        partial: partial.clone(),
        received: 8 * 1024 * 1024,
        total: Some(8 * 1024 * 1024),
        etag: Some("\"fixture-v1\"".into()),
        state: Status::Paused,
        ..Default::default()
    };
    let mut resumed = Manager::spawn(client, vec![row], Arc::new(|| {})).unwrap();
    resumed.resume(42).unwrap();
    wait(&resumed, &mut rows, 42, |r| r.state == Status::Complete).await;
    assert_eq!(
        std::fs::metadata(destination).unwrap().len(),
        8 * 1024 * 1024
    );
    assert!(!partial.exists());
    resumed.shutdown();
}
