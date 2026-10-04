# 生成・計測CLI

コマンドはリポジトリルートで、前に `cargo run -p cad-cli --` を付ける。入力要求・プレビュー・結果は `build/` に保存し、正本と区別する。

壁の2線の生成要求例:

```json
{"layer":"0-1","pen":null,"geometry":{"type":"double_line","p1":[0,0],"p2":[5000,0],"width":100,"centerline":false}}
```

```text
generate PROJECT --drawing NAME --request build/wall.json --out build/wall.edit.json
edit PROJECT --request build/wall.edit.json --out build/wall.preview.json
edit PROJECT --request build/wall.edit.json --apply --out build/wall.result.json
check PROJECT --target cad --format json --out -
```

要求は生成時の図面revisionと全正本manifestに紐付く。正本が変わったら要求を作り直す。適用はBatchとしてUndo/Redoに入り、直接NDJSON編集の履歴とは区別される。新規図形のIDは適用時に確定するため、プレビューの新規IDを別要求の参照先にしない。

`geometry.type` は `double_line`、`centerline`、`door`、`window`、`sliding_door`、`ellipse`、`cubic_bezier`、`circle_tangents`、`tangent_circle`、`calculated_text`、`divide`。寸法はモデルmm、角度は度。要求フィールドは `crates/cad-toolkit/src/drafting.rs` を読む。建具は2D平面記号であり、断面・立面や変更追従する部品パラメータはまだない。Bezierは指定した許容距離でポリラインへ近似する。`divide` は線・円弧に対応する。

`tangent_circle` は `line_ids:[ID1,ID2]`、`radius`、`near:[x,y]` を指定し、2本の直線の延長線に接する円を作る。4候補のうちnearに最も近い中心を選ぶ。同距離は中心x・y順で確定する。線分の範囲外に接点が来る場合があるため、警告とプレビューを確認する。平行・ほぼ平行、重複ID、欠損ID、直線以外、精度不足の座標は拒否する。円・円弧への接円は未対応。Desktopでは2本を選択し、Building toolsのtangent circleで半径・中心付近の点を入力する。パラメータ変更後は再プレビューが必要。

文字のリテラル置換は `generate --text-replace` を使い、入力を次の形にする。選択IDを省略すると対象図面内の全注記文字を対象にする。寸法のvalueは変更しない。

```json
{"find":"既存","replace":"改修","entity_ids":[],"style":null}
```

本文を保って文字styleだけ変更する要求は `{"style":"STYLE","entity_ids":["EXISTING_ID"]}` として `generate --text-style` を使う。空ID一覧は対象図面の全注記文字を意味する。寸法とblock内部は対象外。欠損・重複IDや未定義styleはエラー。変更がない場合も候補を作らない。生成したrevision付き要求は共通の `edit` preview/applyで検査しUndo/Redoに入る。

DesktopのText search and batch editsでは、大小文字を区別したリテラル検索、全図面内または現在の選択範囲、置換本文、styleを指定する。空検索は全注記のstyle変更に使えるが、置換は検索文字列が必要。全件数と変更後の本文・styleを確認してプレビューし適用する。結果一覧の表示は先頭100件で、候補は選んだ範囲の全変更件を含む。入力・対象選択・正本の変更は候補を無効にする。空の選択範囲を全図面へ拡張しない。外部エディタ連携はまだない。

注記の `writing_mode: "vertical_upright"` は上から下へ一Unicode scalarずつ置き、LFで左列へ移る。省略またはhorizontalで従来の横書き。atは縦列の左上、alignは上端/中央/下端を決め、回転・反転も反映する。[配置と出図の制約](../../../../docs/annotation-writing.md)を確認する。新規TextのWriting directionまたは既存注記のPreview writing directionを使う。CLIの `generate --text-edit` に `{"kind":"set_writing_mode","writing_mode":"vertical_upright","entity_ids":["EXISTING_ID"]}` を渡して共通editで適用する。空ID一覧は全注記、寸法・block内部は対象外。縦組み句読点・結合文字・縦中横・ルビの処理は含めない。JWW/DXFは一文字ずつへ展開して報告し、strictでは阻止する。元JWWの縦字フラグは未検証のため取り込み警告を読む。

