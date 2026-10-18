//! Local five-tab startup/idle acceptance using the production browsing contexts.
use shell::{
    app::ShellApp,
    fetcher::{Fetcher, Shared},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
fn cpu_seconds() -> f64 {
    let time = rustix::time::clock_gettime(rustix::time::ClockId::ProcessCPUTime);
    time.tv_sec as f64 + time.tv_nsec as f64 / 1_000_000_000.0
}
#[cfg(not(target_os = "linux"))]
fn cpu_seconds() -> f64 {
    0.0
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base: url::Url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:8099/".into())
        .parse()?;
    if !base.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    }) {
        return Err("Phase 9 acceptance uses a loopback fixture only".into());
    }
    let started = Instant::now();
    let shared = Shared::open(Default::default(), None)?;
    let wake = Arc::new(|| {});
    let fetcher = Fetcher::spawn_shared(shared.clone(), wake.clone())?;
    let mut app = ShellApp::new(fetcher, None, false, None);
    app.configure_rendering(true, false, wake);
    app.initialize_navigation();
    assert_eq!(shared.client.network_attempts(), 0);
    let targets = [
        "/browser/",
        "/browser/article",
        "/modern/",
        "/",
        "/api/data",
    ];
    for (i, path) in targets.iter().enumerate() {
        let url = base.join(path)?;
        if i == 0 {
            app.navigate(url.as_str());
        } else {
            app.new_tab(Some(url.as_str()))?;
        }
    }
    for (i, path) in targets.iter().enumerate() {
        app.select_tab(i);
        loop {
            app.pump();
            if !app.is_loading() && app.snapshot().is_some_and(|page| page.url.path() == *path) {
                break;
            }
            if started.elapsed() > Duration::from_secs(10) {
                return Err(format!("Tab did not load {path}").into());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    let startup = started.elapsed();
    for _ in 0..25 {
        app.pump();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let start_cpu = cpu_seconds();
    let idle = Instant::now();
    while idle.elapsed() < Duration::from_secs(4) {
        app.pump();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let cores = (cpu_seconds() - start_cpu) / idle.elapsed().as_secs_f64();
    #[cfg(target_os = "linux")]
    assert!(
        cores < 0.1,
        "Idle five-tab CPU regression: {:.1}% of one core",
        cores * 100.0
    );
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse::<u64>().ok())
            })
        });
    if let Some(kib) = rss {
        assert!(kib < 500 * 1024, "Five-tab RSS exceeded 500 MiB: {kib} KiB");
    }
    app.shutdown();
    println!("PHASE9 PROBE PASS tabs=5 blank_startup_connections=0 startup_ms={:.1} idle_cpu_percent={:.3} rss_kib={rss:?}",startup.as_secs_f64()*1000.0,cores*100.0);
    Ok(())
}
