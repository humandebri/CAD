---
name: cad-drafting
description: このCADのTOML・NDJSONを直接編集して2D図面を作図・修正し、検査・描画・PDF/JWW出力を行う。平面図と部品資料から室内の壁面展開図を作る場合にも使う。ソフト本体の開発だけを行う依頼には使わない。
---

# CAD作図

エージェントが図形・寸法・文字の座標を編集する。チェッカーはデータの不備を検査し、レンダラーは指定した図形を描画する。どちらも建築的な判断や文字の自動配置を代行しない。

## 作業対象

- リポジトリルートと対象CADプロジェクトを区別する。CLIはリポジトリルートで実行し、対象プロジェクトには絶対パスを渡す。
- 適用されるAGENTS.md、既存差分、対象図面のレイアウト・レイヤー・スタイル・ブロックを読む。ユーザーの既存変更を保持する。
- 現行仕様は `crates/cad-model/src/lib.rs`、寸法は `crates/cad-model/src/dimensions.rs`、コマンドは `crates/cad-cli/src/main.rs` を参照する。`examples/direct-edit-guide` に全図形種・固定点/参照点寸法の検査可能な実例がある。形式・座標換算・直接編集候補・画像一括出力には [直接編集の支援](references/direct-edit.md) を読む。これらのパスはリポジトリルート基準。未知のフィールドを独自に追加しない。
- 新規図面、座標変換、複写、参照寸法、用紙設定には [正本編集の手引き](references/source-editing.md) を読む。Gitの版を比較する依頼には隣の `cad-review` スキルを使える。
- 壁面展開図では [展開図の手順](references/interior-elevations.md) を読む。
- 図面間コピー・正本部品、建具・2線・楕円・自由曲線・接線・接円・分割・倍率・属性取得、文字置換・style変更・縦列注記・計算注記・計測・DXF/JWS取り込み、2.5D投影・日影・天空図、混在縮尺の印刷配置には [生成・計測CLI](references/toolkit.md) を読む。生成した編集要求はプレビューしてから適用でき、アプリ履歴にも残る。

## 取り込みと直接編集

JWWを受け取ったら `inspect-jww` で互換性を確認し、`import-jww INPUT --out NEW_PROJECT` で未使用の出力先に取り込む。警告と取込レポートを確認し、読めない図形を存在しないものとして扱わない。
`exact_only` の保存専用取り込みは編集できない。元JWWの抽出で保持し、来歴を削除して編集制限を回避しない。現行スキーマは0.3のみ。

編集可能な正本は `cad.project.toml`、`rules/*.toml`、`drawings/*/{layouts.toml,entities.ndjson}`、`blocks/*/{definition.toml,entities.ndjson}`。`comments/` はアプリ管理データであり直接編集しない。`interop/`、来歴、履歴・復旧データ、生成物を編集元にしない。

- NDJSONは空でない各行にJSON図形を一つだけ置く。変更する行だけを書き換え、削除は行全体を削除する。
- 閉じたpolylineは `closed: true` に加え、点列の末尾を先頭点と一致させる。
- 更新時のIDを維持する。新規図形は `ent_<26文字ULID>` の有効かつ一意なIDにする。
- レイヤー、ペン、スタイル、レイアウト、ブロック、塗りの参照を維持する。参照寸法の対象を削除・移動する場合はその影響も調べる。
- 書き込み直前に対象ファイルを再読込し、読取り時から変わっていれば差分を取り直す。Desktopと同じ図面を同時に編集しない。直接編集はアプリのトランザクションAPIによる競合防止を通らない。
- 直接編集はDesktop Undo/Redoの対象外。差分で確認し、復旧のために既存変更を破棄するGit操作を行わない。
- `cadc format` を機械的に実行しない。現実装はコメントも書き換える。必要な表記の調整は許可された編集対象だけで行う。

## 検査・描画・出力

以下の各コマンドの前に `cargo run -p cad-cli --` を付ける。変数は対象に置き換え、シェルではパスを引用する。

```text
check PROJECT --target cad --format json --out -
render PROJECT --drawing NAME --layout LAYOUT --format svg --out OUTPUT.svg
export-pdf PROJECT --drawing NAME --out OUTPUT.pdf
export-pdf PROJECT --drawing NAME --layout LAYOUT --out OUTPUT.pdf
check PROJECT --drawing NAME --target jww-v600 --format json --out -
export-jww PROJECT --drawing NAME --out OUTPUT.jww --report REPORT.json
```

直接編集の各まとまりの後にCADチェックを実行する。エラーは作業完了・出力の阻害要因として修正する。長いビルド出力は利用可能なら `qrun --` で抑え、失敗時はログを絞って調べる。
CLIの正本読込コマンドは開始時にアプリ管理の未完了トランザクションを復旧する場合がある。復旧エラーが出たら診断を報告し、ジャーナルや退避候補を手作業で消さない。

SVGを実際に表示して形状・向き・文字と寸法の重なり・欠落を確認する。PDFを納品する場合はPDFも画像化またはビューアで開き、用紙内の収まりと縮尺を確認する。`render --drawing NAME --layout LAYOUT` で対象を明示する。開いているDesktopは正本の変更で再検査・再描画するが、表示を確認できていない場合は目視済みと報告しない。

JWW出力前は専用チェックを実行し、全警告を確認する。通常はbest-effort、近似・代替が許されない依頼では `--strict` を使う。JSONレポートを保持し、互換性の限界を伝える。既存成果物は不用意に上書きせず、`--force` は自動で付けない。

最後に変更範囲と差分を確認し、作った図面、検査結果、未確定事項、互換性警告、未実施の実機確認を簡潔に報告する。チェック合格だけで建築仕様の正しさやJw_cad完全互換を保証しない。
