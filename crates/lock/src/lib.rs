//! 承認記録とロックファイル(FR-007、技術仕様書 §7/§13、SEC-09)。
//!
//! - 承認記録 `kotonoha.approvals.json`: 各記録は自身の正規 JSON の SHA-256(record_hash)を持ち、改ざんを検出する
//! - ロック `kotonoha.lock.json`: ソース・知識・スキーマ・グラフのハッシュと承認済み解釈を固定する
//! - 承認の**真正性**(誰が承認したか)の検証基盤は ADR-007 で未決定。`ApprovalVerifier` を差し替え点として用意し、
//!   既定の `LocalUnsigned` は常に「未検証」を返す。自己申告の記録だけで真正と扱うことはしない。

use kotonoha_diag::{Code, Diagnostic, Span};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const LOCK_FILE: &str = "kotonoha.lock.json";
pub const APPROVALS_FILE: &str = "kotonoha.approvals.json";
pub const LOCK_VERSION: &str = "1";

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn canonical<T: Serialize>(v: &T) -> String {
    // serde_json::Value 経由でキーを UTF-8 バイト順に揃える
    let val = serde_json::to_value(v).expect("直列化できる");
    serde_json::to_string(&val).expect("直列化できる")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub intent_id: String,
    pub sentence: String,
    pub sentence_hash: String,
    /// 承認された正規文(既知の文型に一致しなければならない)
    pub canonical: String,
    pub approver: String,
    /// RFC 3339。VAG には含めない(再現性のため)
    pub approved_at: String,
    #[serde(default)]
    pub note: String,
    /// 上記フィールドの正規 JSON の SHA-256
    pub record_hash: String,
    /// 承認者の Ed25519 署名(record_hash に対する署名。ADR-007)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<RecordSignature>,
}

/// 承認記録の署名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordSignature {
    pub alg: String,
    /// 公開鍵の SHA-256 の先頭 16 桁
    pub key_id: String,
    /// 署名(hex、64 バイト)
    pub sig: String,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// 署名鍵(秘密鍵)。ファイルは hex の 32 バイトシード 1 行。
pub struct SigningKey(ed25519_dalek::SigningKey);

impl SigningKey {
    pub fn generate() -> Self {
        SigningKey(ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng))
    }
    pub fn from_hex(s: &str) -> Result<Self, String> {
        let b = unhex(s.trim()).ok_or("署名鍵の形式が不正です(hex 64 桁)")?;
        let seed: [u8; 32] = b.try_into().map_err(|_| "署名鍵は 32 バイトです".to_string())?;
        Ok(SigningKey(ed25519_dalek::SigningKey::from_bytes(&seed)))
    }
    pub fn to_hex(&self) -> String {
        hex(&self.0.to_bytes())
    }
    pub fn public_hex(&self) -> String {
        hex(self.0.verifying_key().as_bytes())
    }
    pub fn key_id(&self) -> String {
        key_id_of(&self.public_hex())
    }
    /// 記録に署名する(record_hash に対して)
    pub fn sign(&self, r: &mut ApprovalRecord) {
        use ed25519_dalek::Signer;
        let sig = self.0.sign(r.record_hash.as_bytes());
        r.signature = Some(RecordSignature {
            alg: "ed25519".into(),
            key_id: self.key_id(),
            sig: hex(&sig.to_bytes()),
        });
    }
}

pub fn key_id_of(pubkey_hex: &str) -> String {
    sha256_hex(pubkey_hex)[..16].to_string()
}

/// 信頼する承認者の公開鍵の一覧(`kotonoha.trust.json` 形式)。
/// **リポジトリの外**(CI の秘密設定・配布物)から与える: 承認記録と同じ場所にあると、記録を書き換えられる者が鍵も足せるため。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TrustFile {
    pub trusted_keys: Vec<TrustedKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedKey {
    pub owner: String,
    /// 公開鍵(hex、32 バイト)
    pub pubkey: String,
}

/// Ed25519 署名の検証器。全記録が信頼鍵の正しい署名を持つときだけ「検証済み」。
pub struct Ed25519Verifier {
    pub trust: TrustFile,
}

