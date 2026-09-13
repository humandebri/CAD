# テスト整理と実装改善（2026-09-12）

対象は作業開始時点のソース。開始前から存在した変更を基準に比較した。
削除判断は、削除で見逃す現実的な不具合と、実行・変更時の負担を比較した。
下記の「旧テスト名」は、削除だけでなく統合・より具体的な検証への置換を含む。

## 対応と残す保証

| 対象 | 対応 | 残す保証・見逃しを防ぐ不具合 |
| --- | --- | --- |
| crate名、serde既定値、列挙型、ライブラリ出力 | 名前や型・ライブラリ自体を再確認するテストを削除 | 実際のプロジェクト読込・check・render・import/exportで接続不良を検出 |
| check結果のJSON | 手組み結果のsnapshotを実際の参照エラー検査へ統合 | 利用側が使うstatus・診断・フィールドの破損 |
| SVG・diff | 巨大snapshotと内部signatureを、実際の差分理由・図形・選択metadataの確認へ置換 | 形状変更の見落とし、鏡映消失、誤ったentity選択 |
| 編集操作 | 型ごとの成功確認を代表形状の数値・角度・鏡映・undo確認へ統合 | 座標計算の誤り、履歴欠損、元bytesに戻らない不具合 |
| 履歴上限 | 105回の編集を100件の履歴fixture＋1回の実編集へ置換 | 上限超過時に最古を消せない、最新履歴を失う不具合 |
| permission・複数ファイル履歴 | 編集直後とundo直後のpermissionを一つのケースで確認。実際のblock複数ファイル操作を保持 | 実行bit消失、複数ファイルの不整合 |
| clear history | スレッド開始通知による曖昧な同期を廃止 | 共通lock内で削除すること、削除後の編集がundo可能であること |
| watcher | backend種類・手作りmanifestの確認を実ファイル変更通知に置換 | fallbackが通知しない、旧projectのwatcherが残る不具合 |
| Desktop Git | 変更・追加・非ソースstageを一つのfixtureに統合。Git不要ケースからgit init/commitを除去 | HEADとの比較、非ASCII path、HEAD/project切替、失敗からの復旧 |
| JWW | importer計算の再現を固定座標・style・warningへ置換。改ざん/競合ケースの一部を最小入力へ変更 | 実形式の互換性は既存corpus、byte-exact保存、semantic再importで検出。改ざん・競合・既存出力保護は保持 |
| hatch | 不正入力の詳細な場合分けをcheckerに集約。出力側は代表入力で拒否を確認 | checker接続の欠落、不正な出力や上書き |
| PDF | CID番号・重複subset生成を削除。実描画の鏡映、Unicode、欠落glyph、外部出力検証を保持 | 見た目・文字抽出・未対応文字の破損 |
| TS IPC | 複数ファイルのmock戻り値自己検証をcommand/argument/defaultの表へ統合 | Rust呼出名・引数名・revision・PDF layoutの配線不良 |
| TS queue | pollingと重複ケースをdeferredで順序固定したケースへ統合 | stale結果の採用、reset前のrevisionの混入、最新選択の消失 |
| Browser/Native E2E | 重複dragを統合。block子要素から親を選ぶ実操作を保持。nativeケースを別プロジェクト・別起動で実行 | window/crossingの誤選択、不可視layerの選択、前ケース依存 |
| CI | build内の型検査、workspace内のdesktop smoke、PDF fixture生成の重複実行を除去 | 既存の型・Rust・ブラウザ・native・Poppler検証を保持 |

## 実装変更

- 未使用のRust crate名API、codecの置換API・registry、EntityIdの二重保持、公開`export_loaded_project`、CLI `--smoke`を削除。互換shimは追加していない。
- 未使用のTS wrapper、layer queueの旧返却形式、到達しないqueue再開分岐を削除。
- PDFの同じフォントを指すF2と、全CIDが既定幅1000と同じ幅表を削除。
- `source_manifest_for`で対象ファイルだけをhash化する。対象外でもcanonical pathのsymlink検査は継続する。
- スナップ点・曲線bbox、文字重なり、hatch接続先を空間索引で検索する。元の候補順序を保持する。
- スナップcacheはdrawingとlayer rules両方のrevisionで更新する。
- 差分SVGはID索引と計算済みreportを利用し、選択図面のreviewでproject全体をcloneしない。
- HEADのcanonical blobを`git cat-file --batch`でまとめて読む。参照中の一時sourceはcache切替後も保持する。
- AI用差分はHEADとsource manifestで再利用する。source変更中の結果はcacheに格納しない。
- AI contextは生成時刻を除く内容で再利用する。既存のJSON/Markdownが完全に一致する場合だけcurrent manifestを更新する。旧世代を保持し、GCは行わない。

## 性能の実測

同一Mac・debug build・各3回の中央値（履歴上限テストのみ単独1回）。作業開始時の退避ソースと変更後に同一入力を与えた。ビルド等が並行した参考値であり、製品全体の速度保証ではない。

| 処理 | 変更前 | 変更後 |
| --- | ---: | ---: |
| 1万本の線：スナップ索引構築 | 304ms | 811ms |
| 同索引：端点検索1,000回 | 15,359ms | 24.5ms |
| 1万本の変更線：差分SVG | 5,450ms | 2,492ms |
| 円1,000個：交点検索20回 | 11.9ms | 1.0ms |
| 履歴上限の単独テスト | 5.44s | 0.19s |

空間索引は構築時間とメモリを増やす。再利用時の検索負担が減るというトレードオフであり、一度だけの操作がすべて高速になるとは言えない。native E2Eは各ケースを別起動するため起動負担が増える。

