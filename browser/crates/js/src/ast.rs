//! Owned AST; functions share their parsed body across closures/calls.
use crate::Span;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Program {
    pub body: Vec<Stmt>,
    pub strict: bool,
}
#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub enum Literal {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    RegExp(String, String),
}
#[derive(Debug, Clone)]
pub enum ExprKind {
    Literal(Literal),
    Name(String),
    This,
    Array(Vec<Option<Argument>>),
    Object(Vec<ObjectProperty>),
    Function(Arc<Function>),
    Class(Box<Class>),
    Template(Vec<TemplatePart>),
    Unary(String, Box<Expr>),
    Update(String, Box<Expr>, bool),
    Binary(String, Box<Expr>, Box<Expr>),
    Assign(String, Box<Expr>, Box<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Sequence(Vec<Expr>),
    Member(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Argument>),
    New(Box<Expr>, Vec<Argument>),
}
#[derive(Debug, Clone)]
pub struct Argument {
    pub expr: Expr,
    pub spread: bool,
}
#[derive(Debug, Clone)]
pub enum TemplatePart {
    Text(String),
    Expression(Expr),
}
#[derive(Debug, Clone)]
pub enum ObjectProperty {
    Value(PropertyName, Expr),
    Method(PropertyName, Arc<Function>, MethodKind),
    Spread(Expr),
}
#[derive(Debug, Clone)]
pub enum PropertyName {
    String(String),
    Computed(Expr),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodKind {
    Method,
    Get,
    Set,
}
#[derive(Debug, Clone)]
pub enum Pattern {
    Name(String),
    Array(Vec<Option<Pattern>>, Option<Box<Pattern>>),
    Object(Vec<(PropertyName, Pattern)>, Option<Box<Pattern>>),
    Default(Box<Pattern>, Box<Expr>),
    Rest(Box<Pattern>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Var,
    Let,
    Const,
}
#[derive(Debug, Clone)]
pub struct Declaration {
    pub pattern: Pattern,
    pub init: Option<Expr>,
}
#[derive(Debug, Clone)]
pub struct Function {
    pub name: Option<String>,
    pub params: Vec<Pattern>,
    pub body: Vec<Stmt>,
    pub arrow: bool,
    pub strict: bool,
    pub span: Span,
    /// Conservative retained AST charge, computed once by the bounded parser.
    pub retained_bytes: usize,
}
#[derive(Debug, Clone)]
pub struct Class {
    pub name: Option<String>,
    pub extends: Option<Expr>,
    pub methods: Vec<ClassMethod>,
}
#[derive(Debug, Clone)]
pub struct ClassMethod {
    pub name: PropertyName,
    pub function: Arc<Function>,
    pub kind: MethodKind,
    pub is_static: bool,
}
#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub enum StmtKind {
    Empty,
    Expr(Expr),
    Block(Vec<Stmt>),
    Declare(DeclKind, Vec<Declaration>),
    Function(String, Arc<Function>),
    Class(String, Class),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    ForEach(ForBinding, Expr, Box<Stmt>, bool),
    Return(Option<Expr>),
    Throw(Expr),
    Break(Option<String>),
    Continue(Option<String>),
    Try(Box<Stmt>, Option<(Pattern, Box<Stmt>)>, Option<Box<Stmt>>),
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    Label(String, Box<Stmt>),
}
#[derive(Debug, Clone)]
pub enum ForBinding {
    Declare(DeclKind, Pattern),
    Target(Expr),
}
pub(crate) fn strict(body: &[Stmt]) -> bool {
    body.iter().take_while(|s| matches!(&s.kind, StmtKind::Expr(Expr {kind: ExprKind::Literal(Literal::String(_)),..}))).any(|s| matches!(&s.kind, StmtKind::Expr(Expr { kind: ExprKind::Literal(Literal::String(s)), .. }) if s == "use strict"))
}
