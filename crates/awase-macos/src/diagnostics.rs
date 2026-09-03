//! macOS 診断ログの機微情報ゲート。
//!
//! 実体は共通クレートの [`awase::diagnostics`]。macOS 層の呼び出し元がそのまま
//! `crate::diagnostics::…` を使えるよう再エクスポートするだけの薄い層で、
//! ゲートの状態（`OnceLock` で固定された判定結果）は共通クレートと共有される。
//! 共通クレート側の FSM ログと macOS 側の出力ログが同じ 1 つのオプトインで
//! 揃って開閉することが要点 — 片方だけ塞いでも診断ログは共有できない。

pub use awase::diagnostics::{key_content_enabled, KEY_CONTENT_ENV};
