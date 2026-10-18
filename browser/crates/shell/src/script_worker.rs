//! One persistent realm per navigation, on a dedicated cancellable JS thread.
use crate::fetcher::FetchEvent;
use crate::page::Page;
use crate::scripts::{load_scripts_configured, LoadedScripts};
use css::Environment;
use html::NodeId;
use net::{Client, Fetched};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc,
};
use std::time::Duration;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub enum DomCommand {
    Snapshot,
    ExitFullscreen,
    Geometry {
        layout: Arc<layout::LayoutResult>,
        offsets: layout::ScrollOffsets,
        viewport: web_api::runtime::GeometryRect,
    },
    Viewport(web_api::runtime::GeometryRect),
    HistoryGo(i32),
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
    #[cfg(test)]
    Fault,
    Image {
        navigation: u64,
        node: NodeId,
        url: url::Url,
        epoch: u64,
        image: Option<web_api::runtime::CanvasImage>,
    },
    Decision {
        navigation: u64,
        request: u64,
        name: String,
        decision: String,
        action: crate::web_ui::Action,
    },
    Revoke {
        navigation: u64,
        all: bool,
    },
    Complete {
        navigation: u64,
        completion: web_api::WebCompletion,
    },
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
#[derive(Debug)]
pub struct PageLease {
    counter: Arc<std::sync::atomic::AtomicUsize>,
}
impl PageLease {
    fn acquire(counter: &Arc<std::sync::atomic::AtomicUsize>) -> Option<Arc<Self>> {
        let mut count = counter.load(Ordering::Relaxed);
        loop {
            if count >= 3 {
                return None;
            }
            match counter.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(Arc::new(Self {
                        counter: counter.clone(),
                    }))
                }
                Err(new) => count = new,
            }
        }
    }
}
impl Drop for PageLease {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Release);
    }
}
impl ScriptWorker {
    pub fn spawn(
        client: Arc<Client>,
        events: mpsc::Sender<FetchEvent>,
        wake: Arc<dyn Fn() + Send + Sync>,
        active: Arc<AtomicU64>,
    ) -> std::io::Result<Self> {
        let config = web_api::RuntimeConfig {
            cookies: client.cookies.clone(),
            ..Default::default()
        };
        Self::spawn_configured(client, events, wake, active, config)
    }
    pub fn spawn_configured(
        client: Arc<Client>,
        events: mpsc::Sender<FetchEvent>,
        wake: Arc<dyn Fn() + Send + Sync>,
        active: Arc<AtomicU64>,
        config: web_api::RuntimeConfig,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let replies = tx.clone();
        let token = active.clone();
        let crash_events = events.clone();
        let crash_wake = wake.clone();
        let crash_active = token.clone();
        let worker = std::thread::Builder::new()
            .name("js-worker".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
                let mut published=0;
                let mut snapshots=Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let mut resources = None::<tokio::task::JoinHandle<()>>;
                let mut services=web_api::services::Services::new(client.clone(),runtime.handle().clone());
                loop {
                    if current.as_ref().is_some_and(|(id,_)|token.load(Ordering::Relaxed)!=*id){services.cancel();current=None;if let Some(resources)=resources.take(){resources.abort();}}
                    if let Some((id,loaded))=&mut current {
                        for _ in 0..128 {
                            let Some(completion)=services.poll()else{break;};
                            if let Err(error)=loaded.runtime.complete(completion){report(&events,&wake,*id,&error);}
                        }
                        if loaded.runtime.is_stopped(){services.cancel();}else{
                            forward_services(&mut services,loaded,*id,&events,&wake);
                        }
                        if published!=loaded.runtime.revision(){
                            if let Some(page)=send_page(&events,&wake,*id,loaded,false,&snapshots){loaded.page=page.clone();
                            published=loaded.runtime.revision();
                            if let Some(resources)=resources.take(){resources.abort();}
                            resources=Some(load_resources(&runtime,client.clone(),*id,page,loaded.runtime_environment(),ResourceChannels{events:events.clone(),wake:wake.clone(),commands:replies.clone()}));}
                        }
                    }
                    let wait = current
                        .as_ref()
                        .map_or(Duration::from_millis(100), |(_, loaded)| {
                            let timer=loaded.runtime.next_timer();if services.has_pending_work(){timer.min(Duration::from_millis(10))}else{timer}
                        });
                    let command = rx.recv_timeout(wait);
                    match command {
                        #[cfg(test)]
                        Ok(Command::Fault)=>panic!("test-only isolated tab fault"),
                        Ok(Command::Image{navigation,node,url,epoch,image})=>{if let Some((id,loaded))=&mut current{if *id==navigation&&token.load(Ordering::Relaxed)==navigation{let result=if let Some(image)=image{loaded.runtime.image_at(node,&url,epoch,image)}else{loaded.runtime.image_error(node,&url,epoch)};if let Err(error)=result{report(&events,&wake,navigation,&error);}}}}
                        Ok(Command::Revoke{navigation,all})=>{
                            if let Some((id,loaded))=&mut current{if *id==navigation{let result=if all{loaded.runtime.revoke_permissions()}else{loaded.runtime.revoke_origin_permissions()};if let Err(error)=result{loaded.runtime.handle().console(error.to_string());}}}
                        }
                        Ok(Command::Decision{navigation,request,name,decision,action})=>{
                            let Some((id,loaded))=&mut current else{continue;};if *id!=navigation||token.load(Ordering::Relaxed)!=navigation{continue;}
                            match action{
                                crate::web_ui::Action::Permission=>{if let Err(error)=loaded.runtime.complete(web_api::WebCompletion::Permission{id:request,name,decision}){report(&events,&wake,*id,&error);}}
                                crate::web_ui::Action::Clipboard(text)=>{
                                    let granted=matches!(decision.as_str(),"once"|"always"|"granted");
                                    if let Err(error)=loaded.runtime.record_permission(&name,&decision){loaded.runtime.handle().console(error.to_string());}
                                    let complete=|result|Command::Complete{navigation,completion:web_api::WebCompletion::Clipboard{id:request,result}};
                                    if granted{let replies=replies.clone();if let Err(error)=crate::web_ui::spawn_clipboard(text,move|result|{let _=replies.send(Command::Complete{navigation,completion:web_api::WebCompletion::Clipboard{id:request,result}});}){let _=loaded.runtime.complete(web_api::WebCompletion::Clipboard{id:request,result:Err(error)});}}
                                    else{let _=replies.send(complete(Err(web_api::ApiError::new("NotAllowedError","clipboard permission denied"))));}
                                }
                            }
                        }
                        Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            services.cancel();
                            if let Err(error)=config.profile.close_session(&config.session){eprintln!("browser: session close failed: {error}");}
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
                            services.cancel();
                            let fallback = fetched.clone();
                            let mut page_config=config.clone();
                            page_config.platform=Some(Arc::new(BrowserPlatform{events:events.clone(),wake:wake.clone(),active:token.clone(),navigation}));
                            let loaded = runtime.block_on(load_scripts_configured(
                                &client,
                                fetched,
                                environment,
                                Some((token.clone(), navigation)),
                                page_config,
                            ));
                            match loaded {
                                Ok(mut loaded) => {
                                    forward_services(&mut services,&mut loaded,navigation,&events,&wake);
                                    snapshots=Arc::new(std::sync::atomic::AtomicUsize::new(0));
                                    let Some(page) = send_page(&events, &wake, navigation, &loaded, true,&snapshots)else{continue;};
                                    loaded.page = page.clone();
                                    published=loaded.runtime.revision();
                                    resources = Some(load_resources(
                                        &runtime,
                                        client.clone(),
                                        navigation,
                                        page,
                                        environment,
                                        ResourceChannels{events:events.clone(),wake:wake.clone(),commands:replies.clone()},
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
                            if matches!(event,DomCommand::Click(_)|DomCommand::Tab{..}|DomCommand::Key{pressed:true,..}){loaded.runtime.begin_gesture();}
                            let result = handle_dom(loaded, &event);
                            loaded.runtime.end_gesture();
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
                            if matches!(event,DomCommand::Snapshot){
                                if let Some(page)=send_page(&events,&wake,*id,loaded,false,&snapshots){loaded.page=page.clone();published=loaded.runtime.revision();if let Some(old)=resources.take(){old.abort();}resources=Some(load_resources(&runtime,client.clone(),*id,page,loaded.runtime_environment(),ResourceChannels{events:events.clone(),wake:wake.clone(),commands:replies.clone()}));}
                                let(back,forward)=loaded.runtime.history_directions();let _=events.send(FetchEvent::SameDocument{id:*id,url:loaded.runtime.url(),back,forward,replace:true,traversal:None});wake();
                            }
                        }
                        Ok(Command::Complete{navigation,completion})=>{
                            let Some((id,loaded))=&mut current else{continue;};
                            if *id!=navigation||token.load(Ordering::Relaxed)!=*id{continue;}
                            if let Err(error)=loaded.runtime.complete(completion){report(&events,&wake,*id,&error);}
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if let Some((id, loaded)) = &mut current {
                                if token.load(Ordering::Relaxed) != *id {
                                    current = None;
                                    continue;
                                }
                                if !loaded.runtime.next_timer().is_zero(){continue;}
                                if let Err(error) = loaded.runtime.poll_timers() {
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
                            }
                        }
                    }
                }
                }));
                if result.is_err() {
                    let _ = crash_events.send(FetchEvent::Crashed { id: crash_active.fetch_add(1, Ordering::Relaxed), message: "This tab's script worker stopped unexpectedly. Other tabs are still running; reload to recover.".into() }); crash_wake();
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
    pub fn complete(&self, navigation: u64, completion: web_api::WebCompletion) {
        let _ = self.commands.send(Command::Complete {
            navigation,
            completion,
        });
    }
    pub fn decision(
        &self,
        navigation: u64,
        request: u64,
        name: String,
        decision: String,
        action: crate::web_ui::Action,
    ) {
        let _ = self.commands.send(Command::Decision {
            navigation,
            request,
            name,
            decision,
            action,
        });
    }
    pub fn revoke(&self, navigation: u64, all: bool) {
        let _ = self.commands.send(Command::Revoke { navigation, all });
    }
    pub fn shutdown(&mut self) {
        self.active.fetch_add(1, Ordering::Relaxed);
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for ScriptWorker {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.shutdown();
        }
    }
}
struct BrowserPlatform {
    events: mpsc::Sender<FetchEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    active: Arc<AtomicU64>,
    navigation: u64,
}
struct ResourceChannels {
    events: mpsc::Sender<FetchEvent>,
    wake: Arc<dyn Fn() + Send + Sync>,
    commands: mpsc::Sender<Command>,
}
impl web_api::runtime::Platform for BrowserPlatform {
    fn dialog(&self, request: web_api::runtime::Dialog) -> web_api::Result<Option<String>> {
        let (tx, rx) = mpsc::channel();
        self.events
            .send(FetchEvent::Dialog {
                id: self.navigation,
                request,
                reply: tx,
            })
            .map_err(|e| web_api::ApiError::new("AbortError", e.to_string()))?;
        (self.wake)();
        let start = std::time::Instant::now();
        loop {
            if self.active.load(Ordering::Relaxed) != self.navigation
                || start.elapsed() > Duration::from_secs(120)
            {
                return Err(web_api::ApiError::new("AbortError", "dialog cancelled"));
            }
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(value) => return Ok(value),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Ok(None),
            }
        }
    }
}
fn report(
    events: &mpsc::Sender<FetchEvent>,
    wake: &Arc<dyn Fn() + Send + Sync>,
    id: u64,
    error: &js::JsError,
) {
    let _ = events.send(FetchEvent::ScriptError {
        id,
        message: error.to_string(),
        stopped: error.is_resource_limit(),
    });
    wake();
}
fn forward_services(
    services: &mut web_api::services::Services,
    loaded: &mut LoadedScripts,
    id: u64,
    events: &mpsc::Sender<FetchEvent>,
    wake: &Arc<dyn Fn() + Send + Sync>,
) {
    use web_api::WebCommand;
    for command in loaded.runtime.commands() {
        match command {
            WebCommand::Navigate { url, replace } => {
                let _ = events.send(FetchEvent::WebNavigate { id, url, replace });
                wake();
            }
            WebCommand::SameDocument {
                url,
                replace,
                traversal,
            } => {
                let (back, forward) = loaded.runtime.history_directions();
                let _ = events.send(FetchEvent::SameDocument {
                    id,
                    url,
                    back,
                    forward,
                    replace,
                    traversal,
                });
                wake();
            }
            other => {
                let fallback = other.clone();
                match services.dispatch(other) {
                    Ok(Some(command)) => {
                        let command = match command {
                            WebCommand::Permission {
                                id: request,
                                name,
                                gesture,
                            } => {
                                if !web_api::secure_context(&loaded.runtime.url())
                                    || (!gesture
                                        && matches!(
                                            name.as_str(),
                                            "notifications" | "fullscreen" | "persistent-storage"
                                        ))
                                {
                                    let _ = loaded.runtime.complete(
                                        web_api::WebCompletion::Permission {
                                            id: request,
                                            name,
                                            decision: "denied".into(),
                                        },
                                    );
                                    continue;
                                }
                                WebCommand::Permission {
                                    id: request,
                                    name,
                                    gesture,
                                }
                            }
                            WebCommand::Notify { title, body } => {
                                let events = events.clone();
                                let wake = wake.clone();
                                if let Err(error) =
                                    crate::web_ui::spawn_notification(title, body, move |error| {
                                        let _ = events.send(FetchEvent::ScriptError {
                                            id,
                                            message: error.to_string(),
                                            stopped: false,
                                        });
                                        wake();
                                    })
                                {
                                    loaded.runtime.handle().console(error.to_string());
                                }
                                continue;
                            }
                            command => command,
                        };
                        let origin = web_api::origin(&loaded.runtime.url()).unwrap_or_default();
                        let _ = events.send(FetchEvent::WebRequest {
                            id,
                            origin,
                            command,
                        });
                        wake();
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let completion = match fallback {
                            WebCommand::ModuleLoad { id, .. } => {
                                Some(web_api::WebCompletion::Module {
                                    id,
                                    result: Err(error.clone()),
                                })
                            }
                            WebCommand::Fetch { id, .. } => Some(web_api::WebCompletion::Fetch {
                                id,
                                result: Err(error.clone()),
                            }),
                            WebCommand::WorkerStart { id, .. }
                            | WebCommand::WorkerMessage { id, .. }
                            | WebCommand::SocketOpen { id, .. }
                            | WebCommand::SocketSend { id, .. }
                            | WebCommand::SocketClose { id, .. }
                            | WebCommand::EventSource { id, .. } => {
                                Some(web_api::WebCompletion::Error {
                                    id,
                                    error: error.clone(),
                                })
                            }
                            _ => None,
                        };
                        if let Some(completion) = completion {
                            if let Err(error) = loaded.runtime.complete(completion) {
                                report(events, wake, id, &error);
                            }
                        }
                        loaded.runtime.handle().console(error.to_string());
                    }
                }
            }
        }
    }
}
fn send_page(
    events: &mpsc::Sender<FetchEvent>,
    wake: &Arc<dyn Fn() + Send + Sync>,
    id: u64,
    loaded: &LoadedScripts,
    initial: bool,
    snapshots: &Arc<std::sync::atomic::AtomicUsize>,
) -> Option<Arc<Page>> {
    let lease = PageLease::acquire(snapshots);
    if lease.is_none() && !initial {
        return None;
    }
    let mut page = (*loaded.page).clone();
    page.document = Arc::new(loaded.runtime.document());
    page.url = loaded.runtime.url();
    let page = Arc::new(page);
    let styles =
        page.computed_styles_focused(loaded.runtime_environment(), loaded.runtime.focused());
    if initial {
        let _ = events.send(FetchEvent::Done {
            id,
            url: page.url.clone(),
            result: Ok((page.clone(), styles.clone())),
        });
    }
    if let Some(lease) = lease {
        let _ = events.send(FetchEvent::ScriptPage {
            id,
            page: page.clone(),
            styles,
            focused: loaded.runtime.focused(),
            console: loaded.runtime.console(),
            _lease: lease,
        });
    }
    wake();
    Some(page)
}
fn load_resources(
    runtime: &tokio::runtime::Runtime,
    client: Arc<Client>,
    id: u64,
    page: Arc<Page>,
    environment: Environment,
    channels: ResourceChannels,
) -> tokio::task::JoinHandle<()> {
    let ResourceChannels {
        events,
        wake,
        commands,
    } = channels;
    runtime.spawn(async move {
        let styles = page.computed_styles(environment);
        let image_nodes = page
            .document
            .get_elements_by_tag_name("img")
            .into_iter()
            .take(crate::images::MAX_IMAGE_REQUESTS)
            .filter_map(|node| {
                page.document
                    .get_attribute(node, "src")
                    .and_then(|src| page.url.join(src).ok())
                    .map(|url| (node, url, page.document.image_epoch(node)))
            })
            .collect::<Vec<_>>();
        let publish_image = |url: &url::Url| {
            let Some(image) = page.image_cache.get(url) else {
                return;
            };
            let tainted = page
                .image_taint
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(url)
                .copied()
                .unwrap_or_else(|| url.scheme() != "data" && !web_api::same_origin(&page.url, url));
            for (node, source, epoch) in &image_nodes {
                if source == url {
                    let _ = commands.send(Command::Image {
                        navigation: id,
                        node: *node,
                        url: url.clone(),
                        epoch: *epoch,
                        image: image.as_ref().map(|image| web_api::runtime::CanvasImage {
                            width: image.width,
                            height: image.height,
                            pixels: image.rgba.clone(),
                            tainted,
                        }),
                    });
                }
            }
        };
        let mut seen = std::collections::HashSet::new();
        for (_, url, _) in &image_nodes {
            if seen.insert(url.clone()) {
                publish_image(url);
            }
        }
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
            publish_image(&url);
            let _ = events.send(FetchEvent::Image {
                id,
                page: page.clone(),
                url,
                success,
            });
            wake();
        });
        let favicon = async {
            let Some(url) = page
                .document
                .get_elements_by_tag_name("link")
                .into_iter()
                .find_map(|node| {
                    page.document.get_attribute(node, "rel").filter(|rel| {
                        rel.split_ascii_whitespace()
                            .any(|name| name.eq_ignore_ascii_case("icon"))
                    })?;
                    page.document
                        .get_attribute(node, "href")
                        .and_then(|href| page.url.join(href).ok())
                })
            else {
                return;
            };
            let mut icons = page.icons.lock().await;
            if let Some(cached) = icons.cache.get(&url) {
                if let Some(image) = cached {
                    let _ = events.send(FetchEvent::Favicon {
                        id,
                        image: image.clone(),
                    });
                    wake();
                }
                return;
            }
            if icons.attempts >= 8 {
                return;
            }
            icons.attempts += 1;
            let allowed = |target: &url::Url| {
                crate::page::allowed_subresource(&page.url, target)
                    && page
                        .policy
                        .resource_allowed("img-src", &page.url, target, None)
            };
            let bytes = if url.scheme() == "data"
                && page
                    .policy
                    .resource_allowed("img-src", &page.url, &url, None)
            {
                crate::images::data_image(&url).filter(|bytes| bytes.len() <= 256 * 1024)
            } else if allowed(&url) {
                client
                    .fetch_script_for(&page.url, &url, 256 * 1024, &allowed)
                    .await
                    .ok()
                    .map(|f| f.bytes)
            } else {
                None
            };
            let image = if let Some(bytes) = bytes {
                tokio::task::spawn_blocking(move || paint::images::decode_icon(&bytes))
                    .await
                    .ok()
                    .and_then(Result::ok)
            } else {
                None
            };
            icons.cache.insert(url, image.clone());
            if let Some(image) = image {
                let _ = events.send(FetchEvent::Favicon { id, image });
                wake();
            }
        };
        tokio::join!(fonts, images, favicon);
    })
}
fn handle_dom(loaded: &mut LoadedScripts, event: &DomCommand) -> Result<(), js::JsError> {
    match event {
        DomCommand::Snapshot => Ok(()),
        DomCommand::ExitFullscreen => loaded.runtime.fullscreen_exit(),
        DomCommand::Geometry {
            layout,
            offsets,
            viewport,
        } => loaded
            .runtime
            .geometry(crate::viewport::web_geometry(layout, offsets), *viewport),
        DomCommand::Viewport(viewport) => loaded.runtime.viewport(*viewport),
        DomCommand::HistoryGo(delta) => loaded.runtime.history_go(*delta),
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
            loaded.runtime.dispatch(*node, "click", "")?;
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
