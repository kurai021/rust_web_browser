//! Per-page Level 1 host bindings. No network/files/other contexts are exposed.
mod host;
pub mod policy;

use css::{ComputedStyles, Environment, Stylesheet};
use html::{Document, NodeId};
use js::{HostHooks, JsError, JsValue, NullHost, Vm};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};
use url::Url;

pub(crate) const EVENT_BASE: u64 = 1 << 56;
pub(crate) const EVENT_TARGET_BASE: u64 = 1 << 55;
pub(crate) const WINDOW: u64 = u64::MAX - 1;
pub(crate) fn node_host(node: NodeId, kind: u64) -> u64 {
    node as u64 * 8 + kind
}
pub(crate) fn node_id(host: u64) -> Option<NodeId> {
    (host < EVENT_TARGET_BASE).then_some((host / 8) as usize)
}
#[derive(Clone)]
pub(crate) struct Listener {
    node: NodeId,
    event: String,
    callback: JsValue,
    capture: bool,
    once: bool,
    passive: bool,
    inline: bool,
}
#[derive(Clone)]
pub(crate) struct EventData {
    name: String,
    target: NodeId,
    current: NodeId,
    phase: u8,
    bubbles: bool,
    cancelable: bool,
    prevented: bool,
    stopped: bool,
    immediate: bool,
    passive: bool,
    detail: JsValue,
    key: String,
    queue: VecDeque<Listener>,
    old_checked: Option<bool>,
    timestamp: f64,
}
pub(crate) struct Timer {
    callback: JsValue,
    args: Vec<JsValue>,
    due: Instant,
    interval: Option<Duration>,
    animation_frame: bool,
}
pub(crate) struct State {
    pub document: Document,
    pub url: Url,
    pub stylesheets: Vec<Stylesheet>,
    pub environment: Environment,
    pub listeners: Vec<Listener>,
    pub events: HashMap<u64, EventData>,
    pub timers: HashMap<u64, Timer>,
    pub running_timer: Option<u64>,
    pub cancel_running_timer: bool,
    pub next: u64,
    pub revision: u64,
    pub ready: String,
    pub focused: Option<NodeId>,
    pub policy: policy::Policy,
    pub functions: HashMap<String, JsValue>,
    pub console: Vec<String>,
    pub queued: VecDeque<(NodeId, String)>,
    pub navigation: Option<Url>,
    pub computed: Option<ComputedStyles>,
    pub parsing: bool,
    pub disabled: bool,
    pub started: Instant,
}
impl State {
    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.computed = None;
    }
    fn make_event(
        &mut self,
        name: String,
        target: NodeId,
        bubbles: bool,
        cancelable: bool,
        detail: JsValue,
        key: String,
    ) -> u64 {
        self.next = self.next.wrapping_add(1);
        let id = EVENT_BASE + self.next;
        self.events.insert(
            id,
            EventData {
                name,
                target,
                current: target,
                phase: 0,
                bubbles,
                cancelable,
                prevented: false,
                stopped: false,
                immediate: false,
                passive: false,
                detail,
                key,
                queue: VecDeque::new(),
                old_checked: None,
                timestamp: self.started.elapsed().as_secs_f64() * 1000.0,
            },
        );
        id
    }
}
pub struct DomRuntime {
    pub vm: Vm,
    state: Rc<RefCell<State>>,
}
impl DomRuntime {
    pub fn new(
        document: Document,
        url: Url,
        stylesheets: Vec<Stylesheet>,
        environment: Environment,
        policy: policy::Policy,
    ) -> Result<Self, JsError> {
        let state = Rc::new(RefCell::new(State {
            document,
            url,
            stylesheets,
            environment,
            listeners: Vec::new(),
            events: HashMap::new(),
            timers: HashMap::new(),
            running_timer: None,
            cancel_running_timer: false,
            next: 0,
            revision: 0,
            ready: "loading".into(),
            focused: None,
            policy,
            functions: HashMap::new(),
            console: Vec::new(),
            queued: VecDeque::new(),
            navigation: None,
            computed: None,
            parsing: true,
            disabled: false,
            started: Instant::now(),
        }));
        let mut vm = Vm::new(NullHost);
        let host = host::DomHost {
            state: state.clone(),
        };
        host::install(&mut vm, &state)?;
        vm.replace_host(host);
        Ok(Self { vm, state })
    }
    pub fn document(&self) -> Document {
        self.state.borrow().document.clone()
    }
    pub fn replace_document(&mut self, document: Document) {
        self.state.borrow_mut().document = document;
    }
    pub fn stylesheets(&mut self, sheets: Vec<Stylesheet>) {
        self.state.borrow_mut().stylesheets = sheets;
        self.state.borrow_mut().computed = None;
    }
    pub fn set_policy(&mut self, policy: policy::Policy) {
        self.state.borrow_mut().policy = policy;
    }
    pub fn revision(&self) -> u64 {
        self.state.borrow().revision
    }
    pub fn console(&self) -> Vec<String> {
        self.state.borrow().console.clone()
    }
    pub fn focused(&self) -> Option<NodeId> {
        self.state.borrow().focused
    }
    pub fn environment(&mut self, environment: Environment) {
        let mut state = self.state.borrow_mut();
        if state.environment != environment {
            state.environment = environment;
            state.changed();
        }
    }
    pub fn current_environment(&self) -> Environment {
        self.state.borrow().environment
    }
    pub fn request_navigation(&mut self, url: Url) {
        self.state.borrow_mut().navigation = Some(url);
    }
    pub fn take_navigation(&mut self) -> Option<Url> {
        self.state.borrow_mut().navigation.take()
    }
    pub fn eval(&mut self, source: &str) -> Result<JsValue, JsError> {
        if self.state.borrow().disabled {
            return Err(JsError::new(
                js::ErrorKind::Cancelled,
                "page scripting is stopped",
            ));
        }
        let result = self.vm.eval_source(source);
        let result = result.and_then(|value| self.drain_queued().map(|()| value));
        self.finish_task(result)
    }
    fn finish_task<T>(&mut self, result: Result<T, JsError>) -> Result<T, JsError> {
        if result.as_ref().is_err_and(JsError::is_resource_limit) {
            let mut state = self.state.borrow_mut();
            state.disabled = true;
            state.timers.clear();
            state.queued.clear();
            state.running_timer = None;
        }
        result
    }
    pub fn is_stopped(&self) -> bool {
        self.state.borrow().disabled
    }
    pub fn set_ready(&mut self, value: &str) {
        self.state.borrow_mut().ready = value.into();
        if value == "complete" {
            self.state.borrow_mut().parsing = false;
        }
    }
    pub fn dispatch(&mut self, target: NodeId, name: &str, key: &str) -> Result<bool, JsError> {
        let result = self
            .dispatch_one(target, name, key)
            .and_then(|allowed| self.drain_queued().map(|()| allowed));
        self.finish_task(result)
    }
    fn dispatch_one(&mut self, target: NodeId, name: &str, key: &str) -> Result<bool, JsError> {
        if self.state.borrow().disabled {
            return Ok(false);
        }
        let event = self.state.borrow_mut().make_event(
            name.into(),
            target,
            !matches!(name, "focus" | "blur" | "load"),
            matches!(name, "click" | "keydown" | "submit"),
            JsValue::Undefined,
            key.into(),
        );
        let event = self.vm.host_object(event, "Event")?;
        let this = self.vm.host_object(node_host(target, 0), "Element")?;
        let function = self.vm.native("host:dispatchEvent")?;
        let value = self.vm.call(function, this, vec![event])?;
        Ok(value.truthy())
    }
    pub fn input(&mut self, target: NodeId, value: String) -> Result<(), JsError> {
        if let Err(error) = self.vm.check_string_bytes(value.len()) {
            return self.finish_task(Err(error));
        }
        self.state
            .borrow_mut()
            .document
            .set_control_value(target, value);
        self.state.borrow_mut().changed();
        self.dispatch(target, "input", "")?;
        Ok(())
    }
    pub fn focus(&mut self, target: Option<NodeId>) -> Result<(), JsError> {
        let previous = self.state.borrow().focused;
        if previous == target {
            return Ok(());
        }
        if let Some(previous) = previous {
            self.dispatch(previous, "blur", "")?;
        }
        self.state.borrow_mut().focused = target;
        self.state.borrow_mut().changed();
        if let Some(target) = target {
            self.dispatch(target, "focus", "")?;
        }
        Ok(())
    }
    fn drain_queued(&mut self) -> Result<(), JsError> {
        for _ in 0..100 {
            let queued = self.state.borrow_mut().queued.pop_front();
            let Some((node, event)) = queued else {
                break;
            };
            self.dispatch_one(node, &event, "")?;
        }
        if !self.state.borrow().queued.is_empty() {
            self.state.borrow_mut().queued.clear();
            return Err(JsError::new(
                js::ErrorKind::Timeout,
                "event recursion budget exceeded",
            ));
        }
        Ok(())
    }
    pub fn next_timer(&self) -> Duration {
        self.state
            .borrow()
            .timers
            .values()
            .map(|t| t.due.saturating_duration_since(Instant::now()))
            .min()
            .unwrap_or(Duration::from_millis(100))
            .min(Duration::from_millis(100))
    }
    pub fn poll_timers(&mut self) -> Result<(), JsError> {
        let result = self.poll_timers_inner();
        self.finish_task(result)
    }
    fn poll_timers_inner(&mut self) -> Result<(), JsError> {
        if self.state.borrow().disabled {
            return Ok(());
        }
        let now = Instant::now();
        let mut ids = self
            .state
            .borrow()
            .timers
            .iter()
            .filter(|(_, t)| t.due <= now)
            .map(|(&id, timer)| (timer.due, id))
            .collect::<Vec<_>>();
        ids.sort_unstable();
        for (_, id) in ids.into_iter().take(100) {
            let timer = self.state.borrow_mut().timers.remove(&id);
            let Some(mut timer) = timer else {
                continue;
            };
            if timer.animation_frame {
                timer.args = vec![JsValue::Number(
                    self.state.borrow().started.elapsed().as_secs_f64() * 1000.0,
                )];
            }
            self.state.borrow_mut().running_timer = Some(id);
            self.state.borrow_mut().cancel_running_timer = false;
            let result = if let JsValue::String(source) = &timer.callback {
                self.vm.eval_source(source)
            } else {
                self.vm.call(
                    timer.callback.clone(),
                    self.vm.global_value(),
                    timer.args.clone(),
                )
            };
            if result.as_ref().is_err_and(JsError::is_resource_limit) {
                return result.map(|_| ());
            }
            let cancelled = self.state.borrow().cancel_running_timer;
            self.state.borrow_mut().running_timer = None;
            if let Some(interval) = timer.interval.filter(|_| !cancelled) {
                timer.due = Instant::now() + interval;
                self.state.borrow_mut().timers.insert(id, timer);
            }
            if let Err(error) = result {
                self.state.borrow_mut().console.push(error.to_string());
            }
        }
        self.drain_queued()?;
        self.vm.gc_collect();
        Ok(())
    }
    pub fn gc_collect(&mut self) {
        self.vm.gc_collect();
    }
    pub fn listener_count(&self) -> usize {
        self.state.borrow().listeners.len()
    }
}
