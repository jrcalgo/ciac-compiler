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
    }
    .program()
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    diags: &'a mut Diagnostics,
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
            return Some(Ident {
                text: self.src[tok.span.range()].to_owned(),
                span: tok.span,
            });
        }
        self.error_expected("a name");
        None
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
                TokenKind::Service
                | TokenKind::Use
                | TokenKind::Record
                | TokenKind::Stream
                | TokenKind::Api
                | TokenKind::Worker
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
            TokenKind::Service => self.service_decl(),
            TokenKind::Use => self.use_block(),
            TokenKind::Record => self.record_decl(),
            TokenKind::Stream => self.stream_decl(),
            TokenKind::Api => self.api_decl(),
            TokenKind::Worker => self.worker_decl(),
            TokenKind::Crud => self.crud_decl(),
            TokenKind::Events => self.component_decl(Item::Events),
            TokenKind::Pipeline => self.pipeline_decl(),
            _ => {
                self.error_expected(
                    "a declaration (`service`, `use`, `record`, `stream`, `api`, `worker`, \
                     `crud`, `events`, or `pipeline`)",
                );
                None
            }
        }
    }

    fn service_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        let semi = self.expect(TokenKind::Semi)?;
        Some(Item::Service(ServiceDecl {
            span: kw.span.to(semi.span),
            name,
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
        let semi = self.expect(TokenKind::Semi)?;
        Some(Item::Api(ApiDecl {
            span: kw.span.to(semi.span),
            name,
            request,
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
        let semi = self.expect(TokenKind::Semi)?;
        Some(Item::Worker(WorkerDecl {
            span: kw.span.to(semi.span),
            name,
            stream,
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
        let semi = self.expect(TokenKind::Semi)?;
        Some(Item::Crud(CrudDecl {
            span: kw.span.to(semi.span),
            name,
            record,
        }))
    }

    /// `stream <Name>: <Record>;`
    fn stream_decl(&mut self) -> Option<Item> {
        let kw = self.bump();
        let name = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let record = self.expect_ident()?;
        let semi = self.expect(TokenKind::Semi)?;
        Some(Item::Stream(StreamDecl {
            span: kw.span.to(semi.span),
            name,
            record,
        }))
    }

    /// `record <Name> { field: Type; .. }`
    fn record_decl(&mut self) -> Option<Item> {
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
                    let provider = match self.expect_ident() {
                        Some(p) => p,
                        None => {
                            self.recover_inside_block();
                            continue;
                        }
                    };
                    let span = capability.span.to(self.peek().span);
                    if self.expect(TokenKind::Semi).is_none() {
                        self.recover_inside_block();
                        continue;
                    }
                    entries.push(UseEntry {
                        capability,
                        provider,
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
        if self.eat(TokenKind::Publish).is_some() {
            return Some(StepExpr::Publish(self.expect_ident()?));
        }
        if self.at(TokenKind::Ident) {
            return Some(StepExpr::Name(self.expect_ident()?));
        }
        self.error_expected("a step name or `publish <Stream>`");
        None
    }
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
        assert_eq!(block.entries[0].provider.text, "Postgres");
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
}
