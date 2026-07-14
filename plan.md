# CAD 実装計画

## 現在の契約

- schema `0.2` のみを正本として扱う。旧 schema の読み込み、移行、互換
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
  する。JWW export は strict/lossy report を生成し、Windows/Jw_cad 検証が
  完了するまで Experimental 表示を維持する。
- PDF は Rust renderer が active layout を直接解釈し、printable layer のみを
  atomic publish する。

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
pnpm --dir apps/viewer typecheck
pnpm --dir apps/viewer build
pnpm --dir apps/viewer test
pnpm --dir apps/viewer test:e2e
pnpm --dir apps/viewer desktop:smoke
pnpm --dir apps/viewer desktop:e2e
```

CI は `rust-toolchain.toml` と viewer の lockfile を固定し、上記の Rust、型検査、
unit、Playwright、macOS embedded-WebDriver gate を同じ順序で実行する。
Windows/Jw_cad の preserve round-trip と macOS の PDF Preview 確認は
release gate として別途記録する。

## 次のリリースゲート

- native watcher を優先し、poll fallback の対象拡張子と間隔を制限する。
- PDF の pen、line type、line width、text style fidelity を SVG と共通化する。
- checker の大規模 polyline/hatch self-intersection を空間 index で高速化する。
- Windows/Jw_cad preserve import → export → re-import の fixture/hash/report を
  再現可能な artifact として記録する。