計算注記は `generate` のgeometryに `{"type":"calculated_text","at":[1000,1000],"expression":"1000*2000/1e6","precision":2,"prefix":"面積: ","suffix":" m²","style":"note","rotation_deg":0}` を指定する。四則演算・括弧・符号・指数表記のASCII式（4096bytes以内）、小数0～12桁に対応。関数・変数・累乗・単位の自動解釈はない。f64の二進浮動小数点計算と書式丸めを使い、ゼロ除算・オーバーフロー・アンダーフロー・64段を超える括弧は拒否する。接頭辞・接尾辞の合計は4096bytes以内。結果とgeometryプレビューを確認する。保存は通常のTextで、式への参照や自動再計算は持たない。

計測は次の形で、選択IDには `--entity ID` を繰り返す。

```text
measure PROJECT --drawing NAME --format json --out build/measure.json
measure PROJECT --drawing NAME --entity ID --format csv --out build/measure.csv
measure PROJECT --drawing NAME --area-mode union --curve-tolerance-mm 0.1 --out build/union.json
```

長さはmm、面積はmm²。Desktopはm・m²へ換算表示する。入れ子blockの基点・回転・反転・均等倍率を展開して測定する。既定の面積は各図形の値を加算する。`--area-mode union` は同じ図面内の重なりを除き、別図面の領域は独立して集計する。長さは常に図形ごとの合計で、union境界の周長ではない。ハッチはeven-oddで穴・島を判定し、一つのハッチ内の交差・接触境界を拒否する。unionは閉じたpolyline、solid、hatch、円、全楕円の領域を対象にする。円・楕円のunionはモデルmmの許容距離で内接多角形へ近似し、面積減少の合計を誤差見積りとして報告する。浮動小数点計算の誤差まで保証する区間ではない。楕円周長は数値積分で、長さ誤差は細分化差からの推定値。文字、寸法、点、CurveSolidはnullと警告で合計から除外する。unsupported_countは展開後の未対応図形数。CSVのrecord_type=entity行は加算前の各図形、aggregate行は選んだ面積方式の総計。再帰blockや展開・頂点数の上限超過はエラーになる。

`edit` は `DrawingEditRequest` を受ける外部変形APIにも使える。自動の外部プログラム実行やJw_cadのWindowsバッチ互換は提供していない。プレビューで参照寸法の解決や警告への対応が必要なら、要求を修正して再プレビューする。

## 図面間コピー・正本部品

```text
copy-entities SOURCE --drawing NAME --entity ID --base-point 0,0 --dimensions include-references --out PART.cadpart.json
paste-entities TARGET --drawing NAME --part PART.cadpart.json --at 10000,20000 --out build/paste-plan.json
paste-entities TARGET --drawing NAME --part PART.cadpart.json --at 10000,20000 --apply --expected-plan HASH --out build/paste-result.json
check TARGET --target cad --format json --out -
```

`--entity` は繰り返せる。寸法の参照先は `include-references` で同じ図面の geometry を一緒にコピー、`detach-external` で選択外の参照を固定座標へ解除、`reject-external` で拒否する。block内の参照が部品外へ出る場合は解除か拒否。基点と貼付点はモデルmm。部品は `cad-clipboard/1` JSONで、JWK/JWS/JWWのバイナリとは別形式。comments・来歴・用紙は貼付しない。

候補の全変更ファイル・形状差分・ID/定義対応表・警告・CAD検査を確認し、そのplan hashで適用する。既存図形の原文行と定義を保持し、新規IDと名前衝突の解消を行う。shared/nested blockと寸法参照は同じ候補内で付け替える。別図面・別プロジェクトへの貼付も一括Undo/Redoへ記録する。正本が変われば新候補を作る。`--rotation-deg` と `--scale` で貼付点を中心に回転・正の均等倍率を指定できる。元の基点を貼付点へ移してから変換し、block内部と共有紙上styleを保持する。水平/垂直の参照寸法は90度単位に限り、90度で軸を交換する。非均等倍率は不可。

