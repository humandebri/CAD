# 2.5D投影・日影・水平天空の見通し

閉じた平面形状を鉛直な角柱として扱い、2D図形と再計算用JSONを生成する。地形、傾斜屋根、隠線処理、法規の適合判定は対象外。入力・近似条件を報告に残す。

## 図面から作図する

DesktopのBuilding toolsで閉じたpolyline、solid、hatchの境界を選び、massing projection / sun shadow / sky viewを使う。底面はz=0、選択した建物は同じ高さになる。Preview→計算報告と作図候補を確認→Apply。条件を変えると候補を破棄する。生成した図形は普通の2D正本であり、元の平面形状を変更しても自動更新しない。共通の正本CAS・CAD検査・一括Undoを使う。

CLIでは次の要求を `generate PROJECT --drawing NAME --request REQUEST.json --out EDIT.json --analysis-out REPORT.json` へ渡す。REPORTは別の新規ファイルで、正本・Git・履歴等の保護領域には保存できない。EDITは通常の `edit PROJECT --request EDIT.json --out PREVIEW.json` で確認し、確認後に `--apply` を指定する。

```json
{
  "layer": "0-1",
  "pen": null,
  "geometry": {
    "type": "sun_shadow",
    "footprint_ids": ["EXISTING_ENTITY_ID"],
    "height_mm": 5000,
    "sun_azimuth_deg": 90,
    "sun_altitude_deg": 45
  }
}
```

他のgeometry条件は次のとおり。位置・高さ・半径はモデルmm。

| type | footprint_ids・height_mmに加える条件 |
|---|---|
| massing_projection | origin: [x,y], at: [x,y], yaw_deg, elevation_deg |
| sun_shadow | sun_azimuth_deg, sun_altitude_deg |
| sky_view | observer: [x,y,z], azimuth_samples, at: [x,y], radius_mm |

報告 `cad-massing-review/1` は元の図面revision・正本manifest、配置・layer/penを含むgenerator_request、全平面座標・高さ・計算条件、結果・警告を含む。DesktopのSave new massing reportは計算条件から再計算してanalysisの結果を照合し、新規ファイルへ保存する。最後の報告は図面・プロジェクトを切り替えても保持される。過去の計算報告であり、現在の図面や適用完了を証明するものではない。

## 建物ごとの高さ・底面を指定する

`analyze-massing CONDITIONS.json --out RESULT.json` は正本を変更せず、`cad-massing-result/1` を出力する。入力上限は1 MiB。異なる高さ・底面・中庭を表すにはこちらを使う。結果の2D正本への取り込みは別途行う。

```json
{
  "schema_version": "cad-massing/1",
  "buildings": [{
    "id": "building-a",
    "loops": [[[0,0],[4000,0],[4000,4000],[0,4000]]],
    "base_z_mm": 0,
    "height_mm": 5000
  }],
  "analysis": {
    "type": "sun_shadow",
    "sun_azimuth_deg": 90,
    "sun_altitude_deg": 45,
    "ground_z_mm": 0
  }
}
```

projectionのanalysisはorigin・at・yaw_deg・elevation_deg、sky_viewはobserver・azimuth_samplesを指定する。空のbuildingsは開放天空を計算できる。ループは偶奇則で穴・島を扱い、自己交差・接触・交差する境界を拒否する。最大128建物、合計4096頂点、XYは±10億mm、底面zは±1000万mm、高さは0より大きく1000万mm以下。

## 計算方法と検証範囲

座標はx=東、y=北、z=上。太陽方位は北0°、東90°で、[NRELのSolar Position Algorithm](https://docs.nlr.gov/docs/fy08osti/34302.pdf)と同じ方位規約を使う。日時・緯度経度から太陽位置を求める処理は含まない。

投影はyawでXYを回し、画面の縦座標を `sin(elevation) * 回転後y + cos(elevation) * z` とする。仰角0°で側面、90°で平面。底面・屋根・鉛直辺をすべて出力する。重なる辺も残る。水平平行投影であり、透視図や隠線除去ではない。

日影は光線を指定した水平面まで延ばす。高さ差zの点の移動量は `[-sin(azimuth), -cos(azimuth)] * z / tan(altitude)`。底面・屋根・各壁の投影をunionし、穴を保持して重複を除いた面積を報告する。太陽高度は(0°,90°]、地面は全底面以下。低高度で座標範囲を越える影は拒否する。単一時点の日影であり、時刻別の日影時間図ではない。

天空は各方位で最初に角柱へ入る距離と屋根までの高さから最大遮蔽仰角hを求める。水平面の係数は `SVF = (1 / 2π) ∫ cos²(h) d方位`。水平面への放射を重み付けする定義であり、球面の立体角比とは異なる。[HORAYZON論文の水平面SVF](https://gmd.copernicus.org/articles/15/6817/2022/)を計算式の参考とし、独自の角柱交差計算を用いる。同論文の地形モデルや実装は組み込んでいない。

粗い分割N（36～2048）と2Nの方位区間中点で評価し、2Nの平均を採用する。両者の差は収束の参考値であり、誤差上限ではない。狭い障害物をサンプルが見落とす可能性は残る。計算量は合計頂点数×3N≤800万に制限。底面が観測点より高い浮いた構造は拒否し、屋根以下の観測点が建物内部や境界上にある場合も拒否する。天空図は半径 `R*cos(h)` の正射影で、北が+y。法律上の比較建物・測定点・道路境界・斜線制限等はモデル化しない。

Rust検証は四方の太陽45°による矩形の影面積、天頂光と中庭の穴、重複・凹形状、側面/平面の辺・高さ、開放天空、中庭内外・屋根上の観測点、高さ=半径の円形中庭近似（45°地平線・SVF≈0.5）、不正入力の拒否を含む。図面生成は元形状の保持・検査・古い要求の拒否・Undoを検証する。これらは幾何学的な検証であり、Jw_cadの出力一致や法規計算の認証ではない。
