//! 字句解析(技術仕様書 §3.2、正式定義は `docs/spec/lexical.md`)。
//!
//! 2 段構成:
//! 1. `split_lines`: ソース全体を論理行に分割し、エンコーディング・NFC・インデント・
//!    禁止文字(タブ/全角ASCII記号/制御文字)を検査する。意図層の行はここで得た生テキストのまま使う。
//! 2. `tokenize`: 厳密層の行をトークン列にする。
//!
//! 識別子は「語」= 英数字・かな・漢字・`_` の最長連続。助詞を自動分割しない
//! (`注文IDを返す` は 1 語)。予約語は語全体が一致した場合のみ予約語になる。

use kotonoha_diag::{Code, Diagnostic, FileId, Span};
use unicode_normalization::is_nfc;

pub const INDENT_WIDTH: usize = 4;

/// 論理行。空行・コメントのみの行は含まない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// インデント段数(空白 4 個で 1 段)
    pub level: u32,
    /// インデントとコメントを除き、末尾空白を除いた本文
    pub text: String,
    /// 本文の位置
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    App,         // アプリ
    TypeKw,      // 型
    Enum,        // 列挙
    Data,        // データ
    Adapter,     // アダプタ
    Knowledge,   // 知識
    Screen,      // 画面
    Action,      // 動作
    Contract,    // 約束
    Requires,    // 必要
    Effects,     // 効果
    Op,          // 操作
    Strict,      // 厳密
    Transaction, // 取引
    If,          // もし
    Else,        // そうでなければ
    Loop,        // 繰り返す
    From,        // から
    Check,       // を確認
    OrElse,      // または
    Fail,        // 失敗
    Ok,          // 成功
    Return,      // を返す
    True,        // 真
    False,       // 偽
    NoneKw,      // なし
    Record,      // 記録
    Function,    // 関数
    External,    // 外部関数
    Analysis,    // 分析
    Component,   // 部品
    Experiment,  // 実験
}

pub const KEYWORDS: &[(&str, Keyword)] = &[
    ("アプリ", Keyword::App),
    ("型", Keyword::TypeKw),
    ("列挙", Keyword::Enum),
    ("データ", Keyword::Data),
    ("アダプタ", Keyword::Adapter),
    ("知識", Keyword::Knowledge),
    ("画面", Keyword::Screen),
    ("動作", Keyword::Action),
    ("約束", Keyword::Contract),
    ("必要", Keyword::Requires),
    ("効果", Keyword::Effects),
    ("操作", Keyword::Op),
    ("厳密", Keyword::Strict),
    ("取引", Keyword::Transaction),
    ("もし", Keyword::If),
    ("そうでなければ", Keyword::Else),
    ("繰り返す", Keyword::Loop),
    ("から", Keyword::From),
    ("を確認", Keyword::Check),
    ("または", Keyword::OrElse),
    ("失敗", Keyword::Fail),
    ("成功", Keyword::Ok),
    ("を返す", Keyword::Return),
    ("真", Keyword::True),
    ("偽", Keyword::False),
    ("なし", Keyword::NoneKw),
    ("記録", Keyword::Record),
    ("関数", Keyword::Function),
    ("外部関数", Keyword::External),
    ("分析", Keyword::Analysis),
    ("部品", Keyword::Component),
    ("実験", Keyword::Experiment),
];

impl Keyword {
    pub fn from_word(w: &str) -> Option<Keyword> {
        KEYWORDS.iter().find(|(s, _)| *s == w).map(|(_, k)| *k)
    }
    pub fn as_str(self) -> &'static str {
        KEYWORDS.iter().find(|(_, k)| *k == self).map(|(s, _)| *s).unwrap()
    }
}

