# MVP Implementation Plan

Status: implemented through Phase 6

Source: [ADR.md](ADR.md)

## MVP Completion Criteria

MVPは、AIが編集したNDJSON図面を検査し、SVGで確認し、意味付き差分をレビューできる状態で完了とする。

- サンプル図面をNDJSON/TOMLで表現できる。
- `cadc check` が厳格検査できる。
- `cadc render --format svg` がSVGを生成できる。
- `cadc diff --format json/svg` が意味付きdiffを生成できる。
- Preact/Vite viewerでSVG、diff、check結果を確認できる。
- Rust単体テスト、snapshot test、Playwright確認が通る。

MVPに含めないもの:

- DXF/PDF/JWW/DWG export
- QCAD連携
- `cadc serve`
- コメント作成API
- desktop app化
- アプリ内LLM
- クラウド同期

## Phase 0: Repository Scaffold

目的: 実装境界を先に固定し、以後の差分を小さく保つ。

実装:

- Cargo workspaceを作成する。
- Rust crateを作成する。
  - `crates/cad-model`
  - `crates/cad-check`
  - `crates/cad-render-svg`
  - `crates/cad-diff`
  - `crates/cad-cli`
- viewer appを作成する。
  - `apps/viewer`
- サンプルプロジェクトを作成する。
  - `examples/house-small`
- `.gitignore` に生成物を追加する。
  - `build/`
  - `snapshots/`
  - `exports/`
- Apache-2.0ライセンス方針を明記する。

検証:

- `cargo test --workspace` が実行できる。
- viewerの依存解決と起動ができる。
- 空のworkspaceで不要な警告が出ない。

完了条件:

- workspace構成とサンプルディレクトリがADRと一致する。

## Phase 1: CAD Source Model

目的: NDJSON/TOML正本を読み、型付きモデルへ変換できるようにする。

実装:

- `cad.project.toml` を読む。
- `rules/layers.toml` と `rules/styles.toml` を読む。
- `drawings/*/sheet.toml` を読む。
- `entities.ndjson` を読む。
- `schema_version` をプロジェクト単位とエンティティ単位で検証する。
- MVPエンティティ型を定義する。
  - `line`
  - `polyline`
  - `arc`
  - `circle`
  - `text`
  - `dimension`
  - `block_ref`
- mm実寸、X右Y上、degree、小数mmをモデル前提にする。
- ULID形式の永続IDを扱う。
- `cadc format` の土台を作る。

検証:

- 正常なサンプルNDJSON/TOMLを読み込める。
- 不正JSON、不正TOML、未知schema、不明entity typeで失敗する。
- 小数値の丸め方をsnapshot testで固定する。

完了条件:

- `examples/house-small` の図面を型付きモデルとして読み込める。

## Phase 2: Strict Checker

目的: AI編集後の破損を早期検出する。

実装:

- `cadc check project/ --format json --out build/check.json` を実装する。
- 人間向け診断に `miette` を使う。
- JSON診断をviewer用の正式契約にする。
- エラーに以下を含める。
  - file
  - line
  - entity id
  - field
  - code
  - message
- 形式検証を実装する。
  - 必須フィールド
  - 型
  - 未知フィールド
  - schema_version
- 参照検証を実装する。
  - 未定義レイヤ
  - 未定義style
  - 未定義block
  - 重複ID
- 幾何検証を実装する。
  - zero length
  - invalid arc
  - tiny gap
  - closed polyline
  - self intersection
  - bbox
- 最小レイヤ規則を実装する。
  - 非印刷レイヤ上の寸法/文字
  - レイヤ定義と線種/色/線幅解決

検証:

- 各エラーにfixtureを用意する。
- 不正NDJSONは図面全体を失敗にする。
- `--best-effort` は実装しない。

完了条件:

- `cadc check examples/house-small --format json --out examples/house-small/build/check.json` が成功する。
- 代表的な破損fixtureが期待どおり失敗する。

## Phase 3: SVG Renderer

目的: 正本から確認可能な正規SVGを生成する。

実装:

- `cadc render examples/house-small --format svg --out examples/house-small/build/plan_1f.svg` を実装する。
- SVG生成には `svg` crateを使う。
- SVG座標へ変換する時だけY軸を反転する。
- レイヤ、色、線種、線幅、文字style、寸法styleを解決する。
- MVPエンティティ型をSVGへ描画する。
- 各SVG要素へメタ情報を埋める。
  - `data-entity-id`
  - `data-layer`
  - `data-bbox`
