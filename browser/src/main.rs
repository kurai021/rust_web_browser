//! `browser` binary — Phase 9 graphical browser and local acceptance harness.
//!
//! `browser [url] [--profile-dir p] [--software-render] [--perf] [--headless-test url]`
//! (`--headless-test` is a test harness only, never a product).
//!
//! The page viewport uses the owned engines. Test automation is windowless only
//! when explicitly selected; ordinary startup opens the graphical browser.

use std::path::PathBuf;
use std::process::ExitCode;

use net::{FetchOptions, Progress, UserInput};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    url: Option<String>,
    profile_dir: Option<String>,
    software_render: bool,
    perf: bool,
    headless_test: Option<String>,
}

fn print_help() {
    print!(
        "browser {VERSION}\n\
         Graphical web browser in Rust (Phase 9: tabs, sessions and downloads)\n\
         \n\
         Usage: browser [url] [options]\n\
         \n\
         Options:\n  \
           --profile-dir <dir>    Local settings, session, history and site data\n  \
           --software-render      Force the software display-list backend\n  \
           --perf                 Local performance counters\n  \
           --headless-test <url>  Load/layout/paint without a window (test harness)\n  \
           --help                 This help\n  \
           --version              Version\n"
    );
}

/// Parses `argv` (without `argv[0]`). Errors return a message, never panic.
fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut iter = argv.iter().peekable();

    while let Some(tok) = iter.next() {
        match tok.as_str() {
            "--help" => return Err("help".to_owned()),
            "--version" => return Err("version".to_owned()),
            "--software-render" => args.software_render = true,
            "--perf" => args.perf = true,
            "--profile-dir" => {
                let dir = iter.next().ok_or("--profile-dir requires <dir>")?;
                if dir.is_empty() || dir.starts_with("--") {
                    return Err("--profile-dir requires <dir>".into());
                }
                args.profile_dir = Some(dir.clone());
            }
            "--headless-test" => {
                let url = iter.next().ok_or("--headless-test requires <url>")?;
                if url.is_empty() || url.starts_with("--") {
                    return Err("--headless-test requires <url>".into());
                }
                args.headless_test = Some(url.clone());
            }
            flag if flag.starts_with("--") => return Err(format!("unknown flag: {flag}")),
            positional => {
                if args.url.is_some() {
                    return Err("only one positional URL is accepted".to_owned());
                }
                args.url = Some(positional.to_owned());
            }
        }
    }
    if args.url.is_some() && args.headless_test.is_some() {
        return Err("Do not combine a positional URL with --headless-test".into());
    }
    Ok(args)
}

fn profile_dir_of(args: &Args) -> Option<PathBuf> {
    args.profile_dir.as_ref().map(PathBuf::from).or_else(|| {
        if args.headless_test.is_none() {
            profile::default_directory()
        } else {
            None
        }
    })
}

/// Classify startup/headless input into a URL or a search query.
fn classify_startup(text: &str) -> Result<(url::Url, bool), String> {
    match net::classify_user_input(text) {
        UserInput::Url(url) => {
            let bare_host = !text.trim().contains("://");
            Ok((url, bare_host))
        }
        UserInput::Search(query) => Err(query),
    }
}

