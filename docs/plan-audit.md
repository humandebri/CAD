# CADの外で行う建築検査

`cad-audit` は明示した対応台帳から検査と修正候補を生成する静的CLI。AI・ネット接続・APIキーを使わず、同じ入力から同じ結果を得る。CAD本体の壁・部屋・建具モデルや自動補完は追加しない。CADのTOML/NDJSONは図形の正本、案件の `notes/plan-audit-ledger.json` は建築上の対応の正本として分ける。

```sh
cargo run -p cad-plan-audit -- drawing-projects/house-plan/cad \
  --ledger drawing-projects/house-plan/notes/plan-audit-ledger.json \
  --out drawing-projects/house-plan/work/plan-audit/report-NEW \
  --proposals
```

出力先の親は存在する必要がある。新しい外部ディレクトリだけに出力し、既存成果物・CAD正本・Gitメタデータへは書かない。終了コードは `0=全登録検査Pass`、`1=Failあり`、`2=資料不足Unknownあり`、`3=入力・実行エラー`。全登録検査Passでも未登録対象は未検査。

| 出力 | 用途 |
| --- | --- |
| index.html / drawing-NNN.svg | 検査一覧と位置表示 |
| report.json | 状態、実測値、条件、登録数、検査範囲 |
| schedule.json | 図形から再計算した建具幅・設備占有寸法と台帳の製品情報 |
| elevations.svg | 台帳に高さがある片開き建具の寸法模式図。詳細な製品立面ではない |
| repairs.json | 開口移動候補の可否と理由 |
| repair-NNN.* | 個別の編集要求、更新台帳候補、編集プレビュー、修正後検査・SVG |
| manifest.json | 入力正本と台帳のハッシュ、未適用の記録 |

## 対応台帳

スキーマは `cad-plan-audit/1`。実装の型は `crates/cad-plan-audit/src/lib.rs`、現在の住宅の実例は `drawing-projects/house-plan/notes/plan-audit-ledger.json`。未知フィールドは拒否する。単位はモデル上のmm、座標はCADと同じ向き。台帳の作成・資料照合は `cad-plan-audit` スキルが担当し、静的CLIが反復計算する。

```json
{
  "schema_version": "cad-plan-audit/1",
  "tolerance_mm": 0.5,
  "drawings": [{
    "drawing": "plan_1f",
    "walls": {"fill_entity_ids": ["壁のHatch ID"], "outline_entity_ids": ["壁の閉Polyline ID"]},
    "rooms": [{"key": "UT", "inner_polygon": [[0,0],[2000,0],[2000,2000],[0,2000]]}],
    "fixtures": [{"key": "E01", "entity_ids": ["設備図形ID"], "room": "UT"}],
    "openings": [{"key": "O01", "axis": [[400,75],[1178,75]], "depth_mm": 150}],
    "doors": [{"key": "D01", "opening": "O01", "swing_entity_id": "開閉円弧ID", "closed_end": "start"}],
    "clearances": [{"kind": "room_edge", "key": "FRONT", "fixture": "E01", "room": "UT", "side": "min_x", "min_mm": 500, "basis": "本案件の設計条件"}],
    "installations": [{"key": "INSTALL", "fixture": "E01", "room": "UT", "source": "メーカー資料・ページ・基準面", "required_width_mm": 1670, "required_depth_mm": 1670, "required_height_mm": 2730, "available_height_mm": null}]
  }]
}
```

上のIDは説明用であり、実際には既存IDへ置き換える。意味をCAD図形の独自フィールドに埋め込まない。各 `key` は図面内で一意。壁の塗りはHatch、輪郭は閉じたPolyline、部屋は仕上げ内法の単純な多角形。設備は注記を除いた実形状のIDを登録し、占有域は登録図形全体の外接長方形で扱う。ブロック参照は先に展開する。壁付品は `wall_mounted: true` で壁との重なり検査を除外するが、他設備・部屋・扉との検査は残る。