/// 末尾がこれらの予約語で終わる 1 語は「空白の入れ忘れ」の可能性が高い(ヒント用)。
pub const SUFFIX_KEYWORDS: &[&str] = &["を返す", "を確認"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sym {
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Dot,
    Arrow,  // ->
    Assign, // =
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    AndAnd,
    OrOr,
    Bang,
    At,
}

impl Sym {
    pub fn as_str(self) -> &'static str {
        match self {
            Sym::LParen => "(",
            Sym::RParen => ")",
            Sym::LBracket => "[",
            Sym::RBracket => "]",
            Sym::LBrace => "{",
            Sym::RBrace => "}",
            Sym::Comma => ",",
            Sym::Colon => ":",
            Sym::Dot => ".",
            Sym::Arrow => "->",
            Sym::Assign => "=",
            Sym::EqEq => "==",
            Sym::Ne => "!=",
            Sym::Lt => "<",
            Sym::Le => "<=",
            Sym::Gt => ">",
            Sym::Ge => ">=",
            Sym::Plus => "+",
            Sym::Minus => "-",
            Sym::Star => "*",
            Sym::Slash => "/",
            Sym::AndAnd => "&&",
            Sym::OrOr => "||",
            Sym::Bang => "!",
            Sym::At => "@",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Keyword(Keyword),
    Int(i64),
    Decimal(String),
    Str(String),
    Sym(Sym),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// 識別子を構成する文字か。
pub fn is_word_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// 全角 ASCII 記号・全角英数(U+FF01〜U+FF5E)と全角空白(U+3000)。自動同一視しない。
pub fn is_fullwidth_ascii(c: char) -> bool {
    ('\u{FF01}'..='\u{FF5E}').contains(&c) || c == '\u{3000}'
}

fn fullwidth_hint(c: char) -> String {
    if c == '\u{3000}' {
        "全角空白は使えません。半角空白を使ってください".to_string()
    } else {
        let half = char::from_u32(c as u32 - 0xFEE0).unwrap_or('?');
        format!("全角記号は自動で同一視しません。半角の `{half}` を使ってください(koto fmt --zenkaku で一括変換できます)")
    }
}

/// 行内の文字位置ユーティリティ。
struct Cursor<'a> {
    file: FileId,
    line_no: u32,
    /// 行頭のバイトオフセット
    line_start: u32,
    text: &'a str,
}

impl Cursor<'_> {
    /// 行内バイト位置 [a, b) の Span。column は文字数で数える。
    fn span(&self, a: usize, b: usize) -> Span {
        let col = self.text[..a].chars().count() as u32 + 1;
        Span::new(self.file, self.line_no, col, self.line_start + a as u32, self.line_start + b as u32)
    }
}

/// ソース全体の検査と論理行分割。
pub fn split_lines(src: &str, file: FileId) -> (Vec<Line>, Vec<Diagnostic>) {
    let mut lines = Vec::new();
    let mut diags = Vec::new();

    if src.starts_with('\u{FEFF}') {
        diags.push(
            Diagnostic::error(Code::SYN001, Span::new(file, 1, 1, 0, 3), "BOM 付き UTF-8 は使えません").hint("BOM を除去してください(koto fmt で除去できます)"),
        );
        return (lines, diags);
    }

    let mut offset: usize = 0;
    for (idx, raw_line) in src.split('\n').enumerate() {
        let line_no = idx as u32 + 1;
        let line_start = offset;
        offset += raw_line.len() + 1;
        // CRLF は LF と同一視する(NFR-009 Windows)。単独 CR は禁止。
        let raw = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let cur = Cursor {
            file,
            line_no,
            line_start: line_start as u32,
            text: raw,
        };

        if !is_nfc(raw) {
            diags.push(
                Diagnostic::error(Code::SYN002, cur.span(0, raw.len()), "この行は Unicode NFC 正規化されていません")
                    .hint("koto fmt で NFC に正規化してください"),
            );
            continue;
        }

        // 1 文字ずつ検査しつつ、文字列リテラル外の `#` 以降をコメントとして除去する。
        let mut in_dq = false; // "..."
        let mut in_kakko = false; // 「...」
        let mut escaped = false;
        let mut comment_at: Option<usize> = None;
        let mut bad = false;
        for (i, c) in raw.char_indices() {
            if in_dq {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_dq = false;
                }
                if c.is_control() {
                    diags.push(Diagnostic::error(Code::SYN001, cur.span(i, i + c.len_utf8()), "文字列内に制御文字があります"));
                    bad = true;
                }
                continue;
            }
            if in_kakko {
                if c == '」' {
                    in_kakko = false;
                }
                if c.is_control() {
                    diags.push(Diagnostic::error(Code::SYN001, cur.span(i, i + c.len_utf8()), "文字列内に制御文字があります"));
                    bad = true;
                }
                continue;
            }
            match c {
                '"' => in_dq = true,
                '「' => in_kakko = true,
                '#' => {
                    comment_at = Some(i);
                    break;
                }
                '\t' => {
                    diags.push(Diagnostic::error(Code::SYN001, cur.span(i, i + 1), "タブ文字は使えません").hint("インデントは半角空白 4 個です"));
                    bad = true;
                }
                '\r' => {
                    diags.push(Diagnostic::error(Code::SYN001, cur.span(i, i + 1), "単独の CR 改行は使えません"));
                    bad = true;
                }
                c if is_fullwidth_ascii(c) => {
                    diags.push(Diagnostic::error(Code::SYN001, cur.span(i, i + c.len_utf8()), format!("全角文字 `{c}` は使えません")).hint(fullwidth_hint(c)));
                    bad = true;
                }
                c if c.is_control() => {
                    diags.push(Diagnostic::error(Code::SYN001, cur.span(i, i + c.len_utf8()), "制御文字は使えません"));
                    bad = true;
                }
                _ => {}
            }
        }
        if in_dq || in_kakko {
            diags.push(Diagnostic::error(Code::SYN001, cur.span(0, raw.len()), "文字列リテラルが閉じていません"));
            bad = true;
        }
        if bad {
            continue;
        }
        let body = &raw[..comment_at.unwrap_or(raw.len())];
        let content = body.trim_end_matches(' ');
        let indent = content.len() - content.trim_start_matches(' ').len();
        if content.trim().is_empty() {
            continue;
        }
        if indent % INDENT_WIDTH != 0 {
            diags.push(
                Diagnostic::error(Code::SYN001, cur.span(0, indent), format!("インデントが {indent} 個の空白です")).hint("インデントは半角空白 4 個単位です"),
            );
            continue;
        }
        lines.push(Line {
            level: (indent / INDENT_WIDTH) as u32,
            text: content[indent..].to_string(),
            span: cur.span(indent, content.len()),
        });
    }
    (lines, diags)
}