fn run_headless(url_text: &str, perf: bool, profile_dir: Option<PathBuf>) -> ExitCode {
    if url_text.eq_ignore_ascii_case("about:blank") {
        let result = (|| -> Result<(), String> {
            let profile = profile::Store::open(profile_dir.clone()).map_err(|e| e.to_string())?;
            let shared = shell::fetcher::Shared::configured(
                FetchOptions::default(),
                profile_dir,
                profile.preferences.privacy.doh_enabled,
            )?;
            let wake = std::sync::Arc::new(|| {});
            let fetcher = shell::fetcher::Fetcher::spawn_shared(shared.clone(), wake.clone())
                .map_err(|e| e.to_string())?;
            let mut app = shell::app::ShellApp::new(
                fetcher,
                Some(url::Url::parse("about:blank").map_err(|e| e.to_string())?),
                false,
                None,
            );
            app.configure_profile(profile, false);
            app.configure_rendering(true, perf, wake);
            app.initialize_navigation();
            app.pump();
            let attempts = shared.client.network_attempts();
            app.shutdown();
            if attempts != 0 {
                return Err(format!("Unexpected startup traffic: {attempts}"));
            }
            println!("HEADLESS INTERNAL OK url=about:blank connections=0");
            Ok(())
        })();
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("HEADLESS ERROR: {error}");
                ExitCode::from(1)
            }
        };
    }
    let (url, allow_downgrade) = match classify_startup(url_text) {
        Ok(target) => target,
        Err(query) => {
            eprintln!("headless test requires an HTTP(S) URL or about:blank: {query}");
            return ExitCode::from(2);
        }
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        eprintln!("unsupported scheme for headless fetch");
        return ExitCode::from(2);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime builds");
    let started = std::time::Instant::now();
    let result = runtime.block_on(async {
        let client = std::sync::Arc::new(
            net::Client::new(FetchOptions::default()).expect("TLS stack initializes"),
        );
        let config = shell::scripts::headless_config(&client, profile_dir.clone())
            .map_err(|e| net::Error::Transport(e.to_string()))?;
        let mut last = 0usize;
        let fetched = client
            .fetch_with_http_fallback(&url, allow_downgrade, &mut |progress: Progress| {
                last = progress.downloaded;
                if perf {
                    eprintln!("headless progress: {last} bytes");
                }
            })
            .await?;
        let bytes = fetched.bytes.len();
        let mut loaded = shell::scripts::load_scripts_configured(
            &client,
            fetched,
            Default::default(),
            None,
            config.clone(),
        )
        .await
        .map_err(|e| net::Error::Transport(e.to_string()))?;
        shell::scripts::settle(
            client.clone(),
            &mut loaded,
            std::time::Duration::from_secs(2),
        )
        .await
        .map_err(|e| net::Error::Transport(e.to_string()))?;
        let mut page = loaded.snapshot();
        let mut styles = page.computed_styles(Default::default());
        let assets = shell::fonts::load_fonts(&client, &page, Default::default()).await;
        shell::images::load_images(&client, &page, &styles, |_, _| {}).await;
        loaded
            .register_images()
            .map_err(|e| net::Error::Transport(e.to_string()))?;
        page = loaded.snapshot();
        styles = page.computed_styles(Default::default());
        let mut fonts = cosmic_text::FontSystem::new();
        shell::fonts::install_fonts(&mut fonts, &assets);
        let mut frame = shell::viewport::headless_frame(
            &page,
            &styles,
            layout::Size::new(800.0, 600.0),
            &mut fonts,
        );
        let before = loaded.runtime.revision();
        let boxes = shell::viewport::web_geometry(&frame.layout, &Default::default());
        loaded
            .runtime
            .geometry(boxes, shell::viewport::observer_viewport(800.0, 600.0))
            .map_err(|e| net::Error::Transport(e.to_string()))?;
        if before != loaded.runtime.revision() {
            shell::scripts::settle(
                client.clone(),
                &mut loaded,
                std::time::Duration::from_millis(250),
            )
            .await
            .map_err(|e| net::Error::Transport(e.to_string()))?;
            page = loaded.snapshot();
            styles = page.computed_styles(Default::default());
            shell::images::load_images(&client, &page, &styles, |_, _| {}).await;
            loaded
                .register_images()
                .map_err(|e| net::Error::Transport(e.to_string()))?;
            page = loaded.snapshot();
            styles = page.computed_styles(Default::default());
            frame = shell::viewport::headless_frame(
                &page,
                &styles,
                layout::Size::new(800.0, 600.0),
                &mut fonts,
            );
        }
        let article =
            shell::article::article_from_styled_document(&page.document, &page.url, &styles);
        config
            .profile
            .close_session(&config.session)
            .map_err(|e| net::Error::Transport(e.to_string()))?;
        if let Some(dir) = &profile_dir {
            client
                .cookies
                .save(&dir.join("cookies.json"))
                .map_err(|e| net::Error::Transport(e.to_string()))?;
            client
                .hsts()
                .save_to(&dir.join("hsts.json"))
                .map_err(|e| net::Error::Transport(e.to_string()))?;
        }
        Ok::<_, net::Error>((page, article, bytes, frame))
    });
    match result {
        Ok((page, article, bytes, frame)) => {
            println!(
                "HEADLESS OK status={} url={} bytes={} elapsed={:?}",
                page.status,
                page.url.as_str(),
                bytes,
                started.elapsed()
            );
            println!(
                "HEADLESS CSS sheets={} blocks={} warnings={} title={:?}",
                page.stylesheets.len(),
                article.blocks.len(),
                page.warnings.len(),
                article.title
            );
            println!("HEADLESS LAYOUT boxes={} lines={} glyphs={} image_bytes={} paint_hash={:016x} layout_ms={:.3} paint_ms={:.3}", frame.layout.stats.boxes, frame.layout.stats.lines, frame.layout.stats.glyphs, page.image_cache.bytes(), frame.hash, frame.layout_ms, frame.paint_ms);
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("HEADLESS ERROR: {err}");
            ExitCode::from(1)
        }
    }
}

