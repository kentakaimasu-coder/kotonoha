//! 構文解析(正式文法は `docs/spec/grammar.md`)。
//!
//! 行指向の再帰下降パーサ。宣言の見出し行と厳密層の行はトークン化して解析し、
//! 意図層の行(画面本体・約束本体・動作本体の意図行・知識の値)は生テキストのまま AST に格納する。
//! 1 行単位でエラー回復し、可能な限り多くの診断を返す。

pub mod printer;

use kotonoha_ast::*;
use kotonoha_diag::{Code, Diagnostic, FileId, Span};
use kotonoha_lexer::{first_word, split_lines, tokenize, tokenize_str, Keyword, Line, Sym, Token, TokenKind};

/// 1 ファイルを解析する。
pub fn parse_file(src: &str, file: FileId) -> (SourceFile, Vec<Diagnostic>) {
    let (lines, mut diags) = split_lines(src, file);
    let mut p = Parser {
        lines: &lines,
        pos: 0,
        diags: Vec::new(),
    };
    let items = p.parse_top();
    diags.extend(p.diags);
    (SourceFile { file, items }, diags)
}

/// 単独の式を解析する(知識定義の `条件 = ...` や画面意図内の引数で使う)。
pub fn parse_expr_str(text: &str, base: Span) -> Result<Expr, Diagnostic> {
    let toks = tokenize_str(text, base)?;
    let mut ts = TokStream::new(&toks, base);
    let e = ts.expr()?;
    ts.expect_end()?;
    Ok(e)
}

/// 単独の型式を解析する。
pub fn parse_type_str(text: &str, base: Span) -> Result<TypeExpr, Diagnostic> {
    let toks = tokenize_str(text, base)?;
    let mut ts = TokStream::new(&toks, base);
    let t = ts.type_expr()?;
    ts.expect_end()?;
    Ok(t)
}

struct Parser<'a> {
    lines: &'a [Line],
    pos: usize,
    diags: Vec<Diagnostic>,
}