/// 行の本文先頭の語(識別子文字の最長連続)を返す。
pub fn first_word(text: &str) -> &str {
    let end = text.char_indices().find(|(_, c)| !is_word_char(*c)).map(|(i, _)| i).unwrap_or(text.len());
    &text[..end]
}

/// 厳密層の 1 行をトークン列にする。
pub fn tokenize(line: &Line) -> Result<Vec<Token>, Diagnostic> {
    tokenize_str(&line.text, line.span)
}

/// 任意の部分文字列(位置 `base` から始まる)をトークン化する。
/// 知識定義の値(`条件 = 状態 == "確定"`)の式部分などにも使う。
pub fn tokenize_str(text: &str, base: Span) -> Result<Vec<Token>, Diagnostic> {
    let mk = |a: usize, b: usize| -> Span {
        let col = base.column + text[..a].chars().count() as u32;
        Span::new(base.file, base.line, col, base.start + a as u32, base.start + b as u32)
    };
    let mut toks = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    let byte_at = |k: usize| -> usize {
        if k < chars.len() {
            chars[k].0
        } else {
            text.len()
        }
    };
    while i < chars.len() {
        let (b, c) = chars[i];
        if c == ' ' {
            i += 1;
            continue;
        }
        // 文字列リテラル
        if c == '"' {
            let mut s = String::new();
            let mut j = i + 1;
            loop {
                if j >= chars.len() {
                    return Err(Diagnostic::error(Code::SYN001, mk(b, text.len()), "文字列リテラルが閉じていません"));
                }
                let cj = chars[j].1;
                if cj == '"' {
                    break;
                }
                if cj == '\\' {
                    j += 1;
                    let Some(&(_, e)) = chars.get(j) else {
                        return Err(Diagnostic::error(Code::SYN001, mk(b, text.len()), "不正なエスケープです"));
                    };
                    match e {
                        '"' => s.push('"'),
                        '\\' => s.push('\\'),
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'u' => {
                            // \u{XXXX}
                            let rest: String = chars[j + 1..].iter().map(|x| x.1).collect();
                            let close = rest.find('}');
                            let ok = rest.starts_with('{') && close.is_some();
                            let parsed = if ok {
                                u32::from_str_radix(&rest[1..close.unwrap()], 16).ok().and_then(char::from_u32)
                            } else {
                                None
                            };
                            let Some(ch) = parsed else {
                                return Err(Diagnostic::error(
                                    Code::SYN001,
                                    mk(chars[j - 1].0, byte_at(j + 1)),
                                    "不正な \\u{...} エスケープです",
                                ));
                            };
                            s.push(ch);
                            j += rest[..=close.unwrap()].chars().count();
                        }
                        _ => {
                            return Err(Diagnostic::error(
                                Code::SYN001,
                                mk(chars[j - 1].0, byte_at(j + 1)),
                                format!("不正なエスケープ `\\{e}` です"),
                            ));
                        }
                    }
                } else {
                    s.push(cj);
                }
                j += 1;
            }
            toks.push(Token {
                kind: TokenKind::Str(s),
                span: mk(b, byte_at(j + 1)),
            });
            i = j + 1;
            continue;
        }
        if c == '「' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].1 != '」' {
                j += 1;
            }
            if j >= chars.len() {
                return Err(Diagnostic::error(Code::SYN001, mk(b, text.len()), "「 が閉じていません"));
            }
            let s: String = chars[i + 1..j].iter().map(|x| x.1).collect();
            toks.push(Token {
                kind: TokenKind::Str(s),
                span: mk(b, byte_at(j + 1)),
            });
            i = j + 1;
            continue;
        }
        // 数値リテラル(ASCII 数字のみ)
        if c.is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && (chars[j].1.is_ascii_digit() || chars[j].1 == '_') {
                j += 1;
            }
            let mut is_dec = false;
            if j + 1 < chars.len() && chars[j].1 == '.' && chars[j + 1].1.is_ascii_digit() {
                is_dec = true;
                j += 1;
                while j < chars.len() && (chars[j].1.is_ascii_digit() || chars[j].1 == '_') {
                    j += 1;
                }
            }
            let lit_end = byte_at(j);
            if j < chars.len() && is_word_char(chars[j].1) {
                let mut k = j;
                while k < chars.len() && is_word_char(chars[k].1) {
                    k += 1;
                }
                return Err(Diagnostic::error(
                    Code::SYN001,
                    mk(b, byte_at(k)),
                    format!("数値の直後に語 `{}` が続いています", &text[lit_end..byte_at(k)]),
                )
                .hint("厳密層では数値と語の間に空白か記号が必要です(`5件` のような表現は意図層でのみ使えます)"));
            }
            let lit: String = text[b..lit_end].chars().filter(|c| *c != '_').collect();
            if text[b..lit_end].ends_with('_') || text[b..lit_end].contains("__") {
                return Err(Diagnostic::error(Code::SYN001, mk(b, lit_end), "数値リテラルの `_` の位置が不正です"));
            }
            let kind = if is_dec {
                TokenKind::Decimal(lit)
            } else {
                match lit.parse::<i64>() {
                    Ok(v) => TokenKind::Int(v),
                    Err(_) => {
                        return Err(Diagnostic::error(
                            Code::SYN001,
                            mk(b, lit_end),
                            "整数リテラルが 64 ビット整数の範囲を超えています",
                        ));
                    }
                }
            };
            toks.push(Token { kind, span: mk(b, lit_end) });
            i = j;
            continue;
        }
        if is_word_char(c) {
            let mut j = i;
            while j < chars.len() && is_word_char(chars[j].1) {
                j += 1;
            }
            let w = &text[b..byte_at(j)];
            let kind = match Keyword::from_word(w) {
                Some(k) => TokenKind::Keyword(k),
                None => TokenKind::Ident(w.to_string()),
            };
            toks.push(Token { kind, span: mk(b, byte_at(j)) });
            i = j;
            continue;
        }
        let next = chars.get(i + 1).map(|x| x.1);
        let (sym, len) = match (c, next) {
            ('-', Some('>')) => (Sym::Arrow, 2),
            ('=', Some('=')) => (Sym::EqEq, 2),
            ('!', Some('=')) => (Sym::Ne, 2),
            ('<', Some('=')) => (Sym::Le, 2),
            ('>', Some('=')) => (Sym::Ge, 2),
            ('&', Some('&')) => (Sym::AndAnd, 2),
            ('|', Some('|')) => (Sym::OrOr, 2),
            ('(', _) => (Sym::LParen, 1),
            (')', _) => (Sym::RParen, 1),
            ('[', _) => (Sym::LBracket, 1),
            (']', _) => (Sym::RBracket, 1),
            ('{', _) => (Sym::LBrace, 1),
            ('}', _) => (Sym::RBrace, 1),
            (',', _) => (Sym::Comma, 1),
            (':', _) => (Sym::Colon, 1),
            ('.', _) => (Sym::Dot, 1),
            ('=', _) => (Sym::Assign, 1),
            ('<', _) => (Sym::Lt, 1),
            ('>', _) => (Sym::Gt, 1),
            ('+', _) => (Sym::Plus, 1),
            ('-', _) => (Sym::Minus, 1),
            ('*', _) => (Sym::Star, 1),
            ('/', _) => (Sym::Slash, 1),
            ('!', _) => (Sym::Bang, 1),
            ('@', _) => (Sym::At, 1),
            _ => {
                let mut d = Diagnostic::error(Code::SYN001, mk(b, b + c.len_utf8()), format!("不正な文字 `{c}` です"));
                if c == '、' || c == '。' {
                    d = d.hint("厳密層では読点・句点は使えません。区切りは `,` です");
                }
                return Err(d);
            }
        };
        toks.push(Token {
            kind: TokenKind::Sym(sym),
            span: mk(b, byte_at(i + len)),
        });
        i += len;
    }
    Ok(toks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(src: &str) -> (Vec<Line>, Vec<Diagnostic>) {
        split_lines(src, FileId(0))
    }
    fn toks(s: &str) -> Vec<TokenKind> {
        let (ls, d) = lines(s);
        assert!(d.is_empty(), "{d:?}");
        tokenize(&ls[0]).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn splits_lines_with_indent_levels_and_strips_comments() {
        let (ls, d) = lines("データ 商品:  # コメント\n    名前: 文字列\n\n        x\n# 行コメント\n");
        assert!(d.is_empty());
        assert_eq!(ls.len(), 3);
        assert_eq!(ls[0].text, "データ 商品:");
        assert_eq!((ls[1].level, ls[2].level), (1, 2));
        assert_eq!(ls[1].span.line, 2);
        assert_eq!(ls[1].span.column, 5);
    }

    #[test]
    fn hash_inside_strings_is_not_comment() {
        let (ls, _) = lines("x = \"a#b\" # c\ny = 「#」");
        assert_eq!(ls[0].text, "x = \"a#b\"");
        assert_eq!(ls[1].text, "y = 「#」");
    }

    #[test]
    fn rejects_tab_bad_indent_fullwidth_bom_and_non_nfc() {
        assert_eq!(lines("\tx").1[0].code, Code::SYN001);
        assert_eq!(lines("  x").1[0].code, Code::SYN001);
        let d = lines("x ＝ 1").1;
        assert_eq!(d[0].code, Code::SYN001);
        assert!(d[0].hints[0].contains('='));
        assert_eq!(lines("\u{FEFF}x").1[0].code, Code::SYN001);
        // が(U+304C) を か + 濁点(U+304B U+3099) で書いた非 NFC
        assert_eq!(lines("\u{304B}\u{3099}").1[0].code, Code::SYN002);
        assert_eq!(lines("x　y").1[0].code, Code::SYN001);
    }

    #[test]
    fn crlf_is_accepted() {
        let (ls, d) = lines("a\r\n    b\r\n");
        assert!(d.is_empty());
        assert_eq!(ls[1].text, "b");
    }

    #[test]
    fn tokenizes_strict_line_from_spec_sample() {
        let t = toks("数量 > 0 を確認 または 失敗(\"数量不正\")");
        assert_eq!(
            t,
            vec![
                TokenKind::Ident("数量".into()),
                TokenKind::Sym(Sym::Gt),
                TokenKind::Int(0),
                TokenKind::Keyword(Keyword::Check),
                TokenKind::Keyword(Keyword::OrElse),
                TokenKind::Keyword(Keyword::Fail),
                TokenKind::Sym(Sym::LParen),
                TokenKind::Str("数量不正".into()),
                TokenKind::Sym(Sym::RParen),
            ]
        );
    }

    #[test]
    fn words_are_maximal_runs_and_particles_are_not_split() {
        assert_eq!(toks("注文IDを返す"), vec![TokenKind::Ident("注文IDを返す".into())]);
        assert_eq!(
            toks("注文ID を返す"),
            vec![TokenKind::Ident("注文ID".into()), TokenKind::Keyword(Keyword::Return)]
        );
        assert_eq!(toks("取引先"), vec![TokenKind::Ident("取引先".into())]);
        assert_eq!(toks("取引:"), vec![TokenKind::Keyword(Keyword::Transaction), TokenKind::Sym(Sym::Colon)]);
    }

    #[test]
    fn operators_and_literals() {
        assert_eq!(
            toks("a->b == c != d <= e >= f && g || !h"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Sym(Sym::Arrow),
                TokenKind::Ident("b".into()),
                TokenKind::Sym(Sym::EqEq),
                TokenKind::Ident("c".into()),
                TokenKind::Sym(Sym::Ne),
                TokenKind::Ident("d".into()),
                TokenKind::Sym(Sym::Le),
                TokenKind::Ident("e".into()),
                TokenKind::Sym(Sym::Ge),
                TokenKind::Ident("f".into()),
                TokenKind::Sym(Sym::AndAnd),
                TokenKind::Ident("g".into()),
                TokenKind::Sym(Sym::OrOr),
                TokenKind::Sym(Sym::Bang),
                TokenKind::Ident("h".into()),
            ]
        );
        assert_eq!(toks("1_000 3.14"), vec![TokenKind::Int(1000), TokenKind::Decimal("3.14".into())]);
        assert_eq!(toks("\"a\\n\\u{41}\""), vec![TokenKind::Str("a\nA".into())]);
        assert_eq!(toks("「購入」"), vec![TokenKind::Str("購入".into())]);
    }

    #[test]
    fn rejects_number_glued_to_word_and_japanese_punctuation() {
        let (ls, _) = lines("5件");
        assert_eq!(tokenize(&ls[0]).unwrap_err().code, Code::SYN001);
        let (ls, _) = lines("a、b");
        assert_eq!(tokenize(&ls[0]).unwrap_err().code, Code::SYN001);
        let (ls, _) = lines("99999999999999999999");
        assert_eq!(tokenize(&ls[0]).unwrap_err().code, Code::SYN001);
    }

    #[test]
    fn token_spans_count_characters() {
        let (ls, _) = lines("    商品.在庫 = 1");
        let t = tokenize(&ls[0]).unwrap();
        assert_eq!(t[0].span.column, 5);
        assert_eq!(t[1].span.column, 7);
        assert_eq!(t[2].span.column, 8);
        assert_eq!(&"    商品.在庫 = 1"[t[2].span.start as usize..t[2].span.end as usize], "在庫");
    }
}
