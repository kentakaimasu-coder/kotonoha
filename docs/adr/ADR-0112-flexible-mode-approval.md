# ADR-0112 曖昧性モードと承認の適用範囲
状態: 提案 / 日付: 2026-11-02 / 関連: C-07, U-19, FR-006/007, SEC-08
1. 承認の単位は「意図 ID + 文ハッシュ → 既知文型の正規文」。承認記録 `kotonoha.approvals.json`(record_hash 付き)。
2. `@厳密` では承認を適用しない。`@柔軟(確認)` は意図 ID 一致、`@柔軟(自動)` は文ハッシュ一致の承認を再利用する。
3. 候補(`kotonoha.candidates.json`)は resolver アダプタが作る未信頼データ。スキーマ検査 → 仮承認での再解析で成立しない解釈は除外。`unresolved` が空でない候補は承認不可。
4. コマンド: `koto resolve` → `koto approve <id> <n> --approver 名前` → `koto approve --write-lock`。
5. 承認の真正性は ADR-007。
