//! macOS 診断ログの機微情報ゲート。
//!
//! 通常の `RUST_LOG=debug` は状態遷移・件数・タイミングだけを出す。物理キーコードや
//! 注入文字列は入力内容を復元できるため、追加の明示オプトインがある場合だけ出す。

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

#[cfg(test)]
mod tests {
    #[test]
    fn only_exact_one_enables_key_content() {
        assert!(super::key_content_value_enabled(Some("1")));
        assert!(!super::key_content_value_enabled(Some("true")));
        assert!(!super::key_content_value_enabled(Some("0")));
        assert!(!super::key_content_value_enabled(None));
    }
}
