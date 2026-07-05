use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode, FileId, Span};
use logos::Logos;

/// The kinds of tokens in the CIaC language.
///
/// Capability names inside `use { .. }` blocks (`auth`, `db`, `cache`, …)
/// are contextual and lex as plain [`TokenKind::Ident`]s; only structural
/// keywords are reserved.
#[derive(Logos, Debug, Clone, Copy, PartialEq, Eq)]
#[logos(skip r"[ \t\r\n]+")]
#[logos(skip r"//[^\n]*")]
#[logos(skip r"/\*([^*]|\*[^/])*\*/")]
pub enum TokenKind {
    // v0.8 M1: multi-file programs.
    #[token("import")]
    Import,

    // v0.8 M2: parameterized architecture.
    #[token("blueprint")]
    Blueprint,
    #[token("expand")]
    Expand,
    #[token("params")]
    Params,

    #[token("project")]
    Project,
    #[token("service")]
    Service,
    #[token("use")]
    Use,
    #[token("api")]
    Api,
    #[token("worker")]
    Worker,
    #[token("job")]
    Job,
    #[token("channel")]
    Channel,
    #[token("pipeline")]
    Pipeline,
    #[token("crud")]
    Crud,
    #[token("events")]
    Events,
    #[token("record")]
    Record,
    #[token("stream")]
    Stream,
    #[token("handler")]
    Handler,
    #[token("on")]
    On,
    #[token("publish")]
    Publish,
    #[token("call")]
    Call,
    #[token("enum")]
    Enum,
    #[token("match")]
    Match,

    // v0.7 M1: handler-body expression language keywords.
    #[token("let")]
    Let,
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("table")]
    Table,
    #[token("error")]
    Error,
    #[token("extern")]
    Extern,
    #[token("return")]
    Return,
    #[token("fail")]
    Fail,

    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token(";")]
    Semi,
    #[token(":")]
    Colon,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token("->")]
    Arrow,

    // v0.7 M1: expression operators and grouping.
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("=")]
    Eq,
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("<")]
    Lt,
    #[token("<=")]
    LtEq,
    #[token(">")]
    Gt,
    #[token(">=")]
    GtEq,
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("!")]
    Bang,

    #[regex("[0-9]+(\\.[0-9]+)?")]
    Number,
    #[regex(r#""([^"\\]|\\.)*""#)]
    Str,
    #[regex("[A-Za-z_][A-Za-z0-9_]*")]
    Ident,

    /// Synthetic token appended after the last real token.
    Eof,
}

impl TokenKind {
    /// How the token kind is described in "expected X" messages.
    pub fn describe(self) -> &'static str {
        match self {
            TokenKind::Import => "`import`",
            TokenKind::Blueprint => "`blueprint`",
            TokenKind::Expand => "`expand`",
            TokenKind::Params => "`params`",
            TokenKind::Project => "`project`",
            TokenKind::Service => "`service`",
            TokenKind::Use => "`use`",
            TokenKind::Api => "`api`",
            TokenKind::Worker => "`worker`",
            TokenKind::Job => "`job`",
            TokenKind::Channel => "`channel`",
            TokenKind::Pipeline => "`pipeline`",
            TokenKind::Crud => "`crud`",
            TokenKind::Events => "`events`",
            TokenKind::Record => "`record`",
            TokenKind::Stream => "`stream`",
            TokenKind::Handler => "`handler`",
            TokenKind::On => "`on`",
            TokenKind::Publish => "`publish`",
            TokenKind::Call => "`call`",
            TokenKind::Enum => "`enum`",
            TokenKind::Match => "`match`",
            TokenKind::Let => "`let`",
            TokenKind::True => "`true`",
            TokenKind::False => "`false`",
            TokenKind::If => "`if`",
            TokenKind::Else => "`else`",
            TokenKind::Table => "`table`",
            TokenKind::Error => "`error`",
            TokenKind::Extern => "`extern`",
            TokenKind::Return => "`return`",
            TokenKind::Fail => "`fail`",
            TokenKind::LBrace => "`{`",
            TokenKind::RBrace => "`}`",
            TokenKind::Semi => "`;`",
            TokenKind::Colon => "`:`",
            TokenKind::Comma => "`,`",
            TokenKind::Dot => "`.`",
            TokenKind::Arrow => "`->`",
            TokenKind::LParen => "`(`",
            TokenKind::RParen => "`)`",
            TokenKind::LBracket => "`[`",
            TokenKind::RBracket => "`]`",
            TokenKind::Plus => "`+`",
            TokenKind::Minus => "`-`",
            TokenKind::Star => "`*`",
            TokenKind::Slash => "`/`",
            TokenKind::Eq => "`=`",
            TokenKind::EqEq => "`==`",
            TokenKind::NotEq => "`!=`",
            TokenKind::Lt => "`<`",
            TokenKind::LtEq => "`<=`",
            TokenKind::Gt => "`>`",
            TokenKind::GtEq => "`>=`",
            TokenKind::AndAnd => "`&&`",
            TokenKind::OrOr => "`||`",
            TokenKind::Bang => "`!`",
            TokenKind::Number => "a number",
            TokenKind::Str => "a string",
            TokenKind::Ident => "a name",
            TokenKind::Eof => "end of file",
        }
    }
}

