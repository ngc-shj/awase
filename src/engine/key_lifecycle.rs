//! キーの Down/Up ペア追跡
//!
//! Engine が KeyDown を Consume した場合、対応する KeyUp も必ず Consume すべき。
//! KeyDown を PassThrough した場合、対応する KeyUp も PassThrough すべき。
//! この不変条件を保証する。
//!
//! コンテキスト変更（フォーカス移動、IME OFF 等）時は、Consume 済みだが
//! KeyUp が来ていないキーの KeyUp を OS に再注入して状態を整合させる。

use crate::types::{RawKeyEvent, VkCode};

/// Consume 済みで KeyUp 待ちのキー
#[derive(Debug, Clone, Copy)]
struct ActiveKey {
    vk_code: VkCode,
    /// 再注入用の元イベントデータ
    event: RawKeyEvent,
}

/// KeyUp を KeyDown と同じ扱いにするための判定結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyUpDisposition {
    /// 対応する KeyDown が Consume 済み → KeyUp も Consume
    Consume,
    /// 対応する KeyDown を非活性中に PassThrough した → KeyUp も PassThrough
    PassThrough,
    /// 対応する KeyDown を追跡していない → 呼び出し側の通常処理へ委ねる
    Unknown,
}

/// キーの Down/Up ペア追跡
#[derive(Debug)]
pub struct KeyLifecycle {
    /// Consume 済みで KeyUp 待ちのキー一覧
    active_keys: Vec<ActiveKey>,
    /// Engine 非活性中に PassThrough した KeyDown の VK 一覧。
    ///
    /// 対応する KeyUp が「Engine が活性化した後」に届くと、KeyDown を一度も
    /// 処理していないキーを FSM が解釈して出力を出してしまう（macOS 実測:
    /// IME OFF 中に打った `k` の KeyUp が、直後の かな で活性化した engine に
    /// consume され出力が発生した）。モジュール doc が宣言している
    /// 「KeyDown を PassThrough したら KeyUp も PassThrough」を、この
    /// 活性化境界のケースに限って保証するために持つ。
    ///
    /// **素通しした押下の扱いは最初の KeyDown で決まり、KeyUp まで維持する。**
    /// auto-repeat の KeyDown は新しい押下ではないので disposition を変えない。
    /// 変えると「最初は生キー、リピートは変換」という混在が起き、さらに OS へ
    /// 渡した KeyDown に対応する KeyUp が渡らずキーが押しっぱなし扱いになる。
    ///
    /// **この維持は Pass → 活性化の向きだけ**で、逆向き（Consume 済みの押下が
    /// 非活性化をまたぐ）は意図的に維持しない。`Engine::check_active_transition`
    /// が active→inactive 遷移で `flush_pending_key_ups()` を呼び `active_keys` を
    /// 空にするため（「consumed した KeyDown の対応 KeyUp が inactive 時に到着しても
    /// consumed されないように」という既存の設計判断）。非対称なのは、
    /// Consume は OS に何も渡していないので途中で素通しへ切り替えても OS 側の
    /// キー状態が壊れないのに対し、Pass は既に KeyDown を渡してしまっており
    /// 対応する KeyUp を渡さないとキーが押しっぱなしになるため。
    passed_while_inactive: Vec<VkCode>,
}

