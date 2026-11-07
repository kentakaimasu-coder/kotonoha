//! NR-02: コンパイラは任意の入力でパニックしない(乱数入力テスト)。
//! 生成元の文字は、KOTONOHA の予約語・記号・日本語・全角文字・制御文字・インデントを混ぜたもの。

use kotonoha_driver::{analyze, project_from_sources};
use kotonoha_lock::ApprovalsFile;
use proptest::prelude::*;

const PIECES: &[&str] = &[
    "アプリ「",
    "」",
    "データ ",
    "画面 ",
    "動作 ",
    "知識 ",
    "約束",
    "列挙 ",
    "型 ",
    "アダプタ ",
    ":",
    "\n",
    "    ",
    "        ",
    "\t",
    "\r\n",
    "商品",
    "注文",
    "ID",
    "識別子",
    "整数",
    "金額<JPY>",
    "結果<",
    ">",
    "選択<",
    "一覧<",
    "記録{",
    "}",
    "(",
    ")",
    "[",
    "]",
    ",",
    ".",
    "=",
    "==",
    "!=",
    "<=",
    ">=",
    "+",
    "-",
    "*",
    "/",
    "&&",
    "||",
    "!",
    "\"",
    "「購入」",
    "を確認",
    "または",
    "失敗(",
    "成功(",
    "を返す",
    "もし ",
    "そうでなければ",
    "繰り返す ",
    "から",
    "取引",
    "厳密",
    "必要 = [",
    "効果 = [",
    "操作 = 取得(",
    "を一覧表示",
    "を選ぶと ",
    " へ",
    "の",
    "と",
    "を表示",
    "件表示",
    "を押すと ",
    "のフォームを表示",
    "は誰でも閲覧可",
    "未認証なら",
    "不可",
    "が本人のものだけ閲覧可",
    "本人",
    "直近30日",
    "売上金額の合計",
    "期間 = ",
    "対象 = ",
    "指標 = ",
    "@柔軟(確認)",
    "@厳密",
    "#",
    "0",
    "99999999999999999999",
    "1.5",
    "＝",
    "（",
    "\u{3000}",
    "\u{0}",
    "\u{FEFF}",
    "か\u{3099}",
    "😀",
    "\\",
    "\\u{41}",
    "a",
    "_",
];

fn source() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..120).prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn analyze_never_panics(src in source()) {
        let p = project_from_sources("t", &[("src/app.koto", &src)], &["web", "backend", "ios", "android"]);
        let a = analyze(&p, &ApprovalsFile::empty());
        // エラーがあるなら必ず draft、無いなら validated
        prop_assert_eq!(a.vag["status"] == "validated", a.ok());
        if a.ok() {
            for t in ["backend", "web", "ios", "android"] {
                let _ = kotonoha_driver::build_target(&p, &a, t);
            }
        }
    }

    #[test]
    fn arbitrary_unicode_never_panics(src in "\\PC{0,200}") {
        let p = project_from_sources("t", &[("src/app.koto", &src)], &["web"]);
        let _ = analyze(&p, &ApprovalsFile::empty());
    }
}