type PResult<T> = Result<T, Diagnostic>;

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Line> {
        self.lines.get(self.pos)
    }

    /// 現在行の子ブロック(level > parent の連続行)の範囲を取り出して進める。
    fn take_children(&mut self, parent: u32) -> &'a [Line] {
        let start = self.pos;
        while let Some(l) = self.lines.get(self.pos) {
            if l.level <= parent {
                break;
            }
            self.pos += 1;
        }
        &self.lines[start..self.pos]
    }

    fn parse_top(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        let mut annotations = Vec::new();
        while let Some(line) = self.peek() {
            if line.level != 0 {
                self.diags
                    .push(Diagnostic::error(Code::SYN001, line.span, "予期しないインデントです").hint("トップレベルの宣言はインデントなしで書きます"));
                self.pos += 1;
                continue;
            }
            self.pos += 1;
            if line.text.starts_with('@') {
                match self.annotation(line) {
                    Ok(a) => annotations.push(a),
                    Err(d) => self.diags.push(d),
                }
                continue;
            }
            match self.decl(line) {
                Ok(decl) => items.push(Item {
                    annotations: std::mem::take(&mut annotations),
                    decl,
                }),
                Err(d) => {
                    self.diags.push(d);
                    annotations.clear();
                    // 見出しが壊れていても子ブロックは読み飛ばす
                    self.take_children(0);
                }
            }
        }
        if let Some(a) = annotations.first() {
            self.diags.push(Diagnostic::error(Code::SYN001, a.span, "注釈の後に宣言がありません"));
        }
        items
    }

    fn annotation(&mut self, line: &Line) -> PResult<Annotation> {
        let toks = tokenize(line)?;
        let mut ts = TokStream::new(&toks, line.span);
        ts.expect_sym(Sym::At)?;
        let name = ts.ident()?;
        let arg = if ts.eat_sym(Sym::LParen) {
            let a = ts.ident()?;
            ts.expect_sym(Sym::RParen)?;
            Some(a)
        } else {
            None
        };
        ts.expect_end()?;
        Ok(Annotation { name, arg, span: line.span })
    }

    fn decl(&mut self, line: &'a Line) -> PResult<Decl> {
        let kw = Keyword::from_word(first_word(&line.text));
        match kw {
            Some(Keyword::Knowledge) => return self.knowledge(line),
            Some(Keyword::Screen) => return self.screen(line),
            Some(Keyword::Contract) => return self.contract(line),
            _ => {}
        }
        let toks = tokenize(line)?;
        let mut ts = TokStream::new(&toks, line.span);
        match ts.next_kind() {
            Some(TokenKind::Keyword(Keyword::App)) => {
                let name = match ts.next_kind() {
                    Some(TokenKind::Str(s)) => s,
                    _ => return Err(ts.err_here("アプリ名は文字列(「商店」など)で書きます")),
                };
                ts.expect_end()?;
                self.no_children(line);
                Ok(Decl::App(AppDecl { name, span: line.span }))
            }
            Some(TokenKind::Keyword(Keyword::TypeKw)) => {
                let name = ts.ident()?;
                ts.expect_sym(Sym::Assign)?;
                let ty = ts.type_expr()?;
                ts.expect_end()?;
                self.no_children(line);
                Ok(Decl::TypeAlias(TypeAliasDecl { name, ty, span: line.span }))
            }
            Some(TokenKind::Keyword(Keyword::Enum)) => {
                let name = ts.ident()?;
                ts.expect_block_colon()?;
                let mut variants = Vec::new();
                for c in self.flat_children(line)? {
                    let t = tokenize(c)?;
                    let mut cs = TokStream::new(&t, c.span);
                    match cs.ident() {
                        Ok(v) => match cs.expect_end() {
                            Ok(()) => variants.push(v),
                            Err(d) => self.diags.push(d),
                        },
                        Err(d) => self.diags.push(d),
                    }
                }
                Ok(Decl::Enum(EnumDecl {
                    name,
                    variants,
                    span: line.span,
                }))
            }
            Some(TokenKind::Keyword(Keyword::Data)) => {
                let name = ts.ident()?;
                ts.expect_block_colon()?;
                let mut fields = Vec::new();
                for c in self.flat_children(line)? {
                    match field_line(c) {
                        Ok(f) => fields.push(f),
                        Err(d) => self.diags.push(d),
                    }
                }
                Ok(Decl::Data(DataDecl { name, fields, span: line.span }))
            }
            Some(TokenKind::Keyword(Keyword::Adapter)) => {
                let name = ts.ident()?;
                let params = ts.params()?;
                ts.expect_sym(Sym::Arrow)?;
                let ret = ts.type_expr()?;
                ts.expect_block_colon()?;
                let children = self.flat_children(line)?;
                let Some(c) = children.first() else { unreachable!() };
                if let Some(extra) = children.get(1) {
                    return Err(Diagnostic::error(
                        Code::SYN001,
                        extra.span,
                        "アダプタ本体は `操作 = 種別(データ)` の 1 行だけです",
                    ));
                }
                let t = tokenize(c)?;
                let mut cs = TokStream::new(&t, c.span);
                cs.expect_kw(Keyword::Op)?;
                cs.expect_sym(Sym::Assign)?;
                let op_id = cs.ident()?;
                let op = OpKind::from_word(&op_id.name).ok_or_else(|| {
                    Diagnostic::error(Code::SYN001, op_id.span, format!("未知の操作種別 `{}` です", op_id.name))
                        .hint("取得 / 一覧 / 登録 / 更新 / 削除 のいずれかです")
                })?;
                cs.expect_sym(Sym::LParen)?;
                let target = cs.ident()?;
                cs.expect_sym(Sym::RParen)?;
                cs.expect_end()?;
                Ok(Decl::Adapter(AdapterDecl {
                    name,
                    params,
                    ret,
                    op,
                    op_span: op_id.span,
                    target,
                    span: line.span,
                }))
            }
            Some(TokenKind::Keyword(Keyword::Action)) => {
                let name = ts.ident()?;
                let params = ts.params()?;
                let ret = if ts.eat_sym(Sym::Arrow) { Some(ts.type_expr()?) } else { None };
                ts.expect_block_colon()?;
                let items = self.action_body(line)?;
                Ok(Decl::Action(ActionDecl {
                    name,
                    params,
                    ret,
                    items,
                    span: line.span,
                }))
            }
            _ => Err(Diagnostic::error(Code::SYN001, line.span, "宣言として解釈できない行です")
                .hint("トップレベルには アプリ / 型 / 列挙 / データ / アダプタ / 知識 / 画面 / 動作 / 約束 のいずれかを書きます")),
        }
    }

    fn no_children(&mut self, line: &Line) {
        let ch = self.take_children(line.level);
        if let Some(c) = ch.first() {
            self.diags.push(Diagnostic::error(Code::SYN001, c.span, "予期しないインデントです"));
        }
    }

    /// 1 段だけの子行を返す(さらに深い行は SYN001)。空なら SYN001。
    fn flat_children(&mut self, line: &Line) -> PResult<Vec<&'a Line>> {
        let ch = self.take_children(line.level);
        if ch.is_empty() {
            return Err(Diagnostic::error(Code::SYN001, line.span, "ブロックが空です").hint("`:` の次の行から 1 段インデントして本体を書きます"));
        }
        let mut out = Vec::new();
        for c in ch {
            if c.level == line.level + 1 {
                out.push(c);
            } else {
                self.diags.push(Diagnostic::error(Code::SYN001, c.span, "予期しないインデントです"));
            }
        }
        Ok(out)
    }

    fn header_name(&self, line: &Line, kw: &str) -> PResult<(Ident, Span)> {
        let toks = tokenize(line)?;
        let mut ts = TokStream::new(&toks, line.span);
        let k = ts.next().ok_or_else(|| ts.err_here("宣言がありません"))?;
        debug_assert!(matches!(k.kind, TokenKind::Keyword(_)), "{kw}");
        let name = ts.ident()?;
        ts.expect_block_colon()?;
        Ok((name, line.span))
    }

    fn knowledge(&mut self, line: &'a Line) -> PResult<Decl> {
        let (name, span) = self.header_name(line, "知識")?;
        let mut entries = Vec::new();
        for c in self.flat_children(line)? {
            match kv_line(c) {
                Ok(e) => entries.push(e),
                Err(d) => self.diags.push(d),
            }
        }
        Ok(Decl::Knowledge(KnowledgeDecl { name, entries, span }))
    }

    fn screen(&mut self, line: &'a Line) -> PResult<Decl> {
        let (name, span) = self.header_name(line, "画面")?;
        let lines = self.raw_children(line)?;
        Ok(Decl::Screen(ScreenDecl { name, lines, span }))
    }

    fn contract(&mut self, line: &'a Line) -> PResult<Decl> {
        let toks = tokenize(line)?;
        let mut ts = TokStream::new(&toks, line.span);
        ts.expect_kw(Keyword::Contract)?;
        ts.expect_block_colon()?;
        let lines = self.raw_children(line)?;
        Ok(Decl::Contract(ContractDecl { lines, span: line.span }))
    }

    fn raw_children(&mut self, line: &Line) -> PResult<Vec<RawText>> {
        let mut out = Vec::new();
        for c in self.flat_children(line)? {
            if c.text.ends_with(':') {
                self.diags.push(Diagnostic::error(Code::SYN001, c.span, "意図行の中にブロックは書けません"));
                continue;
            }
            out.push(RawText {
                text: c.text.clone(),
                span: c.span,
            });
        }
        Ok(out)
    }

    fn action_body(&mut self, line: &'a Line) -> PResult<Vec<ActionItem>> {
        let ch = self.take_children(line.level);
        if ch.is_empty() {
            return Err(Diagnostic::error(Code::SYN001, line.span, "動作の本体が空です"));
        }
        let level = line.level + 1;
        let mut items = Vec::new();
        let mut i = 0;
        while i < ch.len() {
            let c = &ch[i];
            i += 1;
            if c.level != level {
                self.diags.push(Diagnostic::error(Code::SYN001, c.span, "予期しないインデントです"));
                continue;
            }
            let fw = first_word(&c.text);
            match Keyword::from_word(fw) {
                Some(Keyword::Requires) | Some(Keyword::Effects) => {
                    let r = tokenize(c).and_then(|t| {
                        let mut ts = TokStream::new(&t, c.span);
                        let is_req = matches!(ts.next_kind(), Some(TokenKind::Keyword(Keyword::Requires)));
                        ts.expect_sym(Sym::Assign)?;
                        ts.expect_sym(Sym::LBracket)?;
                        let mut paths: Vec<(Vec<Ident>, Span)> = Vec::new();
                        if !ts.eat_sym(Sym::RBracket) {
                            loop {
                                let first = ts.ident()?;
                                let mut segs = vec![first];
                                while ts.eat_sym(Sym::Dot) {
                                    segs.push(ts.ident()?);
                                }
                                let sp = segs[0].span.to(segs.last().unwrap().span);
                                paths.push((segs, sp));
                                if ts.eat_sym(Sym::RBracket) {
                                    break;
                                }
                                ts.expect_sym(Sym::Comma)?;
                            }
                        }
                        ts.expect_end()?;
                        if is_req {
                            let mut caps = Vec::new();
                            for (segs, sp) in paths {
                                if segs.len() != 2 {
                                    return Err(Diagnostic::error(Code::SYN001, sp, "Capability は `資源.動詞`(例 `商品.更新`)の形で書きます"));
                                }
                                let mut it = segs.into_iter();
                                caps.push(CapRef {
                                    resource: it.next().unwrap(),
                                    verb: it.next().unwrap(),
                                    span: sp,
                                });
                            }
                            Ok(ActionItem::Requires { caps, span: c.span })
                        } else {
                            let effects = paths
                                .into_iter()
                                .map(|(segs, sp)| EffectRef {
                                    path: segs.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("."),
                                    span: sp,
                                })
                                .collect();
                            Ok(ActionItem::Effects { effects, span: c.span })
                        }
                    });
                    match r {
                        Ok(it) => items.push(it),
                        Err(d) => self.diags.push(d),
                    }
                }
                Some(Keyword::Strict) => {
                    let header_ok = tokenize(c).and_then(|t| {
                        let mut ts = TokStream::new(&t, c.span);
                        ts.expect_kw(Keyword::Strict)?;
                        ts.expect_block_colon()
                    });
                    // 子ブロック
                    let start = i;
                    while i < ch.len() && ch[i].level > level {
                        i += 1;
                    }
                    let body = &ch[start..i];
                    if let Err(d) = header_ok {
                        self.diags.push(d);
                        continue;
                    }
                    match self.block(body, level + 1, c.span) {
                        Ok(b) => items.push(ActionItem::Strict(b)),
                        Err(d) => self.diags.push(d),
                    }
                }
                _ => {
                    if c.text.ends_with(':') {
                        self.diags
                            .push(Diagnostic::error(Code::SYN001, c.span, "動作本体で使えるブロックは `厳密:` だけです"));
                        while i < ch.len() && ch[i].level > level {
                            i += 1;
                        }
                        continue;
                    }
                    items.push(ActionItem::Intent(RawText {
                        text: c.text.clone(),
                        span: c.span,
                    }));
                }
            }
        }
        Ok(items)
    }

    /// 厳密層のブロック。`lines` はすべて level 以上。
    fn block(&mut self, lines: &'a [Line], level: u32, header: Span) -> PResult<Block> {
        if lines.is_empty() {
            return Err(Diagnostic::error(Code::SYN001, header, "ブロックが空です"));
        }
        let mut stmts = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let l = &lines[i];
            i += 1;
            if l.level != level {
                self.diags.push(Diagnostic::error(Code::SYN001, l.span, "予期しないインデントです"));
                continue;
            }
            // 子ブロック範囲
            let cstart = i;
            while i < lines.len() && lines[i].level > level {
                i += 1;
            }
            let children = &lines[cstart..i];
            let toks = match tokenize(l) {
                Ok(t) => t,
                Err(d) => {
                    self.diags.push(d);
                    continue;
                }
            };
            let mut ts = TokStream::new(&toks, l.span);
            let res: PResult<Stmt> = match ts.peek_kind() {
                Some(TokenKind::Keyword(Keyword::If)) => {
                    ts.next();
                    let cond = match ts.expr().and_then(|e| ts.expect_block_colon().map(|_| e)) {
                        Ok(c) => c,
                        Err(d) => {
                            self.diags.push(d);
                            continue;
                        }
                    };
                    let first = self.block(children, level + 1, l.span);
                    let mut branches = Vec::new();
                    let mut ok = true;
                    match first {
                        Ok(b) => branches.push((cond, b)),
                        Err(d) => {
                            self.diags.push(d);
                            ok = false;
                        }
                    }
                    let mut else_block = None;
                    let mut end_span = l.span;
                    // 後続の そうでなければ 行
                    while i < lines.len() && lines[i].level == level && first_word(&lines[i].text) == "そうでなければ" {
                        let el = &lines[i];
                        end_span = el.span;
                        i += 1;
                        let es = i;
                        while i < lines.len() && lines[i].level > level {
                            i += 1;
                        }
                        let ech = &lines[es..i];
                        let r = tokenize(el).and_then(|t| {
                            let mut es2 = TokStream::new(&t, el.span);
                            es2.expect_kw(Keyword::Else)?;
                            if es2.eat_kw(Keyword::If) {
                                let c = es2.expr()?;
                                es2.expect_block_colon()?;
                                Ok(Some(c))
                            } else {
                                es2.expect_block_colon()?;
                                Ok(None)
                            }
                        });
                        match r {
                            Ok(Some(c)) => {
                                if else_block.is_some() {
                                    self.diags
                                        .push(Diagnostic::error(Code::SYN001, el.span, "`そうでなければ:` の後に条件分岐は続けられません"));
                                }
                                match self.block(ech, level + 1, el.span) {
                                    Ok(b) => branches.push((c, b)),
                                    Err(d) => self.diags.push(d),
                                }
                            }
                            Ok(None) => {
                                if else_block.is_some() {
                                    self.diags.push(Diagnostic::error(Code::SYN001, el.span, "`そうでなければ:` が重複しています"));
                                }
                                match self.block(ech, level + 1, el.span) {
                                    Ok(b) => else_block = Some(b),
                                    Err(d) => self.diags.push(d),
                                }
                            }
                            Err(d) => self.diags.push(d),
                        }
                    }
                    if !ok {
                        continue;
                    }
                    Ok(Stmt::If {
                        branches,
                        else_block,
                        span: l.span.to(end_span),
                    })
                }
                Some(TokenKind::Keyword(Keyword::Else)) => Err(Diagnostic::error(Code::SYN001, l.span, "対応する `もし` がありません")),
                Some(TokenKind::Keyword(Keyword::Loop)) => {
                    ts.next();
                    let r = (|| {
                        let var = ts.ident()?;
                        ts.expect_kw(Keyword::From)?;
                        let iter = ts.expr()?;
                        ts.expect_block_colon()?;
                        Ok((var, iter))
                    })();
                    match r {
                        Ok((var, iter)) => self.block(children, level + 1, l.span).map(|body| Stmt::Loop { var, iter, body, span: l.span }),
                        Err(d) => Err(d),
                    }
                }
                Some(TokenKind::Keyword(Keyword::Transaction)) => {
                    ts.next();
                    match ts.expect_block_colon() {
                        Ok(()) => self.block(children, level + 1, l.span).map(|body| Stmt::Transaction { body, span: l.span }),
                        Err(d) => Err(d),
                    }
                }
                _ => {
                    if let Some(c) = children.first() {
                        self.diags.push(Diagnostic::error(Code::SYN001, c.span, "予期しないインデントです"));
                    }
                    simple_stmt(&mut ts, l)
                }
            };
            match res {
                Ok(s) => stmts.push(s),
                Err(d) => self.diags.push(d),
            }
        }
        let span = lines
            .first()
            .map(|l| l.span)
            .unwrap_or(header)
            .to(lines.last().map(|l| l.span).unwrap_or(header));
        Ok(Block { stmts, span })
    }
}