impl ApprovalVerifier for Ed25519Verifier {
    fn kind(&self) -> &'static str {
        "ed25519"
    }
    fn verify(&self, approvals: &ApprovalsFile) -> Authenticity {
        use ed25519_dalek::Verifier;
        for r in &approvals.records {
            if r.compute_hash() != r.record_hash {
                return Authenticity::Unverified {
                    reason: format!("承認記録 `{}` が改ざんされています", r.intent_id),
                };
            }
            let Some(s) = &r.signature else {
                return Authenticity::Unverified {
                    reason: format!("承認記録 `{}` に署名がありません(koto approve --sign-key で署名してください)", r.intent_id),
                };
            };
            let Some(k) = self.trust.trusted_keys.iter().find(|k| key_id_of(&k.pubkey) == s.key_id) else {
                return Authenticity::Unverified {
                    reason: format!("承認記録 `{}` の署名鍵 {} は信頼されていません", r.intent_id, s.key_id),
                };
            };
            let ok = (|| {
                let pk: [u8; 32] = unhex(&k.pubkey)?.try_into().ok()?;
                let vk = ed25519_dalek::VerifyingKey::from_bytes(&pk).ok()?;
                let sig: [u8; 64] = unhex(&s.sig)?.try_into().ok()?;
                vk.verify(r.record_hash.as_bytes(), &ed25519_dalek::Signature::from_bytes(&sig)).ok()
            })();
            if s.alg != "ed25519" || ok.is_none() {
                return Authenticity::Unverified {
                    reason: format!("承認記録 `{}` の署名が正しくありません", r.intent_id),
                };
            }
        }
        Authenticity::Verified {
            verifier: format!("ed25519({} 件)", approvals.records.len()),
        }
    }
}

#[derive(Serialize)]
struct RecordBody<'a> {
    intent_id: &'a str,
    sentence: &'a str,
    sentence_hash: &'a str,
    canonical: &'a str,
    approver: &'a str,
    approved_at: &'a str,
    note: &'a str,
}

