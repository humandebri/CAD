# CAD Architecture Decisions

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
- JWWは境界形式として扱い、preserve import と Experimental export を提供する。
- hash付きfixture corpusが主要record classを網羅するまで、exportはExperimentalと表示する。
- Jw_cad本体のコード、画面素材、文言、完全な画面配置は流用しない。

### Source Format

- 図形本体はNDJSONにする。
- 設定ファイルはTOMLにする。`serde_yaml` は使わない。
- `schema_version` をプロジェクト単位とエンティティ単位に持たせる。
- schema `0.2` のみを受け付ける。旧 schema の読み込み・移行・fallback は
  実装しない。

### Project Layout

```text
project/
  cad.project.toml
  rules/
    layers.toml
    styles.toml
  drawings/
    plan_1f/
      layouts.toml
      entities.ndjson
  blocks/
    <block_id>/
      definition.toml
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
- コメントは `comments/<drawing>.ndjson` に保存し、viewerから作成・状態更新できる。

### Drawing Model

- 単位はmm固定の実寸モデル空間。
- 座標系はX右、Y上。SVG/GUI表示時だけY軸反転する。
- 角度はdegreeで保存する。
- 小数mmを許可し、`cadc format` で小数桁を正規化する。
- 内部計算は `f64`、保存は最大3桁程度、比較epsilonは `0.001mm` を目安にする。
- 1図面に複数layoutを許可し、active layoutをrender/print/exportの基準にする。
- 用紙、縮尺、原点、余白、plot areaは `drawings/<drawing>/layouts.toml` で管理する。

### Entity Set

schema `0.2` のエンティティ型は以下を対象とする。

```text
line
polyline
arc
circle
ellipse
text
dimension
point
solid
curve_solid
block_ref
hatch
```

- block definition/reference、hatch、layoutはchecker、renderer、diff、編集、
  JWW、PDFで同じ正本モデルを使う。
- 属性付き建具、部品ライブラリ、建具表、集計、BIM、3Dは対象外とする。

### Layers And Styles

- レイヤはJw_cad風のレイヤグループ + レイヤ番号 + 表示名で扱う。
- エンティティ側はレイヤIDを参照する。
- 線種はID参照にする。dash配列はTOML側で解決する。
- 色はID参照にする。RGBと印刷線幅はTOML側で解決する。
- 線幅は色/レイヤ定義から解決し、エンティティごとの `line_width` override はMVPで入れない。
- 文字スタイルと寸法スタイルはstyle ID参照にする。

JWW境界形式の忠実度を上げるためentity別の
optional pen参照を追加する。未指定entityは従来どおりlayer定義へfallback
する。layer group、表示順、縮尺、表示状態、lock、active layerも
`layers.toml`の正本属性として保持する。

### Checker

- `cadc check` は標準で厳格モードにする。
- 不正NDJSONは図面全体を失敗にする。
- エラーにはファイル、行番号、エンティティID、フィールド、原因を含める。
- 寛容描画は将来 `--best-effort` を明示した場合だけにする。
- 形式、参照、有限値、閉路、自己交差、block cycle、layout範囲を厳格に検査する。
- simple hatchは単一閉loopに限定し、表現不能なJWW要素はstrict blockerまたは
  明示的lossy warningとする。

### Rendering And Diff

- SVGを正規レンダー出力にする。
- PDF直接出力とJWW import/exportは実装済み。DXF/DWGは対象外とする。
- MVPに意味付き図面diff + SVG重ね表示を入れる。
- diffはNDJSONの永続IDを使う。図形生成時にIDを発行し、座標や属性変更ではIDを変えない。
- semantic diff JSON schema `0.2` はentity差分に加えてproject、layout、layer、pen、style、block定義の設定差分を型付きで表現する。
- SVGには `data-entity-id`、`data-layer`、`data-bbox` などのメタ情報を埋める。
- 見た目diffでは追加、削除、変更、線幅変更、文字bbox重なり、用紙外を扱う。
- layer既定値とentity pen overrideのstroke解決は `cad-model` を唯一の契約とし、
  SVGとPDFで共有する。PDFは色、物理線幅、線種、fill、文字style、寸法を保持する。
- color styleの`rgb`は画面表示、任意の`print_rgb`は印刷を表す。JWW v600の
  screen/print paletteを両方保持し、SVGは`rgb`、PDFは`print_rgb`を使用する。
- macOS移行版の日本語PDFはOFL-1.1のM+ 1pを文書単位でdeterministic subsetし、
  CIDFontType2とToUnicode mapとして埋め込む。外部parser/rasterizer CIは
  portable release gateとして残す。

### Editing And History

- AIは直接NDJSONを書き換える。
- AI・GUI編集は共通 transaction と revision 検査を通す。
- canonical source の分類とrevision manifestは `cad-model` を唯一の判定元とし、
  watcher、review、PDF exportから共有する。
- source置換は対応OSのatomic exchangeを使い、競合または曖昧なcrash recovery
  では全候補bytesを `build/.cad-recovery/` に保全してfail closedする。
- entities、comments、layers の before/after manifest を
  `build/.cad-history/` に保存し、Undo/Redoと再起動後復元を提供する。
- 履歴破損、stale revision、checker failureでは正本を変更せず、UIを再同期する。
- `.cad-history` と build成果物はwatcher、review、diff、AI contextから除外する。

### Viewer

- GUIは自前の軽量viewerにする。
- QCAD連携は対象外とする。
- viewerはPreact/Viteで実装する。
- viewerはNDJSONを直接描画せず、`cadc render/diff/check` が生成したSVG/JSONを読む。
- プロジェクト選択、コメント作成、ファイル監視、layout切替、PDF exportを
  Tauri desktopで提供する。
- `cadc serve` とHTTP APIは追加しない。

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

- Rust依存はCargo workspaceの各crateで管理する。

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

- viewer依存はpackage lockfileで固定する。

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

- 開発・基準環境はmacOS Apple Silicon。Windows/Jw_cad実機確認は任意の追加証拠とする。
- 自前core、`cadc`、viewerはApache-2.0を推奨する。
- QCAD連携は別plugin/別repo/明確な境界にする。

## Consequences

- CADとして編集できるGUIを含むローカル環境とし、active layoutを全出力の基準にする。
- JWW/PDFは正本から生成する境界出力であり、正本は常にNDJSON/TOMLである。

## JWW Codec And Experimental Export

- JWWは正本にせず、NDJSON/TOMLとの境界形式として扱う。
- importとversion 600 exportは共有Rust codecを使い、外部CAD processへ依存しない。
- import時の原本bytesとBLAKE3/SHA-256/source revisionをproject内に保存する。
- 未知classと未対応versionはread-onlyで保持し、原本抽出だけを許可する。
- 未編集projectのpreserve exportは原本をbyte-exactで返す。編集後は原headerを
  保持する。manifest 0.2のhash検証済みrecord sidecarで文字type/end、寸法
  SXF/補助record、block metadataを保持する。sidecar欠損・改変はfail closedし、
  表現可能な近似・展開・文字置換はreportへ記録する。
- JWW再import時のstable entity IDと任意編集後の無劣化往復は保証しない。
- 通常exportはbest-effortとし、`--strict`指定時は近似・展開・文字置換を拒否する。
- release gateはhash付きfixture inventory、golden bytes、再import後のsemantic comparisonとする。
