# 正本編集の手引き

以下はリポジトリ内のschema 0.3向け。フィールド一覧は `SOURCE_FORMAT.md` と `crates/cad-model/src/lib.rs` を読む。見た目が近い図形でもJWWの内部レコードを直接作らない。

## 図面と座標

新規プロジェクトでは小さな既存サンプルを参考に、`cad.project.toml`、`rules/layers.toml`、`rules/styles.toml`、`drawings/<NAME>/layouts.toml`、`entities.ndjson` を作る。サンプルから取り込む定義は必要なものを選び、来歴・生成物・コメントはテンプレートとして複製しない。CLIに `new`、`edit`、`render --drawing` は存在しない。DesktopのNew ProjectとCLIを混同しない。

座標はモデル空間のmm、Xは右、Yは上、角度は度。1:50の5000mmは正本で5000のまま。SVGのY反転を正本へ書き戻さない。

用紙設定例（`layouts.toml`）：

```toml
schema_version = "0.3"
active_layout = "default"

[layouts.default]
name = "default"
paper = "A3"
orientation = "landscape"
scale = "1/50"
origin = [0.0, 0.0]
margins = [10.0, 10.0, 10.0, 10.0]
```

`origin` はモデル空間の用紙左下。`margins` は紙上mmで左・上・右・下、原点を移動せず描画をクリップする。`plot_area` はモデル空間の `[xmin,ymin,xmax,ymax]`。文字スタイルの `height/width/spacing` もモデル空間の寸法で、1:50で紙上2.4mmの高さなら `height = 120`。線幅はモデル座標と同じ倍率で増やさず既存の印刷設定に合わせる。

## 生成と変換

- 新規IDは有効なULIDを生成し `ent_` を付ける。UUIDをそのまま使わない。既存プロジェクト全体（ブロックを含む）のIDと衝突しないことを確認する。
- 移動は座標へ同じ差分を加算する。円・楕円の半径や角度、寸法の `offset` は平行移動では変えない。固定寸法アンカーも移動し、参照アンカーには対象図形の移動を二重に適用しない。
- 複写は新しいIDを割り当てる。複写する寸法が複写集合内の図形を参照する場合、旧ID→新IDの対応表で `measurement` 内の `entity_id` も変更する。集合外の参照は元図形との関係を維持するか固定化するか依頼の意味で判断する。
- ブロック定義の編集は全配置へ影響する。一つの配置だけ変えるなら定義を別名へ複製し、その配置の `block` を更新する。ブロック内IDと参照寸法も再割当する。
- 閉じたpolylineは末尾点を先頭点に一致させる。hatchのループは別仕様なので同じ閉じ方を機械的に適用しない。
- `hatch.pattern` は `solid/parallel/cross`、`fill` は定義済み色が必須。parallel/crossの `scale` はモデルmmの線ピッチ。境界は自動追従しないため、壁を動かしたらループも確認する。

回転・反転・拡縮は図形ごとに中心、点列、角度、文字、固定アンカーを整合させる。非一様拡縮や反転を、block_refの負のscaleで表現しない。block_refは正の一様scaleと `mirror_x/mirror_y` を使う。複雑な変換は `cad-edit` の実装と既存テストに照合する。

## 寸法

次の例は同じ図面内の線に追従する水平寸法。`LAYER`、`DIM_STYLE`、IDは実際の定義・新規IDへ置き換える。各オブジェクトは一行にする。

```json
{"schema_version":"0.3","id":"ent_01JZ0000000000000000000000","type":"line","layer":"LAYER","p1":[0,0],"p2":[5000,0]}
{"schema_version":"0.3","id":"ent_01JZ0000000000000000000001","type":"dimension","layer":"LAYER","style":"DIM_STYLE","p1":[0,0],"p2":[5000,0],"offset":300,"value":null,"measurement":{"kind":"horizontal","first":{"kind":"entity","entity_id":"ent_01JZ0000000000000000000000","feature":"start"},"second":{"kind":"entity","entity_id":"ent_01JZ0000000000000000000000","feature":"end"}}}
```

`value: null` なら形状から測定値を表示する。文字で数値を上書きして形状の誤りを隠さない。`measurement` がある寸法はアンカーが測定を決めるため、`p1/p2` だけ変えても参照先は変わらない。

参照は同一図面または同一ブロック内のみ。別図面や配置済みブロック内部は参照できない。対象削除・頂点再編成で参照が無効になる場合、形状を維持する案、寸法を削除する案、最後の有効座標へ固定する案から依頼に合うものを選ぶ。判断に必要な情報がなければ対象を示して確認し、無関係な編集を進める。

## 検査後の確認

CAD checkerのエラー0を確認してから描画する。新しい図面が先頭とは限らないので、対象確認には `export-pdf --drawing NAME` が確実。描画結果では文字・寸法の重なり、用紙外、向き、印刷対象レイヤーを確認する。SVGとPDFでクリップや文字が同じように見えると決めつけない。

JWW経由で再取り込みするとIDは維持されない。Gitで同じ図形の履歴を追いたいときは正本を継続編集する。
