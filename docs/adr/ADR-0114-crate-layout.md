# ADR-0114 クレート構成の差分
状態: 提案 / 日付: 2026-11-02 / 関連: 技術仕様 §17
- `approval` と `lock` は 1 クレート `lock` に統合(承認記録とロックが同じハッシュ規則を共有するため)。
- `diag`(診断・終了コード)と `driver`(パイプライン統合)を新設。
- `adapters/ai` は `resolver` クレート内の trait とアダプタで代替。`adapters/{db,auth}`、`targets/*`、`runtime/*`、`stdlib/*` は未着手のため作っていない(空ディレクトリを作らない)。
