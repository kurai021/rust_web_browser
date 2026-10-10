//! One persistent realm per navigation, on a dedicated cancellable JS thread.
use crate::fetcher::FetchEvent;
use crate::page::Page;
use crate::scripts::{load_scripts, LoadedScripts};
use css::Environment;
use html::NodeId;
use net::{Client, Fetched};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum DomCommand {
    Click(NodeId),
    Key {
        node: Option<NodeId>,
        key: String,
        text: Option<String>,
        pressed: bool,
    },
    Tab {
        reverse: bool,
    },
    Focus(Option<NodeId>),
    Resize(Environment),
}
enum Command {
    Initialize {
        navigation: u64,
        fetched: Fetched,
        environment: Environment,
    },
    Event {
        navigation: u64,
        event: DomCommand,
    },
    Shutdown,
}
pub struct ScriptWorker {
    commands: mpsc::Sender<Command>,
    pub active: Arc<AtomicU64>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl ScriptWorker {
    pub fn spawn(
        client: Arc<Client>,
        events: mpsc::Sender<FetchEvent>,
        wake: Arc<dyn Fn() + Send + Sync>,
        active: Arc<AtomicU64>,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let token = active.clone();
        let worker = std::thread::Builder::new()
            .name("js-worker".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .max_blocking_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("browser: JS runtime init failed: {e}");
                        return;
                    }
                };
                let mut current = None::<(u64, LoadedScripts)>;
                let mut resources = None::<tokio::task::JoinHandle<()>>;
                loop {
                    let wait = current
                        .as_ref()
                        .map_or(Duration::from_millis(100), |(_, loaded)| {
                            loaded.runtime.next_timer()
                        });
                    let command = rx.recv_timeout(wait);
                    match command {
                        Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            if let Some(resources) = resources.take() {
                                resources.abort();
                            }
                            break;
                        }
                        Ok(Command::Initialize {
                            navigation,
                            fetched,
                            environment,
                        }) => {
                            if token.load(Ordering::Relaxed) != navigation {
                                continue;
                            }
                            if let Some(resources) = resources.take() {
                                resources.abort();
                            }
                            let fallback = fetched.clone();
                            let loaded = runtime.block_on(load_scripts(
                                &client,
                                fetched,
                                environment,
                                Some((token.clone(), navigation)),
                            ));
                            match loaded {
                                Ok(mut loaded) => {
                                    let page = send_page(&events, &wake, navigation, &loaded, true);
                                    loaded.page = page.clone();
                                    resources = Some(load_resources(
                                        &runtime,
                                        client.clone(),
                                        events.clone(),
                                        wake.clone(),
                                        navigation,
                                        page,
                                        environment,
                                    ));
                                    current = Some((navigation, loaded));
                                }
                                Err(error) => {
                                    if error.kind != js::ErrorKind::Cancelled {
                                        let active = Some((token.clone(), navigation));
                                        let page = runtime.block_on(async {
                                            tokio::select! {
                                                page = crate::page::load_page(&client, fallback) => Some(Arc::new(page)),
                                                _ = crate::scripts::wait_cancelled(&active) => None,
                                            }
                                        });
                                        let Some(page) = page else { current = None; continue; };
                                        let styles = page.computed_styles(environment);
                                        let _ = events.send(FetchEvent::Done {
                                            id: navigation,
                                            url: page.url.clone(),
                                            result: Ok((page, styles)),
                                        });
                                        let _ = events.send(FetchEvent::ScriptError {
                                            id: navigation,
                                            message: error.to_string(),
                                            stopped: error.is_resource_limit(),
                                        });
                                        wake();
                                    }
                                    current = None;
                                }
                            }
                        }
                        Ok(Command::Event { navigation, event }) => {
                            let Some((id, loaded)) = &mut current else {
                                continue;
                            };
                            if *id != navigation || token.load(Ordering::Relaxed) != *id {
                                continue;
                            }
                            let before = loaded.runtime.revision();
                            let result = handle_dom(loaded, &event);
                            if let Err(error) = result {
                                let _ = events.send(FetchEvent::ScriptError {
                                    id: *id,
                                    message: error.to_string(),
                                    stopped: error.is_resource_limit(),
                                });
                                wake();
                            }
                            if let Some(url) = loaded.runtime.take_navigation() {
                                let _ = events.send(FetchEvent::Navigate { id: *id, url });
                                wake();
                            }
                            loaded.runtime.gc_collect();
                            if before != loaded.runtime.revision() {
                                let page = send_page(&events, &wake, *id, loaded, false);
                                loaded.page = page.clone();
                                if let Some(resources) = resources.take() {
                                    resources.abort();
                                }
                                resources = Some(load_resources(
                                    &runtime,
                                    client.clone(),
                                    events.clone(),
                                    wake.clone(),
                                    *id,
                                    page,
                                    loaded.runtime_environment(),
                                ));
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if let Some((id, loaded)) = &mut current {
                                if token.load(Ordering::Relaxed) != *id {
                                    current = None;
                                    continue;
                                }
                                let before = loaded.runtime.revision();
                                if let Err(error) = loaded.runtime.poll_timers() {
                                    let _ = events.send(FetchEvent::ScriptError {
                                        id: *id,
                                        message: error.to_string(),
                                        stopped: error.is_resource_limit(),
                                    });
                                    wake();
                                }
                                if before != loaded.runtime.revision() {
                                    let page = send_page(&events, &wake, *id, loaded, false);
                                    loaded.page = page.clone();
                                    if let Some(resources) = resources.take() {
                                        resources.abort();
                                    }
                                    resources = Some(load_resources(
                                        &runtime,
                                        client.clone(),
                                        events.clone(),
                                        wake.clone(),
                                        *id,
                                        page,
                                        loaded.runtime_environment(),
                                    ));
                                }
                                if let Some(url) = loaded.runtime.take_navigation() {
                                    let _ = events.send(FetchEvent::Navigate { id: *id, url });
                                    wake();
                                }
                                loaded.runtime.gc_collect();
                            }
                        }
                    }
                }
            })?;
        Ok(Self {
            commands: tx,
            active,
            worker: Some(worker),
        })
    }
    pub fn initialize(&self, navigation: u64, fetched: Fetched, environment: Environment) {
        let _ = self.commands.send(Command::Initialize {
            navigation,
            fetched,
            environment,
        });
    }
    pub fn event(&self, navigation: u64, event: DomCommand) {
        let _ = self.commands.send(Command::Event { navigation, event });
    }
    pub fn shutdown(&mut self) {
        self.active.fetch_add(1, Ordering::Relaxed);
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn send_page(
    events: &mpsc::Sender<FetchEvent>,
    wake: &Arc<dyn Fn() + Send + Sync>,
    id: u64,
    loaded: &LoadedScripts,
    initial: bool,
) -> Arc<Page> {
    let mut page = (*loaded.page).clone();
    page.document = Arc::new(loaded.runtime.document());
    let page = Arc::new(page);
    let styles = css::Cascade {
        environment: loaded.runtime_environment(),
        context: css::selectors::MatchContext {
            focused: loaded.runtime.focused(),
            ..Default::default()
        },
        ..Default::default()
    }
    .compute(&page.document, &page.stylesheets);
    if initial {
        let _ = events.send(FetchEvent::Done {
            id,
            url: page.url.clone(),
            result: Ok((page.clone(), styles.clone())),
        });
    }
    let _ = events.send(FetchEvent::ScriptPage {
        id,
        page: page.clone(),
        styles,
        focused: loaded.runtime.focused(),
        console: loaded.runtime.console(),
    });
    wake();
    page
}
fn load_resources(
    runtime: &tokio::runtime::Runtime,
    client: Arc<Client>,
    events: mpsc::Sender<FetchEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    id: u64,
    page: Arc<Page>,
    environment: Environment,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
        let styles = page.computed_styles(environment);
        let fonts = async {
            let fonts = crate::fonts::load_fonts(&client, &page, environment).await;
            if !fonts.is_empty() {
                let _ = events.send(FetchEvent::Fonts {
                    id,
                    page: page.clone(),
                    assets: fonts,
                });
                wake();
            }
        };
        let images = crate::images::load_images(&client, &page, &styles, |url, success| {
            let _ = events.send(FetchEvent::Image {
                id,
                page: page.clone(),
                url,
                success,
            });
            wake();
        });
        tokio::join!(fonts, images);
    })
}
fn handle_dom(loaded: &mut LoadedScripts, event: &DomCommand) -> Result<(), js::JsError> {
    match event {
        DomCommand::Resize(environment) => {
            loaded.runtime.environment(*environment);
            Ok(())
        }
        DomCommand::Focus(node) => loaded.runtime.focus(*node),
        DomCommand::Tab { reverse } => {
            let document = loaded.runtime.document();
            let focused = loaded
                .runtime
                .focused()
                .unwrap_or_else(|| document.body().unwrap_or(0));
            if !loaded.runtime.dispatch(focused, "keydown", "Tab")? {
                return Ok(());
            }
            let styles = loaded.page.computed_styles(loaded.runtime_environment());
            let mut controls = document
                .elements_from(0)
                .into_iter()
                .enumerate()
                .filter_map(|(order, node)| {
                    let tabindex = document
                        .get_attribute(node, "tabindex")
                        .and_then(|s| s.parse::<i32>().ok());
                    let interactive = ["input", "button", "textarea", "select"]
                        .iter()
                        .any(|tag| document.is_element_named(node, tag))
                        || (document.is_element_named(node, "a")
                            && document.get_attribute(node, "href").is_some());
                    if document.get_attribute(node, "disabled").is_some()
                        || document.get_attribute(node, "type") == Some("hidden")
                        || tabindex.is_some_and(|n| n < 0)
                        || (!interactive && tabindex.is_none())
                    {
                        return None;
                    }
                    let mut ancestor = Some(node);
                    while let Some(id) = ancestor {
                        if styles
                            .get(id)
                            .is_some_and(|s| !s.visible || s.display == css::style::Display::None)
                        {
                            return None;
                        }
                        ancestor = document.get(id).parent;
                    }
                    Some((tabindex.filter(|&n| n > 0).unwrap_or(i32::MAX), order, node))
                })
                .collect::<Vec<_>>();
            controls.sort_unstable();
            if !controls.is_empty() {
                let current = controls
                    .iter()
                    .position(|(_, _, node)| Some(*node) == loaded.runtime.focused());
                let next = match (current, reverse) {
                    (Some(i), false) => (i + 1) % controls.len(),
                    (Some(i), true) => (i + controls.len() - 1) % controls.len(),
                    (None, false) => 0,
                    (None, true) => controls.len() - 1,
                };
                loaded.runtime.focus(Some(controls[next].2))?;
            }
            Ok(())
        }
        DomCommand::Click(node) => {
            let doc = loaded.runtime.document();
            if doc.try_get(*node).is_none() {
                return Ok(());
            }
            let mut focus = Some(*node);
            while let Some(id) = focus {
                if ["input", "button", "textarea", "select", "a"]
                    .iter()
                    .any(|tag| doc.is_element_named(id, tag))
                {
                    break;
                }
                focus = doc.get(id).parent;
            }
            if focus.is_some_and(|id| doc.get_attribute(id, "disabled").is_some()) {
                return Ok(());
            }
            loaded.runtime.focus(focus)?;
            let allowed = loaded.runtime.dispatch(*node, "click", "")?;
            if allowed {
                let document = loaded.runtime.document();
                let mut current = Some(*node);
                while let Some(node) = current {
                    if document.is_element_named(node, "a") {
                        if let Some(href) = document
                            .get_attribute(node, "href")
                            .and_then(|s| loaded.page.url.join(s).ok())
                        {
                            loaded.set_navigation(href);
                        }
                        break;
                    }
                    current = document.get(node).parent;
                }
            }
            Ok(())
        }
        DomCommand::Key {
            node,
            key,
            text,
            pressed,
        } => {
            // Resolve native keys after preceding click/focus commands, rather
            // than trusting asynchronously acknowledged UI focus snapshots.
            let node = (*node)
                .or_else(|| loaded.runtime.focused())
                .or_else(|| loaded.runtime.document().body())
                .unwrap_or(0);
            if !pressed {
                loaded.runtime.dispatch(node, "keyup", key)?;
                return Ok(());
            }
            if !loaded.runtime.dispatch(node, "keydown", key)? {
                return Ok(());
            }
            let document = loaded.runtime.document();
            if document.is_element_named(node, "input")
                || document.is_element_named(node, "textarea")
            {
                if key == "Enter" {
                    let mut parent = document.get(node).parent;
                    while let Some(id) = parent {
                        if document.is_element_named(id, "form") {
                            loaded.runtime.dispatch(id, "submit", "")?;
                            break;
                        }
                        parent = document.get(id).parent;
                    }
                } else {
                    let mut value = document.control_value(node).to_owned();
                    if key == "Backspace" {
                        value.pop();
                    } else if let Some(text) = text {
                        value.push_str(text);
                    }
                    loaded.runtime.input(node, value)?;
                }
            } else if document.is_element_named(node, "button")
                && matches!(key.as_str(), "Enter" | " ")
            {
                loaded.runtime.dispatch(node, "click", "")?;
            }
            Ok(())
        }
    }
}
