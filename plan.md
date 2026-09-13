# CAD 実装計画

## 現在の契約

- schema `0.3` のみを正本として扱う。旧 schema の読み込み、移行、互換
  fallback は実装しない。
- `drawings/<drawing>/layouts.toml` が用紙、向き、縮尺、原点、余白、plot
  area の唯一の正本である。`active_layout` を check、render、diff、PDF、JWW
  export の全てで使用する。
- 図形は `entities.ndjson`、block は `blocks/<id>/definition.toml` と
  `entities.ndjson`、レイヤは `rules/layers.toml`、コメントは
  `comments/<drawing>.ndjson` に保存する。
- `build/`、`build/.cad-history/`、`.cad-transactions/`、`.cad-recovery/` は
  生成物であり、source watcher、review、diff、AI context の入力対象外とする。
- source の編集、コメント、レイヤ更新、Undo/Redo は revision 検査、raw
  bytes・改行・末尾改行・permission 保持、atomic exchange、競合bytesの
  recovery 保全を共通不変条件とする。source の分類とrevision manifestは
  `cad-model` のAPIだけを使用する。
- JWW import の既定値は block preserve、`--flatten` は明示的な変換モードと
  する。v600 import は入力bytesとhash/source provenanceを保存し、未知class・
  未対応versionはread-onlyで保持する。未編集projectは元JWWをbyte-exactで
  保存できる。manifest `0.3` の `mapped_v600` importはhash検証済みの
  `interop/jww/records.ndjson`を使い、文字type/end、寸法SXF/補助record、block
  metadataを編集後も保持する。strict preserveでは既存寸法の表示値だけを変更し、
  通常のbest-effort exportでは形状・offset・style・鏡映変更を決定的なv600 recordへ
  再生成してwarningを記録する。旧manifestまたはsidecar不整合は
  `exact_only`へfail closedする。検証済みsnapshot取得後とpublish直前にcanonical
  source manifestを照合し、編集後のpreserve exportはcheckerとblock参照整合性を
  検査してfail closedする。互換範囲はhash固定fixture、record inventory、byte-exact
  unchanged export、semantic re-importで判定する。
- AIはcanonical TOML/NDJSONを直接編集し、`cadc check --target cad`を必須とする。
  JWW出力前は`--target jww-v600`のwarningを確認し、通常exportはbest-effort、
  `--strict`だけが近似をblockする。直接編集はDesktop履歴ではなくGitで追跡する。
- PDF は Rust renderer が active layout を直接解釈し、printable layer のみを
  atomic publish する。pen、色、線種、物理線幅、fill、文字style、寸法はSVGと
  共通の正本style解決を使用する。
- Desktop watcher は native filesystem watcher を優先する。開始できない場合
  だけ canonical source manifest を1秒周期で比較し、native開始後のruntime
  errorはeventとして通知する。
- Git HEAD reviewはcanonical repository/project pathとcommit OIDをkeyにした
  1-entry cacheを使う。同じOIDではworking treeが変化しても展開済みsourceを
  再利用し、OIDまたはproject変更時に置換する。

## 実装フェーズ

### Phase 1: 操作基盤

command registry、Esc/Enter/右クリックの統一、複数選択、window/crossing
選択、snap、preview、drawing/live-review 切替時の command reset を提供する。

### Phase 2: 編集 transaction

複数 entity の translate/copy/delete、rotate、mirror、offset、trim、extend を
一 request 一 revision 一 publish の transaction として処理する。途中失敗や
checker error では正本を変更しない。

### Phase 3: 履歴

`build/.cad-history/` に entities、comments、layers の before/after manifest
を保存し、Undo/Redo、再起動後の復元、revision conflict、100 世代制限を提供
する。履歴破損時は正本を維持する。

### Phase 4: 図面モデルと境界形式

block definition/reference、hatch、layouts を checker、renderer、diff、編集、
JWW import/export、PDF へ接続する。simple hatch と有限な layout を厳格に
検証し、表現不能な JWW 要素は strict blocker、lossy では warning とする。

## 検証ゲート

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check -p cad-desktop --no-default-features
pnpm --dir apps/viewer build
pnpm --dir apps/viewer test:unit
pnpm --dir apps/viewer test:e2e
pnpm --dir apps/viewer desktop:e2e
```

CI は `rust-toolchain.toml` と viewer の lockfile を固定し、上記の Rust、型検査、
unit、Playwright、macOS embedded-WebDriver gate を同じ順序で実行する。
JWWはfixture manifestとfile-level round-trip、PDFは外部parser/rasterizerを
release gateとする。外部アプリケーション確認は任意の追加証拠として扱う。

## 次のリリースゲート

- checker の大規模 polyline/hatch self-intersection を空間 index で高速化する。
- PDFのOFL日本語font deterministic subset埋め込みと外部
  parser/rasterizer CI gateは完了。
- JWW fixture corpusへsolid、dimension、block、hatch、paletteを追加し、hash、
  record inventory、byte-exact unchanged export、best-effort編集後のsemantic
  re-importをCIで検証する。corpus外のclassやfieldを完全互換とは表明しない。
