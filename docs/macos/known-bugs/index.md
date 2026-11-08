# awase macOS 版 既知の不具合 — 索引

> macOS 固有の不具合は上流の `docs/known-bugs/`（`BUG-NNN`）と番号が被らないよう、
> ここで `MAC-NNN` として採番する（[../README.md](../README.md) 参照）。書式は上流の
> [docs-frontmatter-convention](../../../.claude/rules/docs-frontmatter-convention.md) に揃える。

| MAC | 状態 | 概要 |
|---|---|---|
| [MAC-001](MAC-001.md) | 修正済み | macOS で IME OFF→ON 直後の入力がまれにローマ字リテラルになる（「今日」→ `kilyou`） |
| [MAC-002](MAC-002.md) | 修正済み | macOS で切替キーが効かなかったとき、打鍵前だと張り直しが走らず生キーが漏れる |
| [MAC-003](MAC-003.md) | 修正済み | macOS で 英数 の直後に かな を押すと、かな の KeyDown が NICOLA に食われて IME が ON にならない |
| [MAC-004](MAC-004.md) | 修正済み | awase の張り直しが IME OFF 側の入力ソースをユーザーの選択から勝手に移す |
| [MAC-005](MAC-005.md) | 修正済み | IME 切替に使った親指打鍵が、そのまま親指シフトとしても数えられる |
| [MAC-006](MAC-006.md) | 修正済み | 非活性中に素通しした KeyDown の KeyUp が、活性化後に解釈される |
| [MAC-007](MAC-007.md) | 修正済み | macOS で親指の 英数/かな を押したまま同時打鍵の判定時間が切れると、切替キーの KeyUp が KeyDown より先に注入される |
