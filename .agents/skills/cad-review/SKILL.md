---
name: cad-review
description: このCADのTOML・NDJSON図面をGitの版や別プロジェクトと比較し、図形・設定の意味差分、検査結果、PDF/JWW互換性をレビューする。CADアプリ本体のコードレビューには使わない。
---

# CAD図面レビュー

図面変更の意図、寸法・配置・用紙への影響、互換性を確認する。ソースコードのGit差分だけで図面の妥当性を判断しない。

## 比較対象

リポジトリルートとCADプロジェクトルートを特定し、適用されるAGENTS.mdと `git status --short` を読む。HEAD、ステージ済み、作業中、任意のコミット、別プロジェクトのどれを比較するか依頼から決める。指定がなければHEAD対作業中と明記する。

Desktopと `git-diff` はcommit/ref・index・worktreeを比較できる。画面で選んだ比較対象と返されたsnapshot hashを確認する。Git外・初回コミット前・指定版に正本がない場合は比較不可と説明し、別の比較元があれば使う。空の比較元で変更0を装わない。

CLIはリポジトリルートで実行し、プロジェクトに絶対パスを渡す。Git比較は `git-diff PROJECT --base REF --head index|worktree|REF --format json|svg --out PATH` を使う。比較対象を一時スナップショットへ固定し、作業ツリーとindexを変更しない。正本と必要なJWW来歴を保持する。worktreeが必要ならCodex管理を優先し、`/private/tmp`に作らない。

## 検査と意味差分

各行の前に `cargo run -p cad-cli --` を付け、対象に置き換えて実行する。

```text
check BASE --target cad --format json --out -
check CURRENT --target cad --format json --out -
diff BASE CURRENT --format json --out DIFF.json
diff BASE CURRENT --format svg --out DIFF.svg
export-pdf CURRENT --drawing NAME --out CURRENT.pdf
```

CLIの読込は未完了トランザクションを復旧する場合がある。原本への変更が許されない監査では、一式のコピーを検査する。失敗した復旧データは直接変更しない。

意味差分JSONの `changes` と `configuration_changes` を読む。図形ID・図面名で追加、削除、変更を対応付け、座標、文字、レイヤー、ペン、レイアウト、ブロックの影響を調べる。変更件数だけで完了を判断しない。block定義の差分はファイル署名として報告されるので、内容と影響する全配置を別途確認する。

参照寸法の値・参照切れ、ハッチの境界、印刷対象、縮尺・原点・クリップ、文字と寸法の重なりを確認する。SVG意味差分は完全な完成図表示とは限らないため、重要箇所は両版の対象図面PDF/SVGでも確認する。自動検出された重なり・用紙外警告は目視で照合する。チェッカー合格を建築仕様の正しさと混同しない。
参照先変更による寸法の評価結果と、共有・入れ子ブロックの表示変更は `dependency_changed` として報告される。変更図形を参照する寸法を両版の完成図でも確認する。集計対象外の図面や非対応形式の内容を差分検査済みと扱わない。

混在縮尺のレイアウトは `configuration_changes` の `viewport_sources.DRAWING/LAYOUT` に元図面からの描画署名を持つ。配置先図面の図形が不変でも、元図面・block・styleの変更を検出する。署名は切り取り外やmetadataの変更にも反応し、差分SVGの図形オーバーレイはモデル座標のまま。両版の指定レイアウトをSVG/PDFで確認する。ビューポートのJWW変換は未検証で出力を阻止するため、モデル図面の出力で紙上配置の納品を代替したと扱わない。

一括出図には `export-set PROJECT --revision REF --page DRAWING@LAYOUT --out NEW_DIRECTORY` を使う。`--page` を出図順で繰り返す。省略すると全図面のactive layout。`--jww` はactive layoutのページのみ。`verify-export-set DIRECTORY` は各出力のサイズとhashを検査するが、manifestの改ざんを認証するものではない。dirtyな版はcommit OIDだけで特定せずsnapshot hashを併記する。