DesktopはCAD clipboard and parts→Copy selected to CAD clipboard→図面/プロジェクト切替→Preview CAD paste→Apply reviewed CAD paste。Save new CAD part/Load CAD partで再利用する。読み取り専用図面からのコピーは可能、貼付は不可。レイヤー可視性・ロックも保持するので、隠れた貼付図形の警告を無視しない。新規部品の上書き、正本・interop・履歴・復旧・Git領域への保存を拒否する。ファイル上限64 MiB、図面図形とblock内図形はそれぞれ10万件、block定義は1万件。

## 一枚内の混在縮尺

`docs/sheet-viewports.md` を読み、レイアウトに表示する元図面・モデル原点・紙上位置/幅高さ・縮尺・レイヤーを明示する。`sheet-viewports PROJECT --drawing NAME --layout LAYOUT --request VIEWS.json --preview-svg NEW_PREVIEW.svg --out REVIEW.json` またはDesktopのScaled views on one sheetでプレビューし、報告のplan hashを確認して適用する。全正本CASと一括Undoを使い、元図形や寸法値は変更しない。空の一覧でモデル表示へ戻る。SVG/PDFでクリップと文字サイズを確認する。viewportsのないモデル図面/レイアウトで図形を編集する。異なる縮尺で文字の見かけも変わるため、元図面のモデル寸法styleを確認する。混在縮尺JWWの自動解釈とは別の機能で、このレイアウトのJWW出力は阻止される。

## 2.5D投影・日影・天空図

リポジトリの `docs/massing-calculations.md` を読み、閉じたpolyline・solid・hatch境界のIDと高さから `massing_projection` / `sun_shadow` / `sky_view` を生成する。`generate` に `--analysis-out NEW_REPORT.json` を付け、配置・layer/penを含むgenerator_request、平面座標・高さ、太陽角度または観測点、分割数、結果・警告・元正本revisionを保持する。編集要求をプレビューしてから適用し、CAD検査とSVG表示を確認する。

DesktopでもPreview→報告と候補→Save new massing report→Applyを使える。角柱の底面はz=0・同じ高さ。別の高さ・底面や中庭は `analyze-massing CONDITIONS.json --out RESULT.json` の明示したモデルで計算する。x=東、y=北、z=上、太陽方位は北0°/東90°、高度は(0°,90°]。天空は水平面の放射で重み付けした係数と正射影であり、方位分割の収束差は誤差保証ではない。法規の比較建物・測定点等は扱わない。日時/位置から太陽角度を推測しない。元形状を変えても生成済み2D図形は追従しないため、保存条件と新revisionで再生成する。法規合格やJw_cad結果一致を主張しない。

## 属性取得

DesktopのUse selected attributesは1図形からレイヤー・penと、該当する文字style・寸法style・fillを取得し次の作図へ使う。座標や図形はコピーせず、取得とClear acquired attributesは正本を変更しない。取得レイヤーはactive layerより優先し、active layerを選び直すとその優先だけを外す。残る属性は作図パラメータで確認する。Clearは属性を既定へ戻す。取得・解除は編集中のdraftを破棄する。プロジェクト・図面の切替や取得した参照先の欠損で解除する。非表示・locked layerへの作図は通常の検査で拒否する。

`rectangle` と `insert_block` のEditOperationにも任意の `pen` を指定できる。未指定・nullはレイヤーの線属性を使う。既存図形を更新する操作ではない。

## 図形の均等倍率

Desktopの `scale` は選択IDと基点を指定して正の有限倍率で変形する。CLI `edit` のoperationは `{"kind":"scale","entity_ids":["EXISTING_ID"],"center":[0,0],"factor":2}`。図面名と現在の `expected_revision` を含むDrawingEditRequestでプレビューし、確認後に適用する。円・楕円の半径、寸法offset、block・hatch・pointの図形別倍率も変更する。共有styleの紙上文字寸法や線幅は保持し、参照寸法は元図形に追従する。負倍率と非均等倍率は未対応。Undo/Redoとレイヤ権限は共通編集処理に従う。

## DXF交換

