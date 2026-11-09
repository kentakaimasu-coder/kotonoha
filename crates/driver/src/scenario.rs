//! 「確かめる」(ADR-0130): 日本語で書いた確認の手順を検査し、backend で実行できる形(JSON)にする。
//!
//! 書ける行:
//! - `{人}(役割)が {動作}(引数, …) → 成功` / `→ 失敗("満席")` / `→ 失敗` / `→ ログインが必要` / `→ 権限がない` / `→ 重複` / `→ 見つからない`
//!   人は 名前(役割)・名前(役割を省くと一般の利用者)・`ログインしていない人`
//! - `{人}から {データ}が {N}件見える`(一覧 API で、その人に見える件数)
//! - `{データ}({ID})の{フィールド}は {値}`(データベースの値。だれから見えるかに関係なく確かめる)
//!
//! 引数の値は、動作の引数の型に合わせて読む(数・真/偽・文字列・日付 2026-12-05・日時 2026-12-05 19:00・なし)。
//! 確かめる手順はアプリの意味(VAG)には入らない(生成物の tests/ にだけ出る)。

use kotonoha_ast::{Decl, SourceFile};
use kotonoha_diag::{Code, Diagnostic, Span};
use kotonoha_types::{Env, Type};
use serde_json::{json, Value};

const ARROWS: &[&str] = &["→", "->"];

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => {
                quote = None;
                cur.push(c);
            }
            (Some(_), _) => cur.push(c),
            (None, '「') => {
                quote = Some('」');
                cur.push(c);
            }
            (None, '"') => {
                quote = Some('"');
                cur.push(c);
            }
            (None, '(') => {
                depth += 1;
                cur.push(c);
            }
            (None, ')') => {
                depth -= 1;
                cur.push(c);
            }
            (None, ',' | '、') if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn unquote(s: &str) -> Option<&str> {
    s.strip_prefix('「')
        .and_then(|x| x.strip_suffix('」'))
        .or_else(|| s.strip_prefix('"').and_then(|x| x.strip_suffix('"')))
}

/// 値を型に合わせて JSON にする。読めなければ理由。
fn value(raw: &str, t: &Type, env: &Env) -> Result<Value, String> {
    let raw = raw.trim();
    if let Type::Option(inner) = t {
        if raw == "なし" {
            return Ok(Value::Null);
        }
        return value(raw, inner, env);
    }
    let text = unquote(raw).unwrap_or(raw);
    match t {
        Type::Int | Type::Money(_) => {
            let n: String = text.chars().filter(|c| *c != ',' && *c != '円' && *c != '¥').collect();
            n.parse::<i64>().map(|x| json!(x)).map_err(|_| format!("「{raw}」は数ではありません"))
        }
        Type::Bool => match text {
            "真" | "はい" => Ok(json!(true)),
            "偽" | "いいえ" => Ok(json!(false)),
            _ => Err(format!("「{raw}」は 真 / 偽 ではありません")),
        },
        Type::Enum(e) => {
            if env.enum_has(e, text) {
                Ok(json!(text))
            } else {
                Err(format!("「{text}」は列挙 `{e}` の値ではありません(値: {})", env.enums[e].join(" / ")))
            }
        }
        Type::Date => {
            let d = text.replace('/', "-");
            if d.len() == 10 && d.as_bytes()[4] == b'-' && d.as_bytes()[7] == b'-' {
                Ok(json!(d))
            } else {
                Err(format!("日付は 2026-12-05 の形で書きます(「{raw}」)"))
            }
        }
        Type::DateTime => {
            // 「2026-12-05 19:00」は日本時間として読む
            let d = text.replace('/', "-");
            let (date, time) = d.split_once([' ', 'T']).unwrap_or((&d, "00:00"));
            let ok = date.len() == 10 && time.len() >= 4 && time.contains(':');
            if !ok {
                return Err(format!("日時は 2026-12-05 19:00 の形で書きます(「{raw}」)"));
            }
            let (h, m) = time.split_once(':').unwrap();
            Ok(json!(format!("{date}T{:0>2}:{:0>2}:00+09:00", h, &m[..2.min(m.len())])))
        }
        _ => Ok(json!(text)),
    }
}

struct Ctx<'a> {
    env: &'a Env,
    diags: Vec<Diagnostic>,
}

