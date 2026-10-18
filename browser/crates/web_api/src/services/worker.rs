//! A dedicated worker owns a fresh in-house VM; only data-only messages cross.
use super::*;
use dom_bindings::js::{CloneData, ErrorKind, HostHooks, HostResult, JsError, JsValue, Vm};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc as sync,
    },
    time::Instant,
};
enum Input {
    Message(CloneData),
    Close,
}
pub(super) struct Worker {
    tx: sync::SyncSender<Input>,
    cancel: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub fn spawn(
        id: u64,
        url: Url,
        policy: Csp,
        client: Arc<net::Client>,
        handle: tokio::runtime::Handle,
        events: mpsc::Sender<Completion>,
    ) -> Result<Self> {
        let (tx, rx) = sync::sync_channel(16);
        let cancel = Arc::new(AtomicU64::new(0));
        let token = cancel.clone();
        let thread=std::thread::Builder::new().name(format!("dedicated-worker-{id}")).stack_size(8*1024*1024).spawn(move||{
            let result=(||->std::result::Result<(),JsError>{
                let allow=|target:&Url|same_origin(&url,target)&&policy.allows("worker-src",&url,target,None);
                let fetched=handle.block_on(async{tokio::select!{
                    source=client.fetch_script_for(&url,&url,MAX_BODY,&allow)=>source.map_err(|e|JsError::new(ErrorKind::Error,e.to_string())),
                    _=async{while token.load(Ordering::Relaxed)==0{tokio::time::sleep(Duration::from_millis(10)).await;}}=>Err(JsError::new(ErrorKind::Cancelled,"worker terminated")),
                }})?;
                if fetched.content_type.as_deref().is_some_and(|s|s.starts_with("text/html")||s.starts_with("image/")){return Err(JsError::new(ErrorKind::TypeError,"worker MIME"));}
                let state=Rc::new(RefCell::new(State{callback:JsValue::Undefined,listeners:Vec::new(),out:Vec::new(),closed:false,timers:Vec::new(),next:0,functions:HashMap::new(),eval:policy.eval_allowed()&&Csp::parse(&fetched.script_policies).eval_allowed()}));
                let mut vm=Vm::new(WorkerHost(state.clone()));vm.limits.navigation=Some((token.clone(),0));vm.limits.max_memory_bytes=64*1024*1024;vm.limits.max_objects=100_000;
                vm.set_global_host(1,"DedicatedWorkerGlobalScope")?;let global=vm.global_value();vm.set_global("self",global)?;
                vm.set_global("onmessage",JsValue::Undefined)?;
                for name in ["postMessage","close","addEventListener","removeEventListener","setTimeout","clearTimeout"]{let function=vm.native(&format!("host:{name}"))?;state.borrow_mut().functions.insert(name.into(),function.clone());vm.set_global(name,function)?;}
                 vm.set_source_url(fetched.url.as_str());vm.eval_source(&String::from_utf8_lossy(&fetched.bytes))?;vm.drain_microtasks()?;
                loop {
                    if token.load(Ordering::Relaxed)!=0{return Ok(());}
                    for data in std::mem::take(&mut state.borrow_mut().out){events.try_send(Completion::Message{id,data}).map_err(|_|JsError::new(ErrorKind::MemoryLimit,"worker output queue"))?;}
                    if state.borrow().closed{return Ok(());}
                    let due=Instant::now();let mut callbacks=Vec::new();state.borrow_mut().timers.retain(|(_,at,callback)|{if *at<=due{callbacks.push(callback.clone());false}else{true}});
                     for callback in callbacks{vm.call(callback,JsValue::Undefined,Vec::new())?;vm.drain_microtasks()?;}
                    match rx.recv_timeout(Duration::from_millis(10)){
                        Ok(Input::Close)|Err(sync::RecvTimeoutError::Disconnected)=>return Ok(()),
                        Err(sync::RecvTimeoutError::Timeout)=>{},
                        Ok(Input::Message(data))=>{vm.begin_task();let data=vm.deserialize_clone(&data)?;let event=vm.object()?;vm.set(event.clone(),"data",data)?;
                            let mut callbacks=state.borrow().listeners.clone();let callback=state.borrow().callback.clone();if vm.callable(&callback){callbacks.insert(0,callback);}
                             for callback in callbacks{let global=vm.global_value();vm.call_in_task(callback,global,vec![event.clone()])?;}
                        }
                    }
                    vm.drain_microtasks()?;vm.gc_collect();
                }
            })();
            if let Err(error)=result{if error.kind!=ErrorKind::Cancelled{let _=events.try_send(Completion::Error{id,error:network(error)});}}
            let _=events.try_send(Completion::WorkerClosed{id});
        }).map_err(network)?;
        Ok(Self {
            tx,
            cancel,
            thread: Some(thread),
        })
    }
    pub fn send(&self, data: CloneData) -> Result<()> {
        self.tx
            .try_send(Input::Message(data))
            .map_err(|_| ApiError::new("QuotaExceededError", "worker message queue"))
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(1, Ordering::Relaxed);
        let _ = self.tx.try_send(Input::Close);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
struct State {
    callback: JsValue,
    listeners: Vec<JsValue>,
    out: Vec<CloneData>,
    closed: bool,
    timers: Vec<(u64, Instant, JsValue)>,
    next: u64,
    functions: HashMap<String, JsValue>,
    eval: bool,
}
#[derive(Clone)]
struct WorkerHost(Rc<RefCell<State>>);
impl HostHooks for WorkerHost {
    fn fork(&self) -> Option<Box<dyn HostHooks>> {
        Some(Box::new(self.clone()))
    }
    fn allow_dynamic_code(&self) -> bool {
        self.0.borrow().eval
    }
    fn get(
        &mut self,
        _: &mut Vm,
        id: u64,
        key: &str,
    ) -> std::result::Result<Option<JsValue>, JsError> {
        Ok((id == 1 && key == "onmessage").then(|| self.0.borrow().callback.clone()))
    }
    fn set(
        &mut self,
        _: &mut Vm,
        id: u64,
        key: &str,
        value: JsValue,
    ) -> std::result::Result<bool, JsError> {
        if id == 1 && key == "onmessage" {
            self.0.borrow_mut().callback = value;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    fn roots(&self, _: &std::collections::HashSet<u64>) -> Vec<JsValue> {
        let s = self.0.borrow();
        s.functions
            .values()
            .cloned()
            .chain(s.listeners.iter().cloned())
            .chain(std::iter::once(s.callback.clone()))
            .chain(s.timers.iter().map(|(_, _, f)| f.clone()))
            .collect()
    }
    fn retained_bytes(&self) -> usize {
        let s = self.0.borrow();
        s.out.iter().map(CloneData::retained_bytes).sum::<usize>() + s.timers.len() * 128
    }
    fn call(
        &mut self,
        vm: &mut Vm,
        name: &str,
        _: JsValue,
        args: &[JsValue],
    ) -> std::result::Result<HostResult, JsError> {
        let arg = |i| args.get(i).cloned().unwrap_or(JsValue::Undefined);
        let value = match name {
            "postMessage" => {
                let data = vm.serialize_clone(arg(0))?;
                let mut s = self.0.borrow_mut();
                if s.out.len() >= 16 {
                    return Err(JsError::new(ErrorKind::MemoryLimit, "worker output queue"));
                }
                s.out.push(data);
                JsValue::Undefined
            }
            "close" => {
                self.0.borrow_mut().closed = true;
                JsValue::Undefined
            }
            "addEventListener" | "removeEventListener" => {
                let event = vm.to_string(arg(0))?;
                if event == "message" && vm.callable(&arg(1)) {
                    let mut s = self.0.borrow_mut();
                    if name == "removeEventListener" {
                        s.listeners.retain(|f| *f != arg(1));
                    } else if !s.listeners.contains(&arg(1)) {
                        if s.listeners.len() >= 256 {
                            return Err(JsError::new(ErrorKind::MemoryLimit, "worker listeners"));
                        }
                        s.listeners.push(arg(1));
                    }
                }
                JsValue::Undefined
            }
            "setTimeout" => {
                if !vm.callable(&arg(0)) {
                    return Err(JsError::new(ErrorKind::TypeError, "worker timer callback"));
                }
                let delay = vm.to_number(arg(1))?.clamp(0.0, 60_000.0) as u64;
                let mut s = self.0.borrow_mut();
                if s.timers.len() >= 1024 {
                    return Err(JsError::new(ErrorKind::MemoryLimit, "worker timers"));
                }
                s.next += 1;
                let id = s.next;
                s.timers
                    .push((id, Instant::now() + Duration::from_millis(delay), arg(0)));
                JsValue::Number(id as f64)
            }
            "clearTimeout" => {
                let id = vm.to_number(arg(0))? as u64;
                self.0.borrow_mut().timers.retain(|(i, _, _)| *i != id);
                JsValue::Undefined
            }
            _ => return Err(JsError::new(ErrorKind::TypeError, "worker host method")),
        };
        Ok(HostResult::value(value))
    }
}