DesktopではImport DXF / Export DXFから新規出力先と報告書を選び、警告・阻害要因を確認する。取り込みは新プロジェクトを検査してから一括公開し開く。正本・来歴・履歴・復旧・Git管理領域への出力や、既存出力の置換は拒否される。CLIと同じ変換を使う。保存された報告の `status=ready` は変換準備の成功であり、出力の公開証明ではない。公開時にエラーが出た場合は報告を保持して、出力ファイルも確認する。

```text
export-dxf PROJECT --drawing NAME --out OUTPUT.dxf --report REPORT.json
import-dxf INPUT.dxf --out NEW_PROJECT --report IMPORT_REPORT.json
check NEW_PROJECT --target cad --format json --out -
```

JSON報告は必須。ASCII DXFの平面プリミティブに対応し、出力はR2013モデル空間・mm。寸法・ハッチは展開され、円弧状の塗りは報告付きで多角形近似される。blockの均等倍率・反転・回転を保持する。未対応レコード、3D、paperspace、非均等INSERT、bulge付きpolyline等を検出した入力はプロジェクトを公開しない。単位未指定のDXFは `--unit-mm MM_PER_DXF_UNIT` が必要。

取り込みは新しいIDとA3横・1/100のレイアウトを作り、フォントを代替する。出図用の用紙・縮尺・原点は別途確認する。出力先・報告書は未使用のパスを使う。`export-dxf --strict` はページ・ID等の正本情報の削減も含む全警告を阻害要因にするため、通常のモデル空間交換も阻止される。完全互換・losslessと呼ばず、受け側CADで見た目を確認する。DWG・SXF・JWC/JWKはまだ対象外。

## JWS図形の取り込み

```text
import-jws INPUT.jws --coordinate-scale 100 --out NEW_PROJECT --report IMPORT_REPORT.json
check NEW_PROJECT --target cad --format json --out -
```

対応版は351・420・600。`--coordinate-scale` は保存座標1単位をモデルmmに変換する係数で、必須。紙上座標を1/100の実寸として使うなら100、既にモデルmmなら1とする。値を決める根拠が不明な場合は部品の実寸を確認し、推測した縮尺を確定値として扱わない。レポートのgroup_scalesとplacement_origin_mmを確認する。混在縮尺の自動解釈とJWS再出力はまだない。

参照blockは展開され、色・レイヤー名・線幅・フォント・用紙環境は補完される。JSONの全警告とcheckを読む。未知クラス・未知末尾・再帰block・図形脱落・CADエラーは取り込みを阻止し、外側の報告を保持する。元JWSとJWW来歴を同一視しない。元JWSは新しいプロジェクトに同梱されないので入力ファイルも保持する。取り込み後は正本を編集し、SVG/PDFで実寸・表示を確認する。現行Unicode版やJw_cad実機は未確認。検証範囲は `docs/jws-compatibility.md`。

`copy-jws INPUT.jws --coordinate-scale 100 --out PART.cadpart.json --report JWS_REPORT.json` で検査済みの部品を作り、上の `paste-entities` で配置できる。報告を先に保存し、blockedなら部品を作らない。DesktopはLoad checked JWS partで係数指定→全JSON報告→Preview CAD paste→Apply reviewed CAD paste。基点はplacement_origin_mmを使う。未対応レコードの報告を残し、既存clipboardを維持する。元JWSとCLI報告を保持し、Desktop JSON表示を保存・レビューする。変換完了のstatusは貼付や出図の完了証明ではない。

`list-parts FOLDER --offset 0 --limit 12 --coordinate-scale 100 --out LIBRARY_REPORT.json` とDesktopのPart folder paletteで、非再帰の `.cadpart.json` / `.jws` 一覧・検査・サムネイル・全JWS報告を表示する。JWSを含める場合は係数を決め、勝手に既定値を補わない。未対応・壊れた部品は理由を読んで選択を止める。一覧で得たfile hashを選択時に確認するため、外部変更時は再読込が必要。選択は正本を変更せずclipboardへ読み込み、その後の貼付をプレビュー・検査して適用する。隠れたレイヤーの可視性も保持する。サムネイル上限に達した部品は省略理由と図形件数を確認し、完全な貼付候補で形状を確認する。symlink・64 MiB超のファイルは読込しない。一ページ最大24件、フォルダー走査は2万項目まで。
