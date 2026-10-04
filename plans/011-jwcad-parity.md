# Jw_cad機能差とGit連携の実装

ユーザー指定: P1・P2・P3を含む調査候補すべて（2026-10-03）。
根拠と受入条件は `docs/jwcad-gap-analysis.md`。既存の未commit文書・スキル変更を保持する。

## 状態

| 項目 | 状態 | 実装・次の検証 |
|---|---|---|
| 参照寸法・共有ブロックの依存差分 | 実装・Rustテスト済み | 評価済み寸法とブロック形状を差分SVGへ描画。入れ子ブロックの検証も追加済み |
| 任意commit・index・worktree比較 | CLI・Desktop実装、Rust・IPC・フォーム操作検証済み | `cad-git`、`cadc git-diff`。作業ツリーとindexを変更しないスナップショット |
| Git状態の監視 | 実装、Rust検証済み | Gitdir/common dirのHEAD・refs・index、監視失敗時はmetadata polling。実機の外部commitによる比較再読込も確認済み |
| 指定図面・レイアウトのSVG | CLI実装済み | `render --drawing --layout`。複数図面fixtureで検証する |
| 一括PDF・出図セット・版manifest | 実装・Rustテスト済み | `export-set` / `verify-export-set`。`--page DRAWING@LAYOUT`。JWWセットはactive layout限定。外部PDF解析・描画で2ページとA4縦→A3横の順序を確認済み |
| 図形単位stage・commit | stage・commit CLI・Desktop実装、Rust・ブラウザ検証済み | `git-stage`、参照寸法・style・block依存、全CAD候補検査、既存stage保持、plan hashとindex lock。Desktop実機のプレビュー・index適用・正本保持・commit後監視も確認済み。commitはindex全体と作者・patchをレビューし、無署名・hookなしを明示して確定。index・branchのCAS、準備済みref lock下のHEAD確認、初回commitに対応。merge/rebaseと署名・hook付きcommitは通常Git手順 |
| entity単位3-way merge | 候補生成・既存正本への適用CLI・Desktop実装 | `git-merge-plan` / `git-merge-apply`。field合成、削除競合、設定、全プロジェクト検査、差分。plan hashでレビュー済み候補を適用し一括Undo/Redo。正本・index・HEAD/branch・来歴の変更と実行中Git操作を阻止し、Git branch/indexは変更しない。comments/interop変更、ファイル追加・削除の適用、競合編集UIは残る |
| 図形履歴・blame / PR資料 / Git入口 | 履歴・項目別blame CLI・Desktop実装、Rust・ブラウザ検証済み | `entity-history`、IDで追加・削除・形状・依存変更を追跡。`entity-blame` は正規化した項目の最後の変更commit・作者、図面移動の保持、表示依存の別記、欠損・上限時の暫定表示。固定first-parent・該当commit比較。実機の履歴とblame読込を確認済み。全図面・全レイアウトの両版PDF/SVG・差分・コメント・検査報告を `review-bundle` で生成し、手動CAD CI workflowのartifactへ保存する入口を追加。block内部・全merge parentのblame、branch/worktree導線は残る |
| コメントの版紐付け | Desktop・レビュー資料実装、Rust・実機操作検証済み | 作成時の図形・正本hash・観測したHEADを不変で記録。図形変更・削除・移動、正本全体の変更、未記録・壊れた版情報を表示。コメント更新の正本CASと未知metadata保持。レビュー資料は各固定版で判定し原文保持。全コメントスキーマ・アンカー位置の検証は含めない |
| 建具・2線・中心線 | CLI・Desktop実装、Rust・ブラウザ・実機操作検証済み | 平面記号を既存プリミティブへ生成。寸法・向き・Undoを検証。断面・立面、パラメータ変更追従は残る |
| 図面間コピー・部品パレット | 正本クリップボード・部品ファイル・フォルダーパレットのCLI・Desktop実装 | 図形と参照先のinclude/detach/reject、名前衝突のあるlayer/pen/style/color/group、shared/nested blockの新IDと参照付替え、他プロジェクトへの移植、全正本CAS・検査・一括Undo/Redo。クリップボードは図面・プロジェクト切替で保持。部品JSON保存/読込と保護領域・上書き拒否。JWSは必須係数と全報告を伴いclipboardへ読み込める。回転・均等倍率、水平/垂直参照寸法の90度軸交換、ハッチ方向を扱う。非再帰フォルダー一覧・評価済みサムネイル・ページ切替・選択時file hash検査を追加。JWK・非均等倍率は残る |
| 文字検索・置換・縦書き・計算 | 検索・置換Desktop、置換・style変更・縦列注記・計算CLI/Desktop実装 | 注記Text限定、図面/選択範囲、revisionと全正本CAS、候補破棄・Undo。空選択を全図面へ広げず寸法/block内部を除く。計算は四則・括弧・指数・f64・0～12桁を通常Textに保存。縦列は一Unicode scalarずつ下へ進み、LFで左列へ移動、回転・反転・SVG/PDF/意味差分を反映。JWW/DXFは報告付き一文字展開、strict阻止。字体の縦組み・縦中横・native JWW縦字・式依存再計算・外部エディタは残る。[配置と制約](../docs/annotation-writing.md) |
| 混在縮尺 | 一枚内のビューポートをCLI・Desktop・SVG/PDFへ実装 | 元図面・モデル原点・紙上位置/大きさ・個別縮尺・レイヤーを指定。レビューhash/全正本CAS/Undo、紙上読み取り専用、Git比較の元図面依存署名。1000mmの線の10mm/50mm出図を検証。混在JWW取り込みとクリップ/配置変換、紙上の図形差分オーバーレイは残る。[操作と制約](../docs/sheet-viewports.md) |
| JWW版・文字互換 / fixture | 未着手 | v600以外の仕様・現行版サンプルを確認。未知レコードは推測しない |
| DXF / SXF / JWC / JWK / JWS | DXF CLI・DesktopとJWS取り込み実装、Rust・画面・実機検証済み | DXF R2013モデル空間の入出力、単位・日本語・block・破線・塗り、必須JSON報告。Desktopで警告表示・単位指定・strictを選び、検査済みの新規正本を開く。レポート先行・正本とGit領域の保護・候補の一括公開をCLIと共通化。実機で出力→取り込み→CAD検査と元正本不変を確認。JWS 351/420/600は全バイト読取り、係数を明示して新正本へ変換。公式配布341件を読み、339件がCAD検査通過・2件の塗り形状を阻止。[検証範囲](../docs/jws-compatibility.md)。JWS部品のCLI・Desktop読込と配置は実装。JWSフォルダー一覧・サムネイル・配置も実装。JWS出力、交換先アプリ確認、SXF/JWC/JWKは残る |
| 下絵画像 | 未着手 | 配置、透過、縮尺合わせ、リンク検査とPDF |
| 測定・集計 | CLI・Desktop実装、Rust・フォーム操作検証済み | 線・円・円弧・楕円・polyline・solid・hatch、穴・自己交差・単位・CSV。入れ子block展開と図面別unionを追加。回転・反転・倍率・穴・部分重複・接触・曲線近似をRust検証。Desktop実機でunion選択と正本不変を確認済み |
| 作図短縮・接線・接円・分割・倍率・属性取得 | 円への外点接線・2直線への接円・線/円弧分割・均等倍率・属性取得を実装 | 共通preview/Batch/Undo。接円は指定半径と中心付近の点から4候補を選ぶ。延長線への接触を警告し平行・ほぼ平行・精度不足を拒否。倍率は正の有限値・基点指定、半径・寸法offset・図形別倍率を変更し共有紙上styleを保持。参照寸法は元図形に追従。属性取得は1図形からレイヤー・pen・該当style/fillを次の作図へ使い、元正本を変更しない。欠損参照・図面切替で解除、レイヤー選び直しで取得レイヤー優先を解除。円や円弧への接円・非均等倍率・クロック設定は残る |
| 楕円・自由曲線 | CLI・Desktop実装、Rust検証済み | 楕円生成入口、許容距離付きcubic Bezierポリライン。元Bezier制御点の編集追従は残る |
| 外部変形・設定移行 | revision付き編集要求のCLI実装 | `generate` / `edit` のpreview/check/apply。JWF・Windowsバッチ互換は残る |
| 2.5D・日影・天空図 | 角柱モデルの計算・CLI・Desktop実装、Rust・ブラウザ・実機検証済み | 閉じた平面形状と高さの全辺平行投影、指定太陽角による平らな地面の影union、水平SVFと正射影天空図。元正本と条件・配置・結果の保存、通常2D作図のpreview/CAS/Undo。[方法・範囲](../docs/massing-calculations.md)。地形・屋根・隠線・日時位置からの太陽計算・時間日影・法規比較は残る |

