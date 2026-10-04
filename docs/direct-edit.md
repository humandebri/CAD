# 直接編集の作図支援

正本は従来どおりTOML・NDJSON。以下のコマンドはリポジトリルートから `cargo run -p cad-cli --` を前に付けて実行する。入力要求・生成レポート・候補は作業資料であり、編集可能な図面は正本ファイルにある。

## 記述例とスキーマ

`examples/direct-edit-guide` は全12種の図形、固定点と図形参照の寸法、レイヤー・スタイル・レイアウト・部品を持つ検査可能な例。固定寸法点は `{"kind":"fixed","point":[0,0]}`。文字の位置には `at` を使うが、固定寸法点に `at` は使えない。既存IDを更新時に保持し、新規IDは有効かつ一意な `ent_<ULID>` とする。

```text
source-schema --out build/source-schemas
check examples/direct-edit-guide --target cad --format json --out -
```

Rustモデルから `entity / project / layers / styles / layouts / block` のJSON Schemaを生成する。`draft-request.schema.json` は以下の作図要求用。スキーマは型・必須項目・未知フィールドを検査できるが、ID重複・参照・図形の妥当性にはCADチェックも必要。`entity.schema.json` はNDJSONの**各行**に適用する。複数行を一つのJSONとして検査しない。JSONL対応のエディタで行ごとに関連付けるか、一図形をJSONファイルに抜き出して補完に使える。TOMLはJSON Schema対応のTOMLエディタで各設定スキーマに関連付ける。スキーマを正本へ埋め込まない。

CIは記述例を生成スキーマ・CADチェックの両方で検証する。書式エラーの診断にはファイル・行・パーサーの期待形式を含め、未知/欠損フィールドの場合は `field` も返す。

## 座標を実寸として使う前に

```text
source-coordinates PROJECT --drawing NAME --out -
```

正本の新規作図はモデルmm。用紙への変換は `(model - origin) / scale` で、用紙縮尺を変えても保存座標は変わらない。文字のheight/width、寸法offsetもモデルmm、線幅は紙上mm。

JWW取り込みでは、元の記録座標とblock変換を保持し、レイヤグループの縮尺を図形座標に自動乗算していない。取込報告の `coordinates.model_normalized=false` と、座標レポートの元JWWグループ縮尺を確認する。既存の取り込み済みプロジェクトでも元JWWから報告を作れる。既知の寸法と座標差を照合して換算係数を確定する。例えば保存長37が実寸3700mmなら係数100。既に実寸mmの座標なら1。紙面縮尺1:100という情報だけで係数100と決めない。混在グループには一律の係数を適用しない。

確認済みの係数で幾何図形を派生図面に複写する要求例:

```json
{"key":"normalized-wall","layer":"0-1","geometry":{"type":"coordinate_copy","source_drawing":"SOURCE","entity_ids":["EXISTING_ID"],"coordinate_mm_per_unit":100,"base_point":[0,0],"at_mm":[0,0]}}
```

`entity_ids` は同一レイヤグループの図形を指定する。混在縮尺はグループごとの要求に分ける。点・線・ポリライン・円弧・円・楕円・solid・curve_solid・hatchに対応し、位置と寸法を換算する。`pen` の省略時は図形ごとの元のペン指定を保持し、明示時だけ指定したペンへ置き換える。文字・寸法・blockは拒否する。注記は実寸用styleで作り、blockの複写には既存の `copy-entities` を使う。元の図形は保持し、取り込みの来歴を書き換えない。保存専用exact-original-only取り込みでは生成を拒否する。

## 反復作図と壁面展開図

対象図面の `layouts.toml` と `entities.ndjson` を用意し、使うレイヤー・文字/寸法styleを定義する。空の新規図面ならentities.ndjsonは空ファイルにできる。正本を追加したらCADチェックを実行する。

```text
draft-source PROJECT --drawing NAME --request docs/direct-edit/wall-elevations.json --out build/draft-001
```

