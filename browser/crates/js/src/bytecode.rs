//! Owned portable stack bytecode. Control flow, references and suspension points
//! are explicit; the evaluator never replays already-executed source on resume.
use crate::{ast::*, ErrorKind, JsError, Span};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Debug, Clone, Default)]
pub(crate) struct ScopePlan {
    pub bindings: Vec<(String, DeclKind)>,
    pub functions: Vec<(String, Arc<Function>)>,
}
impl ScopePlan {
    pub fn body(body: &[Stmt]) -> Self {
        fn vars(
            stmt: &Stmt,
            names: &mut BTreeSet<String>,
            functions: &mut Vec<(String, Arc<Function>)>,
        ) {
            match &stmt.kind {
                StmtKind::Declare(DeclKind::Var, ds) => {
                    for d in ds {
                        pattern_names(&d.pattern, names);
                    }
                }
                StmtKind::Function(name, f) => {
                    names.insert(name.clone());
                    functions.push((name.clone(), f.clone()));
                }
                StmtKind::Block(body) => {
                    for stmt in body {
                        vars(stmt, names, functions);
                    }
                }
                StmtKind::If(_, yes, no) => {
                    vars(yes, names, functions);
                    if let Some(no) = no {
                        vars(no, names, functions);
                    }
                }
                StmtKind::While(_, body)
                | StmtKind::DoWhile(body, _)
                | StmtKind::Label(_, body) => vars(body, names, functions),
                StmtKind::For(init, _, _, body) => {
                    if let Some(init) = init {
                        vars(init, names, functions);
                    }
                    vars(body, names, functions);
                }
                StmtKind::ForEach(binding, _, body, _, _) => {
                    if let ForBinding::Declare(DeclKind::Var, p) = binding {
                        pattern_names(p, names);
                    }
                    vars(body, names, functions);
                }
                StmtKind::Try(body, catch, finally) => {
                    vars(body, names, functions);
                    if let Some((_, body)) = catch {
                        vars(body, names, functions);
                    }
                    if let Some(body) = finally {
                        vars(body, names, functions);
                    }
                }
                StmtKind::Switch(_, cases) => {
                    for (_, body) in cases {
                        for stmt in body {
                            vars(stmt, names, functions);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut names = BTreeSet::new();
        let mut functions = Vec::new();
        for stmt in body {
            vars(stmt, &mut names, &mut functions);
        }
        let mut bindings = names
            .into_iter()
            .map(|n| (n, DeclKind::Var))
            .collect::<Vec<_>>();
        for stmt in body {
            match &stmt.kind {
                StmtKind::Declare(kind, ds) if *kind != DeclKind::Var => {
                    for d in ds {
                        let mut names = BTreeSet::new();
                        pattern_names(&d.pattern, &mut names);
                        bindings.extend(names.into_iter().map(|n| (n, *kind)));
                    }
                }
                StmtKind::Class(name, _) => bindings.push((name.clone(), DeclKind::Let)),
                _ => {}
            }
        }
        Self {
            bindings,
            functions,
        }
    }
    pub fn bytes(&self) -> usize {
        self.bindings
            .iter()
            .map(|(name, _)| name.len() + 64)
            .sum::<usize>()
            + self
                .functions
                .iter()
                .map(|(name, _)| name.len() + 64)
                .sum::<usize>()
    }
}
fn pattern_names(p: &Pattern, names: &mut BTreeSet<String>) {
    match p {
        Pattern::Name(n) => {
            names.insert(n.clone());
        }
        Pattern::Default(p, _) | Pattern::Rest(p) => pattern_names(p, names),
        Pattern::Array(parts, rest) => {
            for p in parts.iter().flatten() {
                pattern_names(p, names);
            }
            if let Some(p) = rest {
                pattern_names(p, names);
            }
        }
        Pattern::Object(parts, rest) => {
            for (_, p) in parts {
                pattern_names(p, names);
            }
            if let Some(p) = rest {
                pattern_names(p, names);
            }
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Op {
    Import,
    ImportMeta,
    Literal(Literal),
    Name(String),
    This,
    SuperBase,
    Function(Arc<Function>),
    Class(Box<Class>),
    Pop,
    Dup,
    Last,
    UndefinedLast,
    Object,
    ToKey,
    ObjectData(bool),
    ObjectMethod(MethodKind),
    ObjectSpread,
    Array,
    ArrayItem(bool),
    ArrayHole,
    ArrayFinish,
    Text,
    TextPart,
    TextFinish,
    ReferenceName(String),
    ReferenceProperty(bool),
    ReadReference(bool),
    WriteReference,
    AssignTarget(Box<Expr>),
    Update(String, bool),
    DeleteReference,
    DeleteName,
    TypeofName(String),
    Unary(String),
    Binary(String),
    PrepareCall {
        direct_eval: bool,
        super_call: bool,
    },
    PrepareMemberCall,
    Arguments,
    Argument(bool),
    Call,
    New,
    Jump(usize),
    JumpFalse(usize),
    Logical(String, usize),
    EnterScope(Arc<ScopePlan>),
    ExitScope,
    CloneScope,
    Bind(Box<Pattern>, DeclKind),
    DeclareClass(String),
    AbruptJump {
        target: usize,
        depth: usize,
    },
    Return,
    Throw,
    EnterTry {
        catch: Option<usize>,
        finally: Option<usize>,
        end: usize,
    },
    LeaveTry,
    EndFinally,
    EnterIterator {
        of: bool,
        asynchronous: bool,
        end: usize,
    },
    AsyncIteratorNext,
    AsyncIteratorValue(usize),
    IteratorNext(usize),
    ExitIterator,
    Await,
    Yield(bool),
    End,
}
#[derive(Debug, Clone)]
pub(crate) struct Instruction {
    pub op: Op,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub struct Chunk {
    pub(crate) strict: bool,
    pub(crate) module: bool,
    pub(crate) code: Vec<Instruction>,
    pub(crate) plan: ScopePlan,
    pub(crate) function: bool,
    pub(crate) bytes: usize,
}
impl Chunk {
    pub fn instruction_count(&self) -> usize {
        self.code.len()
    }
    pub fn retained_bytes(&self) -> usize {
        self.bytes
    }
}
struct Control {
    label: Option<String>,
    breaks: Vec<usize>,
    continues: Vec<usize>,
    break_depth: usize,
    continue_depth: usize,
    loop_: bool,
}
struct Compiler {
    code: Vec<Instruction>,
    depth: usize,
    controls: Vec<Control>,
    bytes: usize,
}
impl Compiler {
    fn emit(&mut self, op: Op, span: Span) -> Result<usize, JsError> {
        self.bytes = self.bytes.saturating_add(192);
        if self.code.len() >= 1_000_000 || self.bytes > 64 * 1024 * 1024 {
            return Err(JsError::new(ErrorKind::MemoryLimit, "bytecode budget"));
        }
        let at = self.code.len();
        self.code.push(Instruction { op, span });
        Ok(at)
    }
    fn patch(&mut self, at: usize, target: usize) {
        match &mut self.code[at].op {
            Op::Jump(n) | Op::JumpFalse(n) | Op::IteratorNext(n) | Op::AsyncIteratorValue(n) => {
                *n = target
            }
            Op::Logical(_, n) => *n = target,
            Op::AbruptJump { target: n, .. } => *n = target,
            _ => {}
        }
    }
    fn plan(&mut self, body: &[Stmt]) -> Result<Arc<ScopePlan>, JsError> {
        let plan = ScopePlan::body(body);
        self.bytes = self.bytes.saturating_add(plan.bytes());
        if self.bytes > 64 * 1024 * 1024 {
            return Err(JsError::new(ErrorKind::MemoryLimit, "scope plan budget"));
        }
        Ok(Arc::new(plan))
    }
    fn scope(&mut self, body: &[Stmt], span: Span) -> Result<(), JsError> {
        let plan = self.plan(body)?;
        self.emit(Op::EnterScope(plan), span)?;
        self.depth += 1;
        Ok(())
    }
    fn end_scope(&mut self, span: Span) -> Result<(), JsError> {
        self.emit(Op::ExitScope, span)?;
        self.depth -= 1;
        Ok(())
    }
    fn statements(&mut self, body: &[Stmt]) -> Result<(), JsError> {
        if body.is_empty() {
            self.emit(Op::UndefinedLast, Span::default())?;
        }
        for stmt in body {
            self.stmt(stmt, None)?;
        }
        Ok(())
    }
    fn control(&mut self, label: Option<String>, loop_: bool) {
        self.controls.push(Control {
            label,
            breaks: Vec::new(),
            continues: Vec::new(),
            break_depth: self.depth,
            continue_depth: self.depth,
            loop_,
        });
    }
    fn finish_control(&mut self, break_target: usize, continue_target: usize) {
        if let Some(control) = self.controls.pop() {
            for patch in control.breaks {
                self.patch(patch, break_target);
            }
            for patch in control.continues {
                self.patch(patch, continue_target);
            }
        }
    }
    fn stmt(&mut self, stmt: &Stmt, label: Option<String>) -> Result<(), JsError> {
        let s = stmt.span;
        match &stmt.kind {
            StmtKind::Empty | StmtKind::Function(..) => {
                self.emit(Op::UndefinedLast, s)?;
            }
            StmtKind::Expr(expr) => {
                self.expr(expr)?;
                self.emit(Op::Last, s)?;
            }
            StmtKind::Block(body) => {
                self.scope(body, s)?;
                self.statements(body)?;
                self.end_scope(s)?;
            }
            StmtKind::Declare(kind, ds) => {
                for d in ds {
                    if let Some(expr) = &d.init {
                        self.expr(expr)?;
                        self.emit(Op::Bind(Box::new(d.pattern.clone()), *kind), s)?;
                    } else if *kind != DeclKind::Var {
                        self.emit(Op::Literal(Literal::Undefined), s)?;
                        self.emit(Op::Bind(Box::new(d.pattern.clone()), *kind), s)?;
                    }
                }
                self.emit(Op::UndefinedLast, s)?;
            }
            StmtKind::Class(name, class) => {
                self.emit(Op::Class(Box::new(class.clone())), s)?;
                self.emit(Op::DeclareClass(name.clone()), s)?;
                self.emit(Op::UndefinedLast, s)?;
            }
            StmtKind::If(test, yes, no) => {
                self.expr(test)?;
                let branch = self.emit(Op::JumpFalse(0), s)?;
                self.stmt(yes, None)?;
                let end = self.emit(Op::Jump(0), s)?;
                self.patch(branch, self.code.len());
                if let Some(no) = no {
                    self.stmt(no, None)?;
                } else {
                    self.emit(Op::UndefinedLast, s)?;
                }
                self.patch(end, self.code.len());
            }
            StmtKind::While(test, body) => {
                self.control(label, true);
                let start = self.code.len();
                self.expr(test)?;
                let end = self.emit(Op::JumpFalse(0), s)?;
                self.stmt(body, None)?;
                self.emit(Op::Jump(start), s)?;
                let target = self.code.len();
                self.patch(end, target);
                self.finish_control(target, start);
                self.emit(Op::UndefinedLast, s)?;
            }
            StmtKind::DoWhile(body, test) => {
                self.control(label, true);
                let start = self.code.len();
                self.stmt(body, None)?;
                let next = self.code.len();
                self.expr(test)?;
                let end = self.emit(Op::JumpFalse(0), s)?;
                self.emit(Op::Jump(start), s)?;
                let target = self.code.len();
                self.patch(end, target);
                self.finish_control(target, next);
                self.emit(Op::UndefinedLast, s)?;
            }
            StmtKind::For(init, test, update, body) => {
                let initial = init
                    .as_ref()
                    .map(|s| std::slice::from_ref(s.as_ref()))
                    .unwrap_or(&[]);
                self.scope(initial, s)?;
                if let Some(init) = init {
                    self.stmt(init, None)?;
                }
                self.control(label, true);
                let start = self.code.len();
                let end = if let Some(test) = test {
                    self.expr(test)?;
                    Some(self.emit(Op::JumpFalse(0), s)?)
                } else {
                    None
                };
                self.stmt(body, None)?;
                let next = self.code.len();
                self.emit(Op::CloneScope, s)?;
                if let Some(update) = update {
                    self.expr(update)?;
                    self.emit(Op::Pop, s)?;
                }
                self.emit(Op::Jump(start), s)?;
                let target = self.code.len();
                if let Some(end) = end {
                    self.patch(end, target);
                }
                self.finish_control(target, next);
                self.end_scope(s)?;
                self.emit(Op::UndefinedLast, s)?;
            }
            StmtKind::ForEach(binding, source, body, of, asynchronous) => {
                self.expr(source)?;
                let enter = self.emit(
                    Op::EnterIterator {
                        of: *of,
                        asynchronous: *asynchronous,
                        end: 0,
                    },
                    s,
                )?;
                self.control(label, true);
                let next = self.code.len();
                let exit = if *asynchronous {
                    self.emit(Op::AsyncIteratorNext, s)?;
                    self.emit(Op::Await, s)?;
                    self.emit(Op::AsyncIteratorValue(0), s)?
                } else {
                    self.emit(Op::IteratorNext(0), s)?
                };
                self.scope(&[], s)?;
                match binding {
                    ForBinding::Declare(kind, pattern) => {
                        self.emit(Op::Bind(Box::new(pattern.clone()), *kind), s)?;
                    }
                    ForBinding::Target(expr) => {
                        self.emit(Op::AssignTarget(Box::new(expr.clone())), s)?;
                        self.emit(Op::Pop, s)?;
                    }
                }
                self.stmt(body, None)?;
                self.end_scope(s)?;
                self.emit(Op::Jump(next), s)?;
                let target = self.code.len();
                self.patch(exit, target);
                self.emit(Op::ExitIterator, s)?;
                let end_ip = self.code.len();
                self.finish_control(end_ip, next);
                if let Op::EnterIterator { end, .. } = &mut self.code[enter].op {
                    *end = end_ip;
                }
                self.emit(Op::UndefinedLast, s)?;
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.expr(value)?;
                } else {
                    self.emit(Op::Literal(Literal::Undefined), s)?;
                }
                self.emit(Op::Return, s)?;
            }
            StmtKind::Throw(value) => {
                self.expr(value)?;
                self.emit(Op::Throw, s)?;
            }
            StmtKind::Break(label) | StmtKind::Continue(label) => {
                let continue_ = matches!(stmt.kind, StmtKind::Continue(_));
                let index = self
                    .controls
                    .iter()
                    .rposition(|control| {
                        label
                            .as_ref()
                            .is_none_or(|label| control.label.as_ref() == Some(label))
                            && (!continue_ || control.loop_)
                    })
                    .ok_or_else(|| JsError::new(ErrorKind::SyntaxError, "unresolved loop label"))?;
                let depth = if continue_ {
                    self.controls[index].continue_depth
                } else {
                    self.controls[index].break_depth
                };
                let patch = self.emit(Op::AbruptJump { target: 0, depth }, s)?;
                if continue_ {
                    self.controls[index].continues.push(patch);
                } else {
                    self.controls[index].breaks.push(patch);
                }
            }
            StmtKind::Try(body, catch, finally) => {
                let enter = self.emit(
                    Op::EnterTry {
                        catch: None,
                        finally: None,
                        end: 0,
                    },
                    s,
                )?;
                self.stmt(body, None)?;
                self.emit(Op::LeaveTry, s)?;
                let catch_ip = if let Some((pattern, body)) = catch {
                    let at = self.code.len();
                    self.scope(&[], s)?;
                    self.emit(Op::Bind(Box::new(pattern.clone()), DeclKind::Let), s)?;
                    self.stmt(body, None)?;
                    self.end_scope(s)?;
                    self.emit(Op::LeaveTry, s)?;
                    Some(at)
                } else {
                    None
                };
                let finally_ip = if let Some(body) = finally {
                    let at = self.code.len();
                    self.stmt(body, None)?;
                    self.emit(Op::EndFinally, s)?;
                    Some(at)
                } else {
                    None
                };
                let end = self.code.len();
                self.code[enter].op = Op::EnterTry {
                    catch: catch_ip,
                    finally: finally_ip,
                    end,
                };
            }
            StmtKind::Switch(test, cases) => {
                self.emit(Op::UndefinedLast, s)?;
                self.expr(test)?;
                self.control(label, false);
                let mut branches = Vec::new();
                let mut default = None;
                for (i, (test, _)) in cases.iter().enumerate() {
                    if let Some(test) = test {
                        self.emit(Op::Dup, s)?;
                        self.expr(test)?;
                        self.emit(Op::Binary("===".into()), s)?;
                        let no = self.emit(Op::JumpFalse(0), s)?;
                        self.emit(Op::Pop, s)?;
                        let yes = self.emit(Op::Jump(0), s)?;
                        branches.push((yes, i));
                        self.patch(no, self.code.len());
                    } else {
                        default = Some(i);
                    }
                }
                self.emit(Op::Pop, s)?;
                let choose_default = self.emit(Op::Jump(0), s)?;
                let mut starts = Vec::new();
                for (_, body) in cases {
                    starts.push(self.code.len());
                    self.statements(body)?;
                }
                let end = self.code.len();
                for (patch, index) in branches {
                    self.patch(patch, starts[index]);
                }
                self.patch(choose_default, default.map_or(end, |i| starts[i]));
                self.finish_control(end, end);
            }
            StmtKind::Label(name, body) => {
                let loop_ = matches!(
                    body.kind,
                    StmtKind::While(..)
                        | StmtKind::DoWhile(..)
                        | StmtKind::For(..)
                        | StmtKind::ForEach(..)
                );
                if loop_ {
                    self.stmt(body, Some(name.clone()))?;
                } else {
                    self.control(Some(name.clone()), false);
                    self.stmt(body, None)?;
                    let end = self.code.len();
                    self.finish_control(end, end);
                }
            }
        }
        Ok(())
    }
    fn property(&mut self, name: &PropertyName) -> Result<(), JsError> {
        match name {
            PropertyName::String(name) => {
                self.emit(Op::Literal(Literal::String(name.clone())), Span::default())?;
            }
            PropertyName::Computed(expr) => self.expr(expr)?,
        }
        self.emit(Op::ToKey, Span::default())?;
        Ok(())
    }
    fn reference(&mut self, expr: &Expr) -> Result<(), JsError> {
        match &expr.kind {
            ExprKind::Name(name) => {
                self.emit(Op::ReferenceName(name.clone()), expr.span)?;
            }
            ExprKind::Member(object, key) => {
                let super_ = matches!(&object.kind,ExprKind::Name(n)if n=="super");
                if super_ {
                    self.emit(Op::SuperBase, expr.span)?;
                } else {
                    self.expr(object)?;
                }
                self.expr(key)?;
                self.emit(Op::ReferenceProperty(super_), expr.span)?;
            }
            _ => {
                return Err(JsError::new(
                    ErrorKind::ReferenceError,
                    "assignment reference",
                ))
            }
        }
        Ok(())
    }
    fn args(&mut self, args: &[Argument], span: Span) -> Result<(), JsError> {
        self.emit(Op::Arguments, span)?;
        for arg in args {
            self.expr(&arg.expr)?;
            self.emit(Op::Argument(arg.spread), span)?;
        }
        Ok(())
    }
    fn expr(&mut self, expr: &Expr) -> Result<(), JsError> {
        let s = expr.span;
        match &expr.kind {
            ExprKind::Import(expr) => {
                self.expr(expr)?;
                self.emit(Op::Import, s)?;
            }
            ExprKind::ImportMeta => {
                self.emit(Op::ImportMeta, s)?;
            }
            ExprKind::Literal(value) => {
                self.emit(Op::Literal(value.clone()), s)?;
            }
            ExprKind::Name(name) => {
                self.emit(Op::Name(name.clone()), s)?;
            }
            ExprKind::This => {
                self.emit(Op::This, s)?;
            }
            ExprKind::Function(f) => {
                self.emit(Op::Function(f.clone()), s)?;
            }
            ExprKind::Class(class) => {
                self.emit(Op::Class(class.clone()), s)?;
            }
            ExprKind::Array(parts) => {
                self.emit(Op::Array, s)?;
                for part in parts {
                    if let Some(part) = part {
                        self.expr(&part.expr)?;
                        self.emit(Op::ArrayItem(part.spread), s)?;
                    } else {
                        self.emit(Op::ArrayHole, s)?;
                    }
                }
                self.emit(Op::ArrayFinish, s)?;
            }
            ExprKind::Object(properties) => {
                self.emit(Op::Object, s)?;
                for property in properties {
                    match property {
                        ObjectProperty::Value(key, value) => {
                            self.property(key)?;
                            self.expr(value)?;
                            self.emit(
                                Op::ObjectData(
                                    matches!(key,PropertyName::String(n)if n=="__proto__"),
                                ),
                                s,
                            )?;
                        }
                        ObjectProperty::Method(key, function, kind) => {
                            self.property(key)?;
                            self.emit(Op::Function(function.clone()), s)?;
                            self.emit(Op::ObjectMethod(*kind), s)?;
                        }
                        ObjectProperty::Spread(value) => {
                            self.expr(value)?;
                            self.emit(Op::ObjectSpread, s)?;
                        }
                    }
                }
            }
            ExprKind::Template(parts) => {
                self.emit(Op::Text, s)?;
                for part in parts {
                    match part {
                        TemplatePart::Text(value) => {
                            self.emit(Op::Literal(Literal::String(value.clone())), s)?;
                        }
                        TemplatePart::Expression(value) => self.expr(value)?,
                    }
                    self.emit(Op::TextPart, s)?;
                }
                self.emit(Op::TextFinish, s)?;
            }
            ExprKind::Member(..) => {
                self.reference(expr)?;
                self.emit(Op::ReadReference(false), s)?;
            }
            ExprKind::Call(callee, args) => {
                if let ExprKind::Member(..) = callee.kind {
                    self.reference(callee)?;
                    self.emit(Op::PrepareMemberCall, s)?;
                } else {
                    self.expr(callee)?;
                    self.emit(
                        Op::PrepareCall {
                            direct_eval: matches!(&callee.kind,ExprKind::Name(n)if n=="eval"),
                            super_call: matches!(&callee.kind,ExprKind::Name(n)if n=="super"),
                        },
                        s,
                    )?;
                }
                self.args(args, s)?;
                self.emit(Op::Call, s)?;
            }
            ExprKind::New(callee, args) => {
                self.expr(callee)?;
                self.args(args, s)?;
                self.emit(Op::New, s)?;
            }
            ExprKind::Sequence(values) => {
                for (i, value) in values.iter().enumerate() {
                    self.expr(value)?;
                    if i + 1 < values.len() {
                        self.emit(Op::Pop, s)?;
                    }
                }
            }
            ExprKind::Conditional(test, yes, no) => {
                self.expr(test)?;
                let branch = self.emit(Op::JumpFalse(0), s)?;
                self.expr(yes)?;
                let end = self.emit(Op::Jump(0), s)?;
                self.patch(branch, self.code.len());
                self.expr(no)?;
                self.patch(end, self.code.len());
            }
            ExprKind::Unary(op, value) => {
                if op == "typeof" {
                    if let ExprKind::Name(name) = &value.kind {
                        self.emit(Op::TypeofName(name.clone()), s)?;
                    } else {
                        self.expr(value)?;
                        self.emit(Op::Unary(op.clone()), s)?;
                    }
                } else if op == "delete" {
                    if matches!(value.kind, ExprKind::Member(..)) {
                        self.reference(value)?;
                        self.emit(Op::DeleteReference, s)?;
                    } else {
                        self.emit(Op::DeleteName, s)?;
                    }
                } else {
                    self.expr(value)?;
                    self.emit(Op::Unary(op.clone()), s)?;
                }
            }
            ExprKind::Update(op, value, prefix) => {
                self.reference(value)?;
                self.emit(Op::Update(op.clone(), *prefix), s)?;
            }
            ExprKind::Binary(op, left, right) => {
                self.expr(left)?;
                if matches!(op.as_str(), "&&" | "||" | "??") {
                    let end = self.emit(Op::Logical(op.clone(), 0), s)?;
                    self.expr(right)?;
                    self.patch(end, self.code.len());
                } else {
                    self.expr(right)?;
                    self.emit(Op::Binary(op.clone()), s)?;
                }
            }
            ExprKind::Assign(op, left, right) => {
                if matches!(left.kind, ExprKind::Array(_) | ExprKind::Object(_)) {
                    self.expr(right)?;
                    self.emit(Op::AssignTarget(left.clone()), s)?;
                } else {
                    self.reference(left)?;
                    if op != "=" {
                        self.emit(Op::ReadReference(true), s)?;
                    }
                    self.expr(right)?;
                    if op != "=" {
                        self.emit(Op::Binary(op.trim_end_matches('=').into()), s)?;
                    }
                    self.emit(Op::WriteReference, s)?;
                }
            }
            ExprKind::Await(value) => {
                self.expr(value)?;
                self.emit(Op::Await, s)?;
            }
            ExprKind::Yield(value, delegate) => {
                if let Some(value) = value {
                    self.expr(value)?;
                } else {
                    self.emit(Op::Literal(Literal::Undefined), s)?;
                }
                self.emit(Op::Yield(*delegate), s)?;
            }
        }
        Ok(())
    }
}
pub fn compile(program: &Program) -> Result<Arc<Chunk>, JsError> {
    compile_with_mode(&program.body, false, program.strict, program.module)
}
pub(crate) fn compile_body(body: &[Stmt], function: bool) -> Result<Arc<Chunk>, JsError> {
    compile_with_mode(body, function, crate::ast::strict(body), false)
}
fn compile_with_mode(
    body: &[Stmt],
    function: bool,
    strict: bool,
    module: bool,
) -> Result<Arc<Chunk>, JsError> {
    let mut compiler = Compiler {
        code: Vec::new(),
        depth: 1,
        controls: Vec::new(),
        bytes: 0,
    };
    let plan = ScopePlan::body(body);
    compiler.bytes += plan.bytes();
    compiler.statements(body)?;
    compiler.emit(Op::End, Span::default())?;
    Ok(Arc::new(Chunk {
        strict,
        module,
        code: compiler.code,
        plan,
        function,
        bytes: compiler.bytes,
    }))
}
pub(crate) fn compile_expression(expr: &Expr) -> Result<Arc<Chunk>, JsError> {
    let mut compiler = Compiler {
        code: Vec::new(),
        depth: 1,
        controls: Vec::new(),
        bytes: 0,
    };
    compiler.expr(expr)?;
    compiler.emit(Op::Last, expr.span)?;
    compiler.emit(Op::End, expr.span)?;
    Ok(Arc::new(Chunk {
        strict: false,
        module: false,
        code: compiler.code,
        plan: ScopePlan::default(),
        function: false,
        bytes: compiler.bytes,
    }))
}
