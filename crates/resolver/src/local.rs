//! 手元で動くオープンモデル(Ollama / llama.cpp server / vLLM など、OpenAI 互換の
//! `/chat/completions` を持つもの)に候補を出させるアダプタ(ADR-0127)。
//!
//! - 送信先は**この機械の中(ループバック)だけ**。外部ホストは ADR-006(外部送信の方針)が未決定のため拒否する。
//!   したがってソースが機械の外に出ることはなく、`sends_source()` は false。
//! - 送るのは「解釈する文・確認事項・使える正規文の例(宣言名を含む)・過去に承認された言い換え」だけ。
//!   ソース本文・知識の値・データの中身は送らない。
//! - モデルの出力は未信頼データ。JSON を取り出して [`crate::validate_candidate`] に渡すだけで、評価・実行はしない。
//!   さらに driver が既知の文型で再照合し(成立しない解釈は除外)、人の承認を経て初めて使われる。

use crate::learn::{Memory, PromptContext};
use crate::ResolverAdapter;
use kotonoha_intent::Unresolved;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434/v1";

/// `http://host:port/path` を分解する。ループバック以外・https は拒否する。
pub fn parse_local_endpoint(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        format!("接続先 `{url}` は使えません。手元で動くモデルの http://127.0.0.1:ポート/… を指定してください(外部への送信は ADR-006 が未決定のため不可)")
    })?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].trim_end_matches('/').to_string()),
        None => (rest, String::new()),
    };
    let (host, port) = if let Some(h) = hostport.strip_prefix('[') {
        let end = h.find(']').ok_or("IPv6 アドレスの ] がありません")?;
        let port = h[end + 1..].strip_prefix(':').unwrap_or("80");
        (h[..end].to_string(), port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p),
            None => (hostport.to_string(), "80"),
        }
    };
    let port: u16 = port.parse().map_err(|_| format!("ポート `{port}` が不正です"))?;
    let loopback = host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if !loopback {
        return Err(format!(
            "接続先 `{host}` はこの機械の外です。ソースを外部に送る方針(ADR-006)は未決定のため、手元(127.0.0.1 / localhost)のモデルだけ使えます"
        ));
    }
    Ok((host, port, path))
}

/// 最小の HTTP/1.1 POST(ループバック専用)。Content-Length と chunked の応答に対応。
pub fn post_json(endpoint: &str, path: &str, body: &Value, timeout: Duration, api_key: Option<&str>) -> Result<Value, String> {
    let (host, port, base) = parse_local_endpoint(endpoint)?;
    let addr = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("{host}:{port} を解決できません: {e}"))?
        .find(|a| a.ip().is_loopback())
        .ok_or_else(|| format!("{host} がループバックアドレスに解決されません"))?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| format!("モデルのサーバー({endpoint})に接続できません: {e}。Ollama なら `ollama serve` と `ollama pull <モデル>` を実行してください"))?;
    s.set_read_timeout(Some(timeout)).ok();
    s.set_write_timeout(Some(Duration::from_secs(30))).ok();
    let payload = serde_json::to_vec(body).unwrap();
    let auth = api_key.map(|k| format!("Authorization: Bearer {k}\r\n")).unwrap_or_default();
    let head = format!(
        "POST {base}{path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nAccept: application/json\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes())
        .and_then(|_| s.write_all(&payload))
        .map_err(|e| format!("送信に失敗しました: {e}"))?;
    let mut raw = Vec::new();
    s.take(16 * 1024 * 1024)
        .read_to_end(&mut raw)
        .map_err(|e| format!("モデルの応答を待つ間に時間切れか切断がありました: {e}"))?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("HTTP 応答の形式が不正です")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut body = raw[split + 4..].to_vec();
    let status: u16 = head.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(0);
    let lower = head.to_ascii_lowercase();
    if lower.contains("transfer-encoding: chunked") {
        body = dechunk(&body)?;
    }
    let text = String::from_utf8_lossy(&body);
    if !(200..300).contains(&status) {
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("モデルのサーバーがエラーを返しました(HTTP {status}): {snippet}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("モデルのサーバーの応答が JSON ではありません: {e}"))
}

fn dechunk(b: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let nl = b[i..].windows(2).position(|w| w == b"\r\n").ok_or("chunked 応答が途中で切れています")?;
        let size_s = String::from_utf8_lossy(&b[i..i + nl]).to_string();
        let size = usize::from_str_radix(size_s.split(';').next().unwrap_or("").trim(), 16).map_err(|_| "chunked のサイズが不正です")?;
        i += nl + 2;
        if size == 0 {
            return Ok(out);
        }
        if i + size > b.len() {
            return Err("chunked 応答が途中で切れています".into());
        }
        out.extend_from_slice(&b[i..i + size]);
        i += size + 2;
    }
}

/// モデルの文章から JSON オブジェクトを 1 つ取り出す(```json の囲みや前後の説明文を許す)。
pub fn extract_json_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str::<Value>(&text[start..=end]).ok().filter(|v| v.is_object())
}