## 削除・統合・置換した旧Rustテスト名

### `apps/viewer/src-tauri/src/lib.rs`

- `ai_context_generation_is_content_addressed`
- `git_repo_root_project_can_be_loaded_from_head`
- `git_tracked_project_can_be_loaded_from_head`
- `replacing_project_watcher_stops_old_project_events`
- `staged_new_project_file_does_not_break_head_diff`
- `working_tree_added_entity_is_added_against_head`
- `working_tree_change_is_modified_against_head`

### `apps/viewer/src-tauri/src/project_watch.rs`

- `forced_native_failure_selects_manifest_poll_backend`
- `manifest_diff_reports_create_update_and_delete_only_once`
- `native_preference_uses_a_native_backend`

### `crates/cad-check/src/lib.rs`

- `exposes_crate_name`
- `json_report_shape_is_stable`
- `reports_missing_required_field_as_whole_check_failure`

### `crates/cad-diff/src/lib.rs`

- `detects_added_removed_modified_and_unchanged`
- `ellipse_signature_and_svg_path_include_all_geometry`
- `exposes_crate_name`
- `mirrored_text_and_dimension_metadata_affect_diff_geometry`

### `crates/cad-edit/src/lib.rs`

- `atomic_edit_preserves_unix_permission_bits`
- `clear_history_uses_the_shared_history_lock`
- `history_prunes_to_the_latest_one_hundred_generations`
- `line_intersection_is_reported`
- `manifest_history_restores_multiple_files_as_one_transaction`
- `rotates_and_mirrors_every_supported_entity_family`
- `translates_every_entity_family`

### `crates/cad-export-jww/src/lib.rs`

- `existing_dimension_compatibility_allows_only_value_changes`
- `exposes_crate_name`
- `final_source_manifest_conflict_preserves_existing_output`

### `crates/cad-import-jww/src/lib.rs`

- `creates_dimension_style_from_dimension_text_height`
- `creates_text_styles_from_jww_text_height`
- `separates_text_styles_by_width_and_spacing`

### `crates/cad-jww-codec/src/lib.rs`

- `archive_replacement_preserves_the_original_header_bytes`
- `supported_entity_class_registry_is_complete_and_unique`
- `writes_parseable_signature_and_header`

### `crates/cad-model/src/lib.rs`

- `defaults_text_mirror_fields_when_omitted`
- `exposes_crate_name`
- `loads_house_small_example`
- `obsolete_sheet_file_is_not_loaded_as_a_second_layout_source`
- `offsets_dimension_along_line_normal`
- `parses_all_mvp_entity_types`
- `rejects_invalid_json_line`
- `rejects_unknown_entity_type`

### `crates/cad-render-pdf/src/lib.rs`

- `associative_dimension_and_reflected_blocks_render_to_pdf`
- `embedded_font_subset_is_deterministic_and_rejects_missing_glyphs`

### `crates/cad-render-svg/src/lib.rs`

- `exposes_crate_name`
- `renders_house_small_with_entity_metadata`
- `renders_representative_entities_snapshot`

## 削除・統合・置換した旧TS/E2Eテスト名

### `apps/viewer/tests/desktop/desktop.e2e.mjs`

- edits, comments, changes a layer, restores history, and exports PDF

### `apps/viewer/tests/e2e/viewer.spec.ts`

- Shift-drag above the gesture threshold performs area selection

### `apps/viewer/tests/unit/desktop-ai-context.spec.ts`

- publishes selection clearing after an older write finishes
- serializes writes and keeps only the latest pending selection
- writes AI context through desktop invoke

### `apps/viewer/tests/unit/desktop-comments.spec.ts`

- uses comment create and status command contracts

### `apps/viewer/tests/unit/desktop-editor.spec.ts`

- uses drawing edit command contract
- uses snap query command contract
- uses undo, redo, and history list command contracts

### `apps/viewer/tests/unit/desktop-import.spec.ts`

- invokes import_jww with derived output directory

### `apps/viewer/tests/unit/desktop-layers.spec.ts`

- a reset in flight does not seed the next project revision
- reset prevents queued updates from invoking the backend
- updates visibility without mutating the source state
- uses the Tauri layer command contract
- uses the experimental export command contract
- uses the layout-aware PDF export command contract
- uses the lossless JWW preservation command contracts

### `apps/viewer/tests/unit/desktop-live-review.spec.ts`

- forwards Rust-filtered watch events without a second frontend allowlist
- watch transport uses the desktop command and event contracts

### `apps/viewer/tests/unit/drafting-panel.spec.ts`

- preview transport keeps the source revision and does not invoke apply

### `apps/viewer/tests/unit/svg-selection.spec.ts`

- selects the block reference when clicking expanded child geometry

### `apps/viewer/tests/unit/view-fit.spec.ts`

- uses a multiplicative toolbar zoom step

## 最終検証

macOSで実行し、以下が成功した。

- `cargo fmt --all -- --check`
- `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`
- `cargo test --offline --workspace --all-features --quiet`（doc testを含む）
- `cargo check --offline -p cad-desktop --no-default-features`
- `cargo build --offline -p cad-cli`
- `pnpm --dir apps/viewer test`（型検査＋unit）
- `pnpm --dir apps/viewer exec playwright test --reporter=line`（更新したCLIのSVG/PDF画像比較を含む。snapshot更新なし）
- `pnpm --dir apps/viewer desktop:e2e`（viewer build＋native build＋独立した2ケース）
- Popplerのfont情報、日本語・寸法文字列の抽出、PNG出力
- `git diff --check`、変更したshell/JavaScriptの構文検査

GitHub上のLinux CI自体はこのローカル作業では実行していない。