未着手を実装済みとして扱わない。各まとまりで関連テストを実行し、最後にリポジトリの検査・Desktopの受入を行う。形式資料や実機が必要な項目は、その依存条件と進められる範囲を残す。

角柱計算の検証: `build/massing-core-generator-v5.log` は7件を通過。矩形の四方向の影面積、中庭の穴、重複union・凹形状、側面/平面の投影、開放天空・中庭・屋根上、45°地平線のSVF≈0.5、不正入力、作図のCAD検査・正本保持・古い要求拒否・Undoを確認。`build/massing-browser-v1.log` の7件で条件入力・候補破棄・報告保存/取消・キーボード・小画面を確認。`build/massing-cli-acceptance-v2.log` で計算入力の読取り上限と入力保護、3種類の生成→報告→preview→apply→CAD検査を確認。PDFを画像化し、投影の全辺、地面の影と元形状、天空の輪郭を確認した（出図の原点は別途設定）。vlmkit integrityは3画面幅で問題なし。interactionsの2警告は同じ条件の再Previewと既に消えた候補のCancelで、実操作はブラウザとDesktopで検証する。

失敗記録: generator v1はテスト用polylineの終点閉鎖、v3はCreateが新IDを割り当てることに追従しないテストを修正。Desktop v1/v3は前段でblockへ移した元図形を選ぶテストを修正。v2は生成要求へのSerialize追加漏れを修正。Desktop v4はJS経由のJSON整数/浮動小数点表記差により同じ結果を拒否する実装不具合を検出。型付きの数値で再計算照合し、`build/massing-ipc-v2.log` でJS表記、tamper・既存ファイル・正本出力拒否を確認。失敗ログを保持する。

