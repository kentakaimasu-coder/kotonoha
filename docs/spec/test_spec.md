# テスト仕様とトレーサビリティ

状態: 実装中 / 最終更新: 2026-11-02 / 上位文書: 技術仕様書 §16, §19, 要件 §9

## 1. 実行方法
```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test                                   # 全ユニット・ゴールデン・結合テスト
python3 tools/validate_schemas.py target/debug/koto   # 生成物の JSON Schema 検証
UPDATE_GOLDEN=1 cargo test -p kotonoha-driver # ゴールデン VAG の更新(差分をレビューすること)
```

## 2. 種類
| 種類 | 置き場所 | 内容 |
|---|---|---|
| Unit | 各クレート `src/` 内 `#[cfg(test)]` | lexer / parser / types / knowledge / intent / validator / graph / lock / resolver / codegen |
| Property(簡易) | parser `ast_roundtrip_is_fixpoint_on_examples` | AST ラウンドトリップ。**乱数生成によるプロパティテストは未実装**(R-08) |
| Golden | `examples/shop/expected/vag.json`, `examples/ranking/expected/*.sql` | 意図→VAG、知識→SQL |
| Integration | `crates/codegen/tests/ranking_sqlite.rs` | SQLite で集計 SQL を実行し、参照実装と手計算の期待値に一致 |
| CLI | `crates/cli/tests/cli.rs` | 終了コード契約、resolve→approve→lock→build の流れ |
| Schema | `tools/validate_schemas.py` | 実際の出力を schemas/*.json で検証 |
| Mutation(ロック) | driver `lock_roundtrip_and_mutation_tests` | ソース・知識・スキーマ変更でロック失効 |
| 負例 | 各クレート | 未承認・型不一致・権限不足・ターゲット未対応で必ず失敗(NFR-001) |
| E2E / Security(Web, API) | – | **未実装**(生成器が無いため) |

## 3. 受入テストケース対応
| TC | 内容 | 状況 | テスト |
|---|---|---|---|
| TC-001 | 売れ筋 → 30日・売上金額・20件 | ✅ | knowledge `tc001_ranking_meaning_is_fixed`, codegen `at07_*` |
| TC-002 | 5件表示 → 件数だけ 5 | ✅ | intent `tc001_tc002_*`, codegen `tc002_limit_override_changes_only_count` |
| TC-003 | 未定義「人気」→ 確認要求 | ✅ | intent `tc003_*`, driver `tc003_*` |
| TC-004 | 未承認 → 本番ビルド拒否 | ✅ | driver `tc004_at05_*`, cli `flexible_intent_*` |
| TC-005 | 型不一致 → TYP001 | ✅ | types `tc005_type_mismatch_is_typ001` |
| TC-006 | data.read のみで更新 → SEC001 | ✅ | validator `tc006_*` |
| TC-007 | 権限なし更新 → HTTP 403 | ❌ 未実装 | (Backend 生成・認証 ADR-002 待ち) |
| TC-008 | XSS 非成立 | ❌ 未実装 | (Web 生成 ADR-004 待ち) |
| TC-009 | 同一ロック・AI オフライン → 同一 VAG | ✅ | driver `tc009_*`, cli `graph_is_reproducible_*` |
| TC-010 | 知識変更 → LOCK001 | ✅ | driver `lock_roundtrip_and_mutation_tests`, cli `lock_stale_*` |
| TC-011 | Web 商品一覧の表示・遷移 | ❌ 未実装 | |
| TC-012 | 注文登録と在庫の原子的更新 | △ 静的検査のみ | 取引内更新の型検査(types)。実行時の原子性は未実装 |
| TC-013 | ターゲット未対応 → TGT001 | ✅ | codegen `tc013_*`, driver `tc013_*`, cli |
| TC-014 | iOS/Android 商品一覧(P1) | ❌ P1 | |

| AT | 状況 |
|---|---|
| AT-05 未承認で本番ビルド失敗 | ✅ |
| AT-06 同じロックから AI オフラインで同じ IR | ✅ |
| AT-07 売れ筋の正しい SQL と期待結果 | ✅(フィクスチャの業務仮定は未承認) |
| AT-01〜04, 08〜10 | ❌ 未実装 |

## 4. 要件対応(FR / NFR / SEC)
| ID | 状況 | 備考 |
|---|---|---|
| FR-001 | ✅ | NFC は「検査して拒否」方式(lexical.md) |
| FR-002 | ✅ | 画面/動作/データ/約束 → 型付きグラフ |
| FR-003 | ✅ 静的解析のみ | 評価器(インタプリタ/生成コード)は未実装 |
| FR-004 | ✅ | |
| FR-005 | ✅ | ランキング形状のみ |
| FR-006 | ✅ | 質問は診断ヒント + `koto resolve` の候補ファイル |
| FR-007 | △ | 承認・ロックは実装。真正性(署名)は ADR-007 待ちで、承認入りの本番ビルドは拒否 |
| FR-008 | ✅ | |
| FR-009〜012 | ❌ | Web/Backend 生成なし(SQL 低水準化のみ) |
| FR-016 | △ | VAG に依存辺はあるが影響列挙コマンドは未実装 |
| FR-018 | △ | VAG ノードに source_span。生成コードとの対応は未実装 |
| FR-020 | ✅ | |
| NFR-001 | ✅ | 負例テスト |
| NFR-002 | ✅ | AI 接続コードが存在しない |
| NFR-003 | ❌ 未計測 | 分岐網羅率の計測(cargo-llvm-cov 等)は CI 未導入 |
| NFR-004 | △ | 1,000 行全段検査 < 5 秒のテストのみ。差分検査・基準機は未決定 |
| NFR-009 | △ | CI マトリクス(Linux/macOS/Windows)を定義。実機確認は Linux のみ |
| SEC-01 | ✅ 静的 | デフォルト拒否。実行時認可は未実装 |
| SEC-04 | ✅ クエリ生成 | バインド変数のみ |
| SEC-08 | ✅ | 候補の厳格スキーマ検査 + 文型での再照合 |
| SEC-09 | △ | 改ざん検出のみ。署名は未実装 |
