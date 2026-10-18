//! Resumable bytecode activation records, including lexical scopes and unwinders.
use super::iterators::IteratorRecord;
use super::*;
use crate::bytecode::{Chunk, Op, ScopePlan};
#[derive(Debug, Clone)]
enum Ref {
    Name {
        env: usize,
        name: String,
    },
    Property {
        object: JsValue,
        key: PropertyKey,
        receiver: JsValue,
    },
}
#[derive(Debug, Clone)]
enum Cell {
    Value(JsValue),
    Reference(Ref),
    Key(PropertyKey),
    Call {
        function: JsValue,
        this: JsValue,
        direct: bool,
        super_: bool,
    },
    Arguments(Vec<JsValue>),
    Array {
        values: Vec<JsValue>,
        holes: Vec<usize>,
    },
    Text(String),
}
impl Cell {
    fn roots(&self) -> Vec<JsValue> {
        match self {
            Self::Key(PropertyKey::Symbol(id)) => vec![JsValue::Symbol(*id, "".into())],
            Self::Value(v) => vec![v.clone()],
            Self::Reference(Ref::Property {
                object,
                receiver,
                key,
            }) => {
                let mut roots = vec![object.clone(), receiver.clone()];
                if let PropertyKey::Symbol(id) = key {
                    roots.push(JsValue::Symbol(*id, "".into()));
                }
                roots
            }
            Self::Call { function, this, .. } => vec![function.clone(), this.clone()],
            Self::Arguments(v) | Self::Array { values: v, .. } => v.clone(),
            _ => Vec::new(),
        }
    }
    fn env(&self) -> Option<usize> {
        match self {
            Self::Reference(Ref::Name { env, .. }) => Some(*env),
            _ => None,
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Resume {
    Value(JsValue),
    Throw(JsValue),
    Return(JsValue),
}
#[derive(Debug, Clone)]
enum Abrupt {
    Return(JsValue),
    Throw(JsError),
    Jump { target: usize, depth: usize },
}
#[derive(Debug, Clone)]
enum LoopValues {
    Iterator(IteratorRecord),
    Keys { values: Vec<JsValue>, index: usize },
}
#[derive(Debug, Clone)]
enum Handler {
    Try {
        begin: usize,
        catch: Option<usize>,
        finally: Option<usize>,
        end: usize,
        depth: usize,
        stack: usize,
        catching: bool,
        finalizing: bool,
        saved: Option<Abrupt>,
    },
    Iterator {
        begin: usize,
        end: usize,
        depth: usize,
        stack: usize,
        values: LoopValues,
        asynchronous: bool,
    },
}
#[derive(Debug, Clone, Copy)]
enum DelegateAwait {
    Next,
    Return,
    MissingThrow,
}
#[derive(Debug, Clone)]
pub(crate) struct Frame {
    pub module: Option<usize>,
    asynchronous: bool,
    closing: Option<Abrupt>,
    delegate_awaiting: Option<DelegateAwait>,
    pub source: String,
    pub code: Arc<Chunk>,
    ip: usize,
    scopes: Vec<usize>,
    stack: Vec<Cell>,
    handlers: Vec<Handler>,
    last: JsValue,
    pub strict: bool,
    pub name: String,
    pub span: Span,
    pub delegate: Option<IteratorRecord>,
    pub started: bool,
}
impl Frame {
    pub fn new(code: Arc<Chunk>, env: usize, strict: bool, name: String, span: Span) -> Self {
        Self {
            module: None,
            asynchronous: false,
            closing: None,
            delegate_awaiting: None,
            source: String::new(),
            code,
            ip: 0,
            scopes: vec![env],
            stack: Vec::new(),
            handlers: Vec::new(),
            last: JsValue::Undefined,
            strict,
            name,
            span,
            delegate: None,
            started: false,
        }
    }
    fn env(&self) -> Result<usize, JsError> {
        self.scopes.last().copied().ok_or_else(stack_error)
    }
    fn pop(&mut self) -> Result<Cell, JsError> {
        self.stack.pop().ok_or_else(stack_error)
    }
    fn value(&mut self) -> Result<JsValue, JsError> {
        match self.pop()? {
            Cell::Value(value) => Ok(value),
            _ => Err(stack_error()),
        }
    }
    fn key(&mut self) -> Result<PropertyKey, JsError> {
        match self.pop()? {
            Cell::Key(value) => Ok(value),
            _ => Err(stack_error()),
        }
    }
    fn reference(&mut self) -> Result<Ref, JsError> {
        match self.pop()? {
            Cell::Reference(value) => Ok(value),
            _ => Err(stack_error()),
        }
    }
    pub fn roots(&self) -> Vec<JsValue> {
        let mut out = vec![self.last.clone()];
        match &self.closing {
            Some(Abrupt::Return(v)) => out.push(v.clone()),
            Some(Abrupt::Throw(e)) => out.extend(e.thrown.clone()),
            _ => {}
        }
        out.extend(self.stack.iter().flat_map(Cell::roots));
        if let Some(iterator) = &self.delegate {
            out.extend([iterator.object.clone(), iterator.next.clone()]);
        }
        for handler in &self.handlers {
            match handler {
                Handler::Try {
                    saved: Some(Abrupt::Return(v)),
                    ..
                } => out.push(v.clone()),
                Handler::Try {
                    saved: Some(Abrupt::Throw(e)),
                    ..
                } => out.extend(e.thrown.clone()),
                Handler::Iterator {
                    values: LoopValues::Iterator(iterator),
                    ..
                } => out.extend([iterator.object.clone(), iterator.next.clone()]),
                Handler::Iterator {
                    values: LoopValues::Keys { values, .. },
                    ..
                } => out.extend(values.clone()),
                _ => {}
            }
        }
        out
    }
    pub fn environments(&self) -> Vec<usize> {
        self.scopes
            .iter()
            .copied()
            .chain(self.stack.iter().filter_map(Cell::env))
            .collect()
    }
    pub fn retained_bytes(&self) -> usize {
        self.code.retained_bytes()
            + self.scopes.len() * 8
            + self.stack.len() * 128
            + self.handlers.len() * 192
            + self
                .roots()
                .iter()
                .map(JsValue::retained_bytes)
                .sum::<usize>()
    }
}
#[derive(Debug, Clone)]
pub(crate) struct AsyncData {
    pub module: Option<usize>,
    pub frame: Option<Frame>,
    pub promise: JsValue,
    pub finished: bool,
}
impl AsyncData {
    pub fn roots(&self) -> Vec<JsValue> {
        std::iter::once(self.promise.clone())
            .chain(self.frame.iter().flat_map(Frame::roots))
            .collect()
    }
    pub fn environments(&self) -> Vec<usize> {
        self.frame
            .as_ref()
            .map_or_else(Vec::new, Frame::environments)
    }
    pub fn retained_bytes(&self) -> usize {
        128 + self.frame.as_ref().map_or(0, Frame::retained_bytes)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GeneratorState {
    Start,
    Suspended,
    Executing,
    Done,
}
#[derive(Debug, Clone)]
pub(crate) struct GeneratorData {
    requests: std::collections::VecDeque<GeneratorRequest>,
    pub frame: Option<Frame>,
    pub state: GeneratorState,
    pub asynchronous: bool,
}
#[derive(Debug, Clone)]
struct GeneratorRequest {
    resume: Resume,
    promise: JsValue,
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum GeneratorAwait {
    Body,
    Yield,
    Return,
}
impl GeneratorData {
    pub fn roots(&self) -> Vec<JsValue> {
        let mut out = self.frame.as_ref().map_or_else(Vec::new, Frame::roots);
        for request in &self.requests {
            out.push(request.promise.clone());
            match &request.resume {
                Resume::Value(v) | Resume::Throw(v) | Resume::Return(v) => out.push(v.clone()),
            }
        }
        out
    }
    pub fn environments(&self) -> Vec<usize> {
        self.frame
            .as_ref()
            .map_or_else(Vec::new, Frame::environments)
    }
    pub fn retained_bytes(&self) -> usize {
        128 + self.frame.as_ref().map_or(0, Frame::retained_bytes)
            + self
                .requests
                .iter()
                .map(|request| {
                    96 + request.promise.retained_bytes()
                        + match &request.resume {
                            Resume::Value(v) | Resume::Throw(v) | Resume::Return(v) => {
                                v.retained_bytes()
                            }
                        }
                })
                .sum::<usize>()
    }
}
#[derive(Debug)]
enum Run {
    Done(JsValue),
    Await(JsValue),
    Yield(JsValue),
}
fn stack_error() -> JsError {
    JsError::new(ErrorKind::Error, "invalid bytecode activation")
}
fn thrown(value: JsValue, span: Span) -> JsError {
    JsError {
        kind: ErrorKind::Error,
        message: "JavaScript exception".into(),
        line: span.line,
        column: span.column,
        stack: Vec::new(),
        thrown: Some(value),
    }
}
impl Vm {
    pub(super) fn enter_plan(&mut self, plan: &ScopePlan, env: usize) -> Result<(), JsError> {
        for (name, kind) in &plan.bindings {
            if *kind == DeclKind::Var && self.scope(env)?.bindings.contains_key(name) {
                continue;
            }
            self.declare_name(env, name, *kind, JsValue::Undefined, false)?;
        }
        for (name, function) in &plan.functions {
            let value = self.function(function.clone(), env)?;
            self.declare_name(env, name, DeclKind::Var, value, true)?;
        }
        Ok(())
    }
    fn read_bc_reference(&mut self, r: &Ref) -> Result<JsValue, JsError> {
        match r {
            Ref::Name { env, name } => self.lookup(*env, name),
            Ref::Property {
                object,
                key,
                receiver,
            } => self.get_key_receiver(object.clone(), key.clone(), receiver.clone()),
        }
    }
    fn write_bc_reference(&mut self, r: Ref, value: JsValue) -> Result<(), JsError> {
        match r {
            Ref::Name { env, name } => self.assign_name(env, &name, value),
            Ref::Property {
                object,
                key,
                receiver,
            } => {
                if object != receiver {
                    let mut id = object.object();
                    while let Some(current) = id {
                        let property = self.heap.get(current)?.properties.get(&key).cloned();
                        if let Some(property) = property {
                            match property.value {
                                PropertyValue::Accessor { set, .. } => {
                                    if !self.callable(&set) {
                                        return Err(JsError::new(
                                            ErrorKind::TypeError,
                                            "super property is read only",
                                        ));
                                    }
                                    self.call_inner(set, receiver, vec![value], false, None)?;
                                    return Ok(());
                                }
                                PropertyValue::Data(_) if !property.writable => {
                                    return Err(JsError::new(
                                        ErrorKind::TypeError,
                                        "super property is read only",
                                    ))
                                }
                                _ => break,
                            }
                        }
                        id = self.heap.get(current)?.prototype;
                    }
                }
                self.set_key(receiver, key, value)
            }
        }
    }
    fn unwind(&mut self, frame: &mut Frame, mut abrupt: Abrupt) -> Result<Option<Run>, JsError> {
        loop {
            let Some(handler) = frame.handlers.last().cloned() else {
                return match abrupt {
                    Abrupt::Return(value) => Ok(Some(Run::Done(value))),
                    Abrupt::Throw(error) => Err(error),
                    Abrupt::Jump { target, depth } => {
                        frame.scopes.truncate(depth);
                        frame.ip = target;
                        Ok(None)
                    }
                };
            };
            let (begin, end) = match &handler {
                Handler::Try { begin, end, .. } | Handler::Iterator { begin, end, .. } => {
                    (*begin, *end)
                }
            };
            if matches!(abrupt,Abrupt::Jump{target,..}if target>=begin&&target<end) {
                if let Abrupt::Jump { target, depth } = abrupt {
                    frame.scopes.truncate(depth);
                    frame.ip = target;
                }
                return Ok(None);
            }
            match handler {
                Handler::Try {
                    catch,
                    finally,
                    depth,
                    stack,
                    catching,
                    finalizing,
                    ..
                } => {
                    frame.scopes.truncate(depth);
                    frame.stack.truncate(stack);
                    if let Abrupt::Throw(error) = &abrupt {
                        if !catching && !finalizing {
                            if let Some(target) = catch {
                                let value = if let Some(v) = &error.thrown {
                                    v.clone()
                                } else {
                                    self.error_value(error.clone())?
                                };
                                if let Some(Handler::Try { catching, .. }) =
                                    frame.handlers.last_mut()
                                {
                                    *catching = true;
                                }
                                frame.stack.push(Cell::Value(value));
                                frame.ip = target;
                                return Ok(None);
                            }
                        }
                    }
                    if !finalizing {
                        if let Some(target) = finally {
                            if let Some(Handler::Try {
                                saved, finalizing, ..
                            }) = frame.handlers.last_mut()
                            {
                                *saved = Some(abrupt);
                                *finalizing = true;
                            }
                            frame.ip = target;
                            return Ok(None);
                        }
                    }
                    frame.handlers.pop();
                }
                Handler::Iterator {
                    mut values,
                    depth,
                    stack,
                    asynchronous,
                    ..
                } => {
                    frame.handlers.pop();
                    frame.scopes.truncate(depth);
                    frame.stack.truncate(stack);
                    if let LoopValues::Iterator(iterator) = &mut values {
                        let close = if asynchronous {
                            self.close_async_iterator(iterator)
                        } else {
                            self.close_iterator(iterator).map(|()| None)
                        };
                        match close {
                            Ok(Some(value)) => {
                                frame.closing = Some(abrupt);
                                return Ok(Some(Run::Await(value)));
                            }
                            Ok(None) => {}
                            Err(error) => {
                                if error.is_resource_limit() {
                                    return Err(error);
                                }
                                if !matches!(abrupt, Abrupt::Throw(_)) {
                                    abrupt = Abrupt::Throw(error);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    fn run_frame(&mut self, frame: &mut Frame, resume: Option<Resume>) -> Result<Run, JsError> {
        let code = frame.code.clone();
        let trace = self.call_stack.len();
        self.call_stack
            .push((frame.name.clone(), frame.source.clone(), frame.span));
        let strict = self.strict;
        let source = self.source_url.clone();
        let module = self.current_module;
        self.current_module = frame.module;
        self.source_url = frame.source.clone();
        self.strict = frame.strict;
        let active = self.active.len();
        self.active.extend(frame.scopes.iter().copied());
        let result = (|| {
            if let Some(resume) = resume {
                if let Some(mut abrupt) = frame.closing.take() {
                    let error = match resume {
                        Resume::Throw(value) => Some(thrown(value, frame.span)),
                        Resume::Value(value) if value.object().is_none() => Some(JsError::new(
                            ErrorKind::TypeError,
                            "async iterator.return result",
                        )),
                        _ => None,
                    };
                    if !matches!(abrupt, Abrupt::Throw(_)) {
                        if let Some(error) = error {
                            abrupt = Abrupt::Throw(error);
                        }
                    }
                    if let Some(outcome) = self.unwind(frame, abrupt)? {
                        return Ok(outcome);
                    }
                } else if frame.delegate.is_some() {
                    if let Some(outcome) = self.resume_delegate(frame, resume)? {
                        return Ok(outcome);
                    }
                } else {
                    match resume {
                        Resume::Value(value) => {
                            if frame.started {
                                frame.stack.push(Cell::Value(value));
                            }
                        }
                        Resume::Throw(value) => {
                            if let Some(outcome) =
                                self.unwind(frame, Abrupt::Throw(thrown(value, frame.span)))?
                            {
                                return Ok(outcome);
                            }
                        }
                        Resume::Return(value) => {
                            if let Some(outcome) = self.unwind(frame, Abrupt::Return(value))? {
                                return Ok(outcome);
                            }
                        }
                    }
                }
            }
            frame.started = true;
            loop {
                self.tick()?;
                let instruction = code.code.get(frame.ip).ok_or_else(stack_error)?;
                frame.ip += 1;
                let span = instruction.span;
                frame.span = span;
                if let Some(current) = self.call_stack.last_mut() {
                    current.2 = span;
                }
                let step = (|| -> Result<Option<Run>, JsError> {
                    let env = frame.env()?;
                    match &instruction.op {
                        Op::Import => {
                            let specifier = frame.value()?;
                            frame
                                .stack
                                .push(Cell::Value(self.dynamic_import(specifier)?));
                        }
                        Op::ImportMeta => {
                            frame.stack.push(Cell::Value(self.import_meta()?));
                        }
                        Op::Literal(literal) => {
                            let value = match literal {
                                Literal::Undefined => JsValue::Undefined,
                                Literal::Null => JsValue::Null,
                                Literal::Bool(v) => JsValue::Bool(*v),
                                Literal::Number(v) => JsValue::Number(*v),
                                Literal::String(v) => JsValue::string(v),
                                Literal::BigInt(v) => {
                                    JsValue::BigInt(Arc::new(crate::bigint::BigInt::parse(v)?))
                                }
                                Literal::RegExp(source, flags) => self.regexp(source, flags)?,
                            };
                            frame.stack.push(Cell::Value(value));
                        }
                        Op::Name(name) => frame.stack.push(Cell::Value(self.lookup(env, name)?)),
                        Op::This => frame.stack.push(Cell::Value(self.scope(env)?.this.clone())),
                        Op::SuperBase => frame
                            .stack
                            .push(Cell::Value(self.lookup(env, "\0super-base")?)),
                        Op::Function(f) => frame
                            .stack
                            .push(Cell::Value(self.function(f.clone(), env)?)),
                        Op::Class(class) => frame.stack.push(Cell::Value(self.class(class, env)?)),
                        Op::Pop => {
                            frame.pop()?;
                        }
                        Op::Dup => {
                            let value = frame.stack.last().cloned().ok_or_else(stack_error)?;
                            frame.stack.push(value);
                        }
                        Op::Last => frame.last = frame.value()?,
                        Op::UndefinedLast => frame.last = JsValue::Undefined,
                        Op::Object => frame.stack.push(Cell::Value(self.object()?)),
                        Op::ToKey => {
                            let value = frame.value()?;
                            frame.stack.push(Cell::Key(self.property_key(value)?));
                        }
                        Op::ObjectData(proto) => {
                            let value = frame.value()?;
                            let key = frame.key()?;
                            let object = match frame.stack.last() {
                                Some(Cell::Value(object)) => object.clone(),
                                _ => return Err(stack_error()),
                            };
                            let id = object.object().ok_or_else(stack_error)?;
                            if *proto {
                                if value == JsValue::Null || value.object().is_some() {
                                    self.heap.get_mut(id)?.prototype = value.object();
                                }
                            } else {
                                self.define(id, key, Property::data(value))?;
                            }
                        }
                        Op::ObjectMethod(kind) => {
                            let function = frame.value()?;
                            let key = frame.key()?;
                            let object = match frame.stack.last() {
                                Some(Cell::Value(v)) => v.clone(),
                                _ => return Err(stack_error()),
                            };
                            self.method(object, key, function, *kind, true)?;
                        }
                        Op::ObjectSpread => {
                            let source = frame.value()?;
                            let object = match frame.stack.last() {
                                Some(Cell::Value(v)) => v.clone(),
                                _ => return Err(stack_error()),
                            };
                            if !matches!(source, JsValue::Undefined | JsValue::Null) {
                                for key in self.keys(&source, true)? {
                                    let value = self.get_key(source.clone(), key.clone())?;
                                    self.define(
                                        object.object().ok_or_else(stack_error)?,
                                        key,
                                        Property::data(value),
                                    )?;
                                }
                            }
                        }
                        Op::Array => frame.stack.push(Cell::Array {
                            values: Vec::new(),
                            holes: Vec::new(),
                        }),
                        Op::ArrayItem(spread) => {
                            let value = frame.value()?;
                            let mut added = if *spread {
                                self.iterable_values(value)?
                            } else {
                                vec![value]
                            };
                            let Some(Cell::Array { values, .. }) = frame.stack.last_mut() else {
                                return Err(stack_error());
                            };
                            if values.len().saturating_add(added.len()) > 100_000 {
                                return Err(JsError::new(
                                    ErrorKind::MemoryLimit,
                                    "array literal quota",
                                ));
                            }
                            values.append(&mut added);
                        }
                        Op::ArrayHole => {
                            let Some(Cell::Array { values, holes }) = frame.stack.last_mut() else {
                                return Err(stack_error());
                            };
                            holes.push(values.len());
                            values.push(JsValue::Undefined);
                        }
                        Op::ArrayFinish => {
                            let Cell::Array { values, holes } = frame.pop()? else {
                                return Err(stack_error());
                            };
                            let array = self.array(values)?;
                            for hole in holes {
                                self.delete(array.clone(), hole.to_string().into())?;
                            }
                            frame.stack.push(Cell::Value(array));
                        }
                        Op::Text => frame.stack.push(Cell::Text(String::new())),
                        Op::TextPart => {
                            let value = frame.value()?;
                            let text = self.to_string(value)?;
                            let Some(Cell::Text(out)) = frame.stack.last_mut() else {
                                return Err(stack_error());
                            };
                            self.check_string_bytes(out.len().saturating_add(text.len()))?;
                            out.push_str(&text);
                        }
                        Op::TextFinish => {
                            let Cell::Text(value) = frame.pop()? else {
                                return Err(stack_error());
                            };
                            frame.stack.push(Cell::Value(JsValue::string(value)));
                        }
                        Op::ReferenceName(name) => frame.stack.push(Cell::Reference(Ref::Name {
                            env,
                            name: name.clone(),
                        })),
                        Op::ReferenceProperty(super_) => {
                            let key = frame.value()?;
                            let key = self.property_key(key)?;
                            let object = frame.value()?;
                            let receiver = if *super_ {
                                self.scope(env)?.this.clone()
                            } else {
                                object.clone()
                            };
                            frame.stack.push(Cell::Reference(Ref::Property {
                                object,
                                key,
                                receiver,
                            }));
                        }
                        Op::ReadReference(keep) => {
                            let reference = frame.reference()?;
                            let value = self.read_bc_reference(&reference)?;
                            if *keep {
                                frame.stack.push(Cell::Reference(reference));
                            }
                            frame.stack.push(Cell::Value(value));
                        }
                        Op::WriteReference => {
                            let value = frame.value()?;
                            let reference = frame.reference()?;
                            self.write_bc_reference(reference, value.clone())?;
                            frame.stack.push(Cell::Value(value));
                        }
                        Op::AssignTarget(target) => {
                            let value = frame.value()?;
                            self.assign_target(target, value.clone(), env)?;
                            frame.stack.push(Cell::Value(value));
                        }
                        Op::Update(op, prefix) => {
                            let reference = frame.reference()?;
                            let value = self.read_bc_reference(&reference)?;
                            let value = self.coerce_primitive(value, true)?;
                            let (old, new) = if let JsValue::BigInt(n) = value {
                                let new = if op == "++" {
                                    n.add(&crate::bigint::BigInt::one())?
                                } else {
                                    n.sub(&crate::bigint::BigInt::one())?
                                };
                                (JsValue::BigInt(n), JsValue::BigInt(Arc::new(new)))
                            } else {
                                let old = self.to_number(value)?;
                                (
                                    JsValue::Number(old),
                                    JsValue::Number(old + if op == "++" { 1.0 } else { -1.0 }),
                                )
                            };
                            self.write_bc_reference(reference, new.clone())?;
                            frame
                                .stack
                                .push(Cell::Value(if *prefix { new } else { old }));
                        }
                        Op::DeleteReference => {
                            let reference = frame.reference()?;
                            let value = match reference {
                                Ref::Property { object, key, .. } => self.delete(object, key)?,
                                _ => false,
                            };
                            frame.stack.push(Cell::Value(JsValue::Bool(value)));
                        }
                        Op::DeleteName => {
                            if self.strict {
                                return Err(JsError::new(
                                    ErrorKind::SyntaxError,
                                    "delete identifier in strict mode",
                                ));
                            }
                            frame.stack.push(Cell::Value(JsValue::Bool(false)));
                        }
                        Op::TypeofName(name) => {
                            let value = match self.lookup(env, name) {
                                Ok(v) => v,
                                Err(error)
                                    if error.kind == ErrorKind::ReferenceError
                                        && !self.binding_exists(env, name)? =>
                                {
                                    JsValue::Undefined
                                }
                                Err(error) => return Err(error),
                            };
                            frame
                                .stack
                                .push(Cell::Value(JsValue::string(self.typeof_value(&value))));
                        }
                        Op::Unary(op) => {
                            let value = frame.value()?;
                            let value = if matches!(op.as_str(), "-" | "~") {
                                self.coerce_primitive(value, true)?
                            } else {
                                value
                            };
                            let result = match op.as_str() {
                                "typeof" => JsValue::string(self.typeof_value(&value)),
                                "!" => JsValue::Bool(!value.truthy()),
                                "void" => JsValue::Undefined,
                                "+" => JsValue::Number(self.to_number(value)?),
                                "-" => {
                                    if let JsValue::BigInt(n) = value {
                                        JsValue::BigInt(Arc::new(n.negated()))
                                    } else {
                                        JsValue::Number(-self.to_number(value)?)
                                    }
                                }
                                "~" => {
                                    if let JsValue::BigInt(n) = value {
                                        JsValue::BigInt(Arc::new(n.not()?))
                                    } else {
                                        JsValue::Number((!to_i32(self.to_number(value)?)) as f64)
                                    }
                                }
                                _ => return Err(stack_error()),
                            };
                            frame.stack.push(Cell::Value(result));
                        }
                        Op::Binary(op) => {
                            let right = frame.value()?;
                            let left = frame.value()?;
                            frame.stack.push(Cell::Value(self.binary(op, left, right)?));
                        }
                        Op::PrepareCall {
                            direct_eval,
                            super_call,
                        } => {
                            let function = frame.value()?;
                            let this = if *super_call {
                                self.scope(env)?.this.clone()
                            } else {
                                JsValue::Undefined
                            };
                            frame.stack.push(Cell::Call {
                                function,
                                this,
                                direct: *direct_eval,
                                super_: *super_call,
                            });
                        }
                        Op::PrepareMemberCall => {
                            let reference = frame.reference()?;
                            let function = self.read_bc_reference(&reference)?;
                            let this = match reference {
                                Ref::Property { receiver, .. } => receiver,
                                _ => JsValue::Undefined,
                            };
                            frame.stack.push(Cell::Call {
                                function,
                                this,
                                direct: false,
                                super_: false,
                            });
                        }
                        Op::Arguments => frame.stack.push(Cell::Arguments(Vec::new())),
                        Op::Argument(spread) => {
                            let value = frame.value()?;
                            let mut added = if *spread {
                                self.iterable_values(value)?
                            } else {
                                vec![value]
                            };
                            let Some(Cell::Arguments(values)) = frame.stack.last_mut() else {
                                return Err(stack_error());
                            };
                            if values.len().saturating_add(added.len()) > 100_000 {
                                return Err(JsError::new(ErrorKind::MemoryLimit, "argument quota"));
                            }
                            values.append(&mut added);
                        }
                        Op::Call => {
                            let Cell::Arguments(args) = frame.pop()? else {
                                return Err(stack_error());
                            };
                            let Cell::Call {
                                function,
                                this,
                                direct,
                                super_,
                            } = frame.pop()?
                            else {
                                return Err(stack_error());
                            };
                            let direct_eval=direct&&function.object().and_then(|id|self.heap.get(id).ok()).is_some_and(|o|matches!(&o.kind,ObjectKind::Native(name)if name=="eval"));
                            let value = if direct_eval {
                                if let Some(JsValue::String(source)) = args.first() {
                                    self.dynamic_eval(source, Some(env))?
                                } else {
                                    self.call_inner(function, this, args, false, None)?
                                }
                            } else {
                                self.call_inner(
                                    function,
                                    this,
                                    args,
                                    super_,
                                    if super_ { Some(env) } else { None },
                                )?
                            };
                            frame.stack.push(Cell::Value(value));
                        }
                        Op::New => {
                            let Cell::Arguments(args) = frame.pop()? else {
                                return Err(stack_error());
                            };
                            let function = frame.value()?;
                            frame
                                .stack
                                .push(Cell::Value(self.construct(function, args)?));
                        }
                        Op::Jump(ip) => frame.ip = *ip,
                        Op::JumpFalse(ip) => {
                            if !frame.value()?.truthy() {
                                frame.ip = *ip;
                            }
                        }
                        Op::Logical(op, ip) => {
                            let value = frame.value()?;
                            let retain = op == "&&" && !value.truthy()
                                || op == "||" && value.truthy()
                                || op == "??"
                                    && !matches!(value, JsValue::Null | JsValue::Undefined);
                            if retain {
                                frame.stack.push(Cell::Value(value));
                                frame.ip = *ip;
                            }
                        }
                        Op::EnterScope(plan) => {
                            let env = self.env(env, false, self.scope(env)?.this.clone())?;
                            self.enter_plan(plan, env)?;
                            frame.scopes.push(env);
                        }
                        Op::ExitScope => {
                            if frame.scopes.len() <= 1 {
                                return Err(stack_error());
                            }
                            frame.scopes.pop();
                        }
                        Op::CloneScope => {
                            let parent = self.scope(env)?.parent.unwrap_or(0);
                            let next = self.env(parent, false, self.scope(env)?.this.clone())?;
                            let bindings = self.scope(env)?.bindings.clone();
                            self.reserve_memory(
                                bindings
                                    .iter()
                                    .map(|(name, b)| 192 + name.len() + b.value.retained_bytes())
                                    .sum(),
                            )?;
                            if let Some(scope) = self.environments[next].as_mut() {
                                scope.bindings = bindings;
                            }
                            if let Some(env) = frame.scopes.last_mut() {
                                *env = next;
                            }
                        }
                        Op::Bind(pattern, kind) => {
                            let value = frame.value()?;
                            self.bind(pattern, value, env, *kind)?;
                        }
                        Op::DeclareClass(name) => {
                            let value = frame.value()?;
                            self.declare_name(env, name, DeclKind::Let, value, true)?;
                        }
                        Op::AbruptJump { target, depth } => {
                            return self.unwind(
                                frame,
                                Abrupt::Jump {
                                    target: *target,
                                    depth: *depth,
                                },
                            )
                        }
                        Op::Return => {
                            let value = frame.value()?;
                            return self.unwind(frame, Abrupt::Return(value));
                        }
                        Op::Throw => {
                            let value = frame.value()?;
                            return self.unwind(frame, Abrupt::Throw(thrown(value, span)));
                        }
                        Op::EnterTry {
                            catch,
                            finally,
                            end,
                        } => frame.handlers.push(Handler::Try {
                            begin: frame.ip,
                            catch: *catch,
                            finally: *finally,
                            end: *end,
                            depth: frame.scopes.len(),
                            stack: frame.stack.len(),
                            catching: false,
                            finalizing: false,
                            saved: None,
                        }),
                        Op::LeaveTry => {
                            let Some(Handler::Try {
                                finally,
                                end,
                                depth,
                                stack,
                                ..
                            }) = frame.handlers.last().cloned()
                            else {
                                return Err(stack_error());
                            };
                            frame.scopes.truncate(depth);
                            frame.stack.truncate(stack);
                            if let Some(ip) = finally {
                                if let Some(Handler::Try {
                                    finalizing, saved, ..
                                }) = frame.handlers.last_mut()
                                {
                                    *finalizing = true;
                                    *saved = None;
                                }
                                frame.ip = ip;
                            } else {
                                frame.handlers.pop();
                                frame.ip = end;
                            }
                        }
                        Op::EndFinally => {
                            let Some(Handler::Try { saved, end, .. }) = frame.handlers.pop() else {
                                return Err(stack_error());
                            };
                            if let Some(saved) = saved {
                                return self.unwind(frame, saved);
                            }
                            frame.ip = end;
                        }
                        Op::EnterIterator {
                            of,
                            asynchronous,
                            end,
                        } => {
                            let (of, asynchronous, end) = (*of, *asynchronous, *end);
                            let source = frame.value()?;
                            let values = if of {
                                LoopValues::Iterator(if asynchronous {
                                    self.get_async_iterator(source)?
                                } else {
                                    self.get_iterator(source)?
                                })
                            } else {
                                LoopValues::Keys {
                                    values: self
                                        .enumerable_keys(source)?
                                        .into_iter()
                                        .filter_map(|k| match k {
                                            PropertyKey::String(s) => Some(JsValue::string(s)),
                                            _ => None,
                                        })
                                        .collect(),
                                    index: 0,
                                }
                            };
                            frame.handlers.push(Handler::Iterator {
                                begin: frame.ip,
                                end,
                                depth: frame.scopes.len(),
                                stack: frame.stack.len(),
                                values,
                                asynchronous,
                            });
                        }
                        Op::AsyncIteratorNext => {
                            let iterator = frame
                                .handlers
                                .iter()
                                .rev()
                                .find_map(|h| match h {
                                    Handler::Iterator {
                                        values: LoopValues::Iterator(iterator),
                                        ..
                                    } => Some(iterator.clone()),
                                    _ => None,
                                })
                                .ok_or_else(stack_error)?;
                            let result = self.call_inner(
                                iterator.next.clone(),
                                iterator.object.clone(),
                                Vec::new(),
                                false,
                                None,
                            )?;
                            let result = self.async_iterator_result(&iterator, result)?;
                            frame.stack.push(Cell::Value(result));
                        }
                        Op::AsyncIteratorValue(end) => {
                            let result = frame.value()?;
                            if result.object().is_none() {
                                return Err(JsError::new(
                                    ErrorKind::TypeError,
                                    "async iterator result is not an object",
                                ));
                            }
                            if self.get(result.clone(), "done")?.truthy() {
                                if let Some(Handler::Iterator {
                                    values: LoopValues::Iterator(iterator),
                                    ..
                                }) = frame
                                    .handlers
                                    .iter_mut()
                                    .rev()
                                    .find(|h| matches!(h, Handler::Iterator { .. }))
                                {
                                    iterator.done = true;
                                }
                                frame.ip = *end;
                            } else {
                                let value = self.get(result, "value")?;
                                frame.stack.push(Cell::Value(value));
                            }
                        }
                        Op::IteratorNext(end) => {
                            let index = frame
                                .handlers
                                .iter()
                                .rposition(|h| matches!(h, Handler::Iterator { .. }))
                                .ok_or_else(stack_error)?;
                            let Handler::Iterator { mut values, .. } =
                                frame.handlers[index].clone()
                            else {
                                return Err(stack_error());
                            };
                            let value = match &mut values {
                                LoopValues::Iterator(iterator) => self.iterator_step(iterator)?,
                                LoopValues::Keys { values, index } => {
                                    let value = values.get(*index).cloned();
                                    *index += 1;
                                    value
                                }
                            };
                            if let Handler::Iterator { values: stored, .. } =
                                &mut frame.handlers[index]
                            {
                                *stored = values;
                            }
                            if let Some(value) = value {
                                frame.stack.push(Cell::Value(value));
                            } else {
                                frame.ip = *end;
                            }
                        }
                        Op::ExitIterator => {
                            let Some(Handler::Iterator { .. }) = frame.handlers.pop() else {
                                return Err(stack_error());
                            };
                        }
                        Op::Await => {
                            let value = frame.value()?;
                            return Ok(Some(Run::Await(value)));
                        }
                        Op::Yield(delegate) => {
                            let value = frame.value()?;
                            if *delegate {
                                frame.delegate = Some(if frame.asynchronous {
                                    self.get_async_iterator(value)?
                                } else {
                                    self.get_iterator(value)?
                                });
                                return self
                                    .resume_delegate(frame, Resume::Value(JsValue::Undefined));
                            }
                            return Ok(Some(Run::Yield(value)));
                        }
                        Op::End => {
                            return Ok(Some(Run::Done(if frame.code.function {
                                JsValue::Undefined
                            } else {
                                frame.last.clone()
                            })))
                        }
                    }
                    Ok(None)
                })();
                match step {
                    Ok(Some(outcome)) => return Ok(outcome),
                    Ok(None) => {}
                    Err(mut error) => {
                        if error.line == 0 {
                            error.line = span.line;
                            error.column = span.column;
                        }
                        if error.is_resource_limit() {
                            return Err(error);
                        }
                        if let Some(outcome) = self.unwind(frame, Abrupt::Throw(error))? {
                            return Ok(outcome);
                        }
                    }
                }
                if frame.stack.len() > 100_000 || frame.handlers.len() > 1024 {
                    return Err(JsError::new(
                        ErrorKind::MemoryLimit,
                        "activation stack quota",
                    ));
                }
                if frame.ip & 255 == 0 {
                    self.check_temporary(
                        frame.stack.len() * 128
                            + frame.handlers.len() * 192
                            + frame.scopes.len() * 8,
                    )?;
                }
            }
        })();
        self.strict = strict;
        self.source_url = source;
        self.current_module = module;
        self.active.truncate(active);
        self.call_stack.truncate(trace);
        result.map_err(|mut e| {
            if !frame.name.is_empty() {
                e.stack.push(format!(
                    "{} ({}:{}:{})",
                    frame.name, frame.source, frame.span.line, frame.span.column
                ));
            }
            e
        })
    }
    fn resume_delegate(
        &mut self,
        frame: &mut Frame,
        resume: Resume,
    ) -> Result<Option<Run>, JsError> {
        let Some(mut iterator) = frame.delegate.take() else {
            return Err(stack_error());
        };
        if let Some(mode) = frame.delegate_awaiting.take() {
            let result = match resume {
                Resume::Value(value) => value,
                Resume::Throw(value) => {
                    return self.unwind(frame, Abrupt::Throw(thrown(value, frame.span)))
                }
                Resume::Return(value) => return self.unwind(frame, Abrupt::Return(value)),
            };
            if matches!(mode, DelegateAwait::MissingThrow) {
                if result.object().is_none() {
                    return Err(JsError::new(
                        ErrorKind::TypeError,
                        "async iterator.return result",
                    ));
                }
                return self.unwind(
                    frame,
                    Abrupt::Throw(JsError::new(
                        ErrorKind::TypeError,
                        "delegated iterator has no throw",
                    )),
                );
            }
            return self.delegate_result(
                frame,
                iterator,
                matches!(mode, DelegateAwait::Return),
                result,
            );
        }
        let (returning, value, method) = match resume {
            Resume::Value(value) => (false, value, iterator.next.clone()),
            Resume::Throw(value) => {
                let method = self.get(iterator.object.clone(), "throw")?;
                if matches!(method, JsValue::Undefined | JsValue::Null) {
                    if frame.asynchronous {
                        if let Some(result) = self.close_async_iterator(&mut iterator)? {
                            frame.delegate = Some(iterator);
                            frame.delegate_awaiting = Some(DelegateAwait::MissingThrow);
                            return Ok(Some(Run::Await(result)));
                        }
                    }
                    self.close_iterator(&mut iterator)?;
                    return self.unwind(
                        frame,
                        Abrupt::Throw(JsError::new(
                            ErrorKind::TypeError,
                            "delegated iterator has no throw",
                        )),
                    );
                }
                (false, value, method)
            }
            Resume::Return(value) => {
                let method = self.get(iterator.object.clone(), "return")?;
                if matches!(method, JsValue::Undefined | JsValue::Null) {
                    return self.unwind(frame, Abrupt::Return(value));
                }
                (true, value, method)
            }
        };
        let result = self.call_inner(method, iterator.object.clone(), vec![value], false, None)?;
        if frame.asynchronous {
            let result = self.async_iterator_result(&iterator, result)?;
            frame.delegate = Some(iterator);
            frame.delegate_awaiting = Some(if returning {
                DelegateAwait::Return
            } else {
                DelegateAwait::Next
            });
            return Ok(Some(Run::Await(result)));
        }
        self.delegate_result(frame, iterator, returning, result)
    }
    fn delegate_result(
        &mut self,
        frame: &mut Frame,
        iterator: IteratorRecord,
        returning: bool,
        result: JsValue,
    ) -> Result<Option<Run>, JsError> {
        if result.object().is_none() {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "delegated iterator result",
            ));
        }
        let done = self.get(result.clone(), "done")?.truthy();
        let value = self.get(result, "value")?;
        if done {
            if returning {
                self.unwind(frame, Abrupt::Return(value))
            } else {
                frame.stack.push(Cell::Value(value));
                Ok(None)
            }
        } else {
            frame.delegate = Some(iterator);
            Ok(Some(Run::Yield(value)))
        }
    }
    pub(super) fn execute_chunk(
        &mut self,
        code: Arc<Chunk>,
        env: usize,
    ) -> Result<JsValue, JsError> {
        self.check_temporary(code.retained_bytes())?;
        self.enter_plan(&code.plan, env)?;
        let mut frame = Frame::new(code, env, self.strict, String::new(), Span::default());
        frame.module = self.current_module;
        frame.source = self.source_url.clone();
        match self.run_frame(&mut frame, None)? {
            Run::Done(value) => Ok(value),
            _ => Err(JsError::new(
                ErrorKind::SyntaxError,
                "suspension outside async/generator",
            )),
        }
    }
    pub(super) fn execute_function(
        &mut self,
        function: &UserFunction,
        env: usize,
    ) -> Result<JsValue, JsError> {
        self.enter_plan(&function.code.plan, env)?;
        let mut frame = Frame::new(
            function.code.clone(),
            env,
            function.strict,
            function
                .function
                .name
                .clone()
                .unwrap_or_else(|| "<anonymous>".into()),
            function.function.span,
        );
        frame.asynchronous = function.function.asynchronous;
        frame.module = function.module;
        frame.source = function.source_url.clone();
        if function.function.generator {
            return Ok(JsValue::Object(self.allocate(
                ObjectKind::Generator(Box::new(GeneratorData {
                    requests: Default::default(),
                    frame: Some(frame),
                    state: GeneratorState::Start,
                    asynchronous: function.function.asynchronous,
                })),
                Some(if function.function.asynchronous {
                    self.async_generator_proto
                } else {
                    self.generator_proto
                }),
            )?));
        }
        if function.function.asynchronous {
            let promise = self.promise()?;
            let id = self.allocate(
                ObjectKind::AsyncTask(Box::new(AsyncData {
                    module: None,
                    frame: Some(frame),
                    promise: promise.clone(),
                    finished: false,
                })),
                None,
            )?;
            self.resume_async(id, None)?;
            return Ok(promise);
        }
        match self.run_frame(&mut frame, None)? {
            Run::Done(value) => Ok(value),
            _ => Err(JsError::new(
                ErrorKind::SyntaxError,
                "unexpected suspension",
            )),
        }
    }
    pub(crate) fn resume_async(
        &mut self,
        id: ObjectId,
        resume: Option<Resume>,
    ) -> Result<(), JsError> {
        let (frame, promise, module) = {
            let ObjectKind::AsyncTask(data) = &mut self.heap.get_mut(id)?.kind else {
                return Err(stack_error());
            };
            if data.finished {
                return Ok(());
            }
            let frame = data.frame.take().ok_or_else(stack_error)?;
            (frame, data.promise.clone(), data.module)
        };
        let mut frame = frame;
        match self.run_frame(&mut frame, resume) {
            Ok(Run::Await(value)) => {
                if let ObjectKind::AsyncTask(data) = &mut self.heap.get_mut(id)?.kind {
                    data.frame = Some(frame);
                }
                self.await_async(id, value)?;
            }
            Ok(Run::Done(value)) => {
                if let ObjectKind::AsyncTask(data) = &mut self.heap.get_mut(id)?.kind {
                    data.finished = true;
                }
                let value = if let Some(module) = module {
                    self.module_completed(module)?
                } else {
                    value
                };
                self.resolve_promise(promise, value)?;
            }
            Ok(Run::Yield(_)) => {
                return Err(JsError::new(
                    ErrorKind::SyntaxError,
                    "yield outside generator",
                ))
            }
            Err(error) => {
                if error.is_resource_limit() {
                    return Err(error);
                }
                if let ObjectKind::AsyncTask(data) = &mut self.heap.get_mut(id)?.kind {
                    data.finished = true;
                }
                let reason = if let Some(v) = error.thrown {
                    v
                } else {
                    self.error_value(error)?
                };
                if let Some(module) = module {
                    self.module_failed(module, reason.clone())?;
                }
                self.reject_promise(promise, reason)?;
            }
        }
        Ok(())
    }
    pub(crate) fn generator_resume(
        &mut self,
        id: ObjectId,
        resume: Resume,
    ) -> Result<JsValue, JsError> {
        let ObjectKind::Generator(data) = self.heap.get(id)?.kind.clone() else {
            return Err(JsError::new(ErrorKind::TypeError, "generator receiver"));
        };
        let mut data = *data;
        if data.asynchronous {
            let promise = self.promise()?;
            if data.requests.len() >= 1000 {
                return Err(JsError::new(
                    ErrorKind::MemoryLimit,
                    "async generator request quota",
                ));
            }
            data.requests.push_back(GeneratorRequest {
                resume,
                promise: promise.clone(),
            });
            let start = data.state != GeneratorState::Executing;
            self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
            if start {
                self.pump_async_generator(id, None)?;
            }
            return Ok(promise);
        }
        if data.state == GeneratorState::Executing {
            return Err(JsError::new(
                ErrorKind::TypeError,
                "generator already executing",
            ));
        }
        if data.state == GeneratorState::Done {
            return match resume {
                Resume::Throw(value) => Err(thrown(value, Span::default())),
                Resume::Return(value) => self.iterator_result(value, true),
                _ => self.iterator_result(JsValue::Undefined, true),
            };
        }
        if data.state == GeneratorState::Start {
            match &resume {
                Resume::Return(value) => {
                    data.state = GeneratorState::Done;
                    data.frame = None;
                    self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
                    return self.iterator_result(value.clone(), true);
                }
                Resume::Throw(value) => {
                    data.state = GeneratorState::Done;
                    data.frame = None;
                    self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
                    return Err(thrown(value.clone(), Span::default()));
                }
                _ => {}
            }
        }
        let mut frame = data.frame.take().ok_or_else(stack_error)?;
        data.state = GeneratorState::Executing;
        self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data.clone()));
        let result = self.run_frame(&mut frame, Some(resume));
        match result {
            Ok(Run::Yield(value)) => {
                data.state = GeneratorState::Suspended;
                data.frame = Some(frame);
                self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
                self.iterator_result(value, false)
            }
            Ok(Run::Done(value)) => {
                data.state = GeneratorState::Done;
                self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
                self.iterator_result(value, true)
            }
            Ok(Run::Await(_)) => Err(JsError::new(
                ErrorKind::SyntaxError,
                "await in synchronous generator",
            )),
            Err(error) => {
                data.state = GeneratorState::Done;
                self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
                Err(error)
            }
        }
    }
    pub(super) fn install_generators(&mut self) -> Result<(), JsError> {
        self.generator_proto = self.allocate(ObjectKind::Ordinary, Some(self.iterator_proto))?;
        for name in ["next", "throw", "return"] {
            let value = self.native(&format!("Generator.{name}"))?;
            self.define(
                self.generator_proto,
                name.into(),
                Property {
                    enumerable: false,
                    ..Property::data(value)
                },
            )?;
        }
        self.define(
            self.generator_proto,
            PropertyKey::Symbol(crate::symbols::TO_STRING_TAG),
            Property {
                writable: false,
                enumerable: false,
                configurable: true,
                ..Property::data(JsValue::string("Generator"))
            },
        )?;
        self.async_generator_proto =
            self.allocate(ObjectKind::Ordinary, Some(self.object_proto))?;
        for name in ["next", "throw", "return"] {
            let value = self.native(&format!("Generator.{name}"))?;
            self.define(
                self.async_generator_proto,
                name.into(),
                Property {
                    enumerable: false,
                    ..Property::data(value)
                },
            )?;
        }
        let identity = self.native("Iterator.identity")?;
        self.define(
            self.async_generator_proto,
            PropertyKey::Symbol(crate::symbols::ASYNC_ITERATOR),
            Property {
                enumerable: false,
                ..Property::data(identity)
            },
        )?;
        Ok(())
    }
    pub(crate) fn generator_builtin(
        &mut self,
        name: &str,
        this: JsValue,
        args: &[JsValue],
    ) -> Option<Result<JsValue, JsError>> {
        let method = name.strip_prefix("Generator.")?;
        let value = args.first().cloned().unwrap_or(JsValue::Undefined);
        Some((|| {
            let id = this
                .object()
                .ok_or_else(|| JsError::new(ErrorKind::TypeError, "generator receiver"))?;
            self.generator_resume(
                id,
                match method {
                    "throw" => Resume::Throw(value),
                    "return" => Resume::Return(value),
                    _ => Resume::Value(value),
                },
            )
        })())
    }
    fn pump_async_generator(
        &mut self,
        id: ObjectId,
        resume: Option<Resume>,
    ) -> Result<(), JsError> {
        let ObjectKind::Generator(data) = self.heap.get(id)?.kind.clone() else {
            return Err(stack_error());
        };
        let mut data = *data;
        let Some(request) = data.requests.front().cloned() else {
            return Ok(());
        };
        let resume = resume.unwrap_or(request.resume);
        if data.state == GeneratorState::Done {
            match resume {
                Resume::Throw(reason) => {
                    data.requests.pop_front();
                    self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
                    self.reject_promise(request.promise, reason)?;
                    return self.pump_async_generator(id, None);
                }
                Resume::Value(_) => {
                    data.requests.pop_front();
                    self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
                    let value = self.iterator_result(JsValue::Undefined, true)?;
                    self.resolve_promise(request.promise, value)?;
                    return self.pump_async_generator(id, None);
                }
                Resume::Return(value) => {
                    data.state = GeneratorState::Executing;
                    self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
                    return self.await_generator(id, value, GeneratorAwait::Return);
                }
            }
        }
        if data.state == GeneratorState::Start {
            match &resume {
                Resume::Throw(value) => {
                    data.frame = None;
                    data.state = GeneratorState::Done;
                    self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
                    return self.finish_async_generator(id, Err(value.clone()), true);
                }
                Resume::Return(value) => {
                    data.frame = None;
                    data.state = GeneratorState::Executing;
                    self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
                    return self.await_generator(id, value.clone(), GeneratorAwait::Return);
                }
                _ => {}
            }
        }
        let mut frame = data.frame.take().ok_or_else(stack_error)?;
        data.state = GeneratorState::Executing;
        self.heap.get_mut(id)?.kind = ObjectKind::Generator(Box::new(data));
        let result = self.run_frame(&mut frame, Some(resume));
        match result {
            Ok(Run::Await(value)) => {
                if let ObjectKind::Generator(data) = &mut self.heap.get_mut(id)?.kind {
                    data.frame = Some(frame);
                }
                self.await_generator(id, value, GeneratorAwait::Body)
            }
            Ok(Run::Yield(value)) => {
                if let ObjectKind::Generator(data) = &mut self.heap.get_mut(id)?.kind {
                    data.frame = Some(frame);
                }
                self.await_generator(id, value, GeneratorAwait::Yield)
            }
            Ok(Run::Done(value)) => self.await_generator(id, value, GeneratorAwait::Return),
            Err(error) => {
                if error.is_resource_limit() {
                    return Err(error);
                }
                let reason = if let Some(v) = error.thrown {
                    v
                } else {
                    self.error_value(error)?
                };
                self.finish_async_generator(id, Err(reason), true)
            }
        }
    }
    pub(crate) fn resume_async_generator(
        &mut self,
        id: ObjectId,
        result: Result<JsValue, JsValue>,
        mode: GeneratorAwait,
    ) -> Result<(), JsError> {
        match mode {
            GeneratorAwait::Body => self.pump_async_generator(
                id,
                Some(match result {
                    Ok(value) => Resume::Value(value),
                    Err(value) => Resume::Throw(value),
                }),
            ),
            GeneratorAwait::Yield => match result {
                Ok(value) => self.finish_async_generator(id, Ok(value), false),
                Err(value) => self.pump_async_generator(id, Some(Resume::Throw(value))),
            },
            GeneratorAwait::Return => self.finish_async_generator(id, result, true),
        }
    }
    fn finish_async_generator(
        &mut self,
        id: ObjectId,
        result: Result<JsValue, JsValue>,
        done: bool,
    ) -> Result<(), JsError> {
        let promise = {
            let ObjectKind::Generator(data) = &mut self.heap.get_mut(id)?.kind else {
                return Err(stack_error());
            };
            let request = data.requests.pop_front().ok_or_else(stack_error)?;
            data.state = if done {
                GeneratorState::Done
            } else {
                GeneratorState::Suspended
            };
            if done {
                data.frame = None;
            }
            request.promise
        };
        match result {
            Ok(value) => {
                let value = self.iterator_result(value, done)?;
                self.resolve_promise(promise, value)?;
            }
            Err(reason) => self.reject_promise(promise, reason)?,
        }
        self.pump_async_generator(id, None)
    }
}