検証記録: `build/jwcad-parity-ci-v1.log` は全ワークスペース静的検査・Rustテスト・CLI smoke・viewer build・unit 49件・browser E2E 19件・macOS Desktop 2シナリオを通過。後続のstage/history追加は関連Rust・ブラウザ・vlmkitと `build/jwcad-parity-desktop-v10.log` の実機2シナリオで確認。`build/jwcad-parity-ci-v2.log` も全ワークスペース・unit 49件・browser E2E 22件・macOS Desktop 2シナリオを通過。後続の測定は `build/measurement-desktop-v2.log` の実機2シナリオ、browser E2E 3件、通常・union結果のvlmkit integrityで確認。レビュー資料は複数図面・全レイアウト・参照寸法・共有block・改ざん検出・壊れたNDJSONの固定検査をテスト済み。`build/cad-review-ci-acceptance-v1.log` でCIスクリプトの生成・hash検証が成功し、PDFのページ数を外部解析で確認。GitHub上のworkflow実行は未確認。`build/jwcad-parity-ci-v3.log` は測定・レビュー資料を含む全体検査、unit 49件・browser E2E 23件・macOS Desktop 2シナリオを通過。JWS追加は関連Rustテスト、公式サンプル全バイト読取り、339件の正本検査と2件の阻止、CLI取り込み・SVG/PDF表示を確認。Jw_cad実機は未確認。

コメントとDXF追加の検証: コメント版情報はDesktop Rust 54件、CLIの両版評価・原文保持テスト、実機の作成・設定変更・元図形変更で確認。DXFは交換ライブラリ7件・CLI15件・Desktop54件、ブラウザ2件、`build/dxf-desktop-native-v1.log` の実機2シナリオを通過。モバイルのcommand bar回帰はviewerブラウザ9件とvlmkit integrityで確認。受け側CADのDXF表示比較は未確認。

