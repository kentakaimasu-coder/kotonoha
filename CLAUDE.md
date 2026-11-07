# KOTONOHA — Claude Code 実装ガイド

元の実装指示は [docs/requirements/CLAUDE.original.md](docs/requirements/CLAUDE.original.md)。要件は [01_requirements.md](docs/requirements/01_requirements.md) と **必須の追補 [03_requirements_addendum.md](docs/requirements/03_requirements_addendum.md)(日本人の使いやすさ・セキュリティ・痕跡を出さない・A/B テストと拡張部品)**、技術仕様は [02_technical_spec.md](docs/requirements/02_technical_spec.md)。要件 ID(FR/NFR/SEC/AT)とテスト ID(TC)を追跡し、対応は [docs/spec/test_spec.md](docs/spec/test_spec.md) に記録する。

## 不変ルール
1. 承認されていない仕様を確定と主張しない。不明点は `docs/adr/` に選択肢と推奨案を書き、状態を「未決定」か「提案」にする。ADR 一覧は [docs/adr/README.md](docs/adr/README.md)。
2. AI の解釈は `crates/resolver` に隔離する。AI 出力を直接実行・評価しない。承認対象は「既知の文型の正規文」だけ。
3. ロックなし・未承認・型不一致・権限不足・ターゲット未対応は必ず失敗させる(負例テストを付ける)。
4. 業務ロジックと認可は backend だけで実行する。Web/iOS/Android は共通 API 契約(api.json)のクライアント。構成は [docs/architecture.md](docs/architecture.md)。
5. 未実装機能を完成扱いしない。`koto build` は未対応機能を TGT001 で止める。`docs/status.md` を毎フェーズ更新する。
6. 決定的であること: パイプラインは時刻・ネットワーク・乱数・HashMap 順序に依存しない(BTreeMap / ソート済み配列)。
7. 変更ごとに `cargo fmt --all -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`python3 tools/validate_schemas.py target/debug/koto`、`node --no-warnings --test tests/e2e/*.test.ts`(PostgreSQL は `KOTONOHA_TEST_PG_URL`)、`koto -C examples/shop test`、`tools/mobile-syntax/run.sh`、`tools/verify_android_jvm.sh` を通す。iOS のコンパイルと Compose 画面のビルドはこの環境ではできないため、未検証であることを明記する。生成 TypeScript は型除去モードで動く構文だけを使う(コンストラクタ引数プロパティ・enum・namespace 禁止)。
8. 開発方針: 全体設計に沿って関連機能をまとめて実装する。軽微な不具合は `docs/status.md` の既知の不具合(K-xx)に記録して先に進む。構文・型・Intent Graph・セキュリティの重大な不具合は即時修正する。テストを通すために仕様を変えない。同じエラーの修正が 3 回続いたら原因分析に切り替える。ゴールデン(`examples/*/expected/`)を更新したら差分をレビューする。

## ファイル地図
```
crates/diag       診断・エラーコード・終了コード(docs/spec/errors.md)
crates/ast        Surface AST(docs/spec/ast.md)
crates/lexer      字句(docs/spec/lexical.md)
crates/parser     構文 + 正規形プリンタ(docs/spec/grammar.md)
crates/types      型・宣言環境・厳密層の型検査・効果語彙(docs/spec/type_system.md)
crates/knowledge  知識レジストリ(docs/spec/knowledge.md)
crates/intent     意図文型・承認適用(docs/spec/intent_patterns.md)
crates/validator  効果/Capability 検査
crates/graph      VAG(docs/spec/vag.md, schemas/vag.schema.json)
crates/lock       承認記録・ロック・真正性検証の差し替え点
crates/resolver   候補生成アダプタ(offline/fixture)と候補スキーマ検査
crates/codegen    App IR・共通 API 契約・backend/web/ios/android 生成器・SQL 低水準化
runtime/          生成物に同梱する手書きランタイム(backend / web / ios / android)
crates/driver     パイプライン統合・プロジェクト読込
crates/cli        koto コマンド
examples/shop     MVP 垂直スライスのソースとゴールデン VAG
examples/ranking  売れ筋(UC-04)とゴールデン SQL
tests/fixtures    仕様書サンプル原文など(負例)
tools/            スキーマ検証・モバイル構文検査・Android JVM 検証
tests/e2e         生成物を起動しての結合・E2E テスト(node --test, Playwright)
```