fn simple_stmt(ts: &mut TokStream, l: &Line) -> PResult<Stmt> {
    let e = ts.expr()?;
    if ts.eat_kw(Keyword::Check) {
        ts.expect_kw(Keyword::OrElse)?;
        let ftok = ts.peek().cloned();
        ts.expect_kw(Keyword::Fail)?;
        ts.expect_sym(Sym::LParen)?;
        let arg = ts.expr()?;
        ts.expect_sym(Sym::RParen)?;
        ts.expect_end()?;
        let fspan = ftok.map(|t| t.span).unwrap_or(l.span);
        let fail = Expr {
            kind: ExprKind::Call {
                callee: Ident {
                    name: "失敗".into(),
                    span: fspan,
                },
                args: vec![arg],
            },
            span: fspan.to(ts.last_span()),
        };
        return Ok(Stmt::Check { cond: e, fail, span: l.span });
    }
    if ts.eat_kw(Keyword::Return) {
        ts.expect_end()?;
        return Ok(Stmt::Return { value: e, span: l.span });
    }
    if ts.eat_sym(Sym::Assign) {
        let target = to_place(&e)?;
        let value = ts.expr()?;
        ts.expect_end()?;
        return Ok(Stmt::Assign { target, value, span: l.span });
    }
    ts.expect_end()?;
    match &e.kind {
        ExprKind::Call { .. } => Ok(Stmt::Expr { expr: e, span: l.span }),
        ExprKind::Ident(name) if kotonoha_lexer::SUFFIX_KEYWORDS.iter().any(|s| name.ends_with(s) && name.len() > s.len()) => {
            let kw = kotonoha_lexer::SUFFIX_KEYWORDS.iter().find(|s| name.ends_with(*s)).unwrap();
            let stem = &name[..name.len() - kw.len()];
            Err(Diagnostic::error(Code::SYN001, e.span, format!("`{name}` は 1 つの語として解釈されます"))
                .hint(format!("`{stem} {kw}` のように予約語の前に空白を入れてください(助詞は自動分割しません)")))
        }
        _ => Err(Diagnostic::error(Code::SYN001, e.span, "文として使えない式です").hint("式だけの文は関数・動作の呼出に限られます")),
    }
}

