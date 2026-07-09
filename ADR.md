# CAD MVP Architecture Decisions

Status: accepted

Date: 2026-07-08

## Context

Jw_cad風の2D建築作図体験を、AIが直接編集できるテキスト正本で再構成する。目的はCAD本体のフォークではなく、NDJSON正本、厳格検査、SVG表示、意味付き差分、将来のJWW/DWG連携を持つGit-native CAD環境を作ること。

## Decisions

### Product Boundary

- MVP初日から自前CADソースを正本にする。
- CADプログラムの責務は、正本の読込、整形、検査、描画、差分、将来のexportに限定する。
- AI/エージェントの責務は、編集対象ファイル選択、Git復元点管理、レビュー単位分割に置く。
- アプリ内LLM、APIキー管理、クラウド同期、チーム管理、権限管理はMVPに入れない。
- MVPは完全ローカル。共有と復元はGitに任せる。

### Domain Scope

- 対象はJw_cad風の2D建築作図に限定する。
- MVPは幾何プリミティブを正本にする。
- 壁、柱、建具、部屋などの建築オブジェクトは正本スキーマに入れない。
- BIM、3D、パラメトリック拘束、建築法規検査、会社別製図標準の完全対応はMVP外。
- 寸法は幾何拘束ではなく注釈エンティティとして扱う。

### Jw_cad Compatibility

- 「Jw_cad準拠」は、作図文化、操作感、レイヤ/線色/線種思想の準拠を意味する。
- JWW完全互換、Jw_cad内部挙動完全再現、既存JWWの無劣化往復変換はMVPで保証しない。
- JWW import/exportは早期Post-MVP要件にする。
- Jw_cad本体のコード、画面素材、文言、完全な画面配置は流用しない。

### Source Format

- 図形本体はNDJSONにする。
- 設定ファイルはTOMLにする。`serde_yaml` は使わない。
- `schema_version` をプロジェクト単位とエンティティ単位に持たせる。
- 後方互換読込は増やさず、破壊的変更は `cadc migrate` で明示変換する。
- 通常の `check/render/diff` は最新schemaだけを受ける。

### Project Layout

```text
project/
  cad.project.toml
  rules/
    layers.toml
    styles.toml
  drawings/
    plan_1f/
      sheet.toml
      entities.ndjson
  blocks/
    door_910/
      entities.ndjson
  comments/
    plan_1f.ndjson
  build/
    plan_1f.svg
    plan_1f.diff.svg
    check.json
    diff.json
```

- `build/`, `snapshots/`, `exports/` は生成物としてGit管理しない。
- コメントは将来Git管理するが、MVPでは作成機能をPost-MVPに回す。viewerはコメント表示だけ対応してよい。

### Drawing Model

- 単位はmm固定の実寸モデル空間。
- 座標系はX右、Y上。SVG/GUI表示時だけY軸反転する。
- 角度はdegreeで保存する。
- 小数mmを許可し、`cadc format` で小数桁を正規化する。
- 内部計算は `f64`、保存は最大3桁程度、比較epsilonは `0.001mm` を目安にする。
- MVPは1図面=1用紙に限定する。
- 用紙、縮尺、原点は `sheet.toml` で管理する。

### Entity Set

MVPのエンティティ型は以下に限定する。

```text
line
polyline
arc
circle
text
dimension
block_ref
```

- 楕円、スプライン、ハッチ、画像、塗り、属性表はMVPに入れない。
- ブロックは単純な再利用図形参照に限定する。
- 属性付き建具、部品ライブラリ、建具表、集計はMVP外。

### Layers And Styles

- レイヤはJw_cad風のレイヤグループ + レイヤ番号 + 表示名で扱う。
- エンティティ側はレイヤIDを参照する。
- 線種はID参照にする。dash配列はTOML側で解決する。
- 色はID参照にする。RGBと印刷線幅はTOML側で解決する。
- 線幅は色/レイヤ定義から解決し、エンティティごとの `line_width` override はMVPで入れない。
- 文字スタイルと寸法スタイルはstyle ID参照にする。

### Checker

- `cadc check` は標準で厳格モードにする。
- 不正NDJSONは図面全体を失敗にする。
- エラーにはファイル、行番号、エンティティID、フィールド、原因を含める。
- 寛容描画は将来 `--best-effort` を明示した場合だけにする。
- MVPの検査範囲は、形式検証、参照検証、幾何検証、最小レイヤ規則に限定する。
- 閉領域チェックはMVPに入れるが、ハッチ/塗り生成はしない。

### Rendering And Diff

- SVGを正規レンダー出力にする。
- PDF直接出力、DXF export、JWW import/export、DWG import/exportはPost-MVP。
- MVPに意味付き図面diff + SVG重ね表示を入れる。
- diffはNDJSONの永続IDを使う。図形生成時にIDを発行し、座標や属性変更ではIDを変えない。
- SVGには `data-entity-id`、`data-layer`、`data-bbox` などのメタ情報を埋める。
- 見た目diffでは追加、削除、変更、線幅変更、文字bbox重なり、用紙外を扱う。

### AI Editing

- AIは直接NDJSONを書き換える。
- CAD側にoperationログ、独自Undo、独自トランザクションは入れない。
- 事故防止はGit、`cadc format`、`cadc check`、`cadc render`、`cadc diff`、人間レビューに寄せる。
- AI編集後にタイムラグなく確認したい要求は重要。MVPでは生成済みSVG/JSONをviewerで見る。Post-MVPでファイル監視、`cadc serve`、またはdesktop app統合により低遅延プレビューを実現する。

### Viewer

- MVPのGUIは自前の軽量viewerにする。
- QCAD連携はPost-MVP。
- viewerはPreact/Viteで実装する。
- viewerはNDJSONを直接描画せず、`cadc render/diff/check` が生成したSVG/JSONを読む。
- MVPでは単一プロジェクトパスと固定 `build/` 出力を前提にする。
- 任意プロジェクト選択、コメント作成、ローカルAPI、ファイル監視はPost-MVP。
- `cadc serve` はMVPから外す。ローカルAPIが必要になった段階で追加する。

### Implementation

- 実装言語はRust。
- 初日からCargo workspaceにする。

```text
crates/
  cad-model
  cad-check
  cad-render-svg
  cad-diff
  cad-cli
apps/
  viewer
examples/
  house-small
```

- MVPのRust依存は以下に限定する。

```text
serde
serde_json
toml
clap
thiserror
miette
ulid
tempfile
svg
insta
```

- MVPのviewer依存は以下に限定する。

```text
preact
@preact/preset-vite
vite
typescript
@playwright/test
tailwindcss
lucide-preact
```

### Platform And License

- MVP基準環境はmacOS Apple Silicon。
- 後続でmacOS Intel、Windows、Linuxへ広げる。
- 自前core、`cadc`、viewerはApache-2.0を推奨する。
- QCAD連携は別plugin/別repo/明確な境界にする。

## Consequences

- MVPは「CADとして編集できるGUI」ではなく、「AIが編集したNDJSON図面を厳格に検査し、SVGで確認し、意味付き差分をレビューする環境」になる。
- 低遅延プレビュー要求は強いが、MVPではローカルAPIを入れないため、自動再生成やファイル監視はPost-MVPで扱う。
- JWW/DXF/PDF/DWGを境界形式に回すことで、正本スキーマ、checker、SVG renderer、diffに集中できる。