impl Ctx<'_> {
    fn err(&mut self, span: Span, msg: String) {
        self.diags.push(
            Diagnostic::error(Code::SYN001, span, msg)
                .hint("確かめるの書き方: 「花子(利用者)が 予約する(s1, 2) → 成功」「花子から 予約が 1件見える」「枠(s1)の予約数は 2」"),
        );
    }

    fn person(&mut self, who: &str, span: Span) -> Option<Value> {
        let who = who.trim();
        if who == "ログインしていない人" || who == "だれか" {
            return Some(json!({"label": who}));
        }
        if let Some((name, role)) = who.strip_suffix(')').and_then(|x| x.split_once('(')) {
            if !self.env.roles.contains_key(role) {
                let names: Vec<&String> = self.env.roles.keys().collect();
                self.diags
                    .push(Diagnostic::error(Code::REF001, span, format!("役割 `{role}` はありません")).hint(format!(
                        "役割: {}",
                        if names.is_empty() {
                            "なし".into()
                        } else {
                            names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" / ")
                        }
                    )));
                return None;
            }
            return Some(json!({"label": who, "sub": name.trim(), "role": role}));
        }
        Some(json!({"label": who, "sub": who}))
    }

    fn line(&mut self, text: &str, span: Span) -> Option<Value> {
        let text = text.trim();
        // 1. 動作の結果
        if let Some((left, expect)) = ARROWS.iter().find_map(|a| text.split_once(a)) {
            let (who, call) = left.split_once('が')?;
            let call = call.trim();
            let (name, args) = call.strip_suffix(')').and_then(|x| x.split_once('(')).unwrap_or((call, ""));
            let name = name.trim();
            let Some(sig) = self.env.actions.get(name) else {
                self.diags
                    .push(Diagnostic::error(Code::REF001, span, format!("動作 `{name}` は定義されていません")));
                return None;
            };
            let raw = split_args(args);
            if raw.len() != sig.params.len() {
                self.diags.push(Diagnostic::error(
                    Code::TYP001,
                    span,
                    format!(
                        "動作 `{name}` の引数は {} 個です({})",
                        sig.params.len(),
                        sig.params.iter().map(|(n, t)| format!("{n}: {t}")).collect::<Vec<_>>().join(", ")
                    ),
                ));
                return None;
            }
            let mut body = serde_json::Map::new();
            for ((n, t), r) in sig.params.iter().zip(&raw) {
                match value(r, t, self.env) {
                    Ok(v) => {
                        body.insert(n.clone(), v);
                    }
                    Err(e) => {
                        self.diags.push(Diagnostic::error(Code::TYP001, span, format!("引数 `{n}`: {e}")));
                        return None;
                    }
                }
            }
            let expect = expect.trim();
            let exp = match expect {
                "成功" => json!({"status": 200}),
                "失敗" => json!({"status": 422}),
                "ログインが必要" => json!({"status": 401}),
                "権限がない" => json!({"status": 403}),
                "重複" | "既に登録されている" => json!({"status": 409}),
                "見つからない" => json!({"status": 404}),
                _ => match expect
                    .strip_prefix("失敗(")
                    .and_then(|x| x.strip_suffix(')'))
                    .map(|x| unquote(x.trim()).unwrap_or(x.trim()))
                {
                    Some(v) => json!({"status": 422, "value": v}),
                    None => {
                        self.err(
                            span,
                            format!("結果「{expect}」は 成功 / 失敗(\"値\") / ログインが必要 / 権限がない / 重複 / 見つからない のどれかです"),
                        );
                        return None;
                    }
                },
            };
            let who = self.person(who, span)?;
            return Some(json!({"kind": "action", "text": text, "who": who, "action": name, "body": body, "expect": exp}));
        }
        // 2. 見える件数
        if let Some(rest) = text.strip_suffix("見える").or_else(|| text.strip_suffix("見られる")) {
            let rest = rest.trim().trim_end_matches('件').trim();
            let (head, n) = rest.rsplit_once('が')?;
            let count: String = n.trim().trim_end_matches('件').trim().to_string();
            let count: u64 = match count.parse() {
                Ok(c) => c,
                Err(_) => {
                    self.err(span, format!("件数「{count}」が数ではありません"));
                    return None;
                }
            };
            let (who, data) = head.split_once("から")?;
            let data = data.trim();
            if !self.env.data.contains_key(data) {
                self.diags
                    .push(Diagnostic::error(Code::REF001, span, format!("データ `{data}` は定義されていません")));
                return None;
            }
            let who = self.person(who, span)?;
            return Some(json!({"kind": "count", "text": text, "who": who, "data": data, "count": count}));
        }
        // 3. データの値
        if let Some((left, v)) = text.rsplit_once('は') {
            if let Some((d, rest)) = left.split_once('(') {
                if let Some((id, f)) = rest.split_once(")の") {
                    let d = d.trim();
                    let Some(info) = self.env.data.get(d) else {
                        self.diags
                            .push(Diagnostic::error(Code::REF001, span, format!("データ `{d}` は定義されていません")));
                        return None;
                    };
                    let f = f.trim();
                    let Some(t) = info.field(f).cloned() else {
                        self.diags
                            .push(Diagnostic::error(Code::REF001, span, format!("データ `{d}` にフィールド `{f}` はありません")));
                        return None;
                    };
                    return match value(v, &t, self.env) {
                        Ok(val) => {
                            Some(json!({"kind": "field", "text": text, "data": d, "id": unquote(id.trim()).unwrap_or(id.trim()), "field": f, "value": val}))
                        }
                        Err(e) => {
                            self.diags.push(Diagnostic::error(Code::TYP001, span, e));
                            None
                        }
                    };
                }
            }
        }
        self.err(span, format!("「{text}」は確かめるの書き方として読めません"));
        None
    }
}

/// ソースの「確かめる」をすべて検査し、実行用の JSON(無ければ空の配列)を返す。
pub fn compile(asts: &[SourceFile], env: &Env) -> (Value, Vec<Diagnostic>) {
    let mut c = Ctx { env, diags: Vec::new() };
    let mut out = Vec::new();
    for f in asts {
        for it in &f.items {
            if let Decl::Scenario(s) = &it.decl {
                let steps: Vec<Value> = s.lines.iter().filter_map(|l| c.line(&l.text, l.span)).collect();
                out.push(json!({"name": s.name.name, "steps": steps}));
            }
        }
    }
    (Value::Array(out), c.diags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_and_values() {
        assert_eq!(split_args("s1, 「a, b」, 3"), vec!["s1", "「a, b」", "3"]);
        assert!(split_args("").is_empty());
        let env = Env::default();
        assert_eq!(value("1,000", &Type::Int, &env).unwrap(), json!(1000));
        assert_eq!(value("2026-12-05 9:30", &Type::DateTime, &env).unwrap(), json!("2026-12-05T09:30:00+09:00"));
        assert_eq!(value("なし", &Type::Option(Box::new(Type::Str)), &env).unwrap(), Value::Null);
        assert!(value("たくさん", &Type::Int, &env).is_err());
    }
}