fn to_place(e: &Expr) -> PResult<Place> {
    let mut fields = Vec::new();
    let mut cur = e;
    loop {
        match &cur.kind {
            ExprKind::Field(base, f) => {
                fields.push(f.clone());
                cur = base;
            }
            ExprKind::Ident(n) => {
                fields.reverse();
                return Ok(Place {
                    root: Ident {
                        name: n.clone(),
                        span: cur.span,
                    },
                    fields,
                    span: e.span,
                });
            }
            _ => return Err(Diagnostic::error(Code::SYN001, e.span, "代入先は変数かフィールドでなければなりません")),
        }
    }
}

fn field_line(c: &Line) -> PResult<Field> {
    let t = tokenize(c)?;
    let mut ts = TokStream::new(&t, c.span);
    let name = ts.ident()?;
    ts.expect_sym(Sym::Colon)?;
    let ty = ts.type_expr()?;
    ts.expect_end()?;
    Ok(Field { name, ty, span: c.span })
}

/// `キー = 値`(値は生テキスト)。`==` はキーの区切りとみなさない。
fn kv_line(c: &Line) -> PResult<KvEntry> {
    let key_text = first_word(&c.text);
    if key_text.is_empty() {
        return Err(Diagnostic::error(Code::SYN001, c.span, "`キー = 値` の形で書きます"));
    }
    let col = c.span.column;
    let key = Ident {
        name: key_text.to_string(),
        span: Span::new(c.span.file, c.span.line, col, c.span.start, c.span.start + key_text.len() as u32),
    };
    let rest = &c.text[key_text.len()..];
    let trimmed = rest.trim_start_matches(' ');
    if !trimmed.starts_with('=') || trimmed.starts_with("==") {
        return Err(Diagnostic::error(Code::SYN001, c.span, "`キー = 値` の形で書きます"));
    }
    let after_eq = &trimmed[1..];
    let value_text = after_eq.trim_start_matches(' ');
    if value_text.is_empty() {
        return Err(Diagnostic::error(Code::SYN001, c.span, format!("`{}` の値がありません", key.name)));
    }
    let vstart = c.text.len() - value_text.len();
    let vcol = col + c.text[..vstart].chars().count() as u32;
    let value = RawText {
        text: value_text.to_string(),
        span: Span::new(c.span.file, c.span.line, vcol, c.span.start + vstart as u32, c.span.end),
    };
    Ok(KvEntry { key, value, span: c.span })
}

