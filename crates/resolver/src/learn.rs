//! 学習記録(ADR-0127): 人が承認した「原文 → 正規文」と、選ばれなかった候補を貯め、
//! 1) 次回の候補作成で似た例としてモデルに見せる(学習なしで当たりやすくなる)、
//! 2) オープンモデルの追加学習用データ(SFT / DPO)として書き出す、
//! 3) 新しいモデルを採用する前に、過去の承認をどれだけ当てられるかで評価する(劣化したモデルを入れない)
//!
//! これらに使う。記録は人の承認だけから作られ、ここから自動で承認されることはない。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const MEMORY_FILE: &str = "memory.jsonl";
pub const EVALS_FILE: &str = "evals.jsonl";
/// モデルに見せる正規文の例の最大数
pub const MAX_EXAMPLES: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub v: u32,
    /// 利用者が書いた文
    pub sentence: String,
    /// 承認された正規文
    pub chosen: String,
    #[serde(default)]
    pub rationale: String,
    /// 提示されたが選ばれなかった正規文(DPO の負例)
    #[serde(default)]
    pub rejected: Vec<String>,
    /// 候補作成時に見せた正規文の例(書き出し・評価で同じ問いを再現するため)
    #[serde(default)]
    pub context: Vec<String>,
    #[serde(default)]
    pub questions: Vec<String>,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalRecord {
    pub at: String,
    pub model: String,
    pub cases: usize,
    /// 1 番目の候補が承認済みの解釈と一致した数
    pub top1: usize,
    /// いずれかの候補が一致した数
    pub any: usize,
    /// 応答が壊れていた(JSON でない・スキーマ違反・接続失敗)数
    pub broken: usize,
}

impl EvalRecord {
    pub fn top1_rate(&self) -> f64 {
        if self.cases == 0 {
            0.0
        } else {
            self.top1 as f64 / self.cases as f64
        }
    }
}

/// 学習記録の置き場所: `KOTONOHA_LEARN_DIR`、無ければ `~/.kotonoha/learn`(プロジェクトをまたいで育つ)。
pub fn default_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("KOTONOHA_LEARN_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".kotonoha").join("learn"))
}

pub fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn grams(s: &str) -> Vec<(char, char)> {
    let cs: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    cs.windows(2).map(|w| (w[0], w[1])).collect()
}

/// 文字 2-gram の Dice 係数(0〜1000)。決定的。
pub fn similarity(a: &str, b: &str) -> u32 {
    let (ga, gb) = (grams(a), grams(b));
    if ga.is_empty() || gb.is_empty() {
        return 0;
    }
    let mut used = vec![false; gb.len()];
    let mut hit = 0;
    for g in &ga {
        if let Some(i) = gb.iter().enumerate().position(|(i, x)| !used[i] && x == g) {
            used[i] = true;
            hit += 1;
        }
    }
    (2 * hit * 1000 / (ga.len() + gb.len())) as u32
}

#[derive(Debug, Clone, Default)]
pub struct Memory {
    pub records: Vec<MemoryRecord>,
}

impl Memory {
    /// 記録を読む。ディレクトリやファイルが無ければ空。壊れた行は数えて飛ばす。
    pub fn load(dir: &Path) -> (Memory, usize) {
        let Ok(text) = std::fs::read_to_string(dir.join(MEMORY_FILE)) else {
            return (Memory::default(), 0);
        };
        let mut records = Vec::new();
        let mut bad = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str::<MemoryRecord>(line) {
                Ok(r) if !r.sentence.is_empty() && !r.chosen.is_empty() => records.push(r),
                _ => bad += 1,
            }
        }
        (Memory { records }, bad)
    }

    /// 1 件追記する。同じプロジェクト・同じ文・同じ解釈の記録が既にあれば、負例だけ足して書き直さない。
    /// 戻り値: 新しく追加したら true
    pub fn append(dir: &Path, rec: MemoryRecord) -> Result<bool, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{} を作れません: {e}", dir.display()))?;
        let (mut m, _) = Memory::load(dir);
        if let Some(ex) = m
            .records
            .iter_mut()
            .find(|r| normalize(&r.sentence) == normalize(&rec.sentence) && r.chosen == rec.chosen && r.project == rec.project)
        {
            let before = ex.rejected.len();
            for x in rec.rejected {
                if !ex.rejected.contains(&x) && x != ex.chosen {
                    ex.rejected.push(x);
                }
            }
            if ex.rejected.len() != before {
                m.save(dir)?;
            }
            return Ok(false);
        }
        let line = serde_json::to_string(&rec).unwrap() + "\n";
        use std::io::Write;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(MEMORY_FILE))
            .and_then(|mut f| f.write_all(line.as_bytes()))
            .map_err(|e| format!("学習記録に書けません: {e}"))?;
        Ok(true)
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let text: String = self.records.iter().map(|r| serde_json::to_string(r).unwrap() + "\n").collect();
        std::fs::write(dir.join(MEMORY_FILE), text).map_err(|e| format!("学習記録に書けません: {e}"))
    }

    /// 同じ文で承認された解釈と回数(多い順)。
    pub fn exact(&self, sentence: &str) -> Vec<(String, usize)> {
        let n = normalize(sentence);
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for r in self.records.iter().filter(|r| normalize(&r.sentence) == n) {
            *counts.entry(r.chosen.clone()).or_default() += 1;
        }
        let mut v: Vec<(String, usize)> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// 似た文の承認例(原文, 正規文)を最大 n 件。似ていないもの(Dice 0.3 未満)は出さない。
    pub fn similar(&self, sentence: &str, n: usize) -> Vec<(String, String)> {
        let mut seen = std::collections::BTreeSet::new();
        let mut scored: Vec<(u32, &MemoryRecord)> = self
            .records
            .iter()
            .map(|r| (similarity(sentence, &r.sentence), r))
            .filter(|(s, _)| *s >= 300)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.sentence.cmp(&b.1.sentence)).then(a.1.chosen.cmp(&b.1.chosen)));
        scored
            .into_iter()
            .filter(|(_, r)| seen.insert((normalize(&r.sentence), r.chosen.clone())))
            .take(n)
            .map(|(_, r)| (r.sentence.clone(), r.chosen.clone()))
            .collect()
    }

    /// 指定の文を除いた記録(評価で答えを見せないため)。
    pub fn without(&self, sentence: &str) -> Memory {
        let n = normalize(sentence);
        Memory {
            records: self.records.iter().filter(|r| normalize(&r.sentence) != n).cloned().collect(),
        }
    }
}

