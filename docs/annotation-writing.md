# 注記の縦書き

Text図形の `writing_mode` は省略時または `horizontal` が従来の横書き。`vertical_upright` は一つのUnicode scalarを一つのセルに置く縦方向の注記。数字・英字も上向きのまま一字ずつ並べる。本文・ID・style・位置を保って切り替えられる。

```json
{"schema_version":"0.3","id":"ent_01JZ0000000000000000000099","type":"text","layer":"0-1","style":"note","at":[5000,10000],"rotation_deg":0,"writing_mode":"vertical_upright","value":"室名\nA2"}
```

`at` は最初の列の左上を基準にする。下方向へ `height + spacing` ずつ進み、LFで左へ `width + spacing` ずつ列を移す。styleのalignはleftで列の上端、centerで列の中央、rightで列の下端をatに合わせる。空の列も列幅を持つ。回転・mirror_yはatのまわりで列全体と各字形に適用する。文字の基準位置を変更する切り替えではないため、既存の横書きのベースラインと縦列の上端の意味は異なる。LF以外の制御文字は縦書きのCAD検査で拒否する。

SVG/PDFと差分SVGは同じ配置計算を使い、選択枠・重なりの範囲も列の位置を反映する。元のTextは一つの図形のままで、block内やコピーした注記も設定を保つ。寸法ラベルは横書きのまま。文字の大きさはモデルmmで、印刷レイアウト・ビューポートの縮尺に従う。

DesktopのText作図にはWriting directionがある。既存注記はText search and batch editsで検索・範囲とWriting directionを選び、Preview writing direction→配置確認→Apply text preview。モード切り替えはstyle変更とは別で、style欄は変更しない。空の選択範囲を全図面へ広げない。条件や正本が変わると候補を捨て、適用ではrevisionと全正本manifestを照合し、一括Undo/Redoする。

CLIでは要求JSONを保存し、各行の前に `cargo run -p cad-cli --` を付ける。

```json
{"kind":"set_writing_mode","writing_mode":"vertical_upright","entity_ids":["EXISTING_ID"]}
```

```text
generate PROJECT --drawing NAME --request WRITING.json --text-edit --out EDIT.json
edit PROJECT --request EDIT.json --out PREVIEW.json
edit PROJECT --request EDIT.json --apply --out APPLIED.json
check PROJECT --target cad --format json --out -
```

CLIの空ID一覧は対象図面の全注記。重複・欠損ID、未知モード、変更なしは拒否する。寸法とblock内部は一括変更の対象外で、blockの正本を編集する場合は通常の直接編集・検査手順を使う。生成要求とプレビューの報告先は入力・正本・Git領域を避ける。

これは字体の縦組み処理を実装したものではない。句読点・括弧等の縦組み字体への置換、結合文字のshaping、縦中横、ルビ、自動折返しは含めない。合成済みの日本語文字でも字体・圧縮幅・PDFの埋込フォントを出図で確認する。

JWW v600とDXFは縦列を位置付きの横書き一文字ずつへ展開し、`upright_text_expanded` を報告する。再取り込みで一つのTextや元ID・編集意味を回復しない。JWWは従来どおりCP932・フォント代替・反転制約も報告し、strictでは近似を阻止する。受け側CADでの縦書き比較は未確認。

[公式レコード仕様](https://www.jwcad.net/jwdatafmt.txt)にはJWW文字の属性0x0020が縦字として記載されるが、字体・角度との対応は検証できていない。取り込みでは `native_vertical_text_unmapped` として警告し、来歴内の元バイトは保持する。未変更の互換取り込みは元バイトを正確に出力する。変更した対象レコードを再生成する場合は `native_vertical_text_replaced` と近似報告を出し、未検証の縦字フラグを使わず正本の配置に従う。旗を取り込んだだけで正本の縦組みを再現したとは扱わない。
