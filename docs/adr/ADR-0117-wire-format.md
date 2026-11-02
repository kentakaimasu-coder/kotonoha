# ADR-0117 ワイヤ形式と共通 API 契約
状態: 提案 / 日付: 2026-11-03 / 関連: 要件 §7、docs/architecture.md §3〜4
- すべてのクライアントは backend の同じ HTTP/JSON API を使い、業務ロジックはサーバーだけで実行する。
- 型ごとの JSON 表現は docs/architecture.md §4。整数は JSON number とし、±2^53 を超える値は 422(JS クライアントでの精度落ちを防ぐため)。
- 代替案: 整数を文字列で送る。Swift/Kotlin では扱いやすいが、Web での表示・計算が煩雑になる。i64 全域が必要な業務が出たら再検討。