`build/jwcad-parity-ci-v5.log` はコメント・DXFまでを含む全ワークスペース検査、unit 49件・browser E2E 27件・macOS Desktop 2シナリオを通過。数値整形はコメントのbindingを保持する専用テストを追加。commit追加はGit 16件・CLI 17件、ブラウザ3件、vlmkit integrityとinteractions、実機の候補確認・確定・元index/正本保持・Git watcher更新を確認。

倍率と接円の検証: cad-edit 62件と参照寸法integration 6件、cad-toolkit 13件、倍率フォーム1件・toolkitブラウザ4件が通過。vlmkit integrityは倍率・接円とも問題なし、倍率interactionsも通過。接円interactionsの警告4件は未プレビューのApply/Cancel無効状態。`build/tangent-circle-desktop-v2.log` で実機2シナリオ、倍率と参照寸法追従・接円保存・プレビュー中の正本保持・Undo復元を確認。

`build/jwcad-parity-ci-v6.log` はcommit・倍率・接円を含む全ワークスペース検査、unit 49件・browser E2E 32件・macOS Desktop 2シナリオを通過。

文字編集の検証: style変更は本文・位置・回転・反転・IDと非Text除外をRust検証。Desktop生成はrevisionと正本manifestのCASテストを追加。ブラウザ3件で日本語複数置換、空選択、候補無効化、非同期の旧版結果・拒否を確認。`build/text-style-cli-acceptance-v1.log` のCLI preview/apply/checkで本文・図形の保持、`build/text-tools-desktop-v1.log` の実機2シナリオで置換・styleのみ変更・Undoを確認。vlmkit integrityは問題なし、interactionsは再プレビューの同じ結果とstyle未指定時の無効ボタンで3警告。

`build/jwcad-parity-ci-v7.log` は文字検索・style変更までを含む全ワークスペース検査、unit 49件・browser E2E 35件・macOS Desktop 2シナリオを通過。計算注記はcad-toolkit 15件、toolkitブラウザ5件、CLI生成・適用・CAD検査、`build/text-calculation-desktop-v1.log` の実機2シナリオでゼロ除算阻止・結果表示・保存・Undoを確認。vlmkit integrityは問題なし、interactionsの4警告は未プレビューのApply/Cancel無効状態。

属性取得の検証: 欠損参照・型ごとのstyle取得を含むUI unit 50件、矩形penのpreview/check/Undoとblock referenceへのpen保持をRust検証。ブラウザ1件で取得・元データ保持・独立した座標・解除・空選択を確認。`build/creation-attributes-desktop-v1.log` の実機2シナリオでペン取得→矩形へ引継ぎ→Undoと解除の正本不変を確認。属性や作図文脈を変えた間に古い非同期生成結果が戻る場合は破棄する。

`build/jwcad-parity-ci-v8.log` はRust・CLI・viewer検査を通過した後、DesktopのUndo直後に古い監視レビューが戻る競合で失敗した。保存開始時の旧レビュー無効化とUndo完了後の再読込を追加。`build/history-refresh-desktop-v1.log` の実機2シナリオと、監視の古い読込を拒否するunitテストで確認。失敗ログは保持する。

マージ適用の検証: 複数正本の一括CAD検査・CAS・Undo/Redo、保護領域・欠損ファイル・不正候補の拒否、並行変更時に自分の変更だけをrollbackするRustテストを追加。Git候補は固定hash・改変報告・競合・正本変更・comments/ファイル構成変更を検証。ブラウザ3件、vlmkit integrity/interactionsは問題なし。`build/merge-apply-cli-acceptance-v1.log` はCLI preview→古いhash拒否→apply→CAD検査とindex/HEAD保持を確認。`build/merge-apply-desktop-v1.log` の実機2シナリオで、プロジェクト名のマージ候補・適用・Undoと手元の図形/index/HEAD保持を確認。履歴の検査文脈を全正本へ広げ、設定・他図面の一括Undoにも対応した。

