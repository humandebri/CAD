# ドキュメント案内

起動・検査コマンドと機能概要は [README](../README.md) を参照。

## 作図と操作

| 文書 | 内容 |
| --- | --- |
| [直接編集の作図支援](direct-edit.md) | スキーマ、座標変換、反復作図、壁面展開、検査とプレビュー |
| [注記の縦書き](annotation-writing.md) | 配置方法と出力時の制約 |
| [一枚内の混在縮尺](sheet-viewports.md) | ビューポートの設定とレビュー範囲 |
| [2.5D投影・日影・水平天空](massing-calculations.md) | 計算条件、作図方法、検証範囲 |
| [CADの外で行う建築検査](plan-audit.md) | 対応台帳、干渉検査、修正候補 |

## 正本と設計

| 文書 | 内容 |
| --- | --- |
| [正本の編集契約](../SOURCE_FORMAT.md) | 編集可能なファイル、図形の記述例、参照寸法 |
| [エージェント向け作業規則](../AGENTS.md) | 直接編集、必須検査、JWW出力の規則 |
| [設計判断](../ADR.md) | モデル、境界形式、編集・履歴、実装構成 |
| [CAD作図スキル](../.agents/skills/cad-drafting/SKILL.md) | 正本を直接編集する作図手順 |
| [CADレビュースキル](../.agents/skills/cad-review/SKILL.md) | 図面の版比較、検査、互換性レビュー |

## 検証と互換性

| 文書 | 内容 |
| --- | --- |
| [2D CAD受入図面](cad-acceptance.md) | 自動検査と未実施の表示・印刷確認 |
| [JWW fixture corpus](../examples/jww-fixtures/README.md) | 固定入力、互換性チェックリスト、検証記録 |
| [JWS取り込みの検証範囲](jws-compatibility.md) | 検証した形式・実データと制約 |
| [テスト整理と実装改善](test-cleanup.md) | 2026-09-12の変更記録と検証結果 |

## 実装計画

| 文書 | 内容 |
| --- | --- |
| [現在の契約とリリースゲート](../plan.md) | 正本・履歴・出力の契約と検証ゲート |
| [計画一覧](../plans/README.md) | 計画001–011の状態と依存関係 |
| [Jw_cadとの差とGit連携の候補](jwcad-gap-analysis.md) | 機能差、候補、受入条件 |
| [Jw_cad機能差とGit連携の実装](../plans/011-jwcad-parity.md) | 各機能の実装状態と検証記録 |

リポジトリ共通のサンプルは `examples/` に置く。個別の作図案件と納品物は
Git管理外の `drawing-projects/`・`output/` に置き、共通文書と分けて扱う。
