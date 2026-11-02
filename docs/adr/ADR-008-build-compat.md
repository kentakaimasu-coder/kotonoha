# ADR-008 ビルド互換性ポリシー
状態: 未決定 / 日付: 2026-11-02
## 推奨
compiler_version の major.minor が一致すればロックを再利用可。VAG の schema_version が変わる場合は再ロック必須。
## 暫定運用
compiler_version の完全一致を要求(不一致は LOCK001)。
