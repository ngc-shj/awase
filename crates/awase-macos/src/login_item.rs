//! ログイン項目の登録状態 (`SMAppService`)
//!
//! macOS 13 以降の `SMAppService.mainApp` で、アプリ自身がログイン項目の
//! 登録・解除を行う（旧 `LSSharedFileList` の置き換え）。`.app` バンドルとして
//! 起動している場合のみ有効で、開発時の裸バイナリ実行では機能しない。

/// ログイン項目の状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginItemState {
    /// 登録済み（ログイン時に起動する）
    Enabled,
    /// 未登録
    NotRegistered,
    /// ユーザーの承認待ち（システム設定でオフにされた等）
    RequiresApproval,
    /// 利用不可（バンドル外実行・API 失敗）
    Unavailable,
}

#[cfg(target_os = "macos")]
mod imp {
    #![allow(unsafe_code)] // SMAppService の FFI 呼び出しに必要

    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    use super::LoginItemState;

    /// 現在の登録状態を返す。
    #[must_use]
    pub fn state() -> LoginItemState {
        let service = unsafe { SMAppService::mainAppService() };
        match unsafe { service.status() } {
            SMAppServiceStatus::Enabled => LoginItemState::Enabled,
            SMAppServiceStatus::NotRegistered => LoginItemState::NotRegistered,
            SMAppServiceStatus::RequiresApproval => LoginItemState::RequiresApproval,
            _ => LoginItemState::Unavailable,
        }
    }

    /// ログイン項目の登録/解除を切り替える。成功したら新しい状態を返す。
    ///
    /// 失敗（未署名バンドル・バンドル外実行等）の場合は警告を出して
    /// 現在の状態をそのまま返す。
    #[must_use]
    pub fn toggle() -> LoginItemState {
        let service = unsafe { SMAppService::mainAppService() };
        let enabled = matches!(state(), LoginItemState::Enabled);
        let result = if enabled {
            unsafe { service.unregisterAndReturnError() }
        } else {
            unsafe { service.registerAndReturnError() }
        };
        match result {
            Ok(()) => {
                let new_state = state();
                log::info!(
                    "Login item {}: {new_state:?}",
                    if enabled {
                        "unregistered"
                    } else {
                        "registered"
                    }
                );
                new_state
            }
            Err(err) => {
                // 裸バイナリ実行や署名不備では登録できない（.app が必要）
                log::warn!("Login item toggle failed: {err}");
                state()
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::{state, toggle};

/// 非 macOS ビルド用スタブ。
#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn state() -> LoginItemState {
    LoginItemState::Unavailable
}

/// 非 macOS ビルド用スタブ。
#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn toggle() -> LoginItemState {
    LoginItemState::Unavailable
}
