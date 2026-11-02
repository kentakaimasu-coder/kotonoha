# ADR-0113 ロックのハッシュ範囲
状態: 提案 / 日付: 2026-11-02
- source_hash: `[{path, sha256(内容)}]`(パス順)の正規 JSON の SHA-256。コメント・空白だけの変更でも失効する(意味が変わっていないことを機械的に保証できないため)。
- knowledge_hash / schema_hash: VAG の `knowledge` / `schema` 節の正規 JSON。
- graph_hash: VAG 全体(source_span を含む)。
- 承認記録の集合のハッシュも `approval_reference.records_hash` として固定する。