- `bbox` 計算をrenderer/checker/diffで共有できる形にする。

検証:

- SVG snapshot testを作る。
- 文字、寸法、block_ref、arcの代表例をfixture化する。
- 出力SVGに `data-entity-id` が含まれることを検証する。

完了条件:

- サンプル図面のSVGを生成し、ブラウザで目視確認できる。

## Phase 4: Semantic Diff

目的: NDJSONの永続IDに基づき、AI編集結果をレビュー可能にする。

実装:

- `cadc diff base/ head/ --format json --out build/diff.json` を実装する。
- `cadc diff base/ head/ --format svg --out build/plan_1f.diff.svg` を実装する。
- diff分類を実装する。
  - added
  - removed
  - modified
  - unchanged
- 変更理由をJSONに含める。
  - geometry changed
  - layer changed
  - style changed
  - text changed
  - line width changed
- SVG重ね表示を生成する。
  - 追加: 緑
  - 削除: 赤
  - 変更: 黄
- warningを生成する。
  - 文字bbox重なり
  - 用紙外
  - 線幅変更

検証:

- ID保持時は変更として検出する。
- ID追加/削除時は追加/削除として検出する。
- 座標変更、style変更、文字変更のsnapshot testを作る。

完了条件:

- サンプル図面の変更前後からJSON diffとSVG diffを生成できる。

## Phase 5: Viewer

目的: 生成済みSVG/JSONをGUIで確認する。

実装:

- Preact/Vite viewerを実装する。
- 固定パスの生成物を読む。
  - `/build/plan_1f.svg`
  - `/build/plan_1f.diff.svg`
  - `/build/check.json`
  - `/build/diff.json`
- 通常SVGとdiff SVGを切り替える。
- pan/zoomを実装する。
- SVG内の `data-entity-id` を使ってentity選択を実装する。
- check結果一覧を表示し、該当entityへ移動できるようにする。
- diff結果一覧を表示し、追加/削除/変更を確認できるようにする。
- コメント表示だけ対応する。コメント作成はMVP外。

検証:

- Playwrightでviewerを開く。
- SVGが非空で表示されることを確認する。
- diff切替、entity選択、check結果表示を確認する。
- 主要viewportでテキストやパネルが重ならないことを確認する。

完了条件:

- `build/` の生成物だけでレビュー画面が成立する。

## Phase 6: MVP Hardening

目的: 実装品質と運用手順を固定する。

実装:

- READMEにMVPの使い方を書く。
- サンプル修正フローを書く。
  - AIがNDJSONを編集
  - `cadc format`
  - `cadc check`
  - `cadc render`
  - `cadc diff`
  - viewerで確認
- CI相当のローカルコマンドを作る。
- fixtureとsnapshotを整理する。
- エラーコード一覧を整備する。

検証:

- `cargo test --workspace`
- `cargo run -p cad-cli -- check examples/house-small --format json --out examples/house-small/build/check.json`
- `cargo run -p cad-cli -- render examples/house-small --format svg --out examples/house-small/build/plan_1f.svg`
- `cargo run -p cad-cli -- diff examples/house-small examples/house-small-modified --format json --out examples/house-small/build/diff.json`
- viewerのPlaywright確認

完了条件:

- 新規参加者がREADME手順だけでMVPを再現できる。

## Post-MVP Roadmap

### Phase 7: Low-latency Preview

- ファイル監視を追加する。
- NDJSON/TOML変更時に `check/render/diff` を自動再実行する。
- `cadc serve` を追加する。
- viewerをlocalhost APIへ接続する。
- コメント作成とstatus変更を追加する。

### Phase 8: Desktop App

- Tauri化する。
- `cadc` coreを同梱する。
- ローカルAPIを維持するかIPCへ置換する。
- macOS Apple Silicon向け署名/配布を検証する。

### Phase 9: Boundary Format Export

- DXF exportを追加する。
- PDF exportを追加する。
- JWW import/exportを追加する。
- DWG対応は商用SDK境界を調査してから判断する。

### Phase 10: QCAD Integration

- QCAD pluginまたは外部スクリプトでNDJSON import/exportを試作する。
- QCAD側entityとNDJSON永続IDの対応を検証する。
- Jw_cad風操作実験の場として使う。

## Implementation Order

1. Phase 0
2. Phase 1
3. Phase 2
4. Phase 3
5. Phase 4
6. Phase 5
7. Phase 6

各Phaseは単独でレビュー可能な差分にする。次Phaseへ進む前に、該当Phaseの完了条件を満たす。
