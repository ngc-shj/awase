//! 診断ログの機微情報ゲート。
//!
//! 通常の `RUST_LOG=debug` は状態遷移・件数・タイミングだけを出す。物理キーコードや
//! 注入文字列は入力内容を復元できる（VK の列は本質的にキーログ）ため、追加の明示
//! オプトインがある場合だけ出す。
//!
//! **保証の範囲:** このゲートが覆うのは共通クレート（`src/`）と macOS 層
//! （`crates/awase-macos/`）の出力だけ。`crates/awase-windows/` の固有層には
//! 未ゲートの VK ログが多数あるため、「Windows の debug ログも安全」とは言えない。

use std::fmt;
use std::sync::OnceLock;

/// キー内容を診断ログへ出す明示オプトイン環境変数。
pub const KEY_CONTENT_ENV: &str = "AWASE_LOG_KEY_CONTENT";

static KEY_CONTENT_ENABLED: OnceLock<bool> = OnceLock::new();

fn key_content_value_enabled(value: Option<&str>) -> bool {
    matches!(value, Some("1"))
}

/// キーコード・注入文字列をログへ出してよいか。
///
/// `AWASE_LOG_KEY_CONTENT=1` の完全一致だけを許可する。値は起動後に固定し、別プロセス
/// から環境を書き換えられたとしても実行中に診断範囲が広がらないようにする。
#[must_use]
pub fn key_content_enabled() -> bool {
    *KEY_CONTENT_ENABLED.get_or_init(|| {
        let value = std::env::var(KEY_CONTENT_ENV).ok();
        key_content_value_enabled(value.as_deref())
    })
}

/// キーコードをログへ出す `Display` ラッパ。オプトインが無ければ伏せる。
///
/// `Display::fmt` はレコードが実際に出力されるときだけ呼ばれるので、ログレベルが
/// 無効なら整形コストは発生しない。フック/tap のコールバック内で使うためこの形にする
/// （`format!` を先に評価する書き方だと、レベルが無効でも毎打鍵で確保が走る）。
#[derive(Debug, Clone, Copy)]
pub struct MaskedVk(pub u16);

impl fmt::Display for MaskedVk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if key_content_enabled() {
            write!(f, "0x{:02X}", self.0)
        } else {
            f.write_str("<masked>")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{key_content_value_enabled, MaskedVk};

    #[test]
    fn only_exact_one_enables_key_content() {
        assert!(key_content_value_enabled(Some("1")));
        assert!(!key_content_value_enabled(Some("true")));
        assert!(!key_content_value_enabled(Some("0")));
        assert!(!key_content_value_enabled(Some(" 1")));
        assert!(!key_content_value_enabled(None));
    }

    /// 既定（オプトイン無し）では VK が出ないこと。テストプロセスは
    /// `AWASE_LOG_KEY_CONTENT` を設定しないので `key_content_enabled()` は false。
    #[test]
    fn a_masked_vk_hides_the_code_without_the_opt_in() {
        assert_eq!(MaskedVk(0x28).to_string(), "<masked>");
    }
}
