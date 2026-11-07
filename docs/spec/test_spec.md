# テスト仕様とトレーサビリティ

状態: 実装中 / 最終更新: 2026-11-07 / 上位文書: 技術仕様書 §16, §19, 要件 §9

## 1. 実行方法
```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test                                   # 全ユニット・ゴールデン・結合テスト
python3 tools/validate_schemas.py target/debug/koto   # 生成物の JSON Schema 検証
node --no-warnings --test tests/e2e/backend.test.ts tests/e2e/backend_security.test.ts tests/e2e/postgres.test.ts tests/e2e/web.test.ts tests/e2e/oidc.test.ts tests/e2e/extensions.test.ts
#   生成物を起動しての結合・E2E(cd tests/e2e && npm install が必要。PostgreSQL は KOTONOHA_TEST_PG_URL を設定したときだけ)
./target/debug/koto -C examples/shop test   # API 契約から導いた契約テスト(FR-019)
./target/debug/koto -C examples/lab test    # 拡張(分析・実験)を含む例の契約テスト
tools/mobile-syntax/run.sh                   # 生成 Swift / Kotlin の構文検査
tools/verify_android_jvm.sh                  # Android 非 UI 層の kotlinc コンパイルと実 API 応答の解析
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
| Integration(生成 backend) | `tests/e2e/backend.test.ts` | 認可 401/403、偽造トークン、取引とロールバック、入力検証、SQL インジェクション、売れ筋、秘密情報の非混入 |
| E2E(生成 web) | `tests/e2e/web.test.ts` | SSR 表示、遷移、ボタン → 表示更新、XSS、フォーム、CSRF、CSP |
| 結合(権限・運用) | `tests/e2e/backend_security.test.ts` | 動作単位の許可、行レベル認可、JWKS RS256/ES256・alg 混同・iss/aud、ロール対応、ページング、レート制限、差分マイグレーション |
| PostgreSQL | `tests/e2e/postgres.test.ts` | 一覧・同時注文(SERIALIZABLE)・集計・CHECK 制約・マイグレーション |
| OIDC | `tests/e2e/oidc.test.ts` | 模擬 IdP で認可コード + PKCE → JWKS 検証 → ロールで購入、state 不一致の拒否 |
| アクセシビリティ | `tests/e2e/web.test.ts` | axe-core(WCAG 2.2 A/AA)で 5 画面の違反ゼロ |
| 契約テスト | `koto test` | API 契約の公開範囲・401・403・400 |
| 生成コードのゴールデン | driver `generated_code_matches_golden_snapshots` | 4 ターゲットの生成コード |
| 拡張(結合・E2E) | `tests/e2e/extensions.test.ts` | 関数・TS/Python 外部関数・戻り値の型検証・分析(SQLite/PostgreSQL)・実験の割付と成果・Web の変種表示 |
| 拡張(負例) | driver `pure_functions_*`, `external_files_*`, `analyses_*`, `experiments_*`, `components_*`, `external_functions_declared_effects_*`, ext `rejects_*` | 副作用・パス逸脱・SQL の書込み/他表/関数・実験の誤り・実装欠落で必ず失敗 |
| モバイル | `tools/mobile-syntax`, `tools/verify_android_jvm.sh` | 構文検査・Android 非 UI 層のコンパイル。Compose 画面と iOS のビルドは CI(未実行: K-01) |

## 3. 受入テストケース対応
| TC | 内容 | 状況 | テスト |
|---|---|---|---|
| TC-001 | 売れ筋 → 30日・売上金額・20件 | ✅ | knowledge `tc001_ranking_meaning_is_fixed`, codegen `at07_*` |
| TC-002 | 5件表示 → 件数だけ 5 | ✅ | intent `tc001_tc002_*`, codegen `tc002_limit_override_changes_only_count` |
| TC-003 | 未定義「人気」→ 確認要求 | ✅ | intent `tc003_*`, driver `tc003_*` |
| TC-004 | 未承認 → 本番ビルド拒否 | ✅ | driver `tc004_at05_*`, cli `flexible_intent_*` |
| TC-005 | 型不一致 → TYP001 | ✅ | types `tc005_type_mismatch_is_typ001` |
| TC-006 | data.read のみで更新 → SEC001 | ✅ | validator `tc006_*` |
| TC-007 | 権限なし更新 → HTTP 403 | ✅ | e2e backend `AT-04 / TC-007` |
| TC-008 | XSS 非成立 | ✅ | e2e web `TC-008` |
| TC-009 | 同一ロック・AI オフライン → 同一 VAG | ✅ | driver `tc009_*`, cli `graph_is_reproducible_*` |
| TC-010 | 知識変更 → LOCK001 | ✅ | driver `lock_roundtrip_and_mutation_tests`, cli `lock_stale_*` |
| TC-011 | Web 商品一覧の表示・遷移 | ✅ | e2e web `AT-01 / TC-011` |
| TC-012 | 注文登録と在庫の原子的更新 | ✅ | e2e backend `TC-012`(失敗時ロールバック含む) |
| TC-013 | ターゲット未対応 → TGT001 | ✅ | codegen / driver / cli `tc013_*`(未知機能・小数型) |
| TC-014 | iOS/Android 商品一覧(P1) | ⚠ 生成のみ | 未ビルド(K-01) |

| AT | 状況 |
|---|---|
| AT-05 未承認で本番ビルド失敗 | ✅ |
| AT-06 同じロックから AI オフラインで同じ IR | ✅ |
| AT-07 売れ筋の正しい SQL と期待結果 | ✅(フィクスチャの業務仮定は未承認) |
| AT-01〜04, AT-10 | ✅ E2E |
| AT-08 | ✅ `koto impact`(cli `init_doctor_test_and_impact_commands`) |
| AT-09 | ⚠ 生成のみ |

## 4. 要件対応(FR / NFR / SEC)
| ID | 状況 | 備考 |
|---|---|---|
| FR-001 | ✅ | NFC は「検査して拒否」方式(lexical.md) |
| FR-002 | ✅ | 画面/動作/データ/約束 → 型付きグラフ |
| FR-003 | ✅ | 静的解析 + 生成 backend で実行 |
| FR-004 | ✅ | |
| FR-005 | ✅ | ランキング形状のみ |
| FR-006 | ✅ | 質問は診断ヒント + `koto resolve` の候補ファイル |
| FR-007 | △ | 承認・ロックは実装。真正性(署名)は ADR-007 待ちで、承認入りの本番ビルドは拒否 |
| FR-008 | ✅ | |
| FR-009〜012 | ✅ | Web/Backend 生成と E2E。差分マイグレーションは未実装(K-05) |
| FR-013/014 | ⚠ | 生成のみ(K-01) |
| FR-016 | ✅ | `koto impact <名前>` / `--base <旧 VAG>` |
| FR-019 | ✅ | 契約テストの生成と実行(`koto test`)・業務規則の雛形 |
| FR-018 | △ | VAG ノードに source_span。生成コードとの対応は未実装 |
| FR-020 | ✅ | |
| NFR-001 | ✅ | 負例テスト |
| NFR-002 | ✅ | AI 接続コードが存在しない |
| NFR-003 | ❌ 未計測 | 分岐網羅率の計測(cargo-llvm-cov 等)は CI 未導入 |
| NFR-005 | △ | axe-core 自動監査で違反ゼロ。手動監査は未実施 |
| NFR-004 | △ | 1,000 行全段検査 < 5 秒のテストのみ。差分検査・基準機は未決定 |
| NFR-009 | △ | CI マトリクス(Linux/macOS/Windows)を定義。実機確認は Linux のみ |
| SEC-01 | ✅ | デフォルト拒否(静的 + 実行時) |
| SEC-03 | ✅ | 認可は backend のみ |
| SEC-05 | ✅ | React のエスケープ + JSON 埋め込みのエスケープ + CSP |
| SEC-06 | ✅ | 秘密は環境変数のみ。生成物に含まないことを E2E で確認 |
| NFR-008 | ✅ | JSON ログ + X-Trace-Id |
| SEC-04 | ✅ クエリ生成 | バインド変数のみ |
| SEC-08 | ✅ | 候補の厳格スキーマ検査 + 文型での再照合 |
| SEC-09 | △ | 改ざん検出のみ。署名は未実装 |
| SEC-02 | ✅ | JWT(HS256 / JWKS RS256・ES256)。IdP 選定は ADR-002 |

## 5. 追補要件(NR)対応
| ID | 状況 | テスト |
|---|---|---|
| NR-01 | ✅ | cli `explain_*`・diag excerpt・e2e web(IME・全角数字) |
| NR-02 / NR-03 | ✅ | e2e backend_security(ヘッダ・認証失敗遮断・本番モード)、CI security(cargo audit / npm audit / CodeQL) |
| NR-04 | ✅ Web / backend、モバイルは CI ビルドのみ | e2e extensions `NR-04 *`(割付の Rust/TS 一致・成果の 1 回計上・結果の許可・SSR の変種固定)、driver `experiments_*` |
| NR-05 | ✅ | driver `pure_functions_*`、e2e extensions `NR-05 …` |
| NR-06 | ✅ | e2e extensions `NR-06 …`(戻り値の型検証・内部情報の非開示)、driver `changing_an_implementation_file_changes_the_graph_hash`・`components_*` |
| NR-07 | ✅ | ext `accepts_*`/`rejects_*`、driver `analyses_*`、e2e extensions `NR-07 *`(SQLite / PostgreSQL) |
| NR-08 | ✅ | e2e extensions(Python 外部関数の実行・例外・型違い) |