開口の `axis` は開口中央の両端、`depth_mm` は壁の全厚。片開き建具は開閉円弧へ対応し、`closed_end` は閉じた扉を表す円弧の `start` または `end`。円弧半径を扉の線の幅として照合する。枠外寸とは区別する。開閉角は符号付きで最大180°、円弧の離散化の最大弦偏差は `tolerance_mm`（0より大きく5以下）。

製品情報は設備・建具の `product` に `name/model/source`、必要なら `nominal_width_mm/nominal_height_mm`。資料番号と発注品番を区別する。品番から条件を検索する処理はCLIに含めず、資料で確認した条件を台帳へ記録する。仕上げ厚・公差を差し引いた有効寸法、必要寸法のX/Y方向、必要高さと利用可能高さの基準面を合わせる。天井高CHから据付高さを推測しない。登録部屋の内法が登録壁に重なる場合は台帳不整合をFailとし、据付幅・奥行はUnknownとする。

通路条件は `between`（`first/second/axis:xまたはy`）か `room_edge`（`fixture/room/side:min_x,max_x,min_y,max_y`）。両設備の直交方向投影が重ならない `between`、長方形でない部屋への `room_edge`、出典・設計条件のない判定、必要寸法・利用可能高さの欠落はUnknown。閾値は明示された案件条件であり、法規やメーカー条件を自動で代入しない。

壁ハッチの複数ループは正本と同じ偶奇塗りとして扱う。離れた領域、穴、穴の中の島を含め、ループの順序や向きには依存しない。

据付条件の `room` は設備が実際に収まる部屋を指定する。設備の `room` と異なる指定、または設備の占有域が指定部屋の外にはみ出す指定は入力エラーとして検査を停止する。設備の `room` を省略した場合も据付先への包含を確認する。

## 修正候補

開閉円弧が移動して旧開口と合わないとき、閉じた扉の投影から新開口を求め、旧開口を壁で閉じて新開口を切る候補を作る。旧開口の両端に壁が存在し、旧開口内に壁がないことを確認する。壁と平行な移動だけを扱い、他の登録開口と重なる場合、移動先に十分な壁がない場合、壁の塗りと輪郭の対応が不完全な場合、壁スタイルが混在する場合は候補を拒否する。

候補はCAD検査を通し、対象建具の位置と壁への干渉を再検査する。新しいFailが発生する候補は拒否する。既存の別のFailは修正後レポートに残る。再利用できる図形IDを保持し、追加IDは決定的に生成する。壁の領域を合成するため、差分・参照寸法への影響・壁端部をプレビューで確認する。

編集要求は既存 `cad-edit` の `SourceChecked` とリビジョン照合を使う。元の正本が変われば古い要求は適用できない。検査CLIには適用オプションを設けていない。

```sh
cargo run -p cad-cli -- edit PROJECT --request repair-001.edit.json --out preview.json
```

ユーザーが対象修正を依頼している場合はプレビューと警告を確認して既存の `edit --apply` で適用し、対応する台帳候補も更新して再検査する。各候補は同じ修正前図面から個別に生成されるため、同時に適用せず一件ずつ再生成する。CADと外部台帳の更新は単一トランザクションではない。

## 初期実装の範囲

- 壁との設備干渉、設備同士、部屋内法からのはみ出し、開口の塞がり、片開き扉と開口・壁・設備、および登録された扉同士の開閉範囲を検査する。開口は両端の壁との接続も確認する。
- 表と寸法模式図は実行のたびに再生成する。Desktopでの変更を監視して既存の建具図・設備表を即時更新する処理は含めない。
- 壁・部屋・設備の意味は台帳へ明示する。任意の線だけからの完全自動認識、引戸・折戸・窓の可動域、廊下全体の最狭幅、経路探索、扉の向きの最適化、構造・防火・配管・現場条件を含めた施工成立の認定は未対応。
- 既存のメーカー詳細立面は維持する。ここで生成する模式図を詳細立面や発注図として置き換えない。
