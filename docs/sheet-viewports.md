# 一枚内の混在縮尺

各レイアウトに、モデルの図形を紙上へ配置するビューポートを設定できる。平面1:100と詳細1:20のように異なる縮尺の表示範囲を一枚に置き、SVG/PDFへ出力する。元の図形・寸法値・IDを変更しない。

DesktopのScaled views on one sheetでSource drawing、モデル原点、紙上の位置・幅高さ・縮尺を設定する。Preview→切り取り範囲と報告を確認→Apply。レイアウトは一括Undo/Redoの対象になる。選択したレイアウトだけを変更し、active layoutは変えない。空の一覧を適用すると通常のモデル表示へ戻る。

紙上の表示は読み取り専用で、図形編集は元の図面・ビューポートのないレイアウトで行う。Active layoutまたはCurrent drawingから切り替える。元の図形を更新するとビューポートも再描画される。表示中の図形を移動するために紙上座標をモデル座標として保存しない。

CLIはビューポート配列を要求JSONに保存する。

```json
[
  {"name":"Plan","drawing":"plan_1f","origin":[0,0],"at_mm":[10,10],"size_mm":[160,150],"scale":"1/100","layers":[]},
  {"name":"Detail","drawing":"plan_1f","origin":[0,0],"at_mm":[190,10],"size_mm":[160,150],"scale":"1/20","layers":["0-1"]}
]
```

```text
sheet-viewports PROJECT --drawing SHEET_DRAWING --layout LAYOUT --request VIEWS.json --preview-svg NEW_PREVIEW.svg --out REVIEW.json
sheet-viewports PROJECT --drawing SHEET_DRAWING --layout LAYOUT --request VIEWS.json --apply --expected-plan REVIEWED_HASH --out APPLIED_REVIEW.json
check PROJECT --target cad --format json --out -
render PROJECT --drawing SHEET_DRAWING --layout LAYOUT --format svg --out SHEET.svg
export-pdf PROJECT --drawing SHEET_DRAWING --layout LAYOUT --out SHEET.pdf
```

要求の上限は1 MiB。レビュー報告は適用を証明しない。適用では全正本manifestとplan hashを照合し、レイアウト候補をCAD検査して一括公開する。図形・Git indexは変更しない。既存のlayouts.tomlが必要で、更新時には設定を保持してファイル全体をTOMLとして書き直す。直接編集後は通常のCADチェックも必要。

正本の設定は `drawings/NAME/layouts.toml` の各 `[layouts.LAYOUT]` に `[[layouts.LAYOUT.viewports]]` を追加する。配列は上のJSONと同じ名前の項目を持つ。旧レイアウトは配列なしで同じ動作を保つ。

| 項目 | 意味 |
|---|---|
| name | レイアウト内で一意の名前（最大256 bytes） |
| drawing | 同じプロジェクト内の元図面。元図面のモデル図形を使い、そのレイアウトのビューポートは展開しない |
| origin | 表示範囲の左下、モデルmm・東/北の座標 |
| at_mm | 紙の左下から測った配置位置、紙上mm・右/上 |
| size_mm | 紙上の幅・高さ。両方正で、物理用紙内に収める |
| scale | 1/100等の正で有限の縮尺 |
| layers | 元図形のレイヤーを絞る。空または省略で全レイヤー |

一枚最大32ビュー。指定順に描画し、重なる場合は後のビューが上になる。回転・透視・別プロジェクト参照は含めない。SVGは画面の可視性を、PDFは可視性とprintableを反映する。レイヤーの絞り込みはトップレベル図形に適用し、ブロック内容の元レイヤー・可視性は従来どおりに扱う。

表示するモデル座標は `paper = at_mm + (model - origin) / scale`。各ビューの紙上矩形で切り取り、PDFでは用紙余白・plot_areaとの共通範囲で印刷する。線幅は紙上mmを保つ。文字styleのheight/widthや寸法の矢印・offset等はモデルmmなので、見かけの大きさは縮尺に従う。同じ紙上文字サイズが必要なら、元の詳細図でその縮尺に合うモデル寸法のstyleを使う。

JWWへのクリップ・配置の変換は未検証。ビューポートを持つactive layoutはJWW専用checkでエラーになり、best-effortでも出力しない。普通のモデル図面を代わりに出力して完了扱いにしない。JWWのlayer groupに保持された縮尺値を自動解釈する機能とは別で、混在縮尺JWWの取り込み・再現の課題は残る。

Git・意味差分の `configuration_changes` に `viewport_sources.DRAWING/LAYOUT` の描画署名を含める。元図面の図形・参照寸法・block・styleの変更も、配置先の図面を指定した比較で検出する。署名は切り取り外のSVG要素やmetadataの変更にも反応するため、印刷範囲の画素差を証明しない。差分SVGの図形オーバーレイはモデル座標のままで、紙上ビューポートへ変換しない。両版の対象レイアウトのSVG/PDFで実際の配置を確認する。

部品コピーはモデル図形を対象にする。部品JSONには元のレイアウト情報を記録するが、サムネイルはビューポートなしのモデル表示で描画し、貼付ではレイアウトを移植しない。ビューポート付きの配置先にモデル図形を貼っても紙上に現れるとは限らないため、元図面で確認する。

Rustでは1000mmの線が1:100で10mm、1:20で50mmになり、両方の紙上線幅が同じことを確認。独立したクリップ、選択対象にならない元ID情報、古いhash拒否、正本保持、Undo、JWWの明示的な阻止も検証した。ブラウザは候補破棄・レイヤー選択・空一覧への復帰・キーボード・小画面を確認。Desktop実機は2ビューのpreview→apply→元図形とGit index保持→モデル作図阻止→Undoを確認した。
