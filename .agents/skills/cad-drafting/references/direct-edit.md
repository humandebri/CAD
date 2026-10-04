# 直接編集の支援

リポジトリルートの [docs/direct-edit.md](../../../../docs/direct-edit.md) に現行コマンド・条件例・再生成・確認手順がある。モデル形式や単位で迷ったとき、反復作図・展開図の生成、検査と画像出力をまとめるときに読む。

- `source-schema` はモデル型由来のJSON Schemaを出す。entityのschemaはNDJSONの各行へ適用する。
- `source-coordinates` はJWW由来の未換算座標とグループ縮尺、用紙縮尺を区別する。実寸係数は既知寸法から確定する。
- `draft-source` は正本を変更せず、対象図面全体のNDJSON候補と検査・条件・元版のレポートを出す。元正本が変わっていないことを確認してから直接反映し、CADチェックを実行する。再生成は最後に反映した候補の `--previous report.json` を使い、keyを維持する。手修正との衝突を無断で解決しない。
- `export-set --preview-png` は同じ検査済み版からSVG/PDF/PNGと確認用HTMLを出す。PNGはSVG由来・同梱フォントへの代替なので、PDFの印刷範囲と文字は別に確認する。

直接編集はDesktopのUndo/Redoに入らない。生成要求やレポートを編集正本として扱わず、正本の既存ID・手修正を維持する。
