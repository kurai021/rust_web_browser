//! Realm-owned promises and FIFO jobs. Resolving a thenable never runs its `then`
//! inline; user accessors run without an arena borrow and share the caller budget.
use super::*;
#[derive(Debug, Clone)]
pub struct PromiseNotification {
    pub promise: JsValue,
    pub reason: JsValue,
    pub handled: bool,
}
#[derive(Debug, Clone)]
pub(crate) enum State {
    Pending,
    Fulfilled(JsValue),
    Rejected(JsValue),
}
impl State {
    fn value(&self) -> Option<JsValue> {
        match self {
            Self::Fulfilled(v) | Self::Rejected(v) => Some(v.clone()),
            _ => None,
        }
    }
    fn ok(&self) -> bool {
        matches!(self, Self::Fulfilled(_))
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Capability {
    pub promise: JsValue,
    pub resolve: JsValue,
    pub reject: JsValue,
}
impl Capability {
    fn roots(&self) -> Vec<JsValue> {
        vec![
            self.promise.clone(),
            self.resolve.clone(),
            self.reject.clone(),
        ]
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Reaction {
    pub capability: Capability,
    pub fulfill: JsValue,
    pub reject: JsValue,
}
impl Reaction {
    fn roots(&self) -> Vec<JsValue> {
        let mut out = self.capability.roots();
        out.extend([self.fulfill.clone(), self.reject.clone()]);
        out
    }
}
#[derive(Debug, Clone)]
pub(crate) struct PromiseData {
    pub state: State,
    pub gate: ObjectId,
    pub reactions: Vec<Reaction>,
    pub handled: bool,
    pub reported: bool,
}
impl PromiseData {
    pub fn roots(&self) -> Vec<JsValue> {
        let mut out = vec![JsValue::Object(self.gate)];
        out.extend(self.state.value());
        out.extend(self.reactions.iter().flat_map(Reaction::roots));
        out
    }
    pub fn retained_bytes(&self) -> usize {
        128 + self.state.value().map_or(0, |v| v.retained_bytes()) + self.reactions.len() * 192
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Combine {
    All,
    AllSettled,
    Any,
    Race,
}
#[derive(Debug, Clone)]
pub(crate) enum Internal {
    Gate(bool),
    Capability {
        resolve: JsValue,
        reject: JsValue,
        called: bool,
    },
    Aggregate {
        capability: Capability,
        values: Vec<Option<JsValue>>,
        seen: Vec<bool>,
        remaining: usize,
        mode: Combine,
    },
}
impl Internal {
    pub fn roots(&self) -> Vec<JsValue> {
        match self {
            Self::Gate(_) => Vec::new(),
            Self::Capability {
                resolve, reject, ..
            } => vec![resolve.clone(), reject.clone()],
            Self::Aggregate {
                capability, values, ..
            } => {
                let mut out = capability.roots();
                out.extend(values.iter().flatten().cloned());
                out
            }
        }
    }
    pub fn retained_bytes(&self) -> usize {
        64 + match self {
            Self::Aggregate { values, seen, .. } => {
                values.len() * 48
                    + seen.len()
                    + values
                        .iter()
                        .flatten()
                        .map(JsValue::retained_bytes)
                        .sum::<usize>()
            }
            _ => 0,
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) enum PromiseClosure {
    AsyncIteratorResult {
        done: bool,
    },
    GeneratorResume {
        execution: ObjectId,
        reject: bool,
        mode: super::execution::GeneratorAwait,
    },
    ModuleDependency {
        module: usize,
        reject: bool,
    },
    AsyncResume {
        execution: ObjectId,
        reject: bool,
    },
    Resolver {
        promise: ObjectId,
        gate: ObjectId,
        reject: bool,
    },
    Capability(ObjectId),
    Element {
        state: ObjectId,
        index: usize,
        reject: bool,
    },
    Finally {
        callback: JsValue,
        constructor: JsValue,
        reject: bool,
    },
    Restore {
        value: JsValue,
        reject: bool,
    },
}
impl PromiseClosure {
    pub fn roots(&self) -> Vec<JsValue> {
        match self {
            Self::AsyncIteratorResult { .. } => Vec::new(),
            Self::GeneratorResume { execution, .. } => vec![JsValue::Object(*execution)],
            Self::ModuleDependency { .. } => Vec::new(),
            Self::AsyncResume { execution, .. } => vec![JsValue::Object(*execution)],
            Self::Resolver { promise, gate, .. } => {
                vec![JsValue::Object(*promise), JsValue::Object(*gate)]
            }
            Self::Capability(id) | Self::Element { state: id, .. } => vec![JsValue::Object(*id)],
            Self::Finally {
                callback,
                constructor,
                ..
            } => vec![callback.clone(), constructor.clone()],
            Self::Restore { value, .. } => vec![value.clone()],
        }
    }
    pub fn retained_bytes(&self) -> usize {
        96 + self
            .roots()
            .iter()
            .map(JsValue::retained_bytes)
            .sum::<usize>()
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Job {
    Host(u64),
    Call {
        function: JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    },
    Reaction {
        reaction: Reaction,
        state: State,
    },
    Thenable {
        promise: ObjectId,
        thenable: JsValue,
        then: JsValue,
    },
}
impl Job {
    pub fn roots(&self) -> Vec<JsValue> {
        match self {
            Self::Host(_) => Vec::new(),
            Self::Call {
                function,
                this,
                args,
            } => std::iter::once(function.clone())
                .chain(std::iter::once(this.clone()))
                .chain(args.iter().cloned())
                .collect(),
            Self::Reaction { reaction, state } => {
                reaction.roots().into_iter().chain(state.value()).collect()
            }
            Self::Thenable {
                promise,
                thenable,
                then,
            } => vec![JsValue::Object(*promise), thenable.clone(), then.clone()],
        }
    }
    pub fn retained_bytes(&self) -> usize {
        192 + self
            .roots()
            .iter()
            .map(JsValue::retained_bytes)
            .sum::<usize>()
    }
}
fn argument(args: &[JsValue], n: usize) -> JsValue {
    args.get(n).cloned().unwrap_or(JsValue::Undefined)
}
impl Vm {
    pub(super) fn install_promises(&mut self) -> Result<(), JsError> {
        self.promise_proto = self.allocate(ObjectKind::Ordinary, Some(self.object_proto))?;
        let constructor = self.native("Promise")?;
        self.promise_ctor = constructor
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::Error, "Promise intrinsic"))?;
        self.define(
            self.promise_ctor,
            "prototype".into(),
            Property {
                writable: false,
                enumerable: false,
                configurable: false,
                ..Property::data(JsValue::Object(self.promise_proto))
            },
        )?;
        self.define(
            self.promise_proto,
            "constructor".into(),
            Property {
                enumerable: false,
                ..Property::data(constructor.clone())
            },
        )?;
        for method in ["then", "catch", "finally"] {
            let value = self.native(&format!("Promise.prototype.{method}"))?;
            self.define(
                self.promise_proto,
                method.into(),
                Property {
                    enumerable: false,
                    ..Property::data(value)
                },
            )?;
        }
        for method in [
            "resolve",
            "reject",
            "all",
            "allSettled",
            "any",
            "race",
            "withResolvers",
        ] {
            let value = self.native(&format!("Promise.{method}"))?;
            self.define(
                self.promise_ctor,
                method.into(),
                Property {
                    enumerable: false,
                    ..Property::data(value)
                },
            )?;
        }
        let getter = self.native("Promise.species")?;
        self.define(
            self.promise_ctor,
            PropertyKey::Symbol(crate::symbols::SPECIES),
            Property {
                value: PropertyValue::Accessor {
                    get: getter,
                    set: JsValue::Undefined,
                },
                writable: false,
                enumerable: false,
                configurable: true,
            },
        )?;
        self.define(
            self.promise_proto,
            PropertyKey::Symbol(crate::symbols::TO_STRING_TAG),
            Property {
                writable: false,
                enumerable: false,
                configurable: true,
                ..Property::data(JsValue::string("Promise"))
            },
        )?;
        self.set_global("Promise", constructor)?;
        let microtask = self.native("queueMicrotask")?;
        self.set_global("queueMicrotask", microtask)?;
        let aggregate = self.native("AggregateError")?;
        self.aggregate_error_proto = self.allocate(ObjectKind::Error, Some(self.error_proto))?;
        self.define(
            self.aggregate_error_proto,
            "name".into(),
            Property {
                enumerable: false,
                ..Property::data(JsValue::string("AggregateError"))
            },
        )?;
        self.define(
            self.aggregate_error_proto,
            "message".into(),
            Property {
                enumerable: false,
                ..Property::data(JsValue::string(""))
            },
        )?;
        self.define(
            self.aggregate_error_proto,
            "constructor".into(),
            Property {
                enumerable: false,
                ..Property::data(aggregate.clone())
            },
        )?;
        self.define(
            aggregate
                .object()
                .ok_or_else(|| JsError::new(ErrorKind::Error, "AggregateError intrinsic"))?,
            "prototype".into(),
            Property {
                writable: false,
                enumerable: false,
                configurable: false,
                ..Property::data(JsValue::Object(self.aggregate_error_proto))
            },
        )?;
        self.set_global("AggregateError", aggregate)?;
        Ok(())
    }
    fn closure(&mut self, closure: PromiseClosure) -> Result<JsValue, JsError> {
        Ok(JsValue::Object(self.allocate(
            ObjectKind::PromiseClosure(Box::new(closure)),
            Some(self.function_proto),
        )?))
    }
    fn internal(&mut self, data: Internal) -> Result<ObjectId, JsError> {
        self.allocate(ObjectKind::PromiseInternal(Box::new(data)), None)
    }
    pub fn promise(&mut self) -> Result<JsValue, JsError> {
        let gate = self.internal(Internal::Gate(false))?;
        Ok(JsValue::Object(self.allocate(
            ObjectKind::Promise(Box::new(PromiseData {
                state: State::Pending,
                gate,
                reactions: Vec::new(),
                handled: false,
                reported: false,
            })),
            Some(self.promise_proto),
        )?))
    }
    pub fn promise_result(&self, value: &JsValue) -> Option<Result<JsValue, JsValue>> {
        match value
            .object()
            .and_then(|id| self.heap.get(id).ok())
            .map(|o| &o.kind)
        {
            Some(ObjectKind::Promise(data)) => match &data.state {
                State::Pending => None,
                State::Fulfilled(v) => Some(Ok(v.clone())),
                State::Rejected(v) => Some(Err(v.clone())),
            },
            _ => None,
        }
    }
    pub fn is_promise(&self, value: &JsValue) -> bool {
        value
            .object()
            .and_then(|id| self.heap.get(id).ok())
            .is_some_and(|o| matches!(o.kind, ObjectKind::Promise(_)))
    }
    fn promise_data(&self, id: ObjectId) -> Result<PromiseData, JsError> {
        match &self.heap.get(id)?.kind {
            ObjectKind::Promise(data) => Ok((**data).clone()),
            _ => Err(JsError::new(ErrorKind::TypeError, "Promise receiver")),
        }
    }
    fn resolving_functions(
        &mut self,
        promise: ObjectId,
        gate: Option<ObjectId>,
    ) -> Result<(JsValue, JsValue), JsError> {
        let gate = if let Some(gate) = gate {
            gate
        } else {
            self.internal(Internal::Gate(false))?
        };
        Ok((
            self.closure(PromiseClosure::Resolver {
                promise,
                gate,
                reject: false,
            })?,
            self.closure(PromiseClosure::Resolver {
                promise,
                gate,
                reject: true,
            })?,
        ))
    }
    fn capability(&mut self, constructor: JsValue) -> Result<Capability, JsError> {
        if constructor == JsValue::Object(self.promise_ctor) {
            let promise = self.promise()?;
            let id = promise
                .object()
                .ok_or_else(|| JsError::new(ErrorKind::Error, "promise allocation"))?;
            let gate = self.promise_data(id)?.gate;
            let (resolve, reject) = self.resolving_functions(id, Some(gate))?;
            return Ok(Capability {
                promise,
                resolve,
                reject,
            });
        }
        let cell = self.internal(Internal::Capability {
            resolve: JsValue::Undefined,
            reject: JsValue::Undefined,
            called: false,
        })?;
        let executor = self.closure(PromiseClosure::Capability(cell))?;
        let promise = self.construct(constructor, vec![executor])?;
        let ObjectKind::PromiseInternal(data) = self.heap.get(cell)?.kind.clone() else {
            return Err(JsError::new(ErrorKind::Error, "promise capability"));
        };
        let Internal::Capability {
            resolve, reject, ..
        } = *data
        else {
            return Err(JsError::new(ErrorKind::Error, "promise capability"));
        };
        if !self.callable(&resolve) || !self.callable(&reject) {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "promise resolving functions",
            ));
        }
        Ok(Capability {
            promise,
            resolve,
            reject,
        })
    }
    fn species(&mut self, promise: JsValue) -> Result<JsValue, JsError> {
        let constructor = self.get(promise, "constructor")?;
        if constructor == JsValue::Undefined {
            return Ok(JsValue::Object(self.promise_ctor));
        }
        if constructor.object().is_none() {
            return Err(JsError::new(ErrorKind::TypeError, "promise constructor"));
        }
        let species = self.get_key(constructor, PropertyKey::Symbol(crate::symbols::SPECIES))?;
        if matches!(species, JsValue::Null | JsValue::Undefined) {
            Ok(JsValue::Object(self.promise_ctor))
        } else {
            Ok(species)
        }
    }
    fn error_reason(&mut self, error: JsError) -> Result<JsValue, JsError> {
        if error.is_resource_limit() {
            Err(error)
        } else if let Some(value) = error.thrown {
            Ok(value)
        } else {
            self.error_value(error)
        }
    }
    fn enqueue_microtask(&mut self, job: Job) -> Result<(), JsError> {
        if self.microtasks.len() >= 10000 {
            return Err(JsError::new(
                ErrorKind::MemoryLimit,
                "microtask queue quota",
            ));
        }
        self.reserve_memory(job.retained_bytes())?;
        self.microtasks.push_back(job);
        Ok(())
    }
    pub fn queue_microtask(&mut self, function: JsValue) -> Result<(), JsError> {
        if !self.callable(&function) {
            return Err(JsError::new(ErrorKind::TypeError, "microtask callback"));
        }
        self.enqueue_microtask(Job::Call {
            function,
            this: JsValue::Undefined,
            args: Vec::new(),
        })
    }
    pub fn queue_host_microtask(&mut self, token: u64) -> Result<(), JsError> {
        self.enqueue_microtask(Job::Host(token))
    }
    pub fn has_microtasks(&self) -> bool {
        !self.microtasks.is_empty()
    }
    pub fn take_async_errors(&mut self) -> Vec<JsError> {
        std::mem::take(&mut self.async_errors)
    }
    pub fn take_promise_notifications(&mut self) -> Vec<PromiseNotification> {
        std::mem::take(&mut self.promise_notifications)
    }
    pub fn resolve_promise(&mut self, promise: JsValue, value: JsValue) -> Result<(), JsError> {
        let id = promise
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::TypeError, "Promise receiver"))?;
        let gate = self.promise_data(id)?.gate;
        self.call_promise_closure(
            PromiseClosure::Resolver {
                promise: id,
                gate,
                reject: false,
            },
            vec![value],
        )
        .map(|_| ())
    }
    pub fn reject_promise(&mut self, promise: JsValue, reason: JsValue) -> Result<(), JsError> {
        let id = promise
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::TypeError, "Promise receiver"))?;
        let gate = self.promise_data(id)?.gate;
        self.call_promise_closure(
            PromiseClosure::Resolver {
                promise: id,
                gate,
                reject: true,
            },
            vec![reason],
        )
        .map(|_| ())
    }
    fn settle_promise(&mut self, id: ObjectId, state: State) -> Result<(), JsError> {
        let data = self.promise_data(id)?;
        if !matches!(data.state, State::Pending) {
            return Ok(());
        }
        let reason_bytes = state.value().map_or(0, |v| v.retained_bytes());
        self.reserve_memory(reason_bytes)?;
        if matches!(state, State::Rejected(_)) {
            self.rejected_promises.push(id);
        }
        let reactions = if let ObjectKind::Promise(data) = &mut self.heap.get_mut(id)?.kind {
            data.state = state.clone();
            std::mem::take(&mut data.reactions)
        } else {
            Vec::new()
        };
        for reaction in reactions {
            self.enqueue_microtask(Job::Reaction {
                reaction,
                state: state.clone(),
            })?;
        }
        Ok(())
    }
    fn adopt_promise(&mut self, id: ObjectId, value: JsValue) -> Result<(), JsError> {
        if !matches!(self.promise_data(id)?.state, State::Pending) {
            return Ok(());
        }
        if value == JsValue::Object(id) {
            let reason = self.error_value(JsError::new(
                ErrorKind::TypeError,
                "promise self resolution",
            ))?;
            return self.settle_promise(id, State::Rejected(reason));
        }
        if value.object().is_none() {
            return self.settle_promise(id, State::Fulfilled(value));
        }
        let then = match self.get(value.clone(), "then") {
            Ok(then) => then,
            Err(error) => {
                let reason = self.error_reason(error)?;
                return self.settle_promise(id, State::Rejected(reason));
            }
        };
        if !self.callable(&then) {
            return self.settle_promise(id, State::Fulfilled(value));
        }
        self.enqueue_microtask(Job::Thenable {
            promise: id,
            thenable: value,
            then,
        })
    }
    fn react_promise(&mut self, id: ObjectId, reaction: Reaction) -> Result<(), JsError> {
        let data = self.promise_data(id)?;
        if data.reported && !data.handled {
            if let Some(reason) = data.state.value() {
                self.promise_notifications.push(PromiseNotification {
                    promise: JsValue::Object(id),
                    reason,
                    handled: true,
                });
            }
        }
        if let ObjectKind::Promise(data) = &mut self.heap.get_mut(id)?.kind {
            data.handled = true;
        }
        if matches!(data.state, State::Pending) {
            if data.reactions.len() >= 10000 {
                return Err(JsError::new(
                    ErrorKind::MemoryLimit,
                    "promise reaction quota",
                ));
            }
            self.reserve_memory(192)?;
            if let ObjectKind::Promise(data) = &mut self.heap.get_mut(id)?.kind {
                data.reactions.push(reaction);
            }
            Ok(())
        } else {
            self.enqueue_microtask(Job::Reaction {
                reaction,
                state: data.state,
            })
        }
    }
    pub(crate) fn call_promise_closure(
        &mut self,
        closure: PromiseClosure,
        args: Vec<JsValue>,
    ) -> Result<JsValue, JsError> {
        let value = argument(&args, 0);
        match closure {
            PromiseClosure::AsyncIteratorResult { done } => {
                return self.iterator_result(value, done)
            }
            PromiseClosure::GeneratorResume {
                execution,
                reject,
                mode,
            } => self.resume_async_generator(
                execution,
                if reject { Err(value) } else { Ok(value) },
                mode,
            )?,
            PromiseClosure::ModuleDependency { module, reject } => {
                self.module_dependency(module, if reject { Err(value) } else { Ok(value) })?
            }
            PromiseClosure::AsyncResume { execution, reject } => {
                self.resume_async(
                    execution,
                    Some(if reject {
                        super::execution::Resume::Throw(value)
                    } else {
                        super::execution::Resume::Value(value)
                    }),
                )?;
            }
            PromiseClosure::Resolver {
                promise,
                gate,
                reject,
            } => {
                let ObjectKind::PromiseInternal(data) = &mut self.heap.get_mut(gate)?.kind else {
                    return Err(JsError::new(ErrorKind::Error, "promise gate"));
                };
                let Internal::Gate(used) = data.as_mut() else {
                    return Err(JsError::new(ErrorKind::Error, "promise gate"));
                };
                if *used {
                    return Ok(JsValue::Undefined);
                }
                *used = true;
                if reject {
                    self.settle_promise(promise, State::Rejected(value))?;
                } else {
                    self.adopt_promise(promise, value)?;
                }
            }
            PromiseClosure::Capability(cell) => {
                let ObjectKind::PromiseInternal(data) = &mut self.heap.get_mut(cell)?.kind else {
                    return Err(JsError::new(ErrorKind::Error, "capability cell"));
                };
                let Internal::Capability {
                    resolve,
                    reject,
                    called,
                } = data.as_mut()
                else {
                    return Err(JsError::new(ErrorKind::Error, "capability cell"));
                };
                if *called {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "capability executor called twice",
                    ));
                }
                *called = true;
                *resolve = value;
                *reject = argument(&args, 1);
            }
            PromiseClosure::Restore { value, reject } => {
                if reject {
                    return Err(JsError {
                        kind: ErrorKind::Error,
                        message: "promise rejection".into(),
                        line: 0,
                        column: 0,
                        stack: Vec::new(),
                        thrown: Some(value),
                    });
                }
                return Ok(value);
            }
            PromiseClosure::Finally {
                callback,
                constructor,
                reject,
            } => {
                let result =
                    self.call_inner(callback, JsValue::Undefined, Vec::new(), false, None)?;
                let promise = self.promise_resolve_with(constructor, result)?;
                let restore = self.closure(PromiseClosure::Restore { value, reject })?;
                let then = self.get(promise.clone(), "then")?;
                return self.call_inner(then, promise, vec![restore], false, None);
            }
            PromiseClosure::Element {
                state,
                index,
                reject,
            } => {
                let ObjectKind::PromiseInternal(data) = self.heap.get(state)?.kind.clone() else {
                    return Err(JsError::new(ErrorKind::Error, "aggregate state"));
                };
                let Internal::Aggregate {
                    capability, mode, ..
                } = *data
                else {
                    return Err(JsError::new(ErrorKind::Error, "aggregate state"));
                };
                let value = if mode == Combine::AllSettled {
                    let record = self.object()?;
                    self.set(
                        record.clone(),
                        "status",
                        JsValue::string(if reject { "rejected" } else { "fulfilled" }),
                    )?;
                    self.set(
                        record.clone(),
                        if reject { "reason" } else { "value" },
                        value,
                    )?;
                    record
                } else {
                    value
                };
                let ready = {
                    let ObjectKind::PromiseInternal(data) = &mut self.heap.get_mut(state)?.kind
                    else {
                        return Ok(JsValue::Undefined);
                    };
                    let Internal::Aggregate {
                        seen,
                        values,
                        remaining,
                        ..
                    } = data.as_mut()
                    else {
                        return Ok(JsValue::Undefined);
                    };
                    let Some(done) = seen.get_mut(index) else {
                        return Err(JsError::new(ErrorKind::Error, "aggregate index"));
                    };
                    if *done {
                        return Ok(JsValue::Undefined);
                    }
                    *done = true;
                    values[index] = Some(value.clone());
                    *remaining = remaining.saturating_sub(1);
                    *remaining == 0
                };
                if mode == Combine::All && reject
                    || mode == Combine::Any && !reject
                    || mode == Combine::Race
                {
                    let callback = if reject {
                        capability.reject
                    } else {
                        capability.resolve
                    };
                    self.call_inner(callback, JsValue::Undefined, vec![value], false, None)?;
                } else if ready {
                    self.finish_aggregate(state)?;
                }
            }
        }
        Ok(JsValue::Undefined)
    }
    pub(crate) fn await_async(&mut self, id: ObjectId, value: JsValue) -> Result<(), JsError> {
        let promise = self.promise_resolve_with(JsValue::Object(self.promise_ctor), value)?;
        let promise = promise
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::Error, "await promise"))?;
        let fulfill = self.closure(PromiseClosure::AsyncResume {
            execution: id,
            reject: false,
        })?;
        let reject = self.closure(PromiseClosure::AsyncResume {
            execution: id,
            reject: true,
        })?;
        let capability = self.capability(JsValue::Object(self.promise_ctor))?;
        self.react_promise(
            promise,
            Reaction {
                capability,
                fulfill,
                reject,
            },
        )
    }
    pub(crate) fn watch_module_dependency(
        &mut self,
        module: usize,
        promise: JsValue,
    ) -> Result<(), JsError> {
        let id = promise
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::Error, "module promise"))?;
        let fulfill = self.closure(PromiseClosure::ModuleDependency {
            module,
            reject: false,
        })?;
        let reject = self.closure(PromiseClosure::ModuleDependency {
            module,
            reject: true,
        })?;
        let capability = self.capability(JsValue::Object(self.promise_ctor))?;
        self.react_promise(
            id,
            Reaction {
                capability,
                fulfill,
                reject,
            },
        )
    }
    pub(crate) fn await_generator(
        &mut self,
        execution: ObjectId,
        value: JsValue,
        mode: super::execution::GeneratorAwait,
    ) -> Result<(), JsError> {
        let promise = self.promise_resolve_with(JsValue::Object(self.promise_ctor), value)?;
        let id = promise
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::Error, "async generator promise"))?;
        let fulfill = self.closure(PromiseClosure::GeneratorResume {
            execution,
            reject: false,
            mode,
        })?;
        let reject = self.closure(PromiseClosure::GeneratorResume {
            execution,
            reject: true,
            mode,
        })?;
        let capability = self.capability(JsValue::Object(self.promise_ctor))?;
        self.react_promise(
            id,
            Reaction {
                capability,
                fulfill,
                reject,
            },
        )
    }
    fn aggregate_error(
        &mut self,
        errors: Vec<JsValue>,
        message: String,
    ) -> Result<JsValue, JsError> {
        let value = self.error_value(JsError::new(ErrorKind::Error, message))?;
        if let Some(id) = value.object() {
            self.heap.get_mut(id)?.prototype = Some(self.aggregate_error_proto);
        }
        self.set(value.clone(), "name", JsValue::string("AggregateError"))?;
        let errors = self.array(errors)?;
        if let Some(id) = value.object() {
            self.define(
                id,
                "errors".into(),
                Property {
                    enumerable: false,
                    ..Property::data(errors)
                },
            )?;
        }
        Ok(value)
    }
    fn finish_aggregate(&mut self, id: ObjectId) -> Result<(), JsError> {
        let ObjectKind::PromiseInternal(data) = self.heap.get(id)?.kind.clone() else {
            return Ok(());
        };
        let Internal::Aggregate {
            capability,
            values,
            mode,
            remaining,
            ..
        } = *data
        else {
            return Ok(());
        };
        if remaining != 0 || mode == Combine::Race {
            return Ok(());
        }
        let values = values
            .into_iter()
            .map(|v| v.unwrap_or(JsValue::Undefined))
            .collect::<Vec<_>>();
        let (value, callback) = if mode == Combine::Any {
            (
                self.aggregate_error(values, "All promises were rejected".into())?,
                capability.reject,
            )
        } else {
            (self.array(values)?, capability.resolve)
        };
        self.call_inner(callback, JsValue::Undefined, vec![value], false, None)?;
        Ok(())
    }
    pub(crate) fn unwrap_async_iterator_value(
        &mut self,
        value: JsValue,
        done: bool,
    ) -> Result<JsValue, JsError> {
        let promise = self.promise_resolve_with(JsValue::Object(self.promise_ctor), value)?;
        let id = promise
            .object()
            .ok_or_else(|| JsError::new(ErrorKind::Error, "iterator promise"))?;
        let capability = self.capability(JsValue::Object(self.promise_ctor))?;
        let fulfill = self.closure(PromiseClosure::AsyncIteratorResult { done })?;
        self.react_promise(
            id,
            Reaction {
                capability: capability.clone(),
                fulfill,
                reject: JsValue::Undefined,
            },
        )?;
        Ok(capability.promise)
    }
    fn promise_resolve_with(
        &mut self,
        constructor: JsValue,
        value: JsValue,
    ) -> Result<JsValue, JsError> {
        if self.is_promise(&value) && self.get(value.clone(), "constructor")? == constructor {
            return Ok(value);
        }
        let cap = self.capability(constructor)?;
        self.call_inner(cap.resolve, JsValue::Undefined, vec![value], false, None)?;
        Ok(cap.promise)
    }
    pub(crate) fn promise_builtin(
        &mut self,
        name: &str,
        this: JsValue,
        args: &[JsValue],
        construct: bool,
    ) -> Option<Result<JsValue, JsError>> {
        if name != "Promise"
            && !name.starts_with("Promise.")
            && name != "queueMicrotask"
            && name != "AggregateError"
        {
            return None;
        }
        Some((|| match name {
            "Promise" => {
                if !construct {
                    return Err(JsError::new(ErrorKind::TypeError, "Promise requires new"));
                }
                let executor = argument(args, 0);
                if !self.callable(&executor) {
                    return Err(JsError::new(ErrorKind::TypeError, "Promise executor"));
                }
                let cap = self.capability(JsValue::Object(self.promise_ctor))?;
                let result = self.call_inner(
                    executor,
                    JsValue::Undefined,
                    vec![cap.resolve.clone(), cap.reject.clone()],
                    false,
                    None,
                );
                if let Err(error) = result {
                    let reason = self.error_reason(error)?;
                    self.call_inner(cap.reject, JsValue::Undefined, vec![reason], false, None)?;
                }
                Ok(cap.promise)
            }
            "queueMicrotask" => {
                self.queue_microtask(argument(args, 0))?;
                Ok(JsValue::Undefined)
            }
            "Promise.species" => Ok(this),
            "AggregateError" => {
                let errors = self.iterable_values(argument(args, 0))?;
                let message = if argument(args, 1) == JsValue::Undefined {
                    String::new()
                } else {
                    self.to_string(argument(args, 1))?
                };
                self.aggregate_error(errors, message)
            }
            "Promise.prototype.then" => {
                let id = this
                    .object()
                    .ok_or_else(|| JsError::new(ErrorKind::TypeError, "Promise receiver"))?;
                self.promise_data(id)?;
                let constructor = self.species(this)?;
                let cap = self.capability(constructor)?;
                self.react_promise(
                    id,
                    Reaction {
                        capability: cap.clone(),
                        fulfill: argument(args, 0),
                        reject: argument(args, 1),
                    },
                )?;
                Ok(cap.promise)
            }
            "Promise.prototype.catch" => {
                let then = self.get(this.clone(), "then")?;
                self.call_inner(
                    then,
                    this,
                    vec![JsValue::Undefined, argument(args, 0)],
                    false,
                    None,
                )
            }
            "Promise.prototype.finally" => {
                if this.object().is_none() {
                    return Err(JsError::new(ErrorKind::TypeError, "finally receiver"));
                }
                let constructor = self.species(this.clone())?;
                let callback = argument(args, 0);
                let (ok, err) = if self.callable(&callback) {
                    (
                        self.closure(PromiseClosure::Finally {
                            callback: callback.clone(),
                            constructor: constructor.clone(),
                            reject: false,
                        })?,
                        self.closure(PromiseClosure::Finally {
                            callback,
                            constructor,
                            reject: true,
                        })?,
                    )
                } else {
                    (callback.clone(), callback)
                };
                let then = self.get(this.clone(), "then")?;
                self.call_inner(then, this, vec![ok, err], false, None)
            }
            "Promise.resolve" => self.promise_resolve_with(this, argument(args, 0)),
            "Promise.reject" => {
                let cap = self.capability(this)?;
                self.call_inner(
                    cap.reject,
                    JsValue::Undefined,
                    vec![argument(args, 0)],
                    false,
                    None,
                )?;
                Ok(cap.promise)
            }
            "Promise.withResolvers" => {
                let cap = self.capability(this)?;
                let value = self.object()?;
                for (name, v) in [
                    ("promise", cap.promise),
                    ("resolve", cap.resolve),
                    ("reject", cap.reject),
                ] {
                    self.set(value.clone(), name, v)?;
                }
                Ok(value)
            }
            "Promise.all" | "Promise.allSettled" | "Promise.any" | "Promise.race" => {
                let cap = self.capability(this.clone())?;
                let mode = match name {
                    "Promise.all" => Combine::All,
                    "Promise.allSettled" => Combine::AllSettled,
                    "Promise.any" => Combine::Any,
                    _ => Combine::Race,
                };
                let result = (|| {
                    let resolve = self.get(this.clone(), "resolve")?;
                    if !self.callable(&resolve) {
                        return Err(JsError::new(ErrorKind::TypeError, "constructor.resolve"));
                    }
                    let state = self.internal(Internal::Aggregate {
                        capability: cap.clone(),
                        values: Vec::new(),
                        seen: Vec::new(),
                        remaining: 1,
                        mode,
                    })?;
                    let mut iterator = self.get_iterator(argument(args, 0))?;
                    let mut index = 0usize;
                    while let Some(value) = self.iterator_step(&mut iterator)? {
                        let result = (|| {
                            if index >= 100_000 {
                                return Err(JsError::new(
                                    ErrorKind::MemoryLimit,
                                    "promise iterable quota",
                                ));
                            }
                            self.reserve_memory(64)?;
                            if let ObjectKind::PromiseInternal(data) =
                                &mut self.heap.get_mut(state)?.kind
                            {
                                if let Internal::Aggregate {
                                    values,
                                    seen,
                                    remaining,
                                    ..
                                } = data.as_mut()
                                {
                                    values.push(None);
                                    seen.push(false);
                                    *remaining += 1;
                                }
                            }
                            let promise = self.call_inner(
                                resolve.clone(),
                                this.clone(),
                                vec![value],
                                false,
                                None,
                            )?;
                            let then = self.get(promise.clone(), "then")?;
                            let ok = self.closure(PromiseClosure::Element {
                                state,
                                index,
                                reject: false,
                            })?;
                            let err = self.closure(PromiseClosure::Element {
                                state,
                                index,
                                reject: true,
                            })?;
                            self.call_inner(then, promise, vec![ok, err], false, None)?;
                            Ok(())
                        })();
                        if let Err(error) = result {
                            let close = self.close_iterator(&mut iterator);
                            if error.is_resource_limit() {
                                return Err(error);
                            }
                            let _ = close;
                            return Err(error);
                        }
                        index += 1;
                    }
                    if let ObjectKind::PromiseInternal(data) = &mut self.heap.get_mut(state)?.kind {
                        if let Internal::Aggregate { remaining, .. } = data.as_mut() {
                            *remaining = remaining.saturating_sub(1);
                        }
                    }
                    self.finish_aggregate(state)
                })();
                if let Err(error) = result {
                    let reason = self.error_reason(error)?;
                    self.call_inner(cap.reject, JsValue::Undefined, vec![reason], false, None)?;
                }
                Ok(cap.promise)
            }
            _ => Err(JsError::new(ErrorKind::TypeError, "promise intrinsic")),
        })())
    }
    pub fn drain_microtasks(&mut self) -> Result<(), JsError> {
        if self.draining_microtasks {
            return Ok(());
        }
        self.draining_microtasks = true;
        let result = (|| {
            let mut count = 0usize;
            while let Some(job) = self.microtasks.pop_front() {
                self.memory_bytes = self.memory_bytes.saturating_sub(job.retained_bytes());
                self.tick()?;
                count += 1;
                if count > 10000 {
                    return Err(JsError::new(
                        ErrorKind::Timeout,
                        "microtask checkpoint budget",
                    ));
                }
                match job {
                    Job::Host(token) => {
                        let outcome = self.with_host(|host, vm| host.microtask(vm, token))?;
                        for (function, this, args) in outcome.calls {
                            if let Err(error) = self.call_inner(function, this, args, false, None) {
                                if error.is_resource_limit() {
                                    return Err(error);
                                }
                                self.async_errors.push(error);
                            }
                        }
                        if let Some(token) = outcome.finish {
                            self.with_host(|host, vm| host.finish(vm, token))?;
                        }
                    }
                    Job::Call {
                        function,
                        this,
                        args,
                    } => {
                        if let Err(error) = self.call_inner(function, this, args, false, None) {
                            if error.is_resource_limit() {
                                return Err(error);
                            }
                            self.async_errors.push(error);
                        }
                    }
                    Job::Thenable {
                        promise,
                        thenable,
                        then,
                    } => {
                        let (resolve, reject) = self.resolving_functions(promise, None)?;
                        if let Err(error) = self.call_inner(
                            then,
                            thenable,
                            vec![resolve, reject.clone()],
                            false,
                            None,
                        ) {
                            let reason = self.error_reason(error)?;
                            self.call_inner(reject, JsValue::Undefined, vec![reason], false, None)?;
                        }
                    }
                    Job::Reaction { reaction, state } => {
                        let ok = state.ok();
                        let value = state.value().unwrap_or(JsValue::Undefined);
                        let callback = if ok {
                            reaction.fulfill
                        } else {
                            reaction.reject
                        };
                        let result = if self.callable(&callback) {
                            self.call_inner(
                                callback,
                                JsValue::Undefined,
                                vec![value.clone()],
                                false,
                                None,
                            )
                        } else if ok {
                            Ok(value)
                        } else {
                            Err(JsError {
                                kind: ErrorKind::Error,
                                message: "promise rejection".into(),
                                line: 0,
                                column: 0,
                                stack: Vec::new(),
                                thrown: Some(value),
                            })
                        };
                        match result {
                            Ok(value) => {
                                self.call_inner(
                                    reaction.capability.resolve,
                                    JsValue::Undefined,
                                    vec![value],
                                    false,
                                    None,
                                )?;
                            }
                            Err(error) => {
                                let reason = self.error_reason(error)?;
                                self.call_inner(
                                    reaction.capability.reject,
                                    JsValue::Undefined,
                                    vec![reason],
                                    false,
                                    None,
                                )?;
                            }
                        }
                    }
                }
            }
            for id in std::mem::take(&mut self.rejected_promises) {
                let data = self.promise_data(id)?;
                if !data.handled && !data.reported {
                    if let State::Rejected(reason) = data.state {
                        if self.promise_notifications.len() < 1000 {
                            self.promise_notifications.push(PromiseNotification {
                                promise: JsValue::Object(id),
                                reason,
                                handled: false,
                            });
                        }
                        if let ObjectKind::Promise(data) = &mut self.heap.get_mut(id)?.kind {
                            data.reported = true;
                        }
                    }
                }
            }
            self.finish_memory_check(JsValue::Undefined).map(|_| ())
        })();
        self.draining_microtasks = false;
        if self.gc_requested {
            self.gc_collect();
        }
        result
    }
}
