# macOS 版（フォーク `ngc-shj/awase` の `macos-port` ブランチ）

macOS 版はこのフォークだけで保守する。上流 `cuzic/awase` は 2026-10-22 に
`crates/awase-macos` を「移植予定の無いスタブ」として撤去しており（`70dfcb16`）、
macOS 版を上流へ戻す予定は無い。

## 上流との関係

- 最後に上流 `develop` を丸ごと取り込んだのは 2026-11-08（上流 `1aee5f9e`、
  マージ `2541771c`、タグ `macos-upstream-sync-2026-11-08`）。
- 以後は定期的な丸ごとの取り込みはしない。上流は Windows の IME 対策を足し続けて
  おり（1か月でコア以外を含むコードが約26%増加）、その大半は macOS では到達しない。
  必要なエンジン修正だけを個別に取り込む。
- 上流が `crates/awase-macos` を撤去したので、取り込むときは毎回
  modify/delete の衝突になる。macOS 側を残し、ワークスペースの `members` に
  `crates/awase-macos` を戻すこと。

## コアの扱い

コア（`src/`）は上流と共有のまま。macOS が実際に使うのは `Engine` の
`on_input`/`on_timeout`/`on_command`（`ToggleEngine`・`RefreshState`）/`compute_active`、
`NicolaFsm`、`config`・`yab`・`kana_table`・`scanmap`・`types` だけで、Windows 向けの
分岐（無変換/変換のモードキー設定、Space/Enter 親指、ソロ連打 OFF、n-gram、
`lang_detect` など）は macOS の入力では通らない。macOS 専用にコアを削る案は
検討したが、現状で困っていないため見送った（2026-11-08）。

## 不具合の記録

macOS 固有の不具合は [known-bugs/](known-bugs/index.md) に `MAC-NNN` で記録する。
上流の `docs/known-bugs/`（`BUG-NNN`）とは別の番号体系にすることで、上流の取り込みで
番号やファイルが衝突しないようにしている（過去に BUG-101 → 187 → 228 と2回振り直した）。
1件1ファイル・frontmatter の書式は上流の `docs/known-bugs/` に揃える。