impl ApprovalRecord {
    pub fn new(intent_id: &str, sentence: &str, sentence_hash: &str, canonical: &str, approver: &str, approved_at: &str, note: &str) -> Self {
        let mut r = ApprovalRecord {
            intent_id: intent_id.into(),
            sentence: sentence.into(),
            sentence_hash: sentence_hash.into(),
            canonical: canonical.into(),
            approver: approver.into(),
            approved_at: approved_at.into(),
            note: note.into(),
            record_hash: String::new(),
            signature: None,
        };
        r.record_hash = r.compute_hash();
        r
    }
    pub fn compute_hash(&self) -> String {
        sha256_hex(&canonical(&RecordBody {
            intent_id: &self.intent_id,
            sentence: &self.sentence,
            sentence_hash: &self.sentence_hash,
            canonical: &self.canonical,
            approver: &self.approver,
            approved_at: &self.approved_at,
            note: &self.note,
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ApprovalsFile {
    pub approvals_version: String,
    pub records: Vec<ApprovalRecord>,
}

impl ApprovalsFile {
    pub fn empty() -> Self {
        ApprovalsFile {
            approvals_version: "1".into(),
            records: Vec::new(),
        }
    }
    /// 同じ intent_id の承認は置き換える(最新の承認だけが有効)。
    pub fn upsert(&mut self, r: ApprovalRecord) {
        self.records.retain(|x| x.intent_id != r.intent_id);
        self.records.push(r);
        self.records.sort_by(|a, b| a.intent_id.cmp(&b.intent_id));
    }
    /// 改ざん検出。改ざんされた記録は APP001。
    pub fn verify_integrity(&self) -> Vec<Diagnostic> {
        self.records
            .iter()
            .filter(|r| r.compute_hash() != r.record_hash)
            .map(|r| {
                Diagnostic::error(
                    Code::APP001,
                    Span::default(),
                    format!("承認記録 `{}` のハッシュが一致しません(改ざんの可能性)", r.intent_id),
                )
                .hint("koto approve で承認をやり直してください")
            })
            .collect()
    }
    pub fn records_hash(&self) -> String {
        sha256_hex(&canonical(&self.records))
    }
}

/// 承認の真正性。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Authenticity {
    Verified { verifier: String },
    Unverified { reason: String },
}

/// 承認の真正性検証の差し替え点(ADR-007)。
pub trait ApprovalVerifier {
    fn kind(&self) -> &'static str;
    fn verify(&self, approvals: &ApprovalsFile) -> Authenticity;
}

/// 署名基盤が決まるまでの既定実装。常に未検証を返す。
pub struct LocalUnsigned;

impl ApprovalVerifier for LocalUnsigned {
    fn kind(&self) -> &'static str {
        "local-unsigned"
    }
    fn verify(&self, approvals: &ApprovalsFile) -> Authenticity {
        if approvals.records.is_empty() {
            // 承認対象が無ければ真正性の問題は生じない
            Authenticity::Verified {
                verifier: "no-approvals-required".into(),
            }
        } else {
            Authenticity::Unverified {
                reason: "承認署名基盤が未決定(ADR-007)のため、承認者の真正性を検証できません".into(),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedInterpretation {
    pub intent_id: String,
    pub sentence_hash: String,
    pub canonical: String,
    pub record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalReference {
    pub kind: String,
    pub records_hash: String,
}

/// `kotonoha.lock.json`(技術仕様書 §13 の最低フィールド。`target` は複数ターゲットのため `targets` 配列とする: ADR-0109)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockFile {
    pub lock_version: String,
    pub compiler_version: String,
    pub source_hash: String,
    pub knowledge_hash: String,
    pub schema_hash: String,
    pub graph_hash: String,
    pub targets: Vec<String>,
    pub approved_interpretations: Vec<LockedInterpretation>,
    pub approval_reference: ApprovalReference,
}

impl LockFile {
    pub fn to_pretty_json(&self) -> String {
        let v = serde_json::to_value(self).expect("直列化できる");
        let mut s = serde_json::to_string_pretty(&v).expect("直列化できる");
        s.push('\n');
        s
    }
    pub fn parse(text: &str) -> Result<LockFile, Diagnostic> {
        serde_json::from_str(text).map_err(|e| Diagnostic::error(Code::LOCK001, Span::default(), format!("ロックファイルを読めません: {e}")))
    }

    /// 期待値(現在の入力から計算したロック)と比較し、不一致を LOCK001 で返す。
    pub fn compare(&self, current: &LockFile) -> Vec<Diagnostic> {
        let mut d = Vec::new();
        let mut mismatch = |field: &str, a: &str, b: &str| {
            if a != b {
                d.push(
                    Diagnostic::error(Code::LOCK001, Span::default(), format!("ロック不整合 (LOCK_STALE): {field} が一致しません"))
                        .hint(format!("ロック: {a}"))
                        .hint(format!("現在: {b}"))
                        .hint("変更を確認し、koto approve --write-lock で再承認・再ロックしてください"),
                );
            }
        };
        mismatch("lock_version", &self.lock_version, &current.lock_version);
        mismatch("compiler_version", &self.compiler_version, &current.compiler_version);
        mismatch("source_hash", &self.source_hash, &current.source_hash);
        mismatch("knowledge_hash", &self.knowledge_hash, &current.knowledge_hash);
        mismatch("schema_hash", &self.schema_hash, &current.schema_hash);
        mismatch("graph_hash", &self.graph_hash, &current.graph_hash);
        mismatch(
            "approval_reference.records_hash",
            &self.approval_reference.records_hash,
            &current.approval_reference.records_hash,
        );
        if self.approved_interpretations != current.approved_interpretations {
            mismatch(
                "approved_interpretations",
                &format!("{} 件", self.approved_interpretations.len()),
                &format!("{} 件(内容が異なる)", current.approved_interpretations.len()),
            );
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> ApprovalRecord {
        ApprovalRecord::new(
            "screen.人気#abc",
            "人気商品を表示",
            "h",
            "売れ筋商品を表示",
            "山田",
            "2026-11-03T00:00:00+09:00",
            "",
        )
    }

    #[test]
    fn record_hash_detects_tampering() {
        let mut f = ApprovalsFile::empty();
        f.upsert(rec());
        assert!(f.verify_integrity().is_empty());
        f.records[0].canonical = "別の解釈を表示".into();
        let d = f.verify_integrity();
        assert_eq!(d[0].code, Code::APP001);
    }

    #[test]
    fn upsert_replaces_same_intent() {
        let mut f = ApprovalsFile::empty();
        f.upsert(rec());
        let mut r2 = rec();
        r2.approver = "佐藤".into();
        r2.record_hash = r2.compute_hash();
        f.upsert(r2);
        assert_eq!(f.records.len(), 1);
        assert_eq!(f.records[0].approver, "佐藤");
    }

    #[test]
    fn local_unsigned_never_claims_authenticity() {
        let mut f = ApprovalsFile::empty();
        assert!(matches!(LocalUnsigned.verify(&f), Authenticity::Verified { .. }));
        f.upsert(rec());
        assert!(matches!(LocalUnsigned.verify(&f), Authenticity::Unverified { .. }));
    }

    fn lock() -> LockFile {
        LockFile {
            lock_version: "1".into(),
            compiler_version: "0.1.0".into(),
            source_hash: "s".into(),
            knowledge_hash: "k".into(),
            schema_hash: "c".into(),
            graph_hash: "g".into(),
            targets: vec!["web".into()],
            approved_interpretations: vec![],
            approval_reference: ApprovalReference {
                kind: "local-unsigned".into(),
                records_hash: "r".into(),
            },
        }
    }

    #[test]
    fn compare_reports_each_stale_field() {
        let a = lock();
        let mut b = lock();
        assert!(a.compare(&b).is_empty());
        b.knowledge_hash = "k2".into();
        b.graph_hash = "g2".into();
        let d = a.compare(&b);
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|x| x.code == Code::LOCK001));
        assert!(d[0].message_ja.contains("knowledge_hash"));
    }

    #[test]
    fn lock_roundtrips_through_json() {
        let l = lock();
        assert_eq!(LockFile::parse(&l.to_pretty_json()).unwrap(), l);
        assert_eq!(LockFile::parse("{}").unwrap_err().code, Code::LOCK001);
    }
}

#[cfg(test)]
mod signing_tests {
    use super::*;

    fn rec() -> ApprovalRecord {
        ApprovalRecord::new(
            "screen.人気#abc",
            "人気商品を表示",
            "h",
            "売れ筋商品を表示",
            "山田",
            "2026-11-03T00:00:00+09:00",
            "",
        )
    }

    #[test]
    fn ed25519_signed_records_verify_only_with_trusted_key() {
        let key = SigningKey::generate();
        let mut r = rec();
        key.sign(&mut r);
        let mut f = ApprovalsFile::empty();
        f.upsert(r);
        let trusted = Ed25519Verifier {
            trust: TrustFile {
                trusted_keys: vec![TrustedKey {
                    owner: "山田".into(),
                    pubkey: key.public_hex(),
                }],
            },
        };
        assert!(matches!(trusted.verify(&f), Authenticity::Verified { .. }));
        // 信頼されていない鍵
        let other = SigningKey::generate();
        let untrusted = Ed25519Verifier {
            trust: TrustFile {
                trusted_keys: vec![TrustedKey {
                    owner: "x".into(),
                    pubkey: other.public_hex(),
                }],
            },
        };
        assert!(matches!(untrusted.verify(&f), Authenticity::Unverified { .. }));
        // 記録の改ざん(署名後に解釈を書き換えて record_hash も付け直す)
        let mut t = f.clone();
        t.records[0].canonical = "別の解釈を表示".into();
        t.records[0].record_hash = t.records[0].compute_hash();
        assert!(matches!(trusted.verify(&t), Authenticity::Unverified { .. }));
        // 署名なし
        let mut u = ApprovalsFile::empty();
        u.upsert(rec());
        assert!(matches!(trusted.verify(&u), Authenticity::Unverified { .. }));
    }

    #[test]
    fn signing_key_roundtrip() {
        let k = SigningKey::generate();
        let k2 = SigningKey::from_hex(&k.to_hex()).unwrap();
        assert_eq!(k.public_hex(), k2.public_hex());
        assert!(SigningKey::from_hex("zz").is_err());
    }
}