/// A token with its source location.
#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Lexes `src`, reporting invalid tokens as [`ErrorCode::InvalidToken`].
///
/// Invalid characters are skipped (after reporting) so the parser always
/// receives a well-formed token stream terminated by [`TokenKind::Eof`].
pub fn lex(src: &str, file: FileId, diags: &mut Diagnostics) -> Vec<Token> {
    let mut tokens = Vec::new();
    for (result, range) in TokenKind::lexer(src).spanned() {
        let span = Span::new(file, range.start as u32, range.end as u32);
        match result {
            Ok(kind) => tokens.push(Token { kind, span }),
            Err(()) => {
                diags.push(
                    Diagnostic::new(
                        ErrorCode::InvalidToken,
                        format!("invalid token `{}`", &src[range]),
                    )
                    .with_label(span, "not recognized by the CIaC language"),
                );
            }
        }
    }
    let end = src.len() as u32;
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span::new(file, end, end),
    });
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciac_diagnostics::SourceMap;

    fn lex_kinds(src: &str) -> (Vec<TokenKind>, Diagnostics) {
        let mut sources = SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = Diagnostics::new();
        let tokens = lex(src, file, &mut diags);
        (tokens.into_iter().map(|t| t.kind).collect(), diags)
    }

    #[test]
    fn lexes_pipeline_declaration() {
        let (kinds, diags) = lex_kinds("pipeline Upload: Auth -> StoreVideo;");
        assert!(diags.is_empty());
        assert_eq!(
            kinds,
            vec![
                TokenKind::Pipeline,
                TokenKind::Ident,
                TokenKind::Colon,
                TokenKind::Ident,
                TokenKind::Arrow,
                TokenKind::Ident,
                TokenKind::Semi,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn skips_comments() {
        let (kinds, diags) = lex_kinds("// line\n/* block\n comment */ service X;");
        assert!(diags.is_empty());
        assert_eq!(
            kinds,
            vec![
                TokenKind::Service,
                TokenKind::Ident,
                TokenKind::Semi,
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn reports_invalid_tokens_and_continues() {
        let (kinds, diags) = lex_kinds("service @ X;");
        assert_eq!(diags.codes(), vec![ErrorCode::InvalidToken]);
        assert_eq!(
            kinds,
            vec![
                TokenKind::Service,
                TokenKind::Ident,
                TokenKind::Semi,
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn lexes_attributes_and_match() {
        let (kinds, diags) = lex_kinds(
            r#"api Upload { method: PUT; path: "/videos"; concurrency: 4; } pipeline Upload: match status { Ready -> Return; };"#,
        );
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        assert!(kinds.contains(&TokenKind::Number));
        assert!(kinds.contains(&TokenKind::Str));
        assert!(kinds.contains(&TokenKind::Match));
    }

    #[test]
    fn lexes_table_and_error_and_extern_keywords() {
        let (kinds, diags) = lex_kinds("table Videos: Video; error NotFound { id: Uuid; } extern handler Foo(v: Video) -> Video;");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        assert_eq!(
            kinds,
            vec![
                TokenKind::Table,
                TokenKind::Ident,
                TokenKind::Colon,
                TokenKind::Ident,
                TokenKind::Semi,
                TokenKind::Error,
                TokenKind::Ident,
                TokenKind::LBrace,
                TokenKind::Ident,
                TokenKind::Colon,
                TokenKind::Ident,
                TokenKind::Semi,
                TokenKind::RBrace,
                TokenKind::Extern,
                TokenKind::Handler,
                TokenKind::Ident,
                TokenKind::LParen,
                TokenKind::Ident,
                TokenKind::Colon,
                TokenKind::Ident,
                TokenKind::RParen,
                TokenKind::Arrow,
                TokenKind::Ident,
                TokenKind::Semi,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lexes_expression_operators() {
        let (kinds, diags) =
            lex_kinds(r#"let key = "x" + v.id; if a == b && c != d || !e { } else { }"#);
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        assert_eq!(
            kinds,
            vec![
                TokenKind::Let,
                TokenKind::Ident,
                TokenKind::Eq,
                TokenKind::Str,
                TokenKind::Plus,
                TokenKind::Ident,
                TokenKind::Dot,
                TokenKind::Ident,
                TokenKind::Semi,
                TokenKind::If,
                TokenKind::Ident,
                TokenKind::EqEq,
                TokenKind::Ident,
                TokenKind::AndAnd,
                TokenKind::Ident,
                TokenKind::NotEq,
                TokenKind::Ident,
                TokenKind::OrOr,
                TokenKind::Bang,
                TokenKind::Ident,
                TokenKind::LBrace,
                TokenKind::RBrace,
                TokenKind::Else,
                TokenKind::LBrace,
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lexes_comparison_and_arithmetic_operators() {
        let (kinds, diags) = lex_kinds("a <= b - 1 * 2 / 3 > c; return true; fail false;");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        assert_eq!(
            kinds,
            vec![
                TokenKind::Ident,
                TokenKind::LtEq,
                TokenKind::Ident,
                TokenKind::Minus,
                TokenKind::Number,
                TokenKind::Star,
                TokenKind::Number,
                TokenKind::Slash,
                TokenKind::Number,
                TokenKind::Gt,
                TokenKind::Ident,
                TokenKind::Semi,
                TokenKind::Return,
                TokenKind::True,
                TokenKind::Semi,
                TokenKind::Fail,
                TokenKind::False,
                TokenKind::Semi,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lexes_decimal_number_and_brackets() {
        let (kinds, diags) = lex_kinds("payload[\"key\"] == 3.14");
        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        assert_eq!(
            kinds,
            vec![
                TokenKind::Ident,
                TokenKind::LBracket,
                TokenKind::Str,
                TokenKind::RBracket,
                TokenKind::EqEq,
                TokenKind::Number,
                TokenKind::Eof,
            ]
        );
    }
}
