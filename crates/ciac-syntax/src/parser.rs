use crate::ast::*;
use crate::lexer::{lex, Token, TokenKind};
use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode, FileId};

/// Parses a CIaC source file into a [`Program`].
///
/// Errors are pushed into `diags`; the parser recovers at declaration
/// boundaries (`;` / `}` / next top-level keyword) so multiple problems are
/// reported in one run. The returned program contains every declaration
/// that parsed cleanly.
pub fn parse(src: &str, file: FileId, diags: &mut Diagnostics) -> Program {
    let tokens = lex(src, file, diags);
    Parser {
        src,
        tokens,
        pos: 0,
        diags,
        no_record_lit: false,
    }
    .program()
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    diags: &'a mut Diagnostics,
    /// Suppresses the `{ field: value }` postfix in `expr_postfix` while
    /// parsing an `if` condition or `match` scrutinee (v0.7), so `{`
    /// there opens a block/arm-list rather than a record literal — the
    /// same ambiguity Rust resolves the same way.
    no_record_lit: bool,
}

impl Parser<'_> {
    fn peek(&self) -> Token {
        self.tokens[self.pos]
    }

    fn bump(&mut self) -> Token {
        let tok = self.tokens[self.pos];
        if tok.kind != TokenKind::Eof {
            self.pos += 1;
        }
        tok
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.peek().kind == kind
    }

    fn eat(&mut self, kind: TokenKind) -> Option<Token> {
        self.at(kind).then(|| self.bump())
    }

    fn error_expected(&mut self, expected: &str) {
        let tok = self.peek();
        self.diags.push(
            Diagnostic::new(
                ErrorCode::UnexpectedToken,
                format!("expected {expected}, found {}", tok.kind.describe()),
            )
            .with_label(tok.span, format!("expected {expected} here")),
        );
    }

    fn expect(&mut self, kind: TokenKind) -> Option<Token> {
        if self.at(kind) {
            return Some(self.bump());
        }
        self.error_expected(kind.describe());
        None
    }

    fn expect_ident(&mut self) -> Option<Ident> {
        if self.at(TokenKind::Ident) {
            let tok = self.bump();
            return Some(self.ident_from(tok));
        }
        self.error_expected("a name");
        None
    }

    fn ident_from(&self, tok: Token) -> Ident {
        Ident {
            text: self.src[tok.span.range()].to_owned(),
            span: tok.span,
        }
    }

    /// Skips tokens until a likely declaration boundary, consuming the
    /// terminating `;`/`}` if present so parsing resumes cleanly after it.
    fn recover(&mut self) {
        loop {
            match self.peek().kind {
                TokenKind::Eof => return,
                TokenKind::Semi | TokenKind::RBrace => {
                    self.bump();
                    return;
                }
                TokenKind::Import
                | TokenKind::Blueprint
                | TokenKind::Expand
                | TokenKind::Project
                | TokenKind::Service
                | TokenKind::Use
                | TokenKind::Record
                | TokenKind::Error
                | TokenKind::Stream
                | TokenKind::Table
                | TokenKind::Handler
                | TokenKind::Extern
                | TokenKind::Api
                | TokenKind::Worker
                | TokenKind::Job
                | TokenKind::Channel
                | TokenKind::Pipeline
                | TokenKind::Crud
                | TokenKind::Events => return,
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn program(mut self) -> Program {
        let mut items = Vec::new();
        while !self.at(TokenKind::Eof) {
            match self.item() {
                Some(item) => items.push(item),
                None => self.recover(),
            }
        }
        Program { items }
    }

    fn item(&mut self) -> Option<Item> {
        match self.peek().kind {
            TokenKind::Import => self.import_decl(),
            TokenKind::Blueprint => self.blueprint_decl(),
            TokenKind::Expand => self.expand_stmt(),
            TokenKind::Project => self.project_decl(),
            TokenKind::Service => self.service_item(),
            TokenKind::Use => self.use_block(),
            TokenKind::Record => self.record_decl(RecordKind::Data),
            TokenKind::Error => self.record_decl(RecordKind::Error),
            TokenKind::Stream => self.stream_decl(),
            TokenKind::Table => self.table_decl(),
            TokenKind::Handler => self.handler_decl(),
            TokenKind::Extern => self.extern_handler_decl(),
            TokenKind::Api => self.api_decl(),
            TokenKind::Worker => self.worker_decl(),
            TokenKind::Job => self.job_decl(),
            TokenKind::Channel => self.channel_decl(),
            TokenKind::Crud => self.crud_decl(),
            TokenKind::Events => self.component_decl(Item::Events),
            TokenKind::Pipeline => self.pipeline_decl(),
            _ => {
                self.error_expected(
                    "a declaration (`project`, `service`, `use`, `record`, `error`, `stream`, \
                     `table`, `handler`, `extern`, `api`, `worker`, `job`, `channel`, `crud`, \
                     `events`, or `pipeline`)",
                );
                None
            }
        }
    }

    fn project_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        let semi = self.expect(TokenKind::Semi)?;
        Some(Item::Project(ProjectDecl {
            span: kw.span.to(semi.span),
            name,
        }))
    }

    fn service_item(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        if let Some(semi) = self.eat(TokenKind::Semi) {
            return Some(Item::Service(ServiceDecl {
                span: kw.span.to(semi.span),
                name,
            }));
        }
        self.expect(TokenKind::LBrace)?;
        let mut items = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Use => {
                    if let Some(Item::Use(item)) = self.use_block() {
                        items.push(ServiceItem::Use(item));
                    }
                }
                TokenKind::Api => {
                    if let Some(Item::Api(item)) = self.api_decl() {
                        items.push(ServiceItem::Api(item));
                    }
                }
                TokenKind::Worker => {
                    if let Some(Item::Worker(item)) = self.worker_decl() {
                        items.push(ServiceItem::Worker(item));
                    }
                }
                TokenKind::Job => {
                    if let Some(Item::Job(item)) = self.job_decl() {
                        items.push(ServiceItem::Job(item));
                    }
                }
                TokenKind::Channel => {
                    if let Some(Item::Channel(item)) = self.channel_decl() {
                        items.push(ServiceItem::Channel(item));
                    }
                }
                TokenKind::Crud => {
                    if let Some(Item::Crud(item)) = self.crud_decl() {
                        items.push(ServiceItem::Crud(item));
                    }
                }
                TokenKind::Events => {
                    if let Some(Item::Events(item)) = self.component_decl(Item::Events) {
                        items.push(ServiceItem::Events(item));
                    }
                }
                TokenKind::Handler => {
                    if let Some(Item::Handler(item)) = self.handler_decl() {
                        items.push(ServiceItem::Handler(item));
                    }
                }
                TokenKind::Extern => {
                    if let Some(Item::Handler(item)) = self.extern_handler_decl() {
                        items.push(ServiceItem::Handler(item));
                    }
                }
                TokenKind::Pipeline => {
                    if let Some(Item::Pipeline(item)) = self.pipeline_decl() {
                        items.push(ServiceItem::Pipeline(item));
                    }
                }
                TokenKind::Expand => {
                    if let Some(Item::Expand(item)) = self.expand_stmt() {
                        items.push(ServiceItem::Expand(item));
                    }
                }
                _ => {
                    self.error_expected(
                        "a service item (`use`, `api`, `worker`, `job`, `channel`, `crud`, `events`, \
                         `handler`, `extern`, `pipeline`, `expand`) or `}`",
                    );
                    self.recover_inside_block();
                }
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Some(Item::ServiceBlock(ServiceBlock {
            span: kw.span.to(close.span),
            name,
            items,
        }))
    }

    fn component_decl(&mut self, build: fn(ComponentDecl) -> Item) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        let semi = self.expect(TokenKind::Semi)?;
        Some(build(ComponentDecl {
            span: kw.span.to(semi.span),
            name,
        }))
    }

    /// `api <Name>[: <Record>];`
    fn api_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        let request = match self.eat(TokenKind::Colon) {
            Some(_) => Some(self.expect_ident()?),
            None => None,
        };
        let (attrs, end) = self.decl_tail()?;
        Some(Item::Api(ApiDecl {
            span: kw.span.to(end),
            name,
            request,
            attrs,
        }))
    }

    /// `worker <Name> [on <Stream>];`
    fn worker_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        let stream = match self.eat(TokenKind::On) {
            Some(_) => Some(self.expect_ident()?),
            None => None,
        };
        let (attrs, end) = self.decl_tail()?;
        Some(Item::Worker(WorkerDecl {
            span: kw.span.to(end),
            name,
            stream,
            attrs,
        }))
    }

    /// `job <Name> { schedule: "..."; }`
    fn job_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        let (attrs, end) = self.decl_tail()?;
        Some(Item::Job(JobDecl {
            span: kw.span.to(end),
            name,
            attrs,
        }))
    }

    /// `channel <Name> on <Stream>;`
    fn channel_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        self.expect(TokenKind::On)?;
        let stream = self.expect_ident()?;
        let (attrs, end) = self.decl_tail()?;
        Some(Item::Channel(ChannelDecl {
            span: kw.span.to(end),
            name,
            stream,
            attrs,
        }))
    }

    /// `crud <Name>[: <Record>];`
    fn crud_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        let record = match self.eat(TokenKind::Colon) {
            Some(_) => Some(self.expect_ident()?),
            None => None,
        };
        let (attrs, end) = self.decl_tail()?;
        Some(Item::Crud(CrudDecl {
            span: kw.span.to(end),
            name,
            record,
            attrs,
        }))
    }

    /// `stream <Name>: <Record>;`
    fn stream_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let record = self.expect_ident()?;
        let (attrs, end) = self.decl_tail()?;
        Some(Item::Stream(StreamDecl {
            span: kw.span.to(end),
            name,
            record,
            attrs,
        }))
    }

    /// `table <Name>: <Record>;` (v0.7)
    /// `import "path";` (v0.8) — the path itself is resolved later by
    /// `crate::module`, not here; the parser only extracts the string.
    fn import_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let tok = self.expect(TokenKind::Str)?;
        let path = unquote(&self.src[tok.span.range()]);
        let semi = self.expect(TokenKind::Semi)?;
        Some(Item::Import(ImportDecl {
            span: kw.span.to(semi.span),
            path,
        }))
    }

    fn table_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let record = self.expect_ident()?;
        let semi = self.expect(TokenKind::Semi)?;
        Some(Item::Table(TableDecl {
            span: kw.span.to(semi.span),
            name,
            record,
        }))
    }

    /// `expand <Blueprint><<Record>> { field: value; .. };` (v0.8).
    fn expand_stmt(&mut self) -> Option<Item> {
        let kw = self.bump();
        let blueprint = self.expect_ident()?;
        self.expect(TokenKind::Lt)?;
        let type_arg = self.expect_ident()?;
        self.expect(TokenKind::Gt)?;
        let (args, end) = self.decl_tail()?;
        Some(Item::Expand(ExpandStmt {
            span: kw.span.to(end),
            blueprint,
            type_arg,
            args,
        }))
    }

    /// `blueprint <Name><<TypeParam>: record> { params { .. } <body> }`
    /// (v0.8). `body` accepts only `use`/`crud`/`stream`/`handler`/
    /// `pipeline` — a deliberately narrower set than a full program's
    /// items (see `BlueprintItem`).
    fn blueprint_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        self.expect(TokenKind::Lt)?;
        let type_param = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        self.expect(TokenKind::Record)?;
        self.expect(TokenKind::Gt)?;
        self.expect(TokenKind::LBrace)?;

        let mut params = Vec::new();
        if self.at(TokenKind::Params) {
            self.bump();
            self.expect(TokenKind::LBrace)?;
            loop {
                match self.peek().kind {
                    TokenKind::RBrace | TokenKind::Eof => break,
                    TokenKind::Ident => {
                        let field_name = self.expect_ident()?;
                        if self.expect(TokenKind::Colon).is_none() {
                            self.recover_inside_block();
                            continue;
                        }
                        let Some(ty) = self.type_expr() else {
                            self.recover_inside_block();
                            continue;
                        };
                        let span = field_name.span.to(self.peek().span);
                        if self.expect(TokenKind::Semi).is_none() {
                            self.recover_inside_block();
                            continue;
                        }
                        params.push(Field {
                            name: field_name,
                            ty,
                            span,
                        });
                    }
                    _ => {
                        self.error_expected("a param like `prefix: String;` or `}`");
                        self.recover_inside_block();
                    }
                }
            }
            self.expect(TokenKind::RBrace)?;
        }

        let mut body = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Use => {
                    if let Some(Item::Use(item)) = self.use_block() {
                        body.push(BlueprintItem::Use(item));
                    }
                }
                TokenKind::Crud => {
                    if let Some(Item::Crud(item)) = self.crud_decl() {
                        body.push(BlueprintItem::Crud(item));
                    }
                }
                TokenKind::Stream => {
                    if let Some(Item::Stream(item)) = self.stream_decl() {
                        body.push(BlueprintItem::Stream(item));
                    }
                }
                TokenKind::Handler => {
                    if let Some(Item::Handler(item)) = self.handler_decl() {
                        body.push(BlueprintItem::Handler(item));
                    }
                }
                _ => {
                    self.error_expected(
                        "a blueprint item (`use`, `crud`, `stream`, `handler`) or `}`",
                    );
                    self.recover_inside_block();
                }
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Some(Item::Blueprint(BlueprintDecl {
            span: kw.span.to(close.span),
            name,
            type_param,
            params,
            body,
        }))
    }

    /// Shared declaration tail for attributed components: either `;` or
    /// `{ name: value; .. }`.
    fn decl_tail(&mut self) -> Option<(Vec<Attr>, ciac_diagnostics::Span)> {
        if let Some(semi) = self.eat(TokenKind::Semi) {
            return Some((Vec::new(), semi.span));
        }
        let Some(open) = self.expect(TokenKind::LBrace) else {
            self.recover();
            return None;
        };
        let mut attrs = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Ident => {
                    let name = self.expect_ident()?;
                    if self.expect(TokenKind::Colon).is_none() {
                        self.recover_inside_block();
                        continue;
                    }
                    let Some(value) = self.attr_value() else {
                        self.recover_inside_block();
                        continue;
                    };
                    let span = name.span.to(value.span());
                    if self.expect(TokenKind::Semi).is_none() {
                        self.recover_inside_block();
                        continue;
                    }
                    attrs.push(Attr { name, value, span });
                }
                _ => {
                    self.error_expected("an attribute like `method: POST;` or `}`");
                    self.recover_inside_block();
                }
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Some((attrs, open.span.to(close.span)))
    }

    fn attr_value(&mut self) -> Option<AttrValue> {
        match self.peek().kind {
            TokenKind::Ident => Some(AttrValue::Ident(self.expect_ident()?)),
            // `true`/`false` are now reserved keywords (v0.7 expression
            // grammar), but attribute values like `catch_up: false;`
            // predate that and must keep parsing identically: represent
            // them the same way a bare `Ident` always did.
            TokenKind::True | TokenKind::False => {
                let tok = self.bump();
                Some(AttrValue::Ident(Ident {
                    text: self.src[tok.span.range()].to_owned(),
                    span: tok.span,
                }))
            }
            TokenKind::Number => {
                let tok = self.bump();
                let raw = &self.src[tok.span.range()];
                let value = raw.parse::<u64>().unwrap_or(0);
                Some(AttrValue::Number {
                    value,
                    span: tok.span,
                })
            }
            TokenKind::Str => {
                let tok = self.bump();
                let raw = &self.src[tok.span.range()];
                Some(AttrValue::Str {
                    value: unquote(raw),
                    span: tok.span,
                })
            }
            _ => {
                self.error_expected("an attribute value");
                None
            }
        }
    }

    /// `handler <Name> { db: main; cache: hot; .. }` (the classic
    /// binding-only form, unchanged since v0.1) or, when `(` follows the
    /// name, `handler <Name>(params) -> RetType { <stmts> }` (v0.7 inline
    /// body). The two forms are disambiguated purely on whether `(`
    /// follows the name — the binding form never had a parameter list.
    fn handler_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        if self.at(TokenKind::LParen) {
            let params = self.param_list()?;
            self.expect(TokenKind::Arrow)?;
            let return_ty = self.type_expr()?;
            let (body, close) = self.block()?;
            return Some(Item::Handler(HandlerDecl {
                span: kw.span.to(close),
                name,
                bindings: Vec::new(),
                params,
                return_ty: Some(return_ty),
                body: Some(body),
                is_extern: false,
            }));
        }
        self.expect(TokenKind::LBrace)?;
        let mut bindings = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Ident => {
                    let capability = self.expect_ident()?;
                    if self.expect(TokenKind::Colon).is_none() {
                        self.recover_inside_block();
                        continue;
                    }
                    let Some(instance) = self.expect_ident() else {
                        self.recover_inside_block();
                        continue;
                    };
                    let span = capability.span.to(self.peek().span);
                    if self.expect(TokenKind::Semi).is_none() {
                        self.recover_inside_block();
                        continue;
                    }
                    bindings.push(HandlerBinding {
                        capability,
                        instance,
                        span,
                    });
                }
                _ => {
                    self.error_expected("a binding like `db: main;` or `}`");
                    self.recover_inside_block();
                }
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Some(Item::Handler(HandlerDecl {
            span: kw.span.to(close.span),
            name,
            bindings,
            params: Vec::new(),
            return_ty: None,
            body: None,
            is_extern: false,
        }))
    }

    /// `extern handler <Name>(params) -> RetType;` (v0.7) — a typed
    /// signature with no body; sema treats it exactly like today's
    /// binding-only stub handlers until typeck (M2) lands.
    fn extern_handler_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        self.expect(TokenKind::Handler)?;
        let name = self.expect_ident()?;
        let params = self.param_list()?;
        self.expect(TokenKind::Arrow)?;
        let return_ty = self.type_expr()?;
        let semi = self.expect(TokenKind::Semi)?;
        Some(Item::Handler(HandlerDecl {
            span: kw.span.to(semi.span),
            name,
            bindings: Vec::new(),
            params,
            return_ty: Some(return_ty),
            body: None,
            is_extern: true,
        }))
    }

    /// `(name: Type, name: Type, ..)` (v0.7).
    fn param_list(&mut self) -> Option<Vec<Param>> {
        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                let name = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let ty = self.type_expr()?;
                let span = name.span.to(ty.span());
                params.push(Param { name, ty, span });
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen)?;
        Some(params)
    }

    /// `{ <stmt>* }` (v0.7). Returns the statements and the closing `}`'s
    /// span; recovers at `;` boundaries within the block on a bad statement.
    fn block(&mut self) -> Option<(Vec<Stmt>, ciac_diagnostics::Span)> {
        self.expect(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                _ => match self.stmt() {
                    Some(stmt) => stmts.push(stmt),
                    None => self.recover_inside_block(),
                },
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Some((stmts, close.span))
    }

    /// One statement inside a handler body (v0.7).
    fn stmt(&mut self) -> Option<Stmt> {
        match self.peek().kind {
            TokenKind::Let => {
                let kw = self.bump();
                let name = self.expect_ident()?;
                self.expect(TokenKind::Eq)?;
                let value = self.expr(0)?;
                let semi = self.expect(TokenKind::Semi)?;
                Some(Stmt::Let {
                    span: kw.span.to(semi.span),
                    name,
                    value,
                })
            }
            TokenKind::Return => {
                let kw = self.bump();
                if let Some(semi) = self.eat(TokenKind::Semi) {
                    return Some(Stmt::Return {
                        value: None,
                        span: kw.span.to(semi.span),
                    });
                }
                let value = self.expr(0)?;
                let semi = self.expect(TokenKind::Semi)?;
                Some(Stmt::Return {
                    value: Some(value),
                    span: kw.span.to(semi.span),
                })
            }
            TokenKind::Fail => {
                let kw = self.bump();
                let error = self.expect_ident()?;
                let mut args = Vec::new();
                if self.eat(TokenKind::LParen).is_some() {
                    if !self.at(TokenKind::RParen) {
                        loop {
                            args.push(self.expr(0)?);
                            if self.eat(TokenKind::Comma).is_none() {
                                break;
                            }
                        }
                    }
                    self.expect(TokenKind::RParen)?;
                }
                let semi = self.expect(TokenKind::Semi)?;
                Some(Stmt::Fail {
                    span: kw.span.to(semi.span),
                    error,
                    args,
                })
            }
            TokenKind::Publish => {
                let kw = self.bump();
                let stream = self.expect_ident()?;
                self.expect(TokenKind::LParen)?;
                let value = self.expr(0)?;
                self.expect(TokenKind::RParen)?;
                let semi = self.expect(TokenKind::Semi)?;
                Some(Stmt::Publish {
                    span: kw.span.to(semi.span),
                    stream,
                    value,
                })
            }
            _ => {
                let expr = self.expr(0)?;
                // A block's final statement may omit the `;` — it's the
                // block's tail value (needed for `if`/`match` used as
                // expressions, e.g. `if cond { v } else { v }`). Anywhere
                // else, the `;` is required.
                if self.at(TokenKind::RBrace) {
                    return Some(Stmt::Expr(expr));
                }
                self.expect(TokenKind::Semi)?;
                Some(Stmt::Expr(expr))
            }
        }
    }

    /// Precedence-climbing (Pratt) expression parser. `min_bp` is the
    /// minimum left binding power an infix operator must have to be
    /// consumed at this call depth.
    fn expr(&mut self, min_bp: u8) -> Option<Expr> {
        let mut lhs = self.expr_prefix()?;
        loop {
            let (op, l_bp, r_bp) = match self.peek().kind {
                TokenKind::OrOr => (BinOp::Or, 1, 2),
                TokenKind::AndAnd => (BinOp::And, 3, 4),
                TokenKind::EqEq => (BinOp::Eq, 5, 6),
                TokenKind::NotEq => (BinOp::NotEq, 5, 6),
                TokenKind::Lt => (BinOp::Lt, 5, 6),
                TokenKind::LtEq => (BinOp::LtEq, 5, 6),
                TokenKind::Gt => (BinOp::Gt, 5, 6),
                TokenKind::GtEq => (BinOp::GtEq, 5, 6),
                TokenKind::Plus => (BinOp::Add, 7, 8),
                TokenKind::Minus => (BinOp::Sub, 7, 8),
                TokenKind::Star => (BinOp::Mul, 9, 10),
                TokenKind::Slash => (BinOp::Div, 9, 10),
                _ => break,
            };
            if l_bp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.expr(r_bp)?;
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                span,
            };
        }
        Some(lhs)
    }

    /// Unary prefix operators (`!`, `-`), then postfix chains.
    fn expr_prefix(&mut self) -> Option<Expr> {
        if let Some(tok) = self.eat(TokenKind::Bang) {
            let expr = self.expr_prefix()?;
            let span = tok.span.to(expr.span());
            return Some(Expr::Unary {
                op: UnOp::Not,
                expr: Box::new(expr),
                span,
            });
        }
        if let Some(tok) = self.eat(TokenKind::Minus) {
            let expr = self.expr_prefix()?;
            let span = tok.span.to(expr.span());
            return Some(Expr::Unary {
                op: UnOp::Neg,
                expr: Box::new(expr),
                span,
            });
        }
        self.expr_postfix()
    }

    /// Postfix chain on an atom: `.field`, `[index]`, `(args)`, and
    /// `{ field: value }` record construction/update. The `{` suffix is
    /// skipped when `self.no_record_lit` is set (inside an `if` condition
    /// or `match` scrutinee), mirroring how Rust disambiguates struct
    /// literals from block openers in the same positions.
    fn expr_postfix(&mut self) -> Option<Expr> {
        let mut expr = self.expr_atom()?;
        loop {
            match self.peek().kind {
                TokenKind::Dot => {
                    self.bump();
                    let field = self.expect_ident()?;
                    let span = expr.span().to(field.span);
                    expr = Expr::FieldAccess {
                        base: Box::new(expr),
                        field,
                        span,
                    };
                }
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.expr(0)?;
                    let close = self.expect(TokenKind::RBracket)?;
                    let span = expr.span().to(close.span);
                    expr = Expr::Index {
                        base: Box::new(expr),
                        index: Box::new(index),
                        span,
                    };
                }
                TokenKind::LParen => {
                    self.bump();
                    let mut args = Vec::new();
                    if !self.at(TokenKind::RParen) {
                        loop {
                            args.push(self.expr(0)?);
                            if self.eat(TokenKind::Comma).is_none() {
                                break;
                            }
                        }
                    }
                    let close = self.expect(TokenKind::RParen)?;
                    let span = expr.span().to(close.span);
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args,
                        span,
                    };
                }
                TokenKind::LBrace if !self.no_record_lit => {
                    let (fields, close) = self.field_init_list()?;
                    let span = expr.span().to(close);
                    expr = Expr::RecordCons {
                        base: Box::new(expr),
                        fields,
                        span,
                    };
                }
                _ => break,
            }
        }
        Some(expr)
    }

    /// `{ name: value, .. }` — the field list of a record
    /// construction/update (v0.7).
    fn field_init_list(&mut self) -> Option<(Vec<FieldInit>, ciac_diagnostics::Span)> {
        let open = self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        if !self.at(TokenKind::RBrace) {
            loop {
                let name = self.expect_ident()?;
                self.expect(TokenKind::Colon)?;
                let value = self.expr(0)?;
                let span = name.span.to(value.span());
                fields.push(FieldInit { name, value, span });
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Some((fields, open.span.to(close.span)))
    }

    /// An atom: literal, identifier, parenthesized expression, `if`, or
    /// `match`.
    fn expr_atom(&mut self) -> Option<Expr> {
        match self.peek().kind {
            TokenKind::Number => {
                let tok = self.bump();
                Some(Expr::Number {
                    text: self.src[tok.span.range()].to_owned(),
                    span: tok.span,
                })
            }
            TokenKind::Str => {
                let tok = self.bump();
                Some(Expr::Str {
                    value: unquote(&self.src[tok.span.range()]),
                    span: tok.span,
                })
            }
            TokenKind::True => {
                let tok = self.bump();
                Some(Expr::Bool {
                    value: true,
                    span: tok.span,
                })
            }
            TokenKind::False => {
                let tok = self.bump();
                Some(Expr::Bool {
                    value: false,
                    span: tok.span,
                })
            }
            TokenKind::Ident => Some(Expr::Ident(self.expect_ident()?)),
            TokenKind::LParen => {
                self.bump();
                let inner = self.expr(0)?;
                self.expect(TokenKind::RParen)?;
                Some(inner)
            }
            TokenKind::If => self.if_expr(),
            TokenKind::Match => self.match_expr(),
            _ => {
                self.error_expected("an expression");
                None
            }
        }
    }

    /// `if <cond> { <stmts> } [else { <stmts> }]` (v0.7). `cond` is parsed
    /// with record-literal syntax suppressed (see `expr_postfix`).
    fn if_expr(&mut self) -> Option<Expr> {
        let kw = self.bump();
        self.no_record_lit = true;
        let cond = self.expr(0);
        self.no_record_lit = false;
        let cond = cond?;
        let (then_branch, then_end) = self.block()?;
        let mut end = then_end;
        let else_branch = if self.eat(TokenKind::Else).is_some() {
            let (stmts, close) = self.block()?;
            end = close;
            Some(stmts)
        } else {
            None
        };
        Some(Expr::If {
            span: kw.span.to(end),
            cond: Box::new(cond),
            then_branch,
            else_branch,
        })
    }

    /// `match <scrutinee> { Variant -> { <stmts> } _ -> { <stmts> } }`
    /// (v0.7), reusing [`ArmLabel`] from the pipeline `match` step.
    /// `scrutinee` is parsed with record-literal syntax suppressed.
    fn match_expr(&mut self) -> Option<Expr> {
        let kw = self.bump();
        self.no_record_lit = true;
        let scrutinee = self.expr(0);
        self.no_record_lit = false;
        let scrutinee = scrutinee?;
        self.expect(TokenKind::LBrace)?;
        let mut arms = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Ident => {
                    let label_ident = self.expect_ident()?;
                    let label = if label_ident.text == "_" {
                        ArmLabel::Default(label_ident.span)
                    } else {
                        ArmLabel::Variant(label_ident)
                    };
                    if self.expect(TokenKind::Arrow).is_none() {
                        self.recover_inside_block();
                        continue;
                    }
                    let Some((body, close)) = self.block() else {
                        self.recover_inside_block();
                        continue;
                    };
                    arms.push(ExprArm {
                        span: label.span().to(close),
                        label,
                        body,
                    });
                }
                _ => {
                    self.error_expected("a match arm like `Ready -> { .. }` or `}`");
                    self.recover_inside_block();
                }
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Some(Expr::Match {
            span: kw.span.to(close.span),
            scrutinee: Box::new(scrutinee),
            arms,
        })
    }

    /// `record <Name> { field: Type; .. }` or, when `kind` is
    /// `RecordKind::Error`, `error <Name> { field: Type; .. }` (v0.7) —
    /// identical field grammar, distinguished only by the leading keyword.
    fn record_decl(&mut self, kind: RecordKind) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Ident => {
                    let field_name = self.expect_ident()?;
                    if self.expect(TokenKind::Colon).is_none() {
                        self.recover_inside_block();
                        continue;
                    }
                    let Some(ty) = self.type_expr() else {
                        self.recover_inside_block();
                        continue;
                    };
                    let span = field_name.span.to(self.peek().span);
                    if self.expect(TokenKind::Semi).is_none() {
                        self.recover_inside_block();
                        continue;
                    }
                    fields.push(Field {
                        name: field_name,
                        ty,
                        span,
                    });
                }
                _ => {
                    self.error_expected("a field like `title: String;` or `}`");
                    self.recover_inside_block();
                }
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Some(Item::Record(RecordDecl {
            span: kw.span.to(close.span),
            name,
            fields,
            kind,
        }))
    }

    /// A field type: a named type or `enum { A, B, .. }`.
    fn type_expr(&mut self) -> Option<TypeExpr> {
        if let Some(kw) = self.eat(TokenKind::Enum) {
            self.expect(TokenKind::LBrace)?;
            let mut variants = vec![self.expect_ident()?];
            while self.eat(TokenKind::Comma).is_some() {
                variants.push(self.expect_ident()?);
            }
            let close = self.expect(TokenKind::RBrace)?;
            return Some(TypeExpr::Enum {
                variants,
                span: kw.span.to(close.span),
            });
        }
        if self.at(TokenKind::Ident) {
            return Some(TypeExpr::Named(self.expect_ident()?));
        }
        self.error_expected("a type like `String` or `enum { A, B }`");
        None
    }

    fn use_block(&mut self) -> Option<Item> {
        let kw = self.bump();
        self.expect(TokenKind::LBrace)?;
        let mut entries = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Ident => {
                    let capability = self.expect_ident()?;
                    let first = match self.expect_ident() {
                        Some(p) => p,
                        None => {
                            self.recover_inside_block();
                            continue;
                        }
                    };
                    let (name, provider) = if self.at(TokenKind::Ident) {
                        (Some(first), Some(self.expect_ident()?))
                    } else if providerless_use_entry(&capability.text) {
                        (Some(first), None)
                    } else {
                        (None, Some(first))
                    };
                    let (attrs, end) = match self.decl_tail() {
                        Some(tail) => tail,
                        None => {
                            self.recover_inside_block();
                            continue;
                        }
                    };
                    let span = capability.span.to(end);
                    if !matches!(self.peek().kind, TokenKind::RBrace | TokenKind::Eof)
                        && self.at(TokenKind::Semi)
                    {
                        self.recover_inside_block();
                    }
                    entries.push(UseEntry {
                        capability,
                        name,
                        provider,
                        attrs,
                        span,
                    });
                }
                _ => {
                    self.error_expected("a capability entry like `db Postgres;` or `}`");
                    self.recover_inside_block();
                }
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Some(Item::Use(UseBlock {
            entries,
            span: kw.span.to(close.span),
        }))
    }

    /// Recovery within a `use { .. }` block: skip to after the next `;`,
    /// or stop before `}` / EOF.
    fn recover_inside_block(&mut self) {
        loop {
            match self.peek().kind {
                TokenKind::Eof | TokenKind::RBrace => return,
                TokenKind::Semi => {
                    self.bump();
                    return;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn pipeline_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let mut steps = Vec::new();
        loop {
            steps.push(self.step_expr()?);
            if self.eat(TokenKind::Arrow).is_none() {
                break;
            }
        }
        let semi = self.expect(TokenKind::Semi)?;
        Some(Item::Pipeline(PipelineDecl {
            span: kw.span.to(semi.span),
            name,
            steps,
        }))
    }

    fn step_expr(&mut self) -> Option<StepExpr> {
        if self.at(TokenKind::Match) {
            return self.match_step();
        }
        if self.eat(TokenKind::Call).is_some() {
            return Some(StepExpr::Call(self.qualified_ident()?));
        }
        if self.eat(TokenKind::Publish).is_some() {
            return Some(StepExpr::Publish(self.expect_ident()?));
        }
        if self.at(TokenKind::Ident) {
            return Some(StepExpr::Name(self.expect_ident()?));
        }
        self.error_expected("a step name or `publish <Stream>`");
        None
    }

    fn qualified_ident(&mut self) -> Option<QualifiedIdent> {
        let first = self.expect_ident()?;
        let mut end = first.span;
        let mut segments = vec![first];
        while self.eat(TokenKind::Dot).is_some() {
            let segment = self.expect_ident()?;
            end = segment.span;
            segments.push(segment);
        }
        let span = segments[0].span.to(end);
        Some(QualifiedIdent { segments, span })
    }

    fn match_step(&mut self) -> Option<StepExpr> {
        let kw = self.bump();
        let field = self.expect_ident()?;
        self.expect(TokenKind::LBrace)?;
        let mut arms = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                TokenKind::Ident => {
                    let label_ident = self.expect_ident()?;
                    let label = if label_ident.text == "_" {
                        ArmLabel::Default(label_ident.span)
                    } else {
                        ArmLabel::Variant(label_ident)
                    };
                    if self.expect(TokenKind::Arrow).is_none() {
                        self.recover_inside_block();
                        continue;
                    }
                    let mut steps = Vec::new();
                    loop {
                        let Some(step) = self.step_expr() else {
                            self.recover_inside_block();
                            break;
                        };
                        steps.push(step);
                        if self.eat(TokenKind::Arrow).is_none() {
                            break;
                        }
                    }
                    let span = label.span().to(self.peek().span);
                    if self.expect(TokenKind::Semi).is_none() {
                        self.recover_inside_block();
                        continue;
                    }
                    arms.push(Arm { label, steps, span });
                }
                _ => {
                    self.error_expected("a match arm like `Ready -> Return;` or `}`");
                    self.recover_inside_block();
                }
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Some(StepExpr::Match {
            field,
            arms,
            span: kw.span.to(close.span),
        })
    }
}

fn unquote(raw: &str) -> String {
    let inner = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw);
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn providerless_use_entry(capability: &str) -> bool {
    matches!(capability, "external_http" | "scheduler" | "realtime")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciac_diagnostics::SourceMap;

    fn parse_src(src: &str) -> (Program, Diagnostics) {
        let mut sources = SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = Diagnostics::new();
        let program = parse(src, file, &mut diags);
        (program, diags)
    }

    #[test]
    fn parses_full_program() {
        let (program, diags) = parse_src(
            "service VideoPlatform;\n\
             use { auth JWT; db Postgres; cache Redis; queue NATS; }\n\
             api Upload;\n\
             pipeline Upload: Auth -> StoreVideo -> Queue -> Return;\n",
        );
        assert!(
            diags.is_empty(),
            "unexpected diagnostics: {:?}",
            diags.codes()
        );
        assert_eq!(program.items.len(), 4);
        let Item::Pipeline(pipeline) = &program.items[3] else {
            panic!("expected pipeline");
        };
        let steps: Vec<&str> = pipeline
            .steps
            .iter()
            .map(|s| match s {
                StepExpr::Name(ident) | StepExpr::Publish(ident) => ident.text.as_str(),
                StepExpr::Call(_) => "call",
                StepExpr::Match { .. } => "match",
            })
            .collect();
        assert_eq!(steps, ["Auth", "StoreVideo", "Queue", "Return"]);
    }

    #[test]
    fn parses_record_with_enum_field() {
        let (program, diags) =
            parse_src("record Video { id: Uuid; title: String; status: enum { Pending, Ready }; }");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Record(record) = &program.items[0] else {
            panic!("expected record");
        };
        assert_eq!(record.name.text, "Video");
        assert_eq!(record.fields.len(), 3);
        let TypeExpr::Enum { variants, .. } = &record.fields[2].ty else {
            panic!("expected enum field");
        };
        let names: Vec<&str> = variants.iter().map(|v| v.text.as_str()).collect();
        assert_eq!(names, ["Pending", "Ready"]);
    }

    #[test]
    fn parses_stream_and_typed_components() {
        let (program, diags) = parse_src(
            "stream Uploaded: Video;\napi Upload: Video;\nworker T on Uploaded;\ncrud Note: Video;\n",
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Stream(stream) = &program.items[0] else {
            panic!("expected stream");
        };
        assert_eq!(stream.record.text, "Video");
        let Item::Api(api) = &program.items[1] else {
            panic!("expected api");
        };
        assert_eq!(api.request.as_ref().map(|r| r.text.as_str()), Some("Video"));
        let Item::Worker(worker) = &program.items[2] else {
            panic!("expected worker");
        };
        assert_eq!(
            worker.stream.as_ref().map(|s| s.text.as_str()),
            Some("Uploaded")
        );
        let Item::Crud(crud) = &program.items[3] else {
            panic!("expected crud");
        };
        assert_eq!(crud.record.as_ref().map(|r| r.text.as_str()), Some("Video"));
    }

    #[test]
    fn parses_publish_step() {
        let (program, diags) = parse_src("pipeline Upload: Work -> publish Uploaded -> Return;");
        assert!(diags.is_empty());
        let Item::Pipeline(pipeline) = &program.items[0] else {
            panic!("expected pipeline");
        };
        assert!(matches!(&pipeline.steps[1], StepExpr::Publish(s) if s.text == "Uploaded"));
    }

    #[test]
    fn parses_attribute_blocks() {
        let (program, diags) = parse_src(
            r#"stream Uploaded: Video { subject: "media.uploaded"; }
               api Upload: Video { method: PUT; path: "/videos"; scope: "videos:write"; }
               worker Transcoder on Uploaded { concurrency: 4; max_retries: 2; }
               job Cleanup { schedule: "0 3 * * *"; catch_up: false; }
               channel Progress on Uploaded { path: "/live/progress"; }
               crud Clip: Video { cache_ttl: 60; page_size: 50; }"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Api(api) = &program.items[1] else {
            panic!("expected api");
        };
        assert_eq!(api.attrs.len(), 3);
        assert_eq!(api.attrs[0].name.text, "method");
        assert!(matches!(&api.attrs[0].value, AttrValue::Ident(v) if v.text == "PUT"));
        assert!(matches!(&api.attrs[1].value, AttrValue::Str { value, .. } if value == "/videos"));
        let Item::Worker(worker) = &program.items[2] else {
            panic!("expected worker");
        };
        assert!(matches!(
            &worker.attrs[0].value,
            AttrValue::Number { value: 4, .. }
        ));
        let Item::Job(job) = &program.items[3] else {
            panic!("expected job");
        };
        assert_eq!(job.attrs.len(), 2);
        assert!(
            matches!(&job.attrs[0].value, AttrValue::Str { value, .. } if value == "0 3 * * *")
        );
        let Item::Channel(channel) = &program.items[4] else {
            panic!("expected channel");
        };
        assert_eq!(channel.stream.text, "Uploaded");
        assert_eq!(channel.attrs.len(), 1);
    }

    #[test]
    fn parses_match_step_with_wildcard() {
        let (program, diags) = parse_src(
            "pipeline Transcoder: Transcode -> match status { Ready -> Notify -> publish Done; _ -> publish Dead; };",
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Pipeline(pipeline) = &program.items[0] else {
            panic!("expected pipeline");
        };
        let StepExpr::Match { field, arms, .. } = &pipeline.steps[1] else {
            panic!("expected match step");
        };
        assert_eq!(field.text, "status");
        assert_eq!(arms.len(), 2);
        assert_eq!(arms[0].steps.len(), 2);
        assert!(matches!(arms[1].label, ArmLabel::Default(_)));
    }

    #[test]
    fn recovers_inside_attribute_block() {
        let (program, diags) = parse_src("api Upload { method POST; path: \"/upload\"; }");
        assert_eq!(diags.codes(), vec![ErrorCode::UnexpectedToken]);
        let Item::Api(api) = &program.items[0] else {
            panic!("expected api");
        };
        assert_eq!(api.attrs.len(), 1);
        assert_eq!(api.attrs[0].name.text, "path");
    }

    #[test]
    fn recovers_inside_record_body() {
        let (program, diags) = parse_src("record R { good: String; bad ; also: Int; }");
        assert_eq!(diags.codes(), vec![ErrorCode::UnexpectedToken]);
        let Item::Record(record) = &program.items[0] else {
            panic!("expected record");
        };
        let names: Vec<&str> = record.fields.iter().map(|f| f.name.text.as_str()).collect();
        assert_eq!(names, ["good", "also"]);
    }

    #[test]
    fn parses_use_block_entries() {
        let (program, diags) = parse_src("use { db Postgres; queue NATS; }");
        assert!(diags.is_empty());
        let Item::Use(block) = &program.items[0] else {
            panic!("expected use block");
        };
        assert_eq!(block.entries.len(), 2);
        assert_eq!(block.entries[0].capability.text, "db");
        assert_eq!(
            block.entries[0].provider.as_ref().map(|p| p.text.as_str()),
            Some("Postgres")
        );
        assert!(block.entries[0].name.is_none());
    }

    #[test]
    fn parses_named_use_entries_and_attrs() {
        let (program, diags) = parse_src(
            r#"use {
                db main Postgres;
                object_store media S3 { bucket: "videos"; }
                external_http billing { base_url: "https://billing.internal"; }
            }"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Use(block) = &program.items[0] else {
            panic!("expected use block");
        };
        assert_eq!(block.entries.len(), 3);
        assert_eq!(
            block.entries[0].name.as_ref().map(|n| n.text.as_str()),
            Some("main")
        );
        assert_eq!(
            block.entries[0].provider.as_ref().map(|p| p.text.as_str()),
            Some("Postgres")
        );
        assert_eq!(block.entries[1].attrs.len(), 1);
        assert_eq!(
            block.entries[2].provider.as_ref().map(|p| p.text.as_str()),
            None
        );
    }

    #[test]
    fn parses_handler_bindings() {
        let (program, diags) = parse_src("handler StoreVideo { db: main; cache: hot; }");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler");
        };
        assert_eq!(handler.name.text, "StoreVideo");
        assert_eq!(handler.bindings.len(), 2);
        assert_eq!(handler.bindings[0].capability.text, "db");
        assert_eq!(handler.bindings[0].instance.text, "main");
    }

    #[test]
    fn parses_project_service_blocks_and_call_steps() {
        let (program, diags) = parse_src(
            "project MediaSystem;\n\
             service UploadApi { api Upload: Video; pipeline Upload: call Billing.Charge -> Return; }\n\
             service Billing { api Charge: Video; pipeline Charge: Capture -> Return; }\n",
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        assert!(matches!(program.items[0], Item::Project(_)));
        let Item::ServiceBlock(service) = &program.items[1] else {
            panic!("expected service block");
        };
        assert_eq!(service.name.text, "UploadApi");
        assert_eq!(service.items.len(), 2);
        let ServiceItem::Pipeline(pipeline) = &service.items[1] else {
            panic!("expected pipeline");
        };
        let StepExpr::Call(call) = &pipeline.steps[0] else {
            panic!("expected call step");
        };
        let segments: Vec<&str> = call.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(segments, ["Billing", "Charge"]);
    }

    #[test]
    fn recovers_after_bad_declaration() {
        let (program, diags) = parse_src("api ;\nworker Encode;\n");
        assert_eq!(diags.codes(), vec![ErrorCode::UnexpectedToken]);
        assert_eq!(program.items.len(), 1, "worker should still parse");
        assert!(matches!(program.items[0], Item::Worker(_)));
    }

    #[test]
    fn recovers_inside_use_block() {
        let (program, diags) = parse_src("use { db ; cache Redis; }\nservice X;");
        assert_eq!(diags.codes(), vec![ErrorCode::UnexpectedToken]);
        let Item::Use(block) = &program.items[0] else {
            panic!("expected use block");
        };
        assert_eq!(block.entries.len(), 1);
        assert_eq!(block.entries[0].capability.text, "cache");
        assert!(matches!(program.items[1], Item::Service(_)));
    }

    #[test]
    fn reports_missing_pipeline_steps() {
        let (_, diags) = parse_src("pipeline Upload: ;");
        assert_eq!(diags.codes(), vec![ErrorCode::UnexpectedToken]);
    }

    // -------------------------------------------------------------
    // v0.7 M1: table, error records, handler bodies, extern, expressions.
    // -------------------------------------------------------------

    #[test]
    fn parses_table_decl() {
        let (program, diags) = parse_src("table Videos: Video;");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Table(table) = &program.items[0] else {
            panic!("expected table decl");
        };
        assert_eq!(table.name.text, "Videos");
        assert_eq!(table.record.text, "Video");
    }

    #[test]
    fn parses_import_decl() {
        let (program, diags) = parse_src(r#"import "records/video.ciac";"#);
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Import(import) = &program.items[0] else {
            panic!("expected import decl");
        };
        assert_eq!(import.path, "records/video.ciac");
    }

    #[test]
    fn parses_blueprint_decl() {
        let (program, diags) = parse_src(
            r#"
            blueprint AuditedCrud<R: record> {
                params { prefix: String; }
                use { db main Postgres; }
                crud Resource: R;
                stream Audited: AuditEvent;
                handler AfterWrite(r: R) -> R {
                    return r;
                }
            }
            "#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Blueprint(bp) = &program.items[0] else {
            panic!("expected blueprint decl");
        };
        assert_eq!(bp.name.text, "AuditedCrud");
        assert_eq!(bp.type_param.text, "R");
        assert_eq!(bp.params.len(), 1);
        assert_eq!(bp.params[0].name.text, "prefix");
        assert_eq!(bp.body.len(), 4);
        assert!(matches!(bp.body[0], BlueprintItem::Use(_)));
        assert!(matches!(bp.body[1], BlueprintItem::Crud(_)));
        assert!(matches!(bp.body[2], BlueprintItem::Stream(_)));
        assert!(matches!(bp.body[3], BlueprintItem::Handler(_)));
    }

    #[test]
    fn parses_expand_stmt() {
        let (program, diags) = parse_src(r#"expand AuditedCrud<Video> { prefix: "/v1"; }"#);
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Expand(expand) = &program.items[0] else {
            panic!("expected expand stmt");
        };
        assert_eq!(expand.blueprint.text, "AuditedCrud");
        assert_eq!(expand.type_arg.text, "Video");
        assert_eq!(expand.args.len(), 1);
        assert_eq!(expand.args[0].name.text, "prefix");
    }

    #[test]
    fn parses_bare_expand_stmt() {
        let (program, diags) = parse_src("expand AuditedCrud<Video>;");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Expand(expand) = &program.items[0] else {
            panic!("expected expand stmt");
        };
        assert!(expand.args.is_empty());
    }

    #[test]
    fn parses_error_record() {
        let (program, diags) = parse_src("error NotFound { id: Uuid; }");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Record(decl) = &program.items[0] else {
            panic!("expected record decl");
        };
        assert_eq!(decl.kind, RecordKind::Error);
        assert_eq!(decl.fields.len(), 1);
    }

    #[test]
    fn parses_plain_record_as_data_kind() {
        let (program, diags) = parse_src("record Video { id: Uuid; }");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Record(decl) = &program.items[0] else {
            panic!("expected record decl");
        };
        assert_eq!(decl.kind, RecordKind::Data);
    }

    #[test]
    fn parses_classic_binding_handler_unchanged() {
        let (program, diags) = parse_src("handler StoreVideo { db: main; cache: hot; }");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler decl");
        };
        assert_eq!(handler.bindings.len(), 2);
        assert!(handler.params.is_empty());
        assert!(handler.return_ty.is_none());
        assert!(handler.body.is_none());
        assert!(!handler.is_extern);
    }

    #[test]
    fn parses_inline_body_handler() {
        let (program, diags) = parse_src(
            r#"handler StoreVideo(v: Video) -> Video {
                   let key = "videos/" + v.id;
                   object_store.put(key, v);
                   return v { status: Ready };
               }"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler decl");
        };
        assert!(handler.bindings.is_empty());
        assert!(!handler.is_extern);
        assert_eq!(handler.params.len(), 1);
        assert_eq!(handler.params[0].name.text, "v");
        assert!(matches!(&handler.return_ty, Some(TypeExpr::Named(t)) if t.text == "Video"));
        let body = handler.body.as_ref().expect("inline body");
        assert_eq!(body.len(), 3);
        assert!(matches!(body[0], Stmt::Let { .. }));
        assert!(matches!(body[1], Stmt::Expr(Expr::Call { .. })));
        let Stmt::Return {
            value: Some(Expr::RecordCons { .. }),
            ..
        } = &body[2]
        else {
            panic!("expected `return v {{ .. }}`, got {:?}", body[2]);
        };
    }

    #[test]
    fn parses_extern_handler_decl() {
        let (program, diags) = parse_src("extern handler StoreVideo(v: Video) -> Video;");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler decl");
        };
        assert!(handler.is_extern);
        assert!(handler.body.is_none());
        assert_eq!(handler.params.len(), 1);
        assert!(handler.return_ty.is_some());
    }

    #[test]
    fn parses_extern_handler_inside_service_block() {
        let (program, diags) = parse_src(
            "service X {\n\
                 use { db Postgres; }\n\
                 extern handler StoreVideo(v: Video) -> Video;\n\
             }",
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::ServiceBlock(block) = &program.items[0] else {
            panic!("expected service block");
        };
        assert!(matches!(
            block.items.last(),
            Some(ServiceItem::Handler(h)) if h.is_extern
        ));
    }

    #[test]
    fn parses_binary_precedence() {
        let (program, diags) = parse_src(
            r#"handler F(a: Int) -> Int {
                   let x = 1 + 2 * 3 == 7 && !false;
                   return x;
               }"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler decl");
        };
        let Some(Stmt::Let {
            value: Expr::Binary { op: BinOp::And, .. },
            ..
        }) = handler.body.as_ref().and_then(|b| b.first())
        else {
            panic!("expected top-level `&&`, got {:?}", handler.body);
        };
    }

    #[test]
    fn parses_field_access_index_and_call_chain() {
        let (program, diags) = parse_src(
            r#"handler F(v: Video) -> Video {
                   let a = v.meta["key"].len();
                   return v;
               }"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler decl");
        };
        let Some(Stmt::Let {
            value: Expr::Call { callee, args, .. },
            ..
        }) = handler.body.as_ref().and_then(|b| b.first())
        else {
            panic!("expected a call expression");
        };
        assert!(args.is_empty());
        assert!(matches!(callee.as_ref(), Expr::FieldAccess { .. }));
    }

    #[test]
    fn parses_if_else_expression_without_record_lit_ambiguity() {
        // The `{` after `v.ready` must open the `if`'s block, not a
        // record-update on `v.ready` — this is exactly the ambiguity
        // `no_record_lit` exists to resolve.
        let (program, diags) = parse_src(
            r#"handler F(v: Video) -> Video {
                   let r = if v.ready { v } else { v };
                   return r;
               }"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler decl");
        };
        assert!(matches!(
            handler.body.as_ref().and_then(|b| b.first()),
            Some(Stmt::Let {
                value: Expr::If { .. },
                ..
            })
        ));
    }

    #[test]
    fn parses_record_cons_and_update() {
        let (program, diags) = parse_src(
            r#"handler F(v: Video) -> Video {
                   let a = Video { id: Uuid.new(), title: "x" };
                   return v { status: Ready };
               }"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler decl");
        };
        let body = handler.body.as_ref().unwrap();
        let Stmt::Let {
            value: Expr::RecordCons { fields, .. },
            ..
        } = &body[0]
        else {
            panic!("expected record construction");
        };
        assert_eq!(fields.len(), 2);
        let Stmt::Return {
            value: Some(Expr::RecordCons { base, fields, .. }),
            ..
        } = &body[1]
        else {
            panic!("expected record update");
        };
        assert!(matches!(base.as_ref(), Expr::Ident(id) if id.text == "v"));
        assert_eq!(fields.len(), 1);
    }

    #[test]
    fn parses_match_expression() {
        let (program, diags) = parse_src(
            r#"handler F(v: Video) -> Video {
                   let r = match v.status {
                       Ready -> { return v; }
                       _ -> { return v; }
                   };
                   return r;
               }"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler decl");
        };
        let Some(Stmt::Let {
            value: Expr::Match { arms, .. },
            ..
        }) = handler.body.as_ref().and_then(|b| b.first())
        else {
            panic!("expected match expression");
        };
        assert_eq!(arms.len(), 2);
        assert!(matches!(arms[0].label, ArmLabel::Variant(_)));
        assert!(matches!(arms[1].label, ArmLabel::Default(_)));
    }

    #[test]
    fn parses_fail_statement() {
        let (program, diags) = parse_src(
            r#"handler F(v: Video) -> Video {
                   fail NotFound(v.id);
                   return v;
               }"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler decl");
        };
        let body = handler.body.as_ref().unwrap();
        let Stmt::Fail { error, args, .. } = &body[0] else {
            panic!("expected fail statement");
        };
        assert_eq!(error.text, "NotFound");
        assert_eq!(args.len(), 1);
    }

    #[test]
    fn parses_publish_statement() {
        let (program, diags) = parse_src(
            r#"handler F(v: Video) -> Video {
                   publish Transcoded(v);
                   return v;
               }"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let Item::Handler(handler) = &program.items[0] else {
            panic!("expected handler decl");
        };
        let body = handler.body.as_ref().unwrap();
        let Stmt::Publish { stream, value, .. } = &body[0] else {
            panic!("expected publish statement");
        };
        assert_eq!(stream.text, "Transcoded");
        assert!(matches!(value, Expr::Ident(id) if id.text == "v"));
    }

    #[test]
    fn recovers_across_malformed_expression_statement() {
        let (program, diags) = parse_src(
            r#"handler F(v: Video) -> Video {
                   let x = + ;
                   return v;
               }
               service Y;"#,
        );
        assert!(!diags.is_empty());
        assert!(matches!(program.items.last(), Some(Item::Service(_))));
    }
}