/// 手元のオープンモデルを使うアダプタ。過去の承認(学習記録)を例として添え、使うほど当たりやすくなる。
pub struct LocalModelAdapter {
    pub endpoint: String,
    pub model: String,
    pub timeout: Duration,
    pub api_key: Option<String>,
    pub screen_examples: Vec<String>,
    pub contract_examples: Vec<String>,
    pub memory: Memory,
}

impl LocalModelAdapter {
    pub fn context_for(&self, u: &Unresolved) -> PromptContext {
        let pool = if u.owner.starts_with("action.") {
            &self.contract_examples
        } else {
            &self.screen_examples
        };
        PromptContext::new(&u.sentence, &u.questions, pool)
    }

    /// 1 文の解釈をモデルに問い合わせ、未検証の候補 JSON(validate_candidate の形)を返す。
    pub fn ask(&self, u: &Unresolved, ctx: &PromptContext, memory: &Memory) -> Result<Value, String> {
        let messages = crate::learn::build_messages(ctx, &memory.similar(&u.sentence, 5));
        let body = json!({
            "model": self.model,
            "messages": messages,
            "temperature": 0,
            "seed": 0,
            "stream": false,
            "response_format": {"type": "json_object"},
        });
        let resp = post_json(&self.endpoint, "/chat/completions", &body, self.timeout, self.api_key.as_deref())?;
        let content = resp
            .pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .ok_or("モデルの応答に choices[0].message.content がありません")?;
        let mut v = extract_json_object(content).ok_or_else(|| {
            let s: String = content.chars().take(200).collect();
            format!("モデルの出力から JSON を取り出せませんでした: {s}")
        })?;
        let o = v.as_object_mut().unwrap();
        // ID と根拠はこちらで付ける(モデルが書いた値は使わない)。足りない配列は空で補う。
        o.insert("intent_id".into(), Value::String(u.intent_id.clone()));
        o.insert("sentence_hash".into(), Value::String(u.sentence_hash.clone()));
        for k in ["assumptions", "unresolved", "required_effects"] {
            o.entry(k).or_insert(json!([]));
        }
        o.insert("evidence_refs".into(), json!([format!("手元のモデル {}", self.model)]));
        Ok(v)
    }
}

impl ResolverAdapter for LocalModelAdapter {
    fn name(&self) -> &'static str {
        "local"
    }
    fn sends_source(&self) -> bool {
        // ループバックだけに送る(parse_local_endpoint が外部を拒否する)
        false
    }
    fn propose(&self, u: &Unresolved) -> Result<Value, String> {
        let ctx = self.context_for(u);
        let mut v = self.ask(u, &ctx, &self.memory)?;
        // 過去に同じ文で承認された解釈があれば先頭に置く(最終判断は人の承認)
        let past = self.memory.exact(&u.sentence);
        if !past.is_empty() {
            let it = v["interpretations"].as_array().cloned().unwrap_or_default();
            let mut merged: Vec<Value> = past
                .iter()
                .map(|(c, n)| json!({"canonical": c, "rationale": format!("同じ文が過去に {n} 回この解釈で承認されています")}))
                .collect();
            for x in it {
                if !merged.iter().any(|m| m["canonical"] == x["canonical"]) {
                    merged.push(x);
                }
            }
            v["interpretations"] = Value::Array(merged);
        }
        Ok(v)
    }
}

/// モデルを使わず、学習記録だけから候補を出すアダプタ(同じ文が過去に承認されていれば、その解釈を出す)。
pub struct MemoryAdapter {
    pub memory: Memory,
}

impl ResolverAdapter for MemoryAdapter {
    fn name(&self) -> &'static str {
        "memory"
    }
    fn sends_source(&self) -> bool {
        false
    }
    fn propose(&self, u: &Unresolved) -> Result<Value, String> {
        let past = self.memory.exact(&u.sentence);
        if past.is_empty() {
            return Err(format!("「{}」は学習記録にありません", u.sentence));
        }
        Ok(json!({
            "intent_id": u.intent_id,
            "sentence_hash": u.sentence_hash,
            "interpretations": past.iter().map(|(c, n)| json!({"canonical": c, "rationale": format!("同じ文が過去に {n} 回この解釈で承認されています")})).collect::<Vec<_>>(),
            "assumptions": [],
            "unresolved": [],
            "required_effects": [],
            "evidence_refs": ["学習記録"],
        }))
    }
}