競合確認には `git-merge-plan PROJECT --base BASE --ours worktree --theirs REF --out NEW_PROJECT` を使う。候補と `build/merge-report.json`、差分を確認する。競合やCADエラーがあればコマンドは失敗し、候補・報告を保持する。元の正本とindexは置換されない。

図形の履歴には `entity-history PROJECT --entity ID --revision REF --limit 50 --out build/entity-history.json` を使う。指定版のOIDを固定し、first-parentで最大500コミットを走査する。`truncated` や欠損版の `warnings` があれば履歴全体を確認したと扱わない。図面の図形IDを対象とし、同じcommit内の別図面への移動は削除と追加の組として表す。Desktopでは単一選択の履歴を読み、各イベントからparentと該当commitを比較できる。block内部の図形とmergeの全parent経路の帰属はまだ対象外。

項目の変更元には `entity-blame PROJECT --entity ID --revision REF --limit 50 --out build/entity-blame.json` を使う。正規化したentityのJSON Pointerごとに、現在の値を最後に変更したfirst-parent上のcommit、作者、subject、時刻を記録する。配列は座標などのまとまり単位。省略された既定値は正規化後の値であり、原文の行・文字列に対するGit blameではない。図面移動は同一commitの削除・追加を組にして扱い、図形の全項目を新規作成と誤認しない。図面位置の変更元はdrawing_origin。entity自体が同じまま表示を変えたstyle・block・参照寸法の変更はdependency_changesに別記する。

DesktopのLoad field originsはHEADの確定項目を読み、作業中の未commit変更を含めない。値と比較元OIDを確認し、Compare field changeでparent対commitを比較する。`reliable=false`・走査上限・読めない版・null originがあれば表示された帰属は暫定、nullは未解決と説明する。削除済み図形はentity-historyを使う。mergeコミットはfirst-parentとの差として帰属し、他parent側の元作者まで追ったと主張しない。

## レビュー済みマージの適用

正本へのマージ適用も依頼された場合は `git-merge-apply PROJECT --drawing NAME --base BASE --theirs REF --out build/merge-apply-plan.json` を使う。oursは常にworktree、base/theirsはcommit。全変更ファイル、競合のbase/ours/theirs、設定差分、CAD検査、固定OIDとsnapshot hashを確認する。適用は同じ引数に `--apply --expected-plan HASH --out build/merge-apply-result.json` を付ける。DesktopもMerge CAD source changes→Preview CAD merge→Apply reviewed CAD mergeの順で使う。正本を変更し、アプリのプロジェクト単位Undo/Redoへ記録する。Gitのbranch・indexは変更しない。

競合・CADエラー・正本やGit文脈の変更・実行中のGit操作・comments/interop変更・ファイル追加/削除は適用できない。候補のblockedを上書きせず、原因を解消して新しい候補を確認する。ファイルの追加/削除を含む候補資料は `git-merge-plan` で保持する。これはGit branchのmergeや競合の自動解決ではない。レビューのみの依頼で適用しない。

## 図形単位のステージ

ステージも依頼されている場合のみ `git-stage PROJECT --drawing NAME --entity ID --out build/stage-plan.json` を実行する。`--entity` は繰り返せる。対象図面は既にindexに存在する必要がある。プレビューはindexを変更しない。参照寸法・参照先と必要なレイヤー・style・blockを含めた候補全体が検査される。共有定義が他図面に与える影響、`expanded_ids`、`changed_files`、`diff`、`cad_check` を全て確認する。既存のステージ変更のうち対象・依存範囲外は保持される。

確認した `plan_hash` を使い、同じ選択で `git-stage ... --apply --expected-plan HASH --out build/stage-result.json` を実行する。正本・indexが変われば適用は拒否されるので、新しい候補を再確認する。Desktopでも選択→Preview staging→Stage reviewed candidateの順で適用する。直接ソースの編集、アプリUndo、commit/pushと区別する。indexは変更され、作業中の図面ファイルは保持される。チェッカーエラーを無視しない。