impl Default for KeyLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyLifecycle {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            active_keys: Vec::new(),
            passed_while_inactive: Vec::new(),
        }
    }

    /// KeyDown が Consume された場合に呼ぶ。対応する KeyUp も Consume すべきことを記録。
    pub fn on_key_down_consumed(&mut self, event: &RawKeyEvent) {
        let vk = event.vk_code;
        // 既に PassThrough として追跡中の押下は上書きしない
        // （`passed_while_inactive` の doc「最初の disposition が勝つ」参照）
        if self.passed_while_inactive.contains(&vk) {
            return;
        }
        // 同じキーの重複登録を防ぐ（キーリピートの場合）
        if !self.active_keys.iter().any(|k| k.vk_code == vk) {
            self.active_keys.push(ActiveKey {
                vk_code: vk,
                event: *event,
            });
        }
    }

    /// Engine 非活性中に KeyDown を PassThrough した場合に呼ぶ。
    /// 対応する KeyUp も PassThrough すべきことを記録する
    /// （`passed_while_inactive` の doc 参照）。
    pub fn on_key_down_passed_while_inactive(&mut self, vk_code: VkCode) {
        // 二重登録の防御。Engine 経路では非活性化時に `flush_pending_key_ups()` が
        // 先に `active_keys` を空にするため到達しないが、両方に載ると `on_key_up` が
        // `active_keys` を先に見て PassThrough 側が残留するので、型で防げない以上
        // ここで弾く（逆向きの disposition 維持を意味するものではない —
        // `passed_while_inactive` の doc の非対称の説明を参照）
        if self.active_keys.iter().any(|k| k.vk_code == vk_code) {
            return;
        }
        if !self.passed_while_inactive.contains(&vk_code) {
            self.passed_while_inactive.push(vk_code);
        }
    }

    /// この VK が「非活性中に素通しした押下」として追跡中か。
    ///
    /// auto-repeat の KeyDown を FSM へ入れないために呼び出し側が使う
    /// （`passed_while_inactive` の doc 参照）。
    #[must_use]
    pub fn is_passed_while_inactive(&self, vk_code: VkCode) -> bool {
        self.passed_while_inactive.contains(&vk_code)
    }

    /// KeyUp が到着した場合に呼ぶ。
    pub fn on_key_up(&mut self, vk_code: VkCode) -> KeyUpDisposition {
        if let Some(pos) = self.active_keys.iter().position(|k| k.vk_code == vk_code) {
            self.active_keys.remove(pos);
            return KeyUpDisposition::Consume;
        }
        if let Some(pos) = self
            .passed_while_inactive
            .iter()
            .position(|vk| *vk == vk_code)
        {
            self.passed_while_inactive.remove(pos);
            return KeyUpDisposition::PassThrough;
        }
        KeyUpDisposition::Unknown
    }

    /// コンテキスト変更時: Consume 済みだが KeyUp が来ていないキーの KeyUp を
    /// 再注入用イベントとして返す。OS 側のキーボード状態と整合させる。
    ///
    /// 返されたイベントは `event_type` が `KeyUp` に書き換えられている。
    pub fn flush_pending_key_ups(&mut self) -> Vec<RawKeyEvent> {
        let keys = std::mem::take(&mut self.active_keys);
        keys.into_iter()
            .map(|k| {
                let mut evt = k.event;
                evt.event_type = crate::types::KeyEventType::KeyUp;
                evt
            })
            .collect()
    }

    /// アクティブキーの数
    #[must_use]
    pub const fn active_count(&self) -> usize {
        self.active_keys.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn make_event(vk: VkCode, event_type: KeyEventType) -> RawKeyEvent {
        RawKeyEvent {
            vk_code: vk,
            scan_code: ScanCode(0),
            event_type,
            extra_info: 0,
            timestamp: 0,
            key_classification: KeyClassification::Passthrough,
            physical_pos: None,
            ime_relevance: ImeRelevance::default(),
            modifier_key: None,
            modifier_snapshot: Default::default(),
            injected: false,
        }
    }

    #[test]
    fn consumed_key_down_makes_key_up_consumed() {
        let mut lc = KeyLifecycle::new();
        let event = make_event(VkCode(0x41), KeyEventType::KeyDown);

        lc.on_key_down_consumed(&event);
        assert_eq!(lc.active_count(), 1);

        assert_eq!(lc.on_key_up(VkCode(0x41)), KeyUpDisposition::Consume);
        assert_eq!(lc.active_count(), 0);
    }

    #[test]
    fn untracked_key_up_is_left_to_the_caller() {
        let mut lc = KeyLifecycle::new();
        assert_eq!(lc.on_key_up(VkCode(0x41)), KeyUpDisposition::Unknown);
    }

    /// Engine 非活性中に素通しした KeyDown の KeyUp は、活性化後に届いても
    /// PassThrough でなければならない。ここが `Unknown` に戻ると、KeyDown を
    /// 一度も処理していないキーを FSM が解釈して出力を出す
    /// （macOS 実測: IME OFF 中に打った `k` の KeyUp が、直後の かな で
    /// 活性化した engine に consume され出力が発生した）。
    #[test]
    fn key_down_passed_while_inactive_makes_key_up_pass_through() {
        let mut lc = KeyLifecycle::new();
        lc.on_key_down_passed_while_inactive(VkCode(0x41));
        assert_eq!(lc.on_key_up(VkCode(0x41)), KeyUpDisposition::PassThrough);
        // 一度きり。次の同じキーは通常処理へ戻す
        assert_eq!(lc.on_key_up(VkCode(0x41)), KeyUpDisposition::Unknown);
    }

    /// 押したまま Engine が活性化しても、その押下の扱いは最初の KeyDown のまま。
    /// auto-repeat は新しい押下ではないので、Consume へ切り替えてはならない。
    /// 切り替えると生キーと変換出力が混在し、OS へ渡した KeyDown に対応する
    /// KeyUp が渡らなくなる。
    #[test]
    fn a_press_keeps_its_pass_through_disposition_across_activation() {
        let mut lc = KeyLifecycle::new();
        lc.on_key_down_passed_while_inactive(VkCode(0x41));
        assert!(lc.is_passed_while_inactive(VkCode(0x41)));

        // 活性化後の auto-repeat が Consume として登録されようとしても拒む
        lc.on_key_down_consumed(&make_event(VkCode(0x41), KeyEventType::KeyDown));
        assert_eq!(lc.active_count(), 0, "a repeat must not become a new press");
        assert!(lc.is_passed_while_inactive(VkCode(0x41)));

        // KeyUp まで PassThrough を維持する
        assert_eq!(lc.on_key_up(VkCode(0x41)), KeyUpDisposition::PassThrough);
        assert!(!lc.is_passed_while_inactive(VkCode(0x41)));
    }

    #[test]
    fn passed_while_inactive_is_not_doubled_by_auto_repeat() {
        let mut lc = KeyLifecycle::new();
        lc.on_key_down_passed_while_inactive(VkCode(0x41));
        lc.on_key_down_passed_while_inactive(VkCode(0x41));
        assert_eq!(lc.on_key_up(VkCode(0x41)), KeyUpDisposition::PassThrough);
        assert_eq!(lc.on_key_up(VkCode(0x41)), KeyUpDisposition::Unknown);
    }

    #[test]
    fn flush_returns_pending_key_ups() {
        let mut lc = KeyLifecycle::new();
        lc.on_key_down_consumed(&make_event(VkCode(0x10), KeyEventType::KeyDown));
        lc.on_key_down_consumed(&make_event(VkCode(0x41), KeyEventType::KeyDown));

        let flushed = lc.flush_pending_key_ups();
        assert_eq!(flushed.len(), 2);
        assert!(flushed.iter().all(|e| e.event_type == KeyEventType::KeyUp));
        assert_eq!(lc.active_count(), 0);
    }

    #[test]
    fn duplicate_key_down_not_doubled() {
        let mut lc = KeyLifecycle::new();
        let event = make_event(VkCode(0x41), KeyEventType::KeyDown);
        lc.on_key_down_consumed(&event);
        lc.on_key_down_consumed(&event); // repeat
        assert_eq!(lc.active_count(), 1);
    }

    #[test]
    fn flush_pending_key_ups_sets_event_type_to_keyup() {
        let mut lc = KeyLifecycle::new();
        lc.on_key_down_consumed(&make_event(VkCode(0x41), KeyEventType::KeyDown));
        lc.on_key_down_consumed(&make_event(VkCode(0x42), KeyEventType::KeyDown));

        let flushed = lc.flush_pending_key_ups();
        for evt in &flushed {
            assert_eq!(evt.event_type, KeyEventType::KeyUp);
        }
        assert_eq!(flushed[0].vk_code, VkCode(0x41));
        assert_eq!(flushed[1].vk_code, VkCode(0x42));
    }

    #[test]
    fn on_key_up_for_never_consumed_is_unknown() {
        let mut lc = KeyLifecycle::new();
        // Consume key 0x41 but ask about 0x42
        lc.on_key_down_consumed(&make_event(VkCode(0x41), KeyEventType::KeyDown));
        assert_eq!(lc.on_key_up(VkCode(0x42)), KeyUpDisposition::Unknown);
        // 0x41 still active
        assert_eq!(lc.active_count(), 1);
    }

    #[test]
    fn multiple_keys_consumed_then_flushed() {
        let mut lc = KeyLifecycle::new();
        lc.on_key_down_consumed(&make_event(VkCode(0x10), KeyEventType::KeyDown)); // Shift
        lc.on_key_down_consumed(&make_event(VkCode(0x41), KeyEventType::KeyDown)); // A
        lc.on_key_down_consumed(&make_event(VkCode(0x42), KeyEventType::KeyDown)); // B
        assert_eq!(lc.active_count(), 3);

        let flushed = lc.flush_pending_key_ups();
        assert_eq!(flushed.len(), 3);
        assert_eq!(lc.active_count(), 0);
        // All flushed events should be KeyUp
        assert!(flushed.iter().all(|e| e.event_type == KeyEventType::KeyUp));
    }

    #[test]
    fn consume_keyup_consume_same_key_again() {
        let mut lc = KeyLifecycle::new();
        let event = make_event(VkCode(0x41), KeyEventType::KeyDown);

        // First cycle: consume then key_up
        lc.on_key_down_consumed(&event);
        assert_eq!(lc.active_count(), 1);
        assert_eq!(lc.on_key_up(VkCode(0x41)), KeyUpDisposition::Consume);
        assert_eq!(lc.active_count(), 0);

        // Second cycle: consume same key again
        lc.on_key_down_consumed(&event);
        assert_eq!(lc.active_count(), 1);
        assert_eq!(lc.on_key_up(VkCode(0x41)), KeyUpDisposition::Consume);
        assert_eq!(lc.active_count(), 0);
    }
}