`build/jwcad-parity-ci-v9.log` は計算注記・属性取得・マージ適用とUndo監視競合の修正まで含め、全ワークスペース検査・CLI smoke・viewer build・unit 51件・browser 40件・macOS Desktop 2シナリオを通過した。

クリップボードの検証: 別プロジェクトの同名設定を改名し、既存設定・元のNDJSON行を保持すること、寸法のinclude/detach/rejectと新ID参照、入れ子・反転・倍率付きblockとblock内寸法をRust検証。新規blockの一括作成・途中失敗のrollback・Undo/Redoも確認。部品読込は64 MiBを越えない読取り、保存は検査済みの新規ファイルのみ。Desktop IPCで保存/読込・不正部品・上書き・保護領域の拒否を確認。ブラウザ5件で座標・候補破棄・CADエラー・読み取り専用・キーボードでの貼付と部品読込/保存を確認。vlmkit integrityはクリップボード・候補とも問題なし、interactionsの3警告は同じパス/部品の再読込と初期無効のLoad。`build/clipboard-cli-acceptance-v1.log` で部品生成→プレビュー→古いhash拒否→別プロジェクトへ貼付→CAD検査と元正本保持を確認。`build/clipboard-desktop-v1.log` の実機2シナリオでコピー・保存・再読込・プロジェクト切替での保持・貼付・Undoと元正本/index保持を確認。

`build/jwcad-parity-ci-v10.log` はRustの監視復旧テストで、復旧通知前にStopが処理される競合により失敗。通知を受け取ってから停止するテストへ変更し、停止時に未処理通知を破棄する仕様は維持した。`build/watch-recovery-test-v1.log` で確認。失敗ログを保持する。

JWS部品・変換付き貼付の検証: JWS→検査済みclipboard→CLI新規部品/報告、壊れた入力・不正係数・空図形・64 MiB超の阻止、倍率と基点の一度の変換をRust検証。別CADの正本/来歴/復旧とsymlinkを経た保存先も拒否する。`build/jws-part-desktop-v2.log` の実機2シナリオでJWS読込・全報告・貼付・CAD検査・Undoを確認。v1の失敗はテストが貼付transaction完了前にCLI復旧を開始した競合で、UIの履歴操作が可能になるまで待ってから検査するよう修正した。`build/clipboard-transform-core-v3.log` は8関連テストを通過し、回転・倍率・参照軸交換・ハッチ方向・block内部/紙上style保持・Undo・旧要求の既定値と不正入力阻止を確認。JWS画面のvlmkit integrityは初期/読込後とも問題なし。読込後interactionsの4警告は同じパスへのBrowse、同一部品再読込、初期無効Load。最初の読込後integrityのfixture初期化エラーを修正し再検査した。

`build/jwcad-parity-ci-v11.log` はJWS部品と回転/倍率付き貼付まで含め、全ワークスペース静的検査・Rust・CLI smoke・viewer build・unit 51件・browser 48件・macOS Desktop 2シナリオを通過。`build/format-research/jws-parts-v1/manifest.ndjson` は公式341件を部品変換し、339件通過・2件の塗り境界阻止・全元バイト不変を確認。生成物は再配布せずignored内に保持。

部品パレットの検証: libraryの非再帰走査・安定順/ページ・不正部品/JWS報告・必須係数・symlink/64 MiB拒否・選択時hash検査・大規模block展開時のサムネイル省略と完全読込をRust 3件で確認。ブラウザ2件とclipboard 8件でキーボード、ページ切替、係数/フォルダー変更時の破棄、SVGのscript/image除去、外部変更拒否、モバイル幅を確認。`build/part-library-desktop-v1.log` の実機2シナリオで一覧・未対応JWSの選択阻止・部品選択・正本不変を確認。`build/part-library-cli-acceptance-v1.json` で公式データの変換済み339部品を数え、先頭12件の検査とサムネイルを確認。vlmkit integrityは問題なし、interactionsの8警告は同じ一覧/選択の再実行、初期無効のページ/未対応/JWS係数待ちボタン。初回browserの自動fixture準備がDOM更新前に無効ボタンを押したため、fixtureを修正し再検査した。