commitも明示的に依頼され、hook・署名を省略する方式が依頼の範囲で許される場合に限り、`git-commit PROJECT --message-file MESSAGE.txt --out build/commit-plan.json` で候補を読む。index全体が対象で、選択図形の依存以外のstageも全て含まれる。branch・作者/committer・全ファイル・patch・対象CAD検査を確認する。他プロジェクトやCAD以外の検査を済ませたとは扱わない。適用は同じメッセージファイルと `--apply --expected-plan HASH --without-hooks` を指定する。DesktopもPreview complete index→全差分確認→方式の確認→Create reviewed commit。正本とindexは保持される。hook・署名が必要、merge/rebase中、detached HEAD、patchの表示上限/文字コードで全体を確認できない場合は通常のGit手順を使う。previewはGit objectを作る場合があり、失敗後にcommit objectのOIDが報告されたらbranchの実際の状態を確認してから再試行する。pushは別の依頼が必要。

## 互換性と報告

注記の `writing_mode` を変更した場合は、位置が同じでも基準点の意味・列方向・回転と反転を確認する。vertical_uprightは一Unicode scalarずつの配置で、縦中横や字体の縦組み変換は含めない。JWW/DXFの `upright_text_expanded` は一文字ずつの図形へ展開する近似で、再取り込みで一つの注記やIDを戻せない。取り込みの `native_vertical_text_unmapped` と変更時の `native_vertical_text_replaced` を保持し、元バイトの正確な保持と正本の表示互換を区別する。

JWW納品が依頼に含まれる場合は `check CURRENT --drawing NAME --target jww-v600 --format json --out -` を実行し、全警告を確認する。実際の出力には `export-jww ... --report REPORT.json` を付ける。近似不可なら `--strict`。レポートを保持し、実機未確認やレコードクラスのfixture不足を明示する。

最後に比較元・比較先、変更の意味、位置（図面名・ID）、検査結果、確認できない範囲を短く報告し、生成したレビュー資料をリンクする。修正も依頼されていれば `cad-drafting` の正本編集手順へ進む。レビュー依頼だけでcommit/push、コメント投稿、競合の自動解決を行わない。

## レビュー資料とCI

```text
review-bundle PROJECT --base REF --head REF_OR_WORKTREE --out NEW_DIRECTORY
verify-review-bundle NEW_DIRECTORY
```

両版の全図面・全レイアウトをPDF/SVGにまとめ、図面別の意味差分SVG、全体差分JSON、設定、各版のコメント原文、CAD検査とJWWターゲット検査の警告を保存する。未知・壊れた正本は固定したバイト列の検査結果を残してblockedとなる。両版にプロジェクトが必要で、現在のcheckoutに対象ディレクトリが存在する必要がある。プロジェクト全体の追加・削除はこのコマンドで比較できない。

JWWターゲット検査は実際のJWW出力報告や受け側CADでの確認の代わりにならない。新しいDesktopコメントは作成時の図形と正規化した正本全体のhashを記録する。版ごとの `annotation_files[].version_checks` で図形の変更・削除・移動、正本全体の変更、壊れた版情報を確認する。正本全体の変更は無関係な図面の変更でも発生するため、元図形が変わったという判定と区別する。既存の版情報がないコメントはunboundで、現在の図形との対応を保証しない。JSON構文と版情報を検査するが、コメント全体のスキーマやアンカー位置を検証したとは扱わない。元コメントのバイト列と警告を残し、AIでcommentsを直接修正しない。manifestのhashは改変検出用で、認証ではない。

`CAD_REVIEW_PROJECT`・`CAD_REVIEW_BASE`・`CAD_REVIEW_HEAD`・未使用の`CAD_REVIEW_OUT`を指定して `bash scripts/cad-review-ci.sh` を実行できる。GitHub ActionsのCAD source reviewは手動起動で、blocked報告もartifactへ保存する。Git操作・PRへの投稿は行わない。投稿・pushはユーザーから個別に依頼されている場合だけ行う。