- `wall_elevations`: 室内から見た左端 `left`、右端 `right`、明示した換算係数、天井高、図面内配置から輪郭・開口・幅/高さ寸法・壁名を生成する。水平位置は壁の向きへ投影し、対向壁では左右が逆になる。開口端は壁線から1mm以内、壁の範囲内、床～天井の範囲内とする。開口の重なりを拒否する。高さ・仕上げは推測せず、仮定を使う試作では `note` に明記する。梁型・柱型・設備等は通常の正本図形として追記できる。
- `repeat`: 既存の平面作図generatorを指定回数・移動量で複写する。例は `docs/direct-edit/repeat.json`。解析generatorは対象外。窓は左端枠・右端枠・横桟・中間枠を役割で識別し、分割数の変更で右端枠の手修正や参照が中間枠へ移らない。中間枠は窓幅に対する位置で識別するため、2分割→4分割でも中央枠のIDを保つ。旧レポートからも既存IDを保って再生成できる。移行したIDの由来はレポートの `id_roles` に保持する。手修正した枠をなくす変更は衝突として止まる。
- `coordinate_copy`: 上記の明示的な単位換算複写。

出力の `entities.ndjson` は**対象図面全体の候補**。新規図形だけを追記するファイルではない。`report.json` にCAD検査、要求条件、元正本のハッシュ一覧、役割とID、生成基準値、警告を記録する。コマンドは正本を変更せず、Desktop Undo/Redoにも入らない。

Git差分や候補を確認し、元正本がreportの `source_files` と同じことを確認してから、対象の正本entities.ndjsonへ候補を反映する。同時編集があれば生成し直す。反映した各まとまりの後は必ず次を実行する:

```text
check PROJECT --target cad --format json --out -
```

同じ要求を再生成するときは、最後に反映した候補のレポートを指定する:

```text
draft-source PROJECT --drawing NAME --request CONDITIONS.json --previous build/draft-001/report.json --out build/draft-002
```

プロジェクト名・図面名・要求key・図形の役割からIDを生成する。keyを変えず、壁・開口も固有のkeyを保つ。再生成は前回の生成値・現在の正本・今回の生成値でフィールド単位に比較し、独立した手修正を保持する。同じフィールドを両方が変更した場合、または手修正した図形の削除が必要な場合は衝突として止まる。正本から手で削除した生成図形は復活させない。手で追記した別IDの図形と未変更行の書式は保持する。前回レポートは正本の代わりではなく、再生成比較の基準なので保持する。Gitでプロジェクトと一緒に管理する場合は任意の作業資料ディレクトリへ保存できる。プロジェクト名や図面名を変えた場合はnamespace不一致で再生成を止める。

幅/高さ寸法は輪郭の頂点を参照し、正本で輪郭を直接編集すると追従する。頂点0を左下、1を右下、3を左上として維持する。輪郭を削除する場合は参照する寸法も削除し、残った参照は検査エラーとして扱う。壁長の最小値は換算後のmmで判定する。従来の固定点寸法も、前回レポートを指定した再生成で同じIDの参照寸法へ更新する。寸法の手修正が更新と衝突する場合は停止する。

## 検査と画像出力をまとめる

```text
export-set PROJECT --drawing NAME --out build/preview-001 --preview-png
verify-export-set build/preview-001
```

静的検査に合格した同じ版からSVG・PDF・PNG・確認用 `index.html` とハッシュ付きmanifestを出す。出力先は未使用ディレクトリ。`--page NAME@LAYOUT` でレイアウト、`--preview-dpi 144` でPNG解像度を指定できる（36～300dpi、画像は最大4000万画素）。PNGは同梱Mplusフォントを使うSVGのラスタライズで、ブラウザや外部変換ツールは不要。

PNG/SVGを開いて左右・開口位置・寸法・文字を確認する。SVGは用紙外まで表示する場合があり、フォントも代替されるので、PDFの印刷範囲・文字・縮尺はPDFビューアまたはPDFを画像化して別途確認する。manifestの `visual_review=pending` は静的検査を見た目の確認と混同しないための記録。コマンドは見た目の合格を自動判定しない。

JWWが必要な場合は既存の専用チェックと報告付きexportを使う。今回の座標支援はJWWバイナリ互換性や混在縮尺の自動正規化を保証する機能ではない。
