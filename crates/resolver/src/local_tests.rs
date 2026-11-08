//! 手元のモデルのアダプタと学習記録のテスト(模擬の OpenAI 互換サーバーを使う)。

use crate::learn::*;
use crate::local::*;
use crate::{validate_candidate, ResolverAdapter};
use kotonoha_diag::Span;
use kotonoha_intent::{Mode, Unresolved};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

fn u(sentence: &str) -> Unresolved {
    Unresolved {
        intent_id: "screen.人気#000000000000".into(),
        owner: "screen.人気".into(),
        sentence: sentence.into(),
        sentence_hash: "abc".into(),
        mode: Mode::FlexConfirm,
        questions: vec!["「人気商品」は何を指しますか".into()],
        span: Span::default(),
    }
}

/// 1 回だけ応答する模擬サーバー。受け取った要求本文を返すチャネルも返す。
fn mock(content: &str, chunked: bool) -> (String, mpsc::Receiver<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let body = serde_json::to_string(&json!({"choices": [{"message": {"role": "assistant", "content": content}}]})).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            let n = s.read(&mut tmp).unwrap();
            buf.extend_from_slice(&tmp[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                let len: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .map(|x| x.trim().parse().unwrap())
                    .unwrap_or(0);
                if buf.len() >= i + 4 + len {
                    tx.send(String::from_utf8_lossy(&buf).to_string()).unwrap();
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        let resp = if chunked {
            let b = body.as_bytes();
            let (a, c) = b.split_at(b.len() / 2);
            let mut r = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
            for part in [a, c] {
                r.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
                r.extend_from_slice(part);
                r.extend_from_slice(b"\r\n");
            }
            r.extend_from_slice(b"0\r\n\r\n");
            r
        } else {
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .into_bytes()
        };
        s.write_all(&resp).unwrap();
    });
    (format!("http://127.0.0.1:{port}/v1"), rx)
}

fn adapter(endpoint: &str, memory: Memory) -> LocalModelAdapter {
    LocalModelAdapter {
        endpoint: endpoint.into(),
        model: "qwen2.5:7b".into(),
        timeout: Duration::from_secs(5),
        api_key: None,
        screen_examples: vec!["売れ筋商品を表示".into(), "商品を一覧表示".into(), "売れ筋商品を5件表示".into()],
        contract_examples: vec!["商品は誰でも閲覧可".into()],
        memory,
    }
}

fn rec(sentence: &str, chosen: &str, rejected: &[&str]) -> MemoryRecord {
    MemoryRecord {
        v: 1,
        sentence: sentence.into(),
        chosen: chosen.into(),
        rationale: "人気=売れ筋".into(),
        rejected: rejected.iter().map(|x| x.to_string()).collect(),
        context: vec!["売れ筋商品を表示".into(), "商品を一覧表示".into()],
        questions: vec![],
        project: "shop".into(),
        at: "2026-11-08T00:00:00Z".into(),
    }
}

#[test]
fn only_loopback_endpoints_are_allowed() {
    assert!(parse_local_endpoint("http://127.0.0.1:11434/v1").is_ok());
    assert!(parse_local_endpoint("http://localhost:8080").is_ok());
    assert!(parse_local_endpoint("http://[::1]:8000/v1").is_ok());
    for bad in [
        "https://api.example.com/v1",
        "http://192.168.0.5:11434/v1",
        "http://example.com/v1",
        "http://127.0.0.1.evil.com:80/v1",
        "ftp://127.0.0.1/",
    ] {
        assert!(parse_local_endpoint(bad).is_err(), "{bad} は拒否されるべき");
    }
    let a = adapter("http://10.0.0.1:11434/v1", Memory::default());
    assert!(a.propose(&u("人気商品を表示")).unwrap_err().contains("ADR-006"));
}

#[test]
fn local_model_candidate_is_validated_and_ids_are_ours() {
    let (ep, rx) = mock(
        "はい。\n```json\n{\"intent_id\":\"screen.他#1\",\"interpretations\":[{\"canonical\":\"売れ筋商品を表示\",\"rationale\":\"人気=売れ筋\"}],\"assumptions\":[],\"unresolved\":[]}\n```",
        false,
    );
    let mem = Memory {
        records: vec![rec("人気の商品を表示", "売れ筋商品を表示", &[])],
    };
    let a = adapter(&ep, mem);
    let v = a.propose(&u("人気商品を表示")).unwrap();
    let c = validate_candidate(&v, &u("人気商品を表示")).unwrap();
    assert_eq!(c.intent_id, "screen.人気#000000000000", "モデルが書いた ID は使わない");
    assert_eq!(c.interpretations[0].canonical, "売れ筋商品を表示");
    assert!(c.evidence_refs[0].contains("qwen2.5:7b"));
    // 送った内容: 文・確認事項・正規文の例・似た承認例だけ
    let req = rx.recv().unwrap();
    assert!(req.starts_with("POST /v1/chat/completions HTTP/1.1"));
    let body: Value = serde_json::from_str(&req[req.find("\r\n\r\n").unwrap() + 4..]).unwrap();
    assert_eq!(body["model"], "qwen2.5:7b");
    assert_eq!(body["temperature"], 0);
    let user = body["messages"][1]["content"].as_str().unwrap();
    assert!(user.contains("解釈する文: 「人気商品を表示」"));
    assert!(user.contains("「人気の商品を表示」→「売れ筋商品を表示」"), "似た承認例を見せる: {user}");
    assert!(user.contains("- 売れ筋商品を表示"));
}

#[test]
fn chunked_responses_are_decoded() {
    let (ep, _rx) = mock(
        r#"{"interpretations":[{"canonical":"商品を一覧表示","rationale":"r"}],"assumptions":[],"unresolved":[]}"#,
        true,
    );
    let v = adapter(&ep, Memory::default()).propose(&u("商品を全部見せる")).unwrap();
    assert_eq!(v["interpretations"][0]["canonical"], "商品を一覧表示");
}

#[test]
fn injected_or_broken_model_output_is_rejected() {
    // 余分なキー(コードを紛れ込ませる)はスキーマ検査で拒否
    let (ep, _rx) = mock(r#"{"interpretations":[],"assumptions":[],"unresolved":[],"code":"rm -rf /"}"#, false);
    let v = adapter(&ep, Memory::default()).propose(&u("人気商品を表示")).unwrap();
    assert!(validate_candidate(&v, &u("人気商品を表示")).is_err());
    // 未知の効果を要求しても拒否
    let (ep, _rx) = mock(
        r#"{"interpretations":[],"assumptions":[],"unresolved":[],"required_effects":["shell.exec"]}"#,
        false,
    );
    let v = adapter(&ep, Memory::default()).propose(&u("人気商品を表示")).unwrap();
    assert!(validate_candidate(&v, &u("人気商品を表示")).is_err());
    // JSON でない出力
    let (ep, _rx) = mock("わかりません", false);
    assert!(adapter(&ep, Memory::default()).propose(&u("人気商品を表示")).unwrap_err().contains("JSON"));
    // サーバーが居ない
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let e = adapter(&format!("http://127.0.0.1:{port}/v1"), Memory::default()).propose(&u("x")).unwrap_err();
    assert!(e.contains("接続できません"), "{e}");
}

#[test]
fn past_approvals_come_first_and_memory_adapter_works_offline() {
    let mem = Memory {
        records: vec![
            rec("人気商品を表示", "売れ筋商品を5件表示", &[]),
            rec("人気商品を表示", "売れ筋商品を5件表示", &[]),
        ],
    };
    let (ep, _rx) = mock(
        r#"{"interpretations":[{"canonical":"売れ筋商品を表示","rationale":"r"},{"canonical":"売れ筋商品を5件表示","rationale":"r"}],"assumptions":[],"unresolved":[]}"#,
        false,
    );
    let v = adapter(&ep, mem.clone()).propose(&u("人気商品を表示")).unwrap();
    let its = v["interpretations"].as_array().unwrap();
    assert_eq!(its.len(), 2, "重複は 1 つにまとめる");
    assert_eq!(its[0]["canonical"], "売れ筋商品を5件表示");
    assert!(its[0]["rationale"].as_str().unwrap().contains("2 回"));
    let m = crate::local::MemoryAdapter { memory: mem };
    let c = validate_candidate(&m.propose(&u("人気商品を表示")).unwrap(), &u("人気商品を表示")).unwrap();
    assert_eq!(c.interpretations[0].canonical, "売れ筋商品を5件表示");
    assert!(m.propose(&u("知らない文")).is_err());
}

#[test]
fn memory_append_dedupes_and_exports_training_data() {
    let dir = std::env::temp_dir().join(format!("koto-learn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(Memory::append(&dir, rec("人気商品を表示", "売れ筋商品を表示", &["商品を一覧表示"])).unwrap());
    assert!(!Memory::append(&dir, rec("人気商品を表示", "売れ筋商品を表示", &["売れ筋商品を5件表示"])).unwrap());
    assert!(Memory::append(&dir, rec("おすすめを出す", "売れ筋商品を5件表示", &[])).unwrap());
    std::fs::write(dir.join(MEMORY_FILE), std::fs::read_to_string(dir.join(MEMORY_FILE)).unwrap() + "壊れた行\n").unwrap();
    let (m, bad) = Memory::load(&dir);
    assert_eq!((m.records.len(), bad), (2, 1));
    assert_eq!(m.records[0].rejected, vec!["商品を一覧表示".to_string(), "売れ筋商品を5件表示".to_string()]);
    let (sft, dpo) = export(&m);
    assert_eq!(sft.lines().count(), 2);
    assert_eq!(dpo.lines().count(), 2);
    let first: Value = serde_json::from_str(sft.lines().next().unwrap()).unwrap();
    let msgs = first["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[2]["role"], "assistant");
    let ans: Value = serde_json::from_str(msgs[2]["content"].as_str().unwrap()).unwrap();
    assert_eq!(ans["interpretations"][0]["canonical"], "売れ筋商品を表示");
    // 書き出しの問いには、その文自身の答えを含めない
    assert!(!msgs[1]["content"].as_str().unwrap().contains("→「売れ筋商品を表示」"));
    let d: Value = serde_json::from_str(dpo.lines().next().unwrap()).unwrap();
    assert!(d["rejected"][0]["content"].as_str().unwrap().contains("商品を一覧表示"));
    append_eval(
        &dir,
        &EvalRecord {
            at: "t".into(),
            model: "m".into(),
            cases: 4,
            top1: 3,
            any: 4,
            broken: 0,
        },
    )
    .unwrap();
    assert_eq!(load_evals(&dir)[0].top1_rate(), 0.75);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prompt_examples_are_nearest_and_deterministic() {
    let pool: Vec<String> = (0..50)
        .map(|i| format!("データ{i}を一覧表示"))
        .chain(["売れ筋商品を表示".to_string()])
        .collect();
    let a = PromptContext::new("人気商品を表示", &[], &pool);
    let b = PromptContext::new("人気商品を表示", &[], &pool);
    assert_eq!(a, b);
    assert_eq!(a.examples.len(), MAX_EXAMPLES);
    assert_eq!(a.examples[0], "売れ筋商品を表示");
    assert!(similarity("人気商品を表示", "人気商品を表示") == 1000);
}