/// トークン列の読み取り器。
struct TokStream<'t> {
    toks: &'t [Token],
    pos: usize,
    line: Span,
}

impl<'t> TokStream<'t> {
    fn new(toks: &'t [Token], line: Span) -> Self {
        TokStream { toks, pos: 0, line }
    }
    fn peek(&self) -> Option<&'t Token> {
        self.toks.get(self.pos)
    }
    fn peek_kind(&self) -> Option<&'t TokenKind> {
        self.peek().map(|t| &t.kind)
    }
    fn next(&mut self) -> Option<&'t Token> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn next_kind(&mut self) -> Option<TokenKind> {
        self.next().map(|t| t.kind.clone())
    }
    fn last_span(&self) -> Span {
        if self.pos == 0 {
            self.line
        } else {
            self.toks[self.pos - 1].span
        }
    }
    fn here(&self) -> Span {
        self.peek().map(|t| t.span).unwrap_or_else(|| {
            let mut s = self.line;
            s.column += 1;
            s.start = s.end;
            s
        })
    }
    fn err_here(&self, msg: impl Into<String>) -> Diagnostic {
        Diagnostic::error(Code::SYN001, self.here(), msg)
    }
    fn describe(&self) -> String {
        match self.peek_kind() {
            None => "行末".to_string(),
            Some(TokenKind::Ident(s)) => format!("`{s}`"),
            Some(TokenKind::Keyword(k)) => format!("予約語 `{}`", k.as_str()),
            Some(TokenKind::Int(v)) => format!("`{v}`"),
            Some(TokenKind::Decimal(v)) => format!("`{v}`"),
            Some(TokenKind::Str(s)) => format!("文字列 \"{s}\""),
            Some(TokenKind::Sym(s)) => format!("`{}`", s.as_str()),
        }
    }
    fn eat_sym(&mut self, s: Sym) -> bool {
        if self.peek_kind() == Some(&TokenKind::Sym(s)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, k: Keyword) -> bool {
        if self.peek_kind() == Some(&TokenKind::Keyword(k)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect_sym(&mut self, s: Sym) -> PResult<()> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            Err(self.err_here(format!("`{}` が必要ですが {} があります", s.as_str(), self.describe())))
        }
    }
    fn expect_kw(&mut self, k: Keyword) -> PResult<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            Err(self.err_here(format!("`{}` が必要ですが {} があります", k.as_str(), self.describe())))
        }
    }
    fn expect_end(&mut self) -> PResult<()> {
        if self.peek().is_none() {
            Ok(())
        } else {
            Err(self.err_here(format!("行末が必要ですが {} があります", self.describe())))
        }
    }
    fn expect_block_colon(&mut self) -> PResult<()> {
        self.expect_sym(Sym::Colon)?;
        self.expect_end()
    }
    fn ident(&mut self) -> PResult<Ident> {
        match self.peek() {
            Some(Token {
                kind: TokenKind::Ident(n),
                span,
            }) => {
                self.pos += 1;
                Ok(Ident { name: n.clone(), span: *span })
            }
            Some(Token {
                kind: TokenKind::Keyword(k), ..
            }) => Err(self.err_here(format!("予約語 `{}` は名前に使えません", k.as_str()))),
            _ => Err(self.err_here(format!("名前が必要ですが {} があります", self.describe()))),
        }
    }

    fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect_sym(Sym::LParen)?;
        let mut ps = Vec::new();
        if self.eat_sym(Sym::RParen) {
            return Ok(ps);
        }
        loop {
            let name = self.ident()?;
            self.expect_sym(Sym::Colon)?;
            let ty = self.type_expr()?;
            let span = name.span.to(ty.span());
            ps.push(Param { name, ty, span });
            if self.eat_sym(Sym::RParen) {
                return Ok(ps);
            }
            self.expect_sym(Sym::Comma)?;
        }
    }

    fn type_expr(&mut self) -> PResult<TypeExpr> {
        let start = self.here();
        if self.eat_kw(Keyword::Record) {
            self.expect_sym(Sym::LBrace)?;
            let mut fields = Vec::new();
            if !self.eat_sym(Sym::RBrace) {
                loop {
                    let name = self.ident()?;
                    self.expect_sym(Sym::Colon)?;
                    let ty = self.type_expr()?;
                    let span = name.span.to(ty.span());
                    fields.push(Field { name, ty, span });
                    if self.eat_sym(Sym::RBrace) {
                        break;
                    }
                    self.expect_sym(Sym::Comma)?;
                }
            }
            return Ok(TypeExpr::Record {
                fields,
                span: start.to(self.last_span()),
            });
        }
        let name = self
            .ident()
            .map_err(|_| self.err_here(format!("型が必要ですが {} があります", self.describe())))?;
        let mut args = Vec::new();
        if self.eat_sym(Sym::Lt) {
            loop {
                args.push(self.type_expr()?);
                if self.eat_sym(Sym::Gt) {
                    break;
                }
                self.expect_sym(Sym::Comma)?;
            }
        }
        Ok(TypeExpr::Named {
            span: name.span.to(self.last_span()),
            name,
            args,
        })
    }

    // ---- 式(優先順位は docs/spec/grammar.md §4) ----
    fn expr(&mut self) -> PResult<Expr> {
        self.binary(1)
    }

    fn peek_binop(&self) -> Option<BinOp> {
        Some(match self.peek_kind()? {
            TokenKind::Sym(Sym::OrOr) => BinOp::Or,
            TokenKind::Sym(Sym::AndAnd) => BinOp::And,
            TokenKind::Sym(Sym::EqEq) => BinOp::Eq,
            TokenKind::Sym(Sym::Ne) => BinOp::Ne,
            TokenKind::Sym(Sym::Lt) => BinOp::Lt,
            TokenKind::Sym(Sym::Le) => BinOp::Le,
            TokenKind::Sym(Sym::Gt) => BinOp::Gt,
            TokenKind::Sym(Sym::Ge) => BinOp::Ge,
            TokenKind::Sym(Sym::Plus) => BinOp::Add,
            TokenKind::Sym(Sym::Minus) => BinOp::Sub,
            TokenKind::Sym(Sym::Star) => BinOp::Mul,
            TokenKind::Sym(Sym::Slash) => BinOp::Div,
            _ => return None,
        })
    }

    fn binary(&mut self, min_prec: u8) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        while let Some(op) = self.peek_binop() {
            let prec = op.precedence();
            if prec < min_prec {
                break;
            }
            self.pos += 1;
            let rhs = self.binary(prec + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
            if op.is_non_assoc() {
                if let Some(next) = self.peek_binop() {
                    if next.precedence() == prec {
                        return Err(self
                            .err_here(format!("比較演算子 `{}` は連結できません", next.symbol()))
                            .hint("括弧か && で分けてください"));
                    }
                }
            }
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<Expr> {
        let start = self.here();
        if self.eat_sym(Sym::Minus) {
            let e = self.unary()?;
            let span = start.to(e.span);
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(e),
                },
                span,
            });
        }
        if self.eat_sym(Sym::Bang) {
            let e = self.unary()?;
            let span = start.to(e.span);
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(e),
                },
                span,
            });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        while self.eat_sym(Sym::Dot) {
            let f = self.ident()?;
            let span = e.span.to(f.span);
            e = Expr {
                kind: ExprKind::Field(Box::new(e), f),
                span,
            };
            if self.peek_kind() == Some(&TokenKind::Sym(Sym::LParen)) {
                return Err(self.err_here("メソッド呼出 `x.f(...)` は未対応です"));
            }
        }
        Ok(e)
    }

    fn call_args(&mut self) -> PResult<Vec<Expr>> {
        let mut args = Vec::new();
        if self.eat_sym(Sym::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.expr()?);
            if self.eat_sym(Sym::RParen) {
                return Ok(args);
            }
            self.expect_sym(Sym::Comma)?;
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let Some(tok) = self.next() else {
            return Err(self.err_here("式が必要ですが行末です"));
        };
        let span = tok.span;
        let kind = match &tok.kind {
            TokenKind::Int(v) => ExprKind::Int(*v),
            TokenKind::Decimal(v) => ExprKind::Decimal(v.clone()),
            TokenKind::Str(s) => ExprKind::Str(s.clone()),
            TokenKind::Keyword(Keyword::True) => ExprKind::Bool(true),
            TokenKind::Keyword(Keyword::False) => ExprKind::Bool(false),
            TokenKind::Keyword(Keyword::NoneKw) => ExprKind::None,
            TokenKind::Keyword(k @ (Keyword::Fail | Keyword::Ok)) => {
                self.expect_sym(Sym::LParen)?;
                let args = self.call_args()?;
                let callee = Ident {
                    name: k.as_str().to_string(),
                    span,
                };
                return Ok(Expr {
                    kind: ExprKind::Call { callee, args },
                    span: span.to(self.last_span()),
                });
            }
            TokenKind::Ident(n) => {
                if self.eat_sym(Sym::LParen) {
                    let args = self.call_args()?;
                    let callee = Ident { name: n.clone(), span };
                    return Ok(Expr {
                        kind: ExprKind::Call { callee, args },
                        span: span.to(self.last_span()),
                    });
                }
                ExprKind::Ident(n.clone())
            }
            TokenKind::Sym(Sym::LParen) => {
                let e = self.expr()?;
                self.expect_sym(Sym::RParen)?;
                return Ok(Expr {
                    kind: e.kind,
                    span: span.to(self.last_span()),
                });
            }
            TokenKind::Sym(Sym::LBracket) => {
                let mut items = Vec::new();
                if !self.eat_sym(Sym::RBracket) {
                    loop {
                        items.push(self.expr()?);
                        if self.eat_sym(Sym::RBracket) {
                            break;
                        }
                        self.expect_sym(Sym::Comma)?;
                    }
                }
                return Ok(Expr {
                    kind: ExprKind::List(items),
                    span: span.to(self.last_span()),
                });
            }
            TokenKind::Keyword(k) => {
                self.pos -= 1;
                return Err(self.err_here(format!("予約語 `{}` は式として使えません", k.as_str())));
            }
            TokenKind::Sym(s) => {
                self.pos -= 1;
                return Err(self.err_here(format!("式が必要ですが `{}` があります", s.as_str())));
            }
        };
        Ok(Expr { kind, span })
    }
}

#[cfg(test)]
mod tests;