/// モデルに渡す問いの材料。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptContext {
    pub sentence: String,
    pub questions: Vec<String>,
    pub examples: Vec<String>,
}

impl PromptContext {
    /// 使える正規文の中から、文に近い順に最大 MAX_EXAMPLES 件を選ぶ(決定的)。
    pub fn new(sentence: &str, questions: &[String], pool: &[String]) -> PromptContext {
        let mut scored: Vec<(u32, &String)> = pool.iter().map(|e| (similarity(sentence, e), e)).collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        scored.dedup_by(|a, b| a.1 == b.1);
        PromptContext {
            sentence: sentence.into(),
            questions: questions.to_vec(),
            examples: scored.into_iter().take(MAX_EXAMPLES).map(|(_, e)| e.clone()).collect(),
        }
    }
}

pub const SYSTEM_PROMPT: &str = "あなたは日本語のアプリ記述言語 KOTONOHA の「解釈候補」を作る補助です。\n\
利用者が書いた曖昧な文を、「使える正規文」の一覧にある文型へ言い換えた候補を、もっともらしい順に最大 3 つ出してください。\n\
規則:\n\
- 正規文は一覧の文型どおりに書き、一覧に出てくる名前(データ・画面・動作・知識の名前)だけを使う。新しい名前を作らない。\n\
- 一覧のどれにも当てはめられない、または利用者に確かめないと決められないことは unresolved に質問として書く。\n\
- 出力は次の形の JSON オブジェクト 1 つだけ。説明文やコードは書かない。\n\
{\"interpretations\":[{\"canonical\":\"正規文\",\"rationale\":\"そう解釈した理由\"}],\"assumptions\":[\"置いた前提\"],\"unresolved\":[\"確認が必要なこと\"]}";

pub fn user_prompt(ctx: &PromptContext, similar: &[(String, String)]) -> String {
    let mut s = String::from("使える正規文:\n");
    for e in &ctx.examples {
        s.push_str(&format!("- {e}\n"));
    }
    if !similar.is_empty() {
        s.push_str("\n過去に人が承認した言い換え:\n");
        for (a, b) in similar {
            s.push_str(&format!("- 「{a}」→「{b}」\n"));
        }
    }
    if !ctx.questions.is_empty() {
        s.push_str("\n確認事項:\n");
        for q in &ctx.questions {
            s.push_str(&format!("- {q}\n"));
        }
    }
    s.push_str(&format!("\n解釈する文: 「{}」\n", ctx.sentence));
    s
}

pub fn build_messages(ctx: &PromptContext, similar: &[(String, String)]) -> Vec<Value> {
    vec![
        json!({"role": "system", "content": SYSTEM_PROMPT}),
        json!({"role": "user", "content": user_prompt(ctx, similar)}),
    ]
}

fn answer(canonical: &str, rationale: &str) -> String {
    serde_json::to_string(&json!({
        "interpretations": [{"canonical": canonical, "rationale": rationale}],
        "assumptions": [],
        "unresolved": [],
    }))
    .unwrap()
}

/// 記録の問いを再現する(候補作成時の例 + その文を除いた記録からの似た例)。
pub fn record_messages(m: &Memory, r: &MemoryRecord) -> Vec<Value> {
    let ctx = PromptContext {
        sentence: r.sentence.clone(),
        questions: r.questions.clone(),
        examples: r.context.clone(),
    };
    build_messages(&ctx, &m.without(&r.sentence).similar(&r.sentence, 5))
}

/// 追加学習用データ: SFT(chat 形式)と DPO(prompt / chosen / rejected)の JSONL。
pub fn export(m: &Memory) -> (String, String) {
    let mut sft = String::new();
    let mut dpo = String::new();
    for r in &m.records {
        let prompt = record_messages(m, r);
        let rationale = if r.rationale.is_empty() {
            "過去に承認された解釈"
        } else {
            &r.rationale
        };
        let chosen = json!({"role": "assistant", "content": answer(&r.chosen, rationale)});
        let mut msgs = prompt.clone();
        msgs.push(chosen.clone());
        sft.push_str(&(serde_json::to_string(&json!({"messages": msgs})).unwrap() + "\n"));
        for x in &r.rejected {
            let rej = json!({"role": "assistant", "content": answer(x, "")});
            dpo.push_str(&(serde_json::to_string(&json!({"prompt": prompt, "chosen": [chosen], "rejected": [rej]})).unwrap() + "\n"));
        }
    }
    (sft, dpo)
}

pub fn load_evals(dir: &Path) -> Vec<EvalRecord> {
    std::fs::read_to_string(dir.join(EVALS_FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

pub fn append_eval(dir: &Path, e: &EvalRecord) -> Result<(), String> {
    use std::io::Write;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(EVALS_FILE))
        .and_then(|mut f| f.write_all((serde_json::to_string(e).unwrap() + "\n").as_bytes()))
        .map_err(|e| format!("評価記録に書けません: {e}"))
}