fn run(argv: &[String]) -> ExitCode {
    let args = match parse_args(argv) {
        Ok(a) => a,
        Err(e) if e == "help" => {
            print_help();
            return ExitCode::SUCCESS;
        }
        Err(e) if e == "version" => {
            println!("browser {VERSION}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}");
            print_help();
            return ExitCode::from(2);
        }
    };

    if let Some(url) = &args.headless_test {
        return run_headless(url, args.perf, profile_dir_of(&args));
    }

    let (start_url, start_allow_downgrade, start_search) = match &args.url {
        None => (None, false, None),
        Some(text) if text.trim().starts_with("about:") => {
            (url::Url::parse(text.trim()).ok(), false, None)
        }
        Some(text) => match classify_startup(text) {
            Ok((url, allow_downgrade)) => (Some(url), allow_downgrade, None),
            Err(query) => (None, false, Some(query)),
        },
    };
    let options = shell::Options {
        restore_session: args.url.is_none(),
        start_url,
        start_allow_downgrade,
        start_search,
        fetch: FetchOptions::default(),
        profile_dir: profile_dir_of(&args),
        software_render: args.software_render,
        perf: args.perf,
    };
    match shell::run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    run(&argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn no_args_opens_about_blank() {
        let args = parse_args(&v(&[])).unwrap();
        assert_eq!(args, Args::default());
    }

    #[test]
    fn positional_url_and_flags() {
        let args = parse_args(&v(&[
            "https://example.com",
            "--profile-dir",
            "/tmp/p",
            "--software-render",
            "--perf",
        ]))
        .unwrap();
        assert_eq!(args.url.as_deref(), Some("https://example.com"));
        assert_eq!(args.profile_dir.as_deref(), Some("/tmp/p"));
        assert!(args.software_render && args.perf);
    }

    #[test]
    fn headless_test_stub() {
        let args = parse_args(&v(&["--headless-test", "https://example.com"])).unwrap();
        assert_eq!(args.headless_test.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn unknown_flag_is_error() {
        assert!(parse_args(&v(&["--browse"])).is_err());
    }

    #[test]
    fn profile_without_value_is_error() {
        assert!(parse_args(&v(&["--profile-dir"])).is_err());
    }

    #[test]
    fn double_url_is_error() {
        assert!(parse_args(&v(&["a", "b"])).is_err());
    }
    #[test]
    fn flags_cannot_be_consumed_as_values_and_startup_modes_are_unambiguous() {
        assert!(parse_args(&v(&["--profile-dir", "--perf"])).is_err());
        assert!(parse_args(&v(&["--headless-test", "--perf"])).is_err());
        assert!(parse_args(&v(&[
            "https://example.test/",
            "--headless-test",
            "about:blank"
        ]))
        .is_err());
    }

    #[test]
    fn bare_host_classifies_to_https_url() {
        let (url, allow_downgrade) = classify_startup("example.com").unwrap();
        assert_eq!(url.scheme(), "https");
        assert!(allow_downgrade);
    }

    #[test]
    fn words_classify_to_search() {
        assert!(classify_startup("how to bake bread").is_err());
    }
}
