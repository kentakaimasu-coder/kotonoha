# KOTONOHA

日本語+最小限の記号で「目的」を書き、必要な箇所だけ「厳密な処理」を定義する、意図駆動型・検証可能なアプリケーション言語。
コンパイラ `koto` は Rust で実装している。

> **現状(Phase 0 + M0/M1 の一部)**: 字句・構文・型・知識・意図・効果/権限の検査、Verified Application Graph(VAG)、
> 承認記録とロック、売れ筋知識の SQL 低水準化までが動く。**Web/Backend/iOS/Android のアプリ生成はまだ無い**
> (`koto build` は未対応機能を TGT001 で報告して停止する)。詳細は [docs/status.md](docs/status.md)。

## 使い方
```bash
cargo build
./target/debug/koto -C examples/shop check          # 検査
./target/debug/koto -C examples/shop graph          # VAG を出力
./target/debug/koto -C examples/shop --json check   # 機械可読診断
./target/debug/koto doctor                          # 実装状況と未決定事項
```

```kotonoha
知識 売れ筋商品:
    対象 = 注文明細
    期間 = 直近30日
    指標 = 売上金額の合計
    単位 = 商品ID
    並び = 売上金額の降順, 商品IDの昇順
    件数 = 20
    日時列 = 確定日時
    時刻帯 = Asia/Tokyo
    税区分 = 税込
    条件 = 状態 == "確定"

画面 売れ筋:
    売れ筋商品を表示
```

## 文書
- 要件・仕様(原本): [docs/requirements/](docs/requirements/)
- Phase 0 整合性検証: [docs/phase0/consistency_review.md](docs/phase0/consistency_review.md)
- 言語仕様: [字句](docs/spec/lexical.md) / [文法 EBNF](docs/spec/grammar.md) / [AST](docs/spec/ast.md) / [型](docs/spec/type_system.md) / [意図文型](docs/spec/intent_patterns.md) / [知識](docs/spec/knowledge.md) / [VAG](docs/spec/vag.md) / [エラー](docs/spec/errors.md) / [コンパイラ構成](docs/spec/compiler.md) / [テスト仕様](docs/spec/test_spec.md)
- JSON Schema: [schemas/](schemas/)
- 設計判断: [docs/adr/](docs/adr/README.md)

## テスト
```bash
cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test
python3 tools/validate_schemas.py target/debug/koto
```
