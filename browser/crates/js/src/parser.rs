//! Recursive-descent statements/patterns and precedence-climbing expressions.
use crate::ast::*;
use crate::lexer::{Kind, Lexer, Token};
use crate::{ParseError, Span};
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub struct ParseOpts {
    pub max_source_bytes: usize,
    pub max_tokens: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
}
impl Default for ParseOpts {
    fn default() -> Self {
        Self {
            max_source_bytes: 8 * 1024 * 1024,
            max_tokens: 500_000,
            max_nodes: 100_000,
            max_depth: 128,
        }
    }
}
pub fn parse(src: &str, opts: ParseOpts) -> Result<Program, ParseError> {
    parse_source(src, opts, false)
}
pub fn parse_module(src: &str, opts: ParseOpts) -> Result<Program, ParseError> {
    parse_source(src, opts, true)
}
fn parse_source(src: &str, opts: ParseOpts, module: bool) -> Result<Program, ParseError> {
    let start = Span {
        offset: 0,
        line: 1,
        column: 1,
    };
    if src.len() > opts.max_source_bytes {
        return Err(ParseError::at(start, "source limit exceeded"));
    }
    let mut lexer = Lexer::new(src);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next()?;
        let eof = token.kind == Kind::Eof;
        tokens.push(token);
        if tokens.len() > opts.max_tokens {
            return Err(ParseError::at(start, "token limit exceeded"));
        }
        if eof {
            break;
        }
    }
    let mut p = Parser {
        tokens,
        pos: 0,
        opts,
        nodes: 0,
        depth: 0,
        function: 0,
        loops: 0,
        allow_in: true,
        asynchronous: false,
        generator: false,
        module,
        imports: Vec::new(),
        exports: Vec::new(),
        requests: Vec::new(),
    };
    let mut body = Vec::new();
    while !p.eof() {
        body.push(p.stmt()?);
    }
    Ok(Program {
        strict: module || strict(&body),
        module,
        imports: p.imports,
        exports: p.exports,
        requests: p.requests,
        body,
    })
}
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    opts: ParseOpts,
    nodes: usize,
    depth: usize,
    function: usize,
    loops: usize,
    allow_in: bool,
    asynchronous: bool,
    generator: bool,
    module: bool,
    imports: Vec<ImportEntry>,
    exports: Vec<ExportEntry>,
    requests: Vec<String>,
}
impl Parser {
    fn token(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }
    fn span(&self) -> Span {
        self.token().span
    }
    fn eof(&self) -> bool {
        self.token().kind == Kind::Eof
    }
    fn is(&self, s: &str) -> bool {
        matches!(&self.token().kind,Kind::Name(v)|Kind::Punct(v) if v==s)
    }
    fn at(&self, n: usize, s: &str) -> bool {
        matches!(self.tokens.get(self.pos+n).map(|t|&t.kind),Some(Kind::Name(v)|Kind::Punct(v)) if v==s)
    }
    fn take(&mut self, s: &str) -> bool {
        if self.is(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn require(&mut self, s: &str) -> Result<(), ParseError> {
        if self.take(s) {
            Ok(())
        } else {
            Err(self.err(format!("expected '{s}'")))
        }
    }
    fn err(&self, s: impl Into<String>) -> ParseError {
        ParseError::at(self.span(), s)
    }
    fn name(&mut self) -> Result<String, ParseError> {
        match self.token().kind.clone() {
            Kind::Name(s) => {
                self.pos += 1;
                Ok(s)
            }
            _ => Err(self.err("expected identifier")),
        }
    }
    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        self.nodes += 1;
        if self.depth > self.opts.max_depth || self.nodes > self.opts.max_nodes {
            Err(self.err("AST budget exceeded"))
        } else {
            Ok(())
        }
    }
    fn semi(&mut self) -> Result<(), ParseError> {
        if self.take(";") || self.eof() || self.is("}") || self.token().newline {
            Ok(())
        } else {
            Err(self.err("expected semicolon or line break"))
        }
    }
    fn block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.require("{")?;
        let mut body = Vec::new();
        while !self.take("}") {
            if self.eof() {
                return Err(self.err("unterminated block"));
            }
            body.push(self.stmt()?);
        }
        Ok(body)
    }
    fn stmt(&mut self) -> Result<Stmt, ParseError> {
        self.enter()?;
        let span = self.span();
        let kind = if self.is("import") && !self.at(1, "(") && !self.at(1, ".") {
            if !self.module || self.function != 0 || self.depth != 1 {
                return Err(self.err("import declaration outside module top level"));
            }
            self.pos += 1;
            self.import_declaration()?;
            StmtKind::Empty
        } else if self.take("export") {
            if !self.module || self.function != 0 || self.depth != 1 {
                return Err(self.err("export outside module top level"));
            }
            let kind = self.export_declaration(span)?;
            self.depth -= 1;
            return Ok(Stmt { kind, span });
        } else if self.take(";") {
            StmtKind::Empty
        } else if self.is("{") {
            StmtKind::Block(self.block()?)
        } else if self.is("var") || self.is("let") || self.is("const") {
            let kind = self.decl_kind();
            let ds = self.declarations(kind, true)?;
            self.semi()?;
            StmtKind::Declare(kind, ds)
        } else if self.is("async")
            && self.at(1, "function")
            && !self.tokens.get(self.pos + 1).is_some_and(|t| t.newline)
        {
            self.pos += 2;
            let generator = self.take("*");
            let name = self.name()?;
            let f = self.function_mode(Some(name.clone()), false, span, true, generator)?;
            StmtKind::Function(name, f)
        } else if self.take("function") {
            let generator = self.take("*");
            let name = self.name()?;
            let f = self.function_mode(Some(name.clone()), false, span, false, generator)?;
            StmtKind::Function(name, f)
        } else if self.take("class") {
            let name = self.name()?;
            let class = self.class(Some(name.clone()))?;
            StmtKind::Class(name, class)
        } else if self.take("if") {
            self.require("(")?;
            let test = self.expression()?;
            self.require(")")?;
            let yes = Box::new(self.stmt()?);
            let no = if self.take("else") {
                Some(Box::new(self.stmt()?))
            } else {
                None
            };
            StmtKind::If(test, yes, no)
        } else if self.take("while") {
            self.require("(")?;
            let test = self.expression()?;
            self.require(")")?;
            self.loops += 1;
            let body = Box::new(self.stmt()?);
            self.loops -= 1;
            StmtKind::While(test, body)
        } else if self.take("do") {
            self.loops += 1;
            let body = Box::new(self.stmt()?);
            self.loops -= 1;
            self.require("while")?;
            self.require("(")?;
            let test = self.expression()?;
            self.require(")")?;
            self.take(";");
            StmtKind::DoWhile(body, test)
        } else if self.take("for") {
            self.for_stmt()?
        } else if self.take("return") {
            if self.function == 0 {
                return Err(self.err("return outside function"));
            }
            let value = if self.eof() || self.is(";") || self.is("}") || self.token().newline {
                None
            } else {
                Some(self.expression()?)
            };
            self.semi()?;
            StmtKind::Return(value)
        } else if self.take("throw") {
            if self.token().newline {
                return Err(self.err("newline after throw"));
            }
            let value = self.expression()?;
            self.semi()?;
            StmtKind::Throw(value)
        } else if self.is("break") || self.is("continue") {
            let is_continue = self.take("continue");
            if !is_continue {
                self.require("break")?;
            }
            let label = if !self.token().newline && matches!(self.token().kind, Kind::Name(_)) {
                Some(self.name()?)
            } else {
                None
            };
            if self.loops == 0 && label.is_none() {
                return Err(self.err("loop control outside loop"));
            }
            self.semi()?;
            if is_continue {
                StmtKind::Continue(label)
            } else {
                StmtKind::Break(label)
            }
        } else if self.take("try") {
            let body = Box::new(self.stmt()?);
            let catch = if self.take("catch") {
                self.require("(")?;
                let pat = self.pattern(false)?;
                self.require(")")?;
                Some((pat, Box::new(self.stmt()?)))
            } else {
                None
            };
            let finally = if self.take("finally") {
                Some(Box::new(self.stmt()?))
            } else {
                None
            };
            if catch.is_none() && finally.is_none() {
                return Err(self.err("try requires catch/finally"));
            }
            StmtKind::Try(body, catch, finally)
        } else if self.take("switch") {
            self.require("(")?;
            let value = self.expression()?;
            self.require(")")?;
            self.require("{")?;
            let mut cases = Vec::new();
            let mut default = false;
            self.loops += 1;
            while !self.take("}") {
                let test = if self.take("case") {
                    Some(self.expression()?)
                } else if self.take("default") {
                    if default {
                        return Err(self.err("duplicate default"));
                    }
                    default = true;
                    None
                } else {
                    return Err(self.err("expected case/default"));
                };
                self.require(":")?;
                let mut body = Vec::new();
                while !self.is("case") && !self.is("default") && !self.is("}") {
                    body.push(self.stmt()?);
                }
                cases.push((test, body));
            }
            self.loops -= 1;
            StmtKind::Switch(value, cases)
        } else if matches!(self.token().kind, Kind::Name(_)) && self.at(1, ":") {
            let label = self.name()?;
            self.require(":")?;
            StmtKind::Label(label, Box::new(self.stmt()?))
        } else {
            let expr = self.expression()?;
            self.semi()?;
            StmtKind::Expr(expr)
        };
        self.depth -= 1;
        Ok(Stmt { kind, span })
    }
    fn module_string(&mut self) -> Result<String, ParseError> {
        match self.token().kind.clone() {
            Kind::String(value) => {
                self.pos += 1;
                Ok(value)
            }
            _ => Err(self.err("module specifier must be a string")),
        }
    }
    fn module_name(&mut self) -> Result<String, ParseError> {
        if let Kind::String(value) = self.token().kind.clone() {
            self.pos += 1;
            Ok(value)
        } else {
            self.name()
        }
    }
    fn import_declaration(&mut self) -> Result<(), ParseError> {
        if matches!(self.token().kind, Kind::String(_)) {
            let source = self.module_string()?;
            self.requests.push(source);
            self.semi()?;
            return Ok(());
        }
        let mut bindings = Vec::new();
        if !self.is("{") && !self.is("*") {
            bindings.push(("default".into(), self.name()?, false));
            if !self.take(",") {
                self.require("from")?;
                let source = self.module_string()?;
                self.requests.push(source.clone());
                for (imported, local, namespace) in bindings {
                    self.imports.push(ImportEntry {
                        source: source.clone(),
                        imported,
                        local,
                        namespace,
                    });
                }
                self.semi()?;
                return Ok(());
            }
        }
        if self.take("*") {
            self.require("as")?;
            bindings.push((String::new(), self.name()?, true));
        } else {
            self.require("{")?;
            while !self.take("}") {
                let imported = self.module_name()?;
                let local = if self.take("as") {
                    self.name()?
                } else {
                    imported.clone()
                };
                bindings.push((imported, local, false));
                if !self.take(",") {
                    self.require("}")?;
                    break;
                }
            }
        }
        self.require("from")?;
        let source = self.module_string()?;
        self.requests.push(source.clone());
        for (imported, local, namespace) in bindings {
            self.imports.push(ImportEntry {
                source: source.clone(),
                imported,
                local,
                namespace,
            });
        }
        self.semi()?;
        Ok(())
    }
    fn add_export(&mut self, export: ExportEntry) -> Result<(), ParseError> {
        let name = match &export {
            ExportEntry::Local { exported, .. } | ExportEntry::Indirect { exported, .. } => {
                Some(exported)
            }
            _ => None,
        };
        if let Some(name) = name {
            if self.exports.iter().any(|e| match e {
                ExportEntry::Local { exported, .. } | ExportEntry::Indirect { exported, .. } => {
                    exported == name
                }
                _ => false,
            }) {
                return Err(self.err("duplicate export name"));
            }
        }
        self.exports.push(export);
        Ok(())
    }
    fn export_declaration(&mut self, span: Span) -> Result<StmtKind, ParseError> {
        if self.take("*") {
            let exported = if self.take("as") {
                Some(self.module_name()?)
            } else {
                None
            };
            self.require("from")?;
            let source = self.module_string()?;
            self.requests.push(source.clone());
            if let Some(exported) = exported {
                self.add_export(ExportEntry::Indirect {
                    exported,
                    source,
                    imported: "*namespace*".into(),
                })?;
            } else {
                self.add_export(ExportEntry::Star { source })?;
            }
            self.semi()?;
            return Ok(StmtKind::Empty);
        }
        if self.take("{") {
            let mut entries = Vec::new();
            while !self.take("}") {
                let local = self.module_name()?;
                let exported = if self.take("as") {
                    self.module_name()?
                } else {
                    local.clone()
                };
                entries.push((local, exported));
                if !self.take(",") {
                    self.require("}")?;
                    break;
                }
            }
            let source = if self.take("from") {
                let source = self.module_string()?;
                self.requests.push(source.clone());
                Some(source)
            } else {
                None
            };
            for (local, exported) in entries {
                self.add_export(if let Some(source) = &source {
                    ExportEntry::Indirect {
                        exported,
                        source: source.clone(),
                        imported: local,
                    }
                } else {
                    ExportEntry::Local { exported, local }
                })?;
            }
            self.semi()?;
            return Ok(StmtKind::Empty);
        }
        if self.take("default") {
            let value = self.assign()?;
            let local = match &value.kind {
                ExprKind::Function(f) => f.name.clone(),
                ExprKind::Class(c) => c.name.clone(),
                _ => None,
            }
            .unwrap_or_else(|| "\0default".into());
            self.add_export(ExportEntry::Local {
                exported: "default".into(),
                local: local.clone(),
            })?;
            let declaration = match value.kind {
                ExprKind::Function(f) => StmtKind::Function(local, f),
                ExprKind::Class(c) => StmtKind::Class(local, *c),
                kind => {
                    self.semi()?;
                    StmtKind::Declare(
                        DeclKind::Const,
                        vec![Declaration {
                            pattern: Pattern::Name(local),
                            init: Some(Expr { kind, span }),
                        }],
                    )
                }
            };
            return Ok(declaration);
        }
        let declaration = self.stmt()?;
        let mut names = Vec::new();
        fn collect(pattern: &Pattern, names: &mut Vec<String>) {
            match pattern {
                Pattern::Name(name) => names.push(name.clone()),
                Pattern::Rest(p) | Pattern::Default(p, _) => collect(p, names),
                Pattern::Array(parts, rest) => {
                    for p in parts.iter().flatten() {
                        collect(p, names);
                    }
                    if let Some(p) = rest {
                        collect(p, names);
                    }
                }
                Pattern::Object(parts, rest) => {
                    for (_, p) in parts {
                        collect(p, names);
                    }
                    if let Some(p) = rest {
                        collect(p, names);
                    }
                }
            }
        }
        match &declaration.kind {
            StmtKind::Declare(_, ds) => {
                for d in ds {
                    collect(&d.pattern, &mut names);
                }
            }
            StmtKind::Function(name, _) | StmtKind::Class(name, _) => names.push(name.clone()),
            _ => return Err(self.err("invalid export declaration")),
        }
        for name in names {
            self.add_export(ExportEntry::Local {
                exported: name.clone(),
                local: name,
            })?;
        }
        Ok(declaration.kind)
    }
    fn decl_kind(&mut self) -> DeclKind {
        if self.take("let") {
            DeclKind::Let
        } else if self.take("const") {
            DeclKind::Const
        } else {
            self.take("var");
            DeclKind::Var
        }
    }
    fn declarations(
        &mut self,
        kind: DeclKind,
        require_const: bool,
    ) -> Result<Vec<Declaration>, ParseError> {
        let mut out = Vec::new();
        loop {
            let pattern = self.pattern(false)?;
            let init = if self.take("=") {
                Some(self.assign()?)
            } else {
                None
            };
            if kind == DeclKind::Const && require_const && init.is_none() {
                return Err(self.err("const requires initializer"));
            }
            out.push(Declaration { pattern, init });
            if !self.take(",") {
                break;
            }
        }
        Ok(out)
    }
    fn for_stmt(&mut self) -> Result<StmtKind, ParseError> {
        let asynchronous = self.take("await");
        if asynchronous && !self.asynchronous {
            return Err(self.err("for await outside an async function or module"));
        }
        self.require("(")?;
        let span = self.span();
        self.allow_in = false;
        let init = if self.is(";") {
            None
        } else if self.is("var") || self.is("let") || self.is("const") {
            let kind = self.decl_kind();
            Some(Stmt {
                kind: StmtKind::Declare(kind, self.declarations(kind, false)?),
                span,
            })
        } else {
            Some(Stmt {
                kind: StmtKind::Expr(self.expression()?),
                span,
            })
        };
        self.allow_in = true;
        if self.is("in") || self.is("of") {
            let of = self.take("of");
            if !of {
                if asynchronous {
                    return Err(self.err("for await requires of"));
                }
                self.require("in")?;
            }
            let target = match init.map(|s| s.kind) {
                Some(StmtKind::Declare(k, mut ds)) if ds.len() == 1 => {
                    ForBinding::Declare(k, ds.remove(0).pattern)
                }
                Some(StmtKind::Expr(e)) => ForBinding::Target(e),
                _ => return Err(self.err("invalid iteration binding")),
            };
            let expr = self.expression()?;
            self.require(")")?;
            self.loops += 1;
            let body = Box::new(self.stmt()?);
            self.loops -= 1;
            return Ok(StmtKind::ForEach(target, expr, body, of, asynchronous));
        }
        if asynchronous {
            return Err(self.err("for await requires of"));
        }
        if let Some(Stmt {
            kind: StmtKind::Declare(DeclKind::Const, ds),
            ..
        }) = &init
        {
            if ds.iter().any(|d| d.init.is_none()) {
                return Err(self.err("const requires initializer"));
            }
        }
        self.require(";")?;
        let test = if self.is(";") {
            None
        } else {
            Some(self.expression()?)
        };
        self.require(";")?;
        let update = if self.is(")") {
            None
        } else {
            Some(self.expression()?)
        };
        self.require(")")?;
        self.loops += 1;
        let body = Box::new(self.stmt()?);
        self.loops -= 1;
        Ok(StmtKind::For(init.map(Box::new), test, update, body))
    }
    fn pattern(&mut self, default: bool) -> Result<Pattern, ParseError> {
        self.enter()?;
        let pattern = if self.take("...") {
            Pattern::Rest(Box::new(self.pattern(false)?))
        } else if self.take("[") {
            let mut parts = Vec::new();
            let mut rest = None;
            while !self.take("]") {
                if self.take(",") {
                    parts.push(None);
                    continue;
                }
                if self.take("...") {
                    rest = Some(Box::new(self.pattern(false)?));
                    self.require("]")?;
                    break;
                }
                parts.push(Some(self.pattern(true)?));
                if !self.take(",") {
                    self.require("]")?;
                    break;
                }
            }
            Pattern::Array(parts, rest)
        } else if self.take("{") {
            let mut parts = Vec::new();
            let mut rest = None;
            while !self.take("}") {
                if self.take("...") {
                    rest = Some(Box::new(self.pattern(false)?));
                    self.require("}")?;
                    break;
                }
                let key = self.property_name()?;
                let value = if self.take(":") {
                    self.pattern(true)?
                } else {
                    let PropertyName::String(s) = &key else {
                        return Err(self.err("computed pattern requires colon"));
                    };
                    let mut pat = Pattern::Name(s.clone());
                    if self.take("=") {
                        pat = Pattern::Default(Box::new(pat), Box::new(self.assign()?));
                    }
                    pat
                };
                parts.push((key, value));
                if !self.take(",") {
                    self.require("}")?;
                    break;
                }
            }
            Pattern::Object(parts, rest)
        } else {
            Pattern::Name(self.name()?)
        };
        let pattern = if default && self.take("=") {
            Pattern::Default(Box::new(pattern), Box::new(self.assign()?))
        } else {
            pattern
        };
        self.depth -= 1;
        Ok(pattern)
    }
    fn function_mode(
        &mut self,
        name: Option<String>,
        arrow: bool,
        span: Span,
        asynchronous: bool,
        generator: bool,
    ) -> Result<Arc<Function>, ParseError> {
        let parent_context = (self.asynchronous, self.generator);
        self.asynchronous = false;
        self.generator = false;
        let nodes_before = self.nodes;
        self.require("(")?;
        let mut params = Vec::new();
        while !self.take(")") {
            let pat = self.pattern(true)?;
            let rest = matches!(pat, Pattern::Rest(_));
            params.push(pat);
            if rest {
                self.require(")")?;
                break;
            }
            if !self.take(",") {
                self.require(")")?;
                break;
            }
        }
        self.function += 1;
        self.asynchronous = asynchronous;
        self.generator = generator;
        let old_loop = self.loops;
        self.loops = 0;
        let body = self.block()?;
        self.loops = old_loop;
        self.function -= 1;
        (self.asynchronous, self.generator) = parent_context;
        let strict = strict(&body);
        Ok(Arc::new(Function {
            asynchronous,
            generator,
            name,
            params,
            body,
            arrow,
            strict,
            span,
            retained_bytes: (self.nodes - nodes_before)
                .saturating_mul(256)
                .saturating_add(
                    self.span()
                        .offset
                        .saturating_sub(span.offset)
                        .saturating_mul(4),
                ),
        }))
    }
    fn class(&mut self, name: Option<String>) -> Result<Class, ParseError> {
        let extends = if self.take("extends") {
            Some(self.lhs(false)?)
        } else {
            None
        };
        self.require("{")?;
        let mut methods = Vec::new();
        while !self.take("}") {
            if self.take(";") {
                continue;
            }
            let span = self.span();
            let is_static = self.is("static") && !self.at(1, "(");
            if is_static {
                self.pos += 1;
            }
            let asynchronous = self.is("async")
                && !self.at(1, "(")
                && !self.tokens.get(self.pos + 1).is_some_and(|t| t.newline);
            if asynchronous {
                self.pos += 1;
            }
            let generator = self.take("*");
            let mut name = self.property_name()?;
            let mut kind = MethodKind::Method;
            if matches!(&name,PropertyName::String(s) if s=="get"||s=="set") && !self.is("(") {
                kind = if matches!(&name,PropertyName::String(s) if s=="get") {
                    MethodKind::Get
                } else {
                    MethodKind::Set
                };
                name = self.property_name()?;
            }
            let mut f = (*self.function_mode(None, false, span, asynchronous, generator)?).clone();
            f.strict = true;
            methods.push(ClassMethod {
                name,
                function: Arc::new(f),
                kind,
                is_static,
            });
        }
        Ok(Class {
            name,
            extends,
            methods,
        })
    }
    fn property_name(&mut self) -> Result<PropertyName, ParseError> {
        if self.take("[") {
            let expr = self.expression()?;
            self.require("]")?;
            return Ok(PropertyName::Computed(expr));
        }
        let key = match self.token().kind.clone() {
            Kind::Name(s) | Kind::String(s) => s,
            Kind::Number(n) => n.to_string(),
            _ => return Err(self.err("invalid property name")),
        };
        self.pos += 1;
        Ok(PropertyName::String(key))
    }
    fn expression(&mut self) -> Result<Expr, ParseError> {
        let first = self.assign()?;
        if !self.take(",") {
            return Ok(first);
        }
        let span = first.span;
        let mut list = vec![first];
        loop {
            list.push(self.assign()?);
            if !self.take(",") {
                break;
            }
        }
        Ok(Expr {
            kind: ExprKind::Sequence(list),
            span,
        })
    }
    fn assign(&mut self) -> Result<Expr, ParseError> {
        self.enter()?;
        if self.generator && self.take("yield") {
            let span = self.tokens[self.pos - 1].span;
            let empty = self.token().newline
                || self.eof()
                || self.is(";")
                || self.is("}")
                || self.is(")")
                || self.is("]")
                || self.is(",");
            let delegate = !empty && self.take("*");
            let value = if empty {
                None
            } else {
                Some(Box::new(self.assign()?))
            };
            self.depth -= 1;
            return Ok(Expr {
                kind: ExprKind::Yield(value, delegate),
                span,
            });
        }
        if self.is("async") && !self.tokens.get(self.pos + 1).is_some_and(|t| t.newline) {
            let single = matches!(
                self.tokens.get(self.pos + 1).map(|t| &t.kind),
                Some(Kind::Name(_))
            ) && self.at(2, "=>");
            let parenthesized = if self.at(1, "(") {
                let mut cursor = self.pos + 1;
                let mut depth = 0;
                let mut found = false;
                while cursor < self.tokens.len() {
                    if matches!(&self.tokens[cursor].kind,Kind::Punct(s)if s=="(") {
                        depth += 1;
                    }
                    if matches!(&self.tokens[cursor].kind,Kind::Punct(s)if s==")") {
                        depth -= 1;
                        if depth == 0 {
                            found = matches!(self.tokens.get(cursor+1).map(|t|&t.kind),Some(Kind::Punct(s))if s=="=>");
                            break;
                        }
                    }
                    cursor += 1;
                }
                found
            } else {
                false
            };
            if single || parenthesized {
                let span = self.span();
                self.pos += 1;
                let mut params = Vec::new();
                if parenthesized {
                    self.require("(")?;
                    while !self.take(")") {
                        params.push(self.pattern(true)?);
                        if !self.take(",") {
                            self.require(")")?;
                            break;
                        }
                    }
                } else {
                    params.push(Pattern::Name(self.name()?));
                }
                self.require("=>")?;
                let result = self.arrow_mode(params, span, true);
                self.depth -= 1;
                return result;
            }
        }
        let left = self.binary(1)?;
        let span = left.span;
        if self.take("=>") {
            let params = match left.kind {
                ExprKind::Sequence(list) => list
                    .into_iter()
                    .map(to_pattern)
                    .collect::<Result<Vec<_>, _>>()?,
                other => vec![to_pattern(Expr { kind: other, span })?],
            };
            let expr = self.arrow(params, span)?;
            self.depth -= 1;
            return Ok(expr);
        }
        let left = if self.take("?") {
            let yes = self.assign()?;
            self.require(":")?;
            let no = self.assign()?;
            Expr {
                kind: ExprKind::Conditional(Box::new(left), Box::new(yes), Box::new(no)),
                span,
            }
        } else {
            left
        };
        let op = match &self.token().kind {
            Kind::Punct(s)
                if matches!(
                    s.as_str(),
                    "=" | "+="
                        | "-="
                        | "*="
                        | "/="
                        | "%="
                        | "&="
                        | "|="
                        | "^="
                        | "<<="
                        | ">>="
                        | ">>>="
                        | "**="
                ) =>
            {
                Some(s.clone())
            }
            _ => None,
        };
        let expr = if let Some(op) = op {
            self.pos += 1;
            let right = self.assign()?;
            Expr {
                kind: ExprKind::Assign(op, Box::new(left), Box::new(right)),
                span,
            }
        } else {
            left
        };
        self.depth -= 1;
        Ok(expr)
    }
    fn arrow(&mut self, params: Vec<Pattern>, span: Span) -> Result<Expr, ParseError> {
        self.arrow_mode(params, span, false)
    }
    fn arrow_mode(
        &mut self,
        params: Vec<Pattern>,
        span: Span,
        asynchronous: bool,
    ) -> Result<Expr, ParseError> {
        let parent_context = (self.asynchronous, self.generator);
        self.asynchronous = asynchronous;
        self.generator = false;
        let nodes_before = self.nodes;
        self.function += 1;
        let body = if self.is("{") {
            self.block()?
        } else {
            vec![Stmt {
                kind: StmtKind::Return(Some(self.assign()?)),
                span,
            }]
        };
        self.function -= 1;
        (self.asynchronous, self.generator) = parent_context;
        Ok(Expr {
            kind: ExprKind::Function(Arc::new(Function {
                asynchronous,
                generator: false,
                name: None,
                params,
                strict: strict(&body),
                body,
                arrow: true,
                span,
                retained_bytes: (self.nodes - nodes_before)
                    .saturating_mul(256)
                    .saturating_add(
                        self.span()
                            .offset
                            .saturating_sub(span.offset)
                            .saturating_mul(4),
                    ),
            })),
            span,
        })
    }
    fn binary(&mut self, min: u8) -> Result<Expr, ParseError> {
        self.enter()?;
        let mut left = self.unary()?;
        let mut chain = 0;
        while let Kind::Name(s) | Kind::Punct(s) = &self.token().kind {
            let op = s.clone();
            if op == "in" && !self.allow_in {
                break;
            }
            let prec = match op.as_str() {
                "||" | "??" => 1,
                "&&" => 2,
                "|" => 3,
                "^" => 4,
                "&" => 5,
                "==" | "!=" | "===" | "!==" => 6,
                "<" | ">" | "<=" | ">=" | "in" | "instanceof" => 7,
                "<<" | ">>" | ">>>" => 8,
                "+" | "-" => 9,
                "*" | "/" | "%" => 10,
                "**" => 11,
                _ => break,
            };
            if prec < min {
                break;
            }
            chain += 1;
            if chain > self.opts.max_depth {
                return Err(self.err("expression chain limit"));
            }
            self.pos += 1;
            let right = self.binary(if op == "**" { prec } else { prec + 1 })?;
            let span = left.span;
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
        self.depth -= 1;
        Ok(left)
    }
    fn unary(&mut self) -> Result<Expr, ParseError> {
        self.enter()?;
        let span = self.span();
        if (self.asynchronous || self.module && self.function == 0) && self.take("await") {
            let operand = self.unary()?;
            self.depth -= 1;
            return Ok(Expr {
                kind: ExprKind::Await(Box::new(operand)),
                span,
            });
        }
        let op = match &self.token().kind {
            Kind::Name(s) | Kind::Punct(s)
                if matches!(
                    s.as_str(),
                    "!" | "~" | "+" | "-" | "typeof" | "void" | "delete" | "++" | "--"
                ) =>
            {
                Some(s.clone())
            }
            _ => None,
        };
        let mut expr = if let Some(op) = op {
            self.pos += 1;
            let operand = self.unary()?;
            Expr {
                kind: if op == "++" || op == "--" {
                    ExprKind::Update(op, Box::new(operand), true)
                } else {
                    ExprKind::Unary(op, Box::new(operand))
                },
                span,
            }
        } else {
            self.lhs(true)?
        };
        if !self.token().newline && (self.is("++") || self.is("--")) {
            let op = if self.take("++") {
                "++"
            } else {
                self.require("--")?;
                "--"
            };
            expr = Expr {
                kind: ExprKind::Update(op.into(), Box::new(expr), false),
                span,
            };
        }
        self.depth -= 1;
        Ok(expr)
    }
    fn args(&mut self) -> Result<Vec<Argument>, ParseError> {
        self.require("(")?;
        let mut args = Vec::new();
        while !self.take(")") {
            let spread = self.take("...");
            let expr = self.assign()?;
            args.push(Argument { expr, spread });
            if !self.take(",") {
                self.require(")")?;
                break;
            }
        }
        Ok(args)
    }
    fn lhs(&mut self, calls: bool) -> Result<Expr, ParseError> {
        self.enter()?;
        let span = self.span();
        let mut expr = if self.take("new") {
            let callee = self.lhs(false)?;
            let args = if self.is("(") {
                self.args()?
            } else {
                Vec::new()
            };
            Expr {
                kind: ExprKind::New(Box::new(callee), args),
                span,
            }
        } else {
            self.primary()?
        };
        let mut chain = 0;
        loop {
            chain += 1;
            if chain > self.opts.max_depth {
                return Err(self.err("member chain limit"));
            }
            if self.take(".") {
                let key = self.name()?;
                expr = Expr {
                    kind: ExprKind::Member(
                        Box::new(expr),
                        Box::new(Expr {
                            kind: ExprKind::Literal(Literal::String(key)),
                            span: self.span(),
                        }),
                    ),
                    span,
                };
            } else if self.take("[") {
                let key = self.expression()?;
                self.require("]")?;
                expr = Expr {
                    kind: ExprKind::Member(Box::new(expr), Box::new(key)),
                    span,
                };
            } else if calls && self.is("(") {
                let args = self.args()?;
                expr = Expr {
                    kind: ExprKind::Call(Box::new(expr), args),
                    span,
                };
            } else {
                break;
            }
        }
        self.depth -= 1;
        Ok(expr)
    }
    fn primary(&mut self) -> Result<Expr, ParseError> {
        let span = self.span();
        let token = self.token().kind.clone();
        let kind = match token {
            Kind::Name(s) if s == "import" => {
                self.pos += 1;
                if self.take(".") {
                    if !self.module {
                        return Err(self.err("import.meta outside module"));
                    }
                    self.require("meta")?;
                    ExprKind::ImportMeta
                } else {
                    self.require("(")?;
                    let specifier = self.assign()?;
                    self.require(")")?;
                    ExprKind::Import(Box::new(specifier))
                }
            }
            Kind::BigInt(value) => {
                self.pos += 1;
                ExprKind::Literal(Literal::BigInt(value))
            }
            Kind::Number(n) => {
                self.pos += 1;
                ExprKind::Literal(Literal::Number(n))
            }
            Kind::String(s) => {
                self.pos += 1;
                ExprKind::Literal(Literal::String(s))
            }
            Kind::RegExp(p, f) => {
                self.pos += 1;
                ExprKind::Literal(Literal::RegExp(p, f))
            }
            Kind::Template(parts) => {
                self.pos += 1;
                let mut out = Vec::new();
                for (text, source) in parts {
                    out.push(TemplatePart::Text(text));
                    if let Some(source) = source {
                        let program = parse(
                            &format!("({source});"),
                            ParseOpts {
                                max_depth: self.opts.max_depth.saturating_sub(self.depth),
                                max_nodes: self.opts.max_nodes.saturating_sub(self.nodes),
                                ..self.opts
                            },
                        )?;
                        let Some(Stmt {
                            kind: StmtKind::Expr(expr),
                            ..
                        }) = program.body.into_iter().next()
                        else {
                            return Err(self.err("template expression"));
                        };
                        out.push(TemplatePart::Expression(expr));
                    }
                }
                ExprKind::Template(out)
            }
            Kind::Name(s) if s == "null" || s == "true" || s == "false" => {
                self.pos += 1;
                ExprKind::Literal(match s.as_str() {
                    "null" => Literal::Null,
                    "true" => Literal::Bool(true),
                    _ => Literal::Bool(false),
                })
            }
            Kind::Name(s) if s == "this" => {
                self.pos += 1;
                ExprKind::This
            }
            Kind::Name(s) if s == "function" => {
                self.pos += 1;
                let generator = self.take("*");
                let name = if matches!(self.token().kind, Kind::Name(_)) {
                    Some(self.name()?)
                } else {
                    None
                };
                ExprKind::Function(self.function_mode(name, false, span, false, generator)?)
            }
            Kind::Name(s)
                if s == "async"
                    && self.at(1, "function")
                    && !self.tokens.get(self.pos + 1).is_some_and(|t| t.newline) =>
            {
                self.pos += 2;
                let generator = self.take("*");
                let name = if matches!(self.token().kind, Kind::Name(_)) {
                    Some(self.name()?)
                } else {
                    None
                };
                ExprKind::Function(self.function_mode(name, false, span, true, generator)?)
            }
            Kind::Name(s) if s == "class" => {
                self.pos += 1;
                let name = if matches!(self.token().kind, Kind::Name(_)) {
                    Some(self.name()?)
                } else {
                    None
                };
                ExprKind::Class(Box::new(self.class(name)?))
            }
            Kind::Name(s) => {
                self.pos += 1;
                ExprKind::Name(s)
            }
            Kind::Punct(s) if s == "(" => {
                self.pos += 1;
                if self.take(")") {
                    self.require("=>")?;
                    return self.arrow(Vec::new(), span);
                }
                let saved = self.pos;
                let mut nesting = 1;
                let mut cursor = saved;
                let mut arrow = false;
                while cursor < self.tokens.len() {
                    match &self.tokens[cursor].kind {
                        Kind::Punct(s) if s == "(" => nesting += 1,
                        Kind::Punct(s) if s == ")" => {
                            nesting -= 1;
                            if nesting == 0 {
                                arrow = matches!(self.tokens.get(cursor+1).map(|t|&t.kind),Some(Kind::Punct(s))if s=="=>");
                                break;
                            }
                        }
                        _ => {}
                    }
                    cursor += 1;
                }
                if arrow {
                    let mut params = Vec::new();
                    loop {
                        params.push(self.pattern(true)?);
                        if !self.take(",") {
                            break;
                        }
                    }
                    self.require(")")?;
                    self.require("=>")?;
                    return self.arrow(params, span);
                }
                let expr = self.expression()?;
                self.require(")")?;
                return Ok(expr);
            }
            Kind::Punct(s) if s == "[" => {
                self.pos += 1;
                let mut values = Vec::new();
                while !self.take("]") {
                    if self.take(",") {
                        values.push(None);
                        continue;
                    }
                    let spread = self.take("...");
                    values.push(Some(Argument {
                        expr: self.assign()?,
                        spread,
                    }));
                    if !self.take(",") {
                        self.require("]")?;
                        break;
                    }
                }
                ExprKind::Array(values)
            }
            Kind::Punct(s) if s == "{" => {
                self.pos += 1;
                let mut props = Vec::new();
                while !self.take("}") {
                    if self.take("...") {
                        props.push(ObjectProperty::Spread(self.assign()?));
                    } else {
                        let asynchronous = self.is("async")
                            && !self.at(1, "(")
                            && !self.at(1, ":")
                            && !self.at(1, ",")
                            && !self.at(1, "}")
                            && !self.tokens.get(self.pos + 1).is_some_and(|t| t.newline);
                        if asynchronous {
                            self.pos += 1;
                        }
                        let generator = self.take("*");
                        let mut key = self.property_name()?;
                        let mut method = MethodKind::Method;
                        if matches!(&key,PropertyName::String(s) if s=="get"||s=="set")
                            && !self.is(":")
                            && !self.is(",")
                            && !self.is("}")
                            && !self.is("(")
                        {
                            method = if matches!(&key,PropertyName::String(s)if s=="get") {
                                MethodKind::Get
                            } else {
                                MethodKind::Set
                            };
                            key = self.property_name()?;
                        }
                        if self.take(":") {
                            props.push(ObjectProperty::Value(key, self.assign()?));
                        } else if self.is("(") {
                            let f =
                                self.function_mode(None, false, span, asynchronous, generator)?;
                            props.push(ObjectProperty::Method(key, f, method));
                        } else {
                            let PropertyName::String(name) = &key else {
                                return Err(self.err("computed shorthand"));
                            };
                            let mut expr = Expr {
                                kind: ExprKind::Name(name.clone()),
                                span,
                            };
                            if self.take("=") {
                                expr = Expr {
                                    kind: ExprKind::Assign(
                                        "=".into(),
                                        Box::new(expr),
                                        Box::new(self.assign()?),
                                    ),
                                    span,
                                };
                            }
                            props.push(ObjectProperty::Value(key, expr));
                        }
                    }
                    if !self.take(",") {
                        self.require("}")?;
                        break;
                    }
                }
                ExprKind::Object(props)
            }
            _ => return Err(self.err("expected expression")),
        };
        Ok(Expr { kind, span })
    }
}
fn to_pattern(expr: Expr) -> Result<Pattern, ParseError> {
    let span = expr.span;
    match expr.kind {
        ExprKind::Name(s) => Ok(Pattern::Name(s)),
        ExprKind::Assign(op, l, r) if op == "=" => {
            Ok(Pattern::Default(Box::new(to_pattern(*l)?), r))
        }
        ExprKind::Array(list) => {
            let mut values = Vec::new();
            let mut rest = None;
            for item in list {
                match item {
                    Some(Argument { expr, spread: true }) => {
                        rest = Some(Box::new(to_pattern(expr)?))
                    }
                    Some(Argument { expr, .. }) => values.push(Some(to_pattern(expr)?)),
                    None => values.push(None),
                }
            }
            Ok(Pattern::Array(values, rest))
        }
        ExprKind::Object(list) => {
            let mut values = Vec::new();
            let mut rest = None;
            for prop in list {
                match prop {
                    ObjectProperty::Value(k, v) => values.push((k, to_pattern(v)?)),
                    ObjectProperty::Spread(v) => rest = Some(Box::new(to_pattern(v)?)),
                    _ => return Err(ParseError::at(span, "invalid destructuring method")),
                }
            }
            Ok(Pattern::Object(values, rest))
        }
        _ => Err(ParseError::at(span, "invalid binding pattern")),
    }
}
