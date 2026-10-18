//! Page-local host objects, deferred results and message-only service boundary.
mod bindings;
mod canvas_bindings;
mod deferred;
mod indexed_db;
mod observers;
pub use canvas_bindings::CanvasImage;
pub use observers::{GeometryBox, GeometryRect};
mod utilities;
use crate::{
    browsing::History,
    headers::Headers,
    http::{Request, Response},
    policy::Csp,
    storage::Profile,
    *,
};
pub use bindings::install;
use dom_bindings::{
    js::{CloneData, ErrorKind, HostHooks, HostResult, JsError, JsValue, Vm},
    DomHandle, DomRuntime,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    rc::Rc,
    sync::Arc,
};
pub const WEB_BASE: u64 = 1 << 57;
type ParamValues = Vec<(String, String)>;
pub const WINDOW: u64 = u64::MAX - 1;
const LOCATION: u64 = WEB_BASE + 1;
const HISTORY: u64 = WEB_BASE + 2;
const NAVIGATOR: u64 = WEB_BASE + 3;
const LOCAL: u64 = WEB_BASE + 4;
const SESSION: u64 = WEB_BASE + 5;
const CLIPBOARD: u64 = WEB_BASE + 6;
const GEO: u64 = WEB_BASE + 7;
const PERMISSIONS: u64 = WEB_BASE + 8;
const STORAGE_MANAGER: u64 = WEB_BASE + 9;
const INDEXED_DB: u64 = WEB_BASE + 10;
#[derive(Debug, Clone)]
pub enum Command {
    ModuleLoad {
        known: Vec<String>,
        id: u64,
        url: Url,
        document: Url,
        policy: Csp,
    },
    Fetch {
        id: u64,
        request: Request,
        document: Url,
        policy: Csp,
    },
    Abort {
        id: u64,
    },
    Navigate {
        url: Url,
        replace: bool,
    },
    SameDocument {
        url: Url,
        replace: bool,
    },
    Permission {
        id: u64,
        name: String,
        gesture: bool,
    },
    Clipboard {
        id: u64,
        text: Option<String>,
        gesture: bool,
        permission: Option<String>,
    },
    Popup {
        url: Url,
        gesture: bool,
    },
    Notice(String),
    WorkerStart {
        id: u64,
        url: Url,
        policy: Csp,
    },
    WorkerMessage {
        id: u64,
        data: CloneData,
    },
    WorkerStop {
        id: u64,
    },
    SocketOpen {
        id: u64,
        url: Url,
        protocols: Vec<String>,
        document: Url,
        policy: Csp,
    },
    SocketSend {
        id: u64,
        data: Vec<u8>,
        text: bool,
    },
    SocketClose {
        id: u64,
        code: u16,
        reason: String,
    },
    EventSource {
        id: u64,
        url: Url,
        credentials: bool,
        document: Url,
        policy: Csp,
    },
    Fullscreen {
        enter: bool,
    },
    Notify {
        title: String,
        body: String,
    },
}
#[derive(Debug, Clone)]
pub enum Completion {
    Module {
        id: u64,
        result: Result<crate::modules::ModuleGraph>,
    },
    WorkerClosed {
        id: u64,
    },
    Text {
        id: u64,
        text: String,
    },
    Binary {
        id: u64,
        bytes: Vec<u8>,
    },
    StreamRetry {
        id: u64,
    },
    Sent {
        id: u64,
        bytes: usize,
    },
    Fetch {
        id: u64,
        result: Result<Response>,
    },
    Permission {
        id: u64,
        name: String,
        decision: String,
    },
    Clipboard {
        id: u64,
        result: Result<String>,
    },
    Message {
        id: u64,
        data: CloneData,
    },
    Opened {
        id: u64,
        protocol: String,
    },
    Closed {
        id: u64,
        code: u16,
        reason: String,
    },
    Error {
        id: u64,
        error: ApiError,
    },
    ServerEvent {
        id: u64,
        event: String,
        data: String,
        last_id: String,
    },
}
#[derive(Debug, Clone)]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
}
#[derive(Debug, Clone)]
pub struct Dialog {
    pub origin: String,
    pub kind: DialogKind,
    pub message: String,
    pub default: String,
}
pub trait Platform: Send + Sync {
    fn dialog(&self, request: Dialog) -> Result<Option<String>>;
}
#[derive(Clone)]
pub struct Config {
    pub profile: Arc<Profile>,
    pub session: String,
    pub cookies: Arc<net::cookies::CookieJar>,
    pub platform: Option<Arc<dyn Platform>>,
    pub language: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            profile: Arc::new(Profile::default()),
            session: "memory-window".into(),
            cookies: Arc::new(net::cookies::CookieJar::default()),
            platform: None,
            language: "en-US".into(),
        }
    }
}
#[derive(Clone)]
pub(super) struct Promise {
    native: JsValue,
    result: Option<(bool, JsValue)>,
}
#[derive(Clone)]
pub(super) struct Xhr {
    operation: Option<u64>,
    mime_override: Option<String>,
    request: Option<Request>,
    response: Option<Response>,
    ready: u8,
    sent: bool,
    response_type: String,
    with_credentials: bool,
    timeout: u64,
    aborted: bool,
}
#[derive(Clone)]
pub(super) enum Kind {
    XhrOperation {
        target: u64,
    },
    CanvasContext {
        node: usize,
    },
    Observer(observers::Observer),
    Idb(indexed_db::Idb),
    GeoRequest {
        success: JsValue,
        failure: JsValue,
    },
    Location,
    History,
    Navigator,
    Storage(bool),
    Clipboard,
    Geo,
    Permissions,
    StorageManager,
    IndexedDb,
    Headers(Headers),
    HeaderView {
        owner: u64,
        immutable: bool,
    },
    Request(Request),
    Response {
        response: Response,
        used: bool,
    },
    Promise(Promise),
    Params {
        values: Vec<(String, String)>,
        owner: Option<u64>,
    },
    Url(Url),
    FormData(Vec<(String, FormValue)>),
    Blob {
        bytes: Vec<u8>,
        mime: String,
    },
    FileReader {
        ready: u8,
        result: JsValue,
        error: JsValue,
    },
    Xhr(Box<Xhr>),
    Worker {
        alive: bool,
    },
    Socket {
        ready: u8,
        protocol: String,
        buffered: usize,
        url: Url,
    },
    EventSource {
        ready: u8,
        url: Url,
        credentials: bool,
    },
}
#[derive(Clone)]
pub(super) enum FormValue {
    Text(String),
    Blob {
        bytes: Vec<u8>,
        mime: String,
        name: String,
    },
}
pub(super) struct Object {
    kind: Kind,
    properties: BTreeMap<String, JsValue>,
    listeners: Vec<(String, JsValue, bool)>,
}
pub(super) enum Job {
    Settle {
        id: u64,
        ok: bool,
        value: JsValue,
    },
    Observer {
        id: u64,
    },
    Database(indexed_db::Task),
    Permission {
        id: u64,
        name: String,
        decision: String,
    },
    ReaderComplete {
        id: u64,
        value: JsValue,
    },
    Callback {
        function: JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    },
    Event {
        target: u64,
        name: String,
        data: JsValue,
    },
    XhrState {
        id: u64,
        operation: u64,
        ready: u8,
    },
    XhrEvent {
        id: u64,
        operation: u64,
        name: String,
        data: JsValue,
    },
    DomEvent {
        name: String,
        data: JsValue,
    },
}
pub(super) struct State {
    dom: DomHandle,
    config: Config,
    next: u64,
    objects: HashMap<u64, Object>,
    functions: HashMap<String, JsValue>,
    jobs: VecDeque<Job>,
    commands: VecDeque<Command>,
    pending: HashSet<u64>,
    history: History,
    gesture: bool,
    grants: HashSet<String>,
    permission_bools: HashSet<u64>,
    used_bodies: HashSet<u64>,
    blocked_databases: HashMap<u64, indexed_db::Task>,
    geometry: HashMap<usize, GeometryBox>,
    geometry_ready: bool,
    viewport: GeometryRect,
    canvases: HashMap<usize, crate::canvas::Canvas>,
    canvas_contexts: HashMap<usize, u64>,
    images: HashMap<usize, CanvasImage>,
    image_versions: HashMap<usize, u64>,
    canvas_fonts: Option<cosmic_text::FontSystem>,
    fullscreen: Option<usize>,
    fullscreen_requests: HashMap<u64, usize>,
}
impl State {
    fn headers(&self, id: u64) -> Option<(Headers, Option<u64>, bool)> {
        match self.objects.get(&id).map(|o| &o.kind) {
            Some(Kind::Headers(h)) => Some((h.clone(), None, false)),
            Some(Kind::HeaderView { owner, immutable }) => {
                match self.objects.get(owner).map(|o| &o.kind) {
                    Some(Kind::Request(r)) => Some((r.headers.clone(), Some(*owner), *immutable)),
                    Some(Kind::Response { response, .. }) => {
                        Some((response.headers.clone(), Some(*owner), *immutable))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
    fn params(&self, id: u64) -> Option<(ParamValues, Option<u64>)> {
        match self.objects.get(&id).map(|o| &o.kind) {
            Some(Kind::Params { values, owner }) => {
                let values = owner
                    .and_then(|owner| self.objects.get(&owner))
                    .and_then(|o| match &o.kind {
                        Kind::Url(url) => Some(
                            url.query_pairs()
                                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                                .collect(),
                        ),
                        _ => None,
                    })
                    .unwrap_or_else(|| values.clone());
                Some((values, *owner))
            }
            _ => None,
        }
    }
    fn allocate(&mut self, kind: Kind) -> Result<u64> {
        if self.objects.len() >= 50_000 {
            return Err(ApiError::new("QuotaExceededError", "web object quota"));
        }
        self.next += 1;
        let id = WEB_BASE + self.next;
        self.objects.insert(
            id,
            Object {
                kind,
                properties: BTreeMap::new(),
                listeners: Vec::new(),
            },
        );
        Ok(id)
    }
    fn function(&self, name: &str) -> JsValue {
        self.functions
            .get(name)
            .cloned()
            .unwrap_or(JsValue::Undefined)
    }
    fn enqueue(&mut self, command: Command) -> Result<()> {
        if let Command::Abort { id } = &command {
            self.commands.retain(|command|!matches!(command,Command::Fetch{id:pending,..}|Command::EventSource{id:pending,..}if pending==id));
        }
        if self.commands.len() >= 1024
            || self
                .commands
                .iter()
                .map(Command::retained_bytes)
                .sum::<usize>()
                .saturating_add(command.retained_bytes())
                > 32 * 1024 * 1024
        {
            match &command {
                Command::Fetch { id, .. }
                | Command::ModuleLoad { id, .. }
                | Command::Permission { id, .. }
                | Command::Clipboard { id, .. }
                | Command::WorkerStart { id, .. }
                | Command::SocketOpen { id, .. }
                | Command::EventSource { id, .. } => {
                    self.pending.remove(id);
                }
                _ => {}
            }
            return Err(ApiError::new("QuotaExceededError", "web command quota"));
        }
        self.commands.push_back(command);
        Ok(())
    }
}
impl Command {
    fn retained_bytes(&self) -> usize {
        fn policy(p: &Csp) -> usize {
            p.inner
                .policies
                .iter()
                .flatten()
                .map(|(k, v)| k.len() + v.iter().map(String::len).sum::<usize>() + 64)
                .sum()
        }
        256 + match self {
            Self::Fetch {
                request, policy: p, ..
            } => {
                request.body.len()
                    + request
                        .headers
                        .0
                        .iter()
                        .map(|(k, v)| k.len() + v.len() + 64)
                        .sum::<usize>()
                    + policy(p)
            }
            Self::WorkerMessage { data, .. } => data.retained_bytes(),
            Self::ModuleLoad {
                url,
                known,
                policy: p,
                ..
            } => url.as_str().len() + known.iter().map(|s| s.len() + 32).sum::<usize>() + policy(p),
            Self::SocketSend { data, .. } => data.len(),
            Self::Clipboard { text, .. } => text.as_ref().map_or(0, String::len),
            Self::Notice(s) => s.len(),
            Self::Notify { title, body } => title.len() + body.len(),
            Self::WorkerStart { policy: p, .. }
            | Self::SocketOpen { policy: p, .. }
            | Self::EventSource { policy: p, .. } => policy(p),
            _ => 0,
        }
    }
}
#[derive(Clone)]
pub(super) struct WebHost {
    state: Rc<RefCell<State>>,
}
pub struct PageRuntime {
    pub dom: DomRuntime,
    state: Rc<RefCell<State>>,
}
impl std::ops::Deref for PageRuntime {
    type Target = DomRuntime;
    fn deref(&self) -> &Self::Target {
        &self.dom
    }
}
impl std::ops::DerefMut for PageRuntime {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.dom
    }
}
impl PageRuntime {
    pub fn new(
        document: dom_bindings::html::Document,
        url: Url,
        sheets: Vec<dom_bindings::css::Stylesheet>,
        environment: dom_bindings::css::Environment,
        policy: dom_bindings::policy::Policy,
        config: Config,
    ) -> std::result::Result<Self, JsError> {
        let dom = DomRuntime::new(document, url.clone(), sheets, environment, policy)?;
        let state = Rc::new(RefCell::new(State {
            dom: dom.handle(),
            config,
            next: 100,
            objects: HashMap::new(),
            functions: HashMap::new(),
            jobs: VecDeque::new(),
            commands: VecDeque::new(),
            pending: HashSet::new(),
            history: History::new(url),
            gesture: false,
            grants: HashSet::new(),
            permission_bools: HashSet::new(),
            used_bodies: HashSet::new(),
            blocked_databases: HashMap::new(),
            geometry: HashMap::new(),
            geometry_ready: false,
            viewport: GeometryRect {
                x: 0.0,
                y: 0.0,
                width: environment.width as f64,
                height: environment.height as f64,
            },
            canvases: HashMap::new(),
            canvas_contexts: HashMap::new(),
            images: HashMap::new(),
            image_versions: HashMap::new(),
            canvas_fonts: None,
            fullscreen: None,
            fullscreen_requests: HashMap::new(),
        }));
        let mut runtime = Self { dom, state };
        runtime
            .dom
            .vm
            .set_source_url(runtime.state.borrow().dom.url().as_str());
        install(&mut runtime)?;
        Ok(runtime)
    }
    pub fn eval(&mut self, source: &str) -> std::result::Result<JsValue, JsError> {
        let result = self.dom.eval(source);
        self.finish_web_task(result)
    }
    pub fn eval_module_graph(
        &mut self,
        graph: &crate::modules::ModuleGraph,
    ) -> std::result::Result<JsValue, JsError> {
        self.dom.vm.begin_task();
        let result = self.install_module_graph(graph);
        self.finish_web_task(result)
    }
    pub fn dispatch(
        &mut self,
        node: usize,
        name: &str,
        key: &str,
    ) -> std::result::Result<bool, JsError> {
        let result = self.dom.dispatch(node, name, key);
        self.finish_web_task(result)
    }
    pub fn input(&mut self, node: usize, value: String) -> std::result::Result<(), JsError> {
        let result = self.dom.input(node, value);
        self.finish_web_task(result)
    }
    pub fn focus(&mut self, node: Option<usize>) -> std::result::Result<(), JsError> {
        let result = self.dom.focus(node);
        self.finish_web_task(result)
    }
    pub fn poll_timers(&mut self) -> std::result::Result<(), JsError> {
        for _ in 0..100 {
            let result = self.dom.poll_one_timer();
            if !self.finish_web_task(result)? {
                break;
            }
        }
        self.dom.gc_collect();
        Ok(())
    }
    pub fn begin_gesture(&mut self) {
        self.state.borrow_mut().gesture = true;
    }
    pub fn url(&self) -> Url {
        self.state.borrow().dom.url()
    }
    pub fn history_directions(&self) -> (bool, bool) {
        let s = self.state.borrow();
        (
            s.history.position > 0,
            s.history.position + 1 < s.history.entries.len(),
        )
    }
    pub fn history_go(&mut self, delta: i32) -> std::result::Result<(), JsError> {
        self.dom.vm.begin_task();
        WebHost {
            state: self.state.clone(),
        }
        .invoke(
            &mut self.dom.vm,
            "history:go",
            HISTORY,
            &[JsValue::Number(delta as f64)],
        )?;
        self.drain_jobs()
    }
    pub fn navigate(&mut self, url: Url, replace: bool) -> std::result::Result<(), JsError> {
        self.dom.vm.begin_task();
        let gesture = self.state.borrow().gesture;
        self.begin_gesture();
        let result = WebHost {
            state: self.state.clone(),
        }
        .navigate(&mut self.dom.vm, url, replace);
        self.state.borrow_mut().gesture = gesture;
        result?;
        self.drain_jobs()
    }
    pub fn permission(&self, name: &str) -> Option<String> {
        let s = self.state.borrow();
        if s.grants.contains(name) {
            Some("granted".into())
        } else {
            s.config.profile.permission(&s.dom.url(), name)
        }
    }
    pub fn record_permission(&mut self, name: &str, decision: &str) -> Result<()> {
        if !matches!(
            name,
            "geolocation"
                | "notifications"
                | "clipboard-read"
                | "fullscreen"
                | "persistent-storage"
        ) {
            return Err(ApiError::new("NotSupportedError", "permission name"));
        }
        let mut s = self.state.borrow_mut();
        if matches!(decision, "always" | "never") {
            s.config.profile.set_permission(
                &s.dom.url(),
                name,
                if decision == "always" {
                    "granted"
                } else {
                    "denied"
                },
            )?;
        }
        if matches!(decision, "once" | "always" | "granted") {
            s.grants.insert(name.into());
        } else {
            s.grants.remove(name);
        }
        Ok(())
    }
    pub fn revoke_permissions(&mut self) -> Result<()> {
        let mut s = self.state.borrow_mut();
        s.config.profile.revoke_permissions()?;
        s.grants.clear();
        drop(s);
        self.reset_permission_state();
        Ok(())
    }
    pub fn revoke_origin_permissions(&mut self) -> Result<()> {
        let mut s = self.state.borrow_mut();
        s.config.profile.revoke_origin_permissions(&s.dom.url())?;
        s.grants.clear();
        drop(s);
        self.reset_permission_state();
        Ok(())
    }
    fn reset_permission_state(&mut self) {
        let ctor = self.state.borrow().function("Notification");
        let _ = self
            .dom
            .vm
            .set(ctor, "permission", JsValue::string("default"));
        let _ = self.fullscreen_exit();
    }
    pub fn fullscreen_exit(&mut self) -> std::result::Result<(), JsError> {
        self.dom.vm.begin_task();
        let result = WebHost {
            state: self.state.clone(),
        }
        .invoke(&mut self.dom.vm, "fullscreen:exit", 0, &[])
        .map(|_| ());
        self.finish_web_task(result)
    }
    pub fn end_gesture(&mut self) {
        self.state.borrow_mut().gesture = false;
    }
    pub fn commands(&mut self) -> Vec<Command> {
        self.state.borrow_mut().commands.drain(..).collect()
    }
    pub fn config(&self) -> Config {
        self.state.borrow().config.clone()
    }
    pub fn has_pending(&self) -> bool {
        !self.state.borrow().pending.is_empty()
    }
    pub fn complete(&mut self, completion: Completion) -> std::result::Result<(), JsError> {
        self.dom.vm.begin_task();
        let result = self.accept(completion);
        self.finish_web_task(result)
    }
    fn finish_web_task<T>(
        &mut self,
        result: std::result::Result<T, JsError>,
    ) -> std::result::Result<T, JsError> {
        if let Err(error) = &result {
            if error.is_resource_limit() {
                let _ = self.dom.stop(error.clone());
                return result;
            }
        }
        if let Err(error) = self.drain_jobs() {
            if error.is_resource_limit() {
                let _ = self.dom.stop(error.clone());
            }
            return Err(error);
        }
        result
    }
}
pub(super) fn error(vm: &mut Vm, error: ApiError) -> JsError {
    let mut e = JsError::new(
        if error.name == "TypeError" {
            ErrorKind::TypeError
        } else {
            ErrorKind::Error
        },
        error.message.clone(),
    );
    if let Ok(object) = vm.error_value(e.clone()) {
        let _ = vm.set(object.clone(), "name", JsValue::string(error.name));
        let _ = vm.set(object.clone(), "message", JsValue::string(error.message));
        e.thrown = Some(object);
    }
    e
}
pub(super) fn wrap(vm: &mut Vm, id: u64, class: &str) -> std::result::Result<JsValue, JsError> {
    vm.host_object(id, class)
}
pub(super) fn get(vm: &mut Vm, value: JsValue, key: &str) -> std::result::Result<JsValue, JsError> {
    if value.object().is_some() {
        vm.get(value, key)
    } else {
        Ok(JsValue::Undefined)
    }
}
pub(super) fn arg(args: &[JsValue], i: usize) -> JsValue {
    args.get(i).cloned().unwrap_or(JsValue::Undefined)
}
pub(super) fn text(
    vm: &mut Vm,
    args: &[JsValue],
    i: usize,
) -> std::result::Result<String, JsError> {
    vm.to_string(arg(args, i))
}