`build/jwcad-parity-ci-v12.log` はフォルダーパレットと他プロジェクトの報告出力先保護まで含め、全ワークスペース静的検査・Rust・CLI smoke・viewer build・unit 51件・browser 50件・macOS Desktop 2シナリオを通過。

`build/massing-desktop-v5.log` の実機2シナリオが通過。3種類のプレビューは正本を変更せず、計算報告・配置要求を保存し、適用→CAD検査→Undoで元のNDJSONを回復し、Git indexを保持することを確認。

`build/jwcad-parity-ci-v13.log` は全ワークスペースfmt/clippy/Rustテスト、no-default Desktop、CLI smoke、viewer build、unit 51件、browser E2E 52件、macOS Desktop 2シナリオを通過。P3角柱計算を含む全体回帰を確認した。

混在縮尺の検証: `build/viewports-render-v1.log` は1:100と1:20の実寸、共通紙上線幅、クリップをSVG/PDFで確認。`build/viewports-toolkit-v4.log` の34件はレイアウトの全正本CAS・Undo、部品モデル表示と元レイアウトmetadata保持を含む。`build/viewports-diff-v1.log` は配置先に図形変更がなくても元図面の変更を報告することを確認。`build/viewports-browser-v3.log` の2件で候補破棄・空一覧・レイヤー・キーボード・小画面・local clip保持とscript/image除去を確認。`build/viewports-desktop-v1.log` の実機2シナリオで適用・元図形/index保持・紙上モデル作図阻止・Undoを確認。`build/viewports-cli-acceptance-v1.log` と `build/viewports-cli-preview-v2.log` でCLI適用・CAD検査・JWW専用check阻止・候補SVG/報告・一枚PDFを確認。PDFのA3横一ページを外部解析・画像化して確認。vlmkit integrity v3は問題なし。interactionsの2警告は同条件の再Previewと折り畳んだ読み取り専用報告で、キーボード操作をブラウザ検証した。初回browserのfixture内script終端、追加clipboard警告の変数参照漏れを修正し、失敗ログも保持する。

`build/jwcad-parity-ci-v14.log` は混在縮尺まで含め、全ワークスペース静的検査・Rust・no-default Desktop・CLI smoke・viewer build・unit 51件・browser 54件・macOS Desktop 2シナリオを通過。

縦列注記の検証: `build/vertical-core-v4.log` は7関連crateのテストを通過し、旧Textの既定値、セル幅/高さ/間隔、列・align・回転・反転、SVG選択枠・PDF紙上座標、JWW/DXF一文字展開とstrict阻止、意味差分・全正本CAS・Undoを確認。`build/vertical-native-boundary-v1.log` でnative縦字フラグの取り込み警告、未変更バイト一致、編集後の明示的な再生成報告と旗の除去を確認。`build/vertical-cli-acceptance-v1.log` は候補・適用・古い要求拒否・CAD/JWW専用check・全警告の記録・SVG/PDF・strict報告保持・再取り込みを確認。A3横PDFの二列を外部描画して確認。`build/vertical-browser-v1.log` は4件で検索/選択範囲、候補破棄、キーボード・小画面を確認。`build/vertical-desktop-v1.log` の実機2シナリオでプレビュー中の正本保持・ID/本文/style/at保持とUndoを確認。vlmkit integrityは問題なし。interactionsの9警告はfixtureの同じrevision再読込と初期無効の置換/style/Apply/Cancelで、書字方向のEnter操作をブラウザで検証。初回coreのSVGテスト用依存、strict報告のblockers/warningsの混同、PDF行列のゼロを整形前の文字列と比較したテストを修正し失敗ログを保持する。交換先CADと字体の縦組みは未確認。

`build/jwcad-parity-ci-v15.log` は縦列注記と部品への保持・制御文字の阻止まで含め、全ワークスペースfmt/clippy/Rust、no-default Desktop、CLI smoke、viewer build、unit 51件・browser 55件・macOS Desktop 2シナリオを通過。`build/vertical-request-protection-v1.log` で生成要求と出力先が同じ場合の入力/正本保持を確認。両skillsのquick_validateも通過。
