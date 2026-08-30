//! macOS キー出力 (CGEventPost)

use awase::types::SpecialKey;

/// SpecialKey を macOS keycode に変換する
#[must_use]
pub const fn special_key_to_keycode(sk: SpecialKey) -> u16 {
    match sk {
        SpecialKey::Backspace => 0x33,
        SpecialKey::Escape => 0x35,
        SpecialKey::Enter => 0x24,
        SpecialKey::Space => 0x31,
        SpecialKey::Delete => 0x75, // Forward Delete
        SpecialKey::Insert => 0x72, // Help/Insert
        SpecialKey::Up => 0x7E,
        SpecialKey::Down => 0x7D,
        SpecialKey::Left => 0x7B,
        SpecialKey::Right => 0x7C,
        SpecialKey::Home => 0x73,
        SpecialKey::End => 0x77,
        SpecialKey::PageUp => 0x74,
        SpecialKey::PageDown => 0x79,
    }
}

/// ASCII 文字を macOS keycode に変換する
#[must_use]
pub const fn ascii_to_keycode(ch: char) -> Option<(u16, bool)> {
    match ch {
        'a'..='z' => {
            // macOS keycodes are NOT sequential like VK codes
            // Map each letter individually
            let keycode = match ch {
                'a' => 0x00,
                'b' => 0x0B,
                'c' => 0x08,
                'd' => 0x02,
                'e' => 0x0E,
                'f' => 0x03,
                'g' => 0x05,
                'h' => 0x04,
                'i' => 0x22,
                'j' => 0x26,
                'k' => 0x28,
                'l' => 0x25,
                'm' => 0x2E,
                'n' => 0x2D,
                'o' => 0x1F,
                'p' => 0x23,
                'q' => 0x0C,
                'r' => 0x0F,
                's' => 0x01,
                't' => 0x11,
                'u' => 0x20,
                'v' => 0x09,
                'w' => 0x0D,
                'x' => 0x07,
                'y' => 0x10,
                'z' => 0x06,
                _ => return None,
            };
            Some((keycode, false))
        }
        'A'..='Z' => {
            // Same keycode as lowercase, but with shift
            let lower = (ch as u8 + 32) as char;
            if let Some((kc, _)) = ascii_to_keycode(lower) {
                Some((kc, true))
            } else {
                None
            }
        }
        '0' => Some((0x1D, false)),
        '1' => Some((0x12, false)),
        '2' => Some((0x13, false)),
        '3' => Some((0x14, false)),
        '4' => Some((0x15, false)),
        '5' => Some((0x17, false)),
        '6' => Some((0x16, false)),
        '7' => Some((0x1A, false)),
        '8' => Some((0x1C, false)),
        '9' => Some((0x19, false)),
        '-' => Some((0x1B, false)),
        '.' => Some((0x2F, false)),
        ',' => Some((0x2B, false)),
        '/' => Some((0x2C, false)),
        // ── 記号（JIS 配列の物理位置。scanmap と同じく Jis 固定前提） ──
        '[' => Some((0x1E, false)), // kVK_ANSI_RightBracket (JIS: [)
        ']' => Some((0x2A, false)), // kVK_ANSI_Backslash (JIS: ])
        '{' => Some((0x1E, true)),
        '}' => Some((0x2A, true)),
        '(' => Some((0x1C, true)), // Shift+8
        ')' => Some((0x19, true)), // Shift+9
        '?' => Some((0x2C, true)), // Shift+/
        '!' => Some((0x12, true)), // Shift+1
        '^' => Some((0x18, false)), // kVK_ANSI_Equal (JIS: ^)
        '~' => Some((0x18, true)),  // Shift+^
        '@' => Some((0x21, false)), // kVK_ANSI_LeftBracket (JIS: @)
        ';' => Some((0x29, false)), // kVK_ANSI_Semicolon (JIS: ;)
        ':' => Some((0x27, false)), // kVK_ANSI_Quote (JIS: :)
        '_' => Some((0x5E, true)),  // Shift+ろ (kVK_JIS_Underscore = 0x5E)
        '"' => Some((0x13, true)),  // Shift+2 (JIS)
        '#' => Some((0x14, true)),  // Shift+3
        '$' => Some((0x15, true)),  // Shift+4
        '%' => Some((0x17, true)),  // Shift+5
        '&' => Some((0x16, true)),  // Shift+6
        '\'' => Some((0x1A, true)), // Shift+7 (JIS)
        '=' => Some((0x1B, true)),  // Shift+- (JIS)
        '+' => Some((0x29, true)),  // Shift+; (JIS)
        '*' => Some((0x27, true)),  // Shift+: (JIS)
        '<' => Some((0x2B, true)),  // Shift+,
        '>' => Some((0x2F, true)),  // Shift+.
        '|' => Some((0x5D, true)),  // Shift+¥ (kVK_JIS_Yen = 0x5D)
        '\\' => Some((0x5D, false)), // kVK_JIS_Yen（JIS では ¥、IME 経由で ￥/＼）
        _ => None,
    }
}

/// awase 自身が注入したイベントを tap 側で識別するためのマーカー。
/// `EVENT_SOURCE_USER_DATA` (kCGEventSourceUserData) に載せ、フック側は
/// この値を見て自分の注入イベントを Engine に通さず素通しする。
pub const INJECT_MARKER: i64 = 0x0A0A_5E00;

/// 出力方式（config.toml の `macos_output_style`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStyle {
    /// ローマ字キーストロークを注入（IME はローマ字入力モード）
    Romaji,
    /// JIS かな配列のキーストロークを注入（IME はかな入力モード）。
    /// 1 かな 1 打（濁点・半濁点は追い打ち）でイベント数が少なく、
    /// 高速打鍵時の注入取りこぼしに強い（Lacaille と同方式）
    Kana,
}

/// 濁点・半濁点を分解する（かな入力モード注入用。例: が → か + ゛）。
const fn split_voicing(ch: char) -> (char, Option<char>) {
    match ch {
        'が' => ('か', Some('゛')),
        'ぎ' => ('き', Some('゛')),
        'ぐ' => ('く', Some('゛')),
        'げ' => ('け', Some('゛')),
        'ご' => ('こ', Some('゛')),
        'ざ' => ('さ', Some('゛')),
        'じ' => ('し', Some('゛')),
        'ず' => ('す', Some('゛')),
        'ぜ' => ('せ', Some('゛')),
        'ぞ' => ('そ', Some('゛')),
        'だ' => ('た', Some('゛')),
        'ぢ' => ('ち', Some('゛')),
        'づ' => ('つ', Some('゛')),
        'で' => ('て', Some('゛')),
        'ど' => ('と', Some('゛')),
        'ば' => ('は', Some('゛')),
        'び' => ('ひ', Some('゛')),
        'ぶ' => ('ふ', Some('゛')),
        'べ' => ('へ', Some('゛')),
        'ぼ' => ('ほ', Some('゛')),
        'ゔ' => ('う', Some('゛')),
        'ぱ' => ('は', Some('゜')),
        'ぴ' => ('ひ', Some('゜')),
        'ぷ' => ('ふ', Some('゜')),
        'ぺ' => ('へ', Some('゜')),
        'ぽ' => ('ほ', Some('゜')),
        _ => (ch, None),
    }
}

/// JIS X 6002 かな配列: かな 1 文字 → (macOS keycode, Shift 要否)。
///
/// かな入力モードの IME はこのキーストロークを対応するかなとして受け取る。
#[must_use]
pub const fn jis_kana_keycode(ch: char) -> Option<(u16, bool)> {
    let stroke = match ch {
        // ── 数字段 ──
        'ぬ' => (0x12, false), // 1
        'ふ' => (0x13, false), // 2
        'あ' => (0x14, false), // 3
        'う' => (0x15, false), // 4
        'え' => (0x17, false), // 5
        'お' => (0x16, false), // 6
        'や' => (0x1A, false), // 7
        'ゆ' => (0x1C, false), // 8
        'よ' => (0x19, false), // 9
        'わ' => (0x1D, false), // 0
        'ほ' => (0x1B, false), // -
        'へ' => (0x18, false), // ^
        'ー' => (0x5D, false), // ¥ (kVK_JIS_Yen)
        'ぁ' => (0x14, true),
        'ぅ' => (0x15, true),
        'ぇ' => (0x17, true),
        'ぉ' => (0x16, true),
        'ゃ' => (0x1A, true),
        'ゅ' => (0x1C, true),
        'ょ' => (0x19, true),
        'を' => (0x1D, true),
        // ── Q 段 ──
        'た' => (0x0C, false), // Q
        'て' => (0x0D, false), // W
        'い' => (0x0E, false), // E
        'す' => (0x0F, false), // R
        'か' => (0x11, false), // T
        'ん' => (0x10, false), // Y
        'な' => (0x20, false), // U
        'に' => (0x22, false), // I
        'ら' => (0x1F, false), // O
        'せ' => (0x23, false), // P
        '゛' => (0x21, false), // @
        '゜' => (0x1E, false), // [
        'ぃ' => (0x0E, true),
        '「' => (0x1E, true),
        // ── A 段 ──
        'ち' => (0x00, false), // A
        'と' => (0x01, false), // S
        'し' => (0x02, false), // D
        'は' => (0x03, false), // F
        'き' => (0x05, false), // G
        'く' => (0x04, false), // H
        'ま' => (0x26, false), // J
        'の' => (0x28, false), // K
        'り' => (0x25, false), // L
        'れ' => (0x29, false), // ;
        'け' => (0x27, false), // :
        'む' => (0x2A, false), // ]
        '」' => (0x2A, true),
        // ── Z 段 ──
        'つ' => (0x06, false), // Z
        'さ' => (0x07, false), // X
        'そ' => (0x08, false), // C
        'ひ' => (0x09, false), // V
        'こ' => (0x0B, false), // B
        'み' => (0x2D, false), // N
        'も' => (0x2E, false), // M
        'ね' => (0x2B, false), // ,
        'る' => (0x2F, false), // .
        'め' => (0x2C, false), // /
        'ろ' => (0x5E, false), // ろ (kVK_JIS_Underscore)
        'っ' => (0x06, true),
        '、' => (0x2B, true),
        '。' => (0x2F, true),
        '・' => (0x2C, true),
        _ => return None,
    };
    Some(stroke)
}

#[cfg(target_os = "macos")]
mod imp {
    use awase::kana_table::KanaTable;
    use awase::types::{KeyAction, KeyEventType, SpecialKey, VkCode};
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, EventField};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use log::warn;

    use super::{
        ascii_to_keycode, jis_kana_keycode, special_key_to_keycode, split_voicing, OutputStyle,
        INJECT_MARKER,
    };

    /// kVK_Shift（Romaji/KeySequence の大文字送出用）
    const KEYCODE_SHIFT: u16 = 0x38;
    /// kVK_Option（記号の変換回避ストローク用）
    const KEYCODE_OPTION: u16 = 0x3A;

    /// かな記号（CJK 記号ブロック）→ IME が同等文字に変換するキーストローク。
    ///
    /// `KanaTable` はかな文字のみを収録するため、.yab の Literal 由来で
    /// `Char` に載ってくる記号類はここで補う。'・' は IME の記号変換（/→・）に
    /// 任せる。'／'（全角 ASCII 側）は意図的にキーストローク化しない — "/" だと
    /// IME 設定次第で ・ に化けるため、正確な ／ が必要な親指シフト面（親指+2）は
    /// 直接注入で出す（変換中のみ composing フォールバックで "/" になる）。
    const fn kana_symbol_to_ascii(ch: char) -> Option<&'static str> {
        match ch {
            'ー' => Some("-"),
            '、' => Some(","),
            '。' => Some("."),
            '・' => Some("/"),
            '「' => Some("["),
            '」' => Some("]"),
            // ￥ は U+FFE5 で全角 ASCII 範囲外のため個別に対応（JIS ¥ キー）
            '￥' => Some("\\"),
            _ => None,
        }
    }

    /// IME の記号変換を回避して正確な文字を得るキーストローク。
    ///
    /// 日本語 IME は `/` を ・ に、`[` `]` を 「 」 に変換してしまうが、
    /// **Option（⌥）修飾を付けると変換されず記号そのものが入力される**。
    /// Unicode 直接注入と違い未確定文字列の中でも失われない（macOS の
    /// 親指シフトエミュレータ Lacaille が同じ手法を採っている）。
    ///
    /// 戻り値は (keycode, shift 要否)。Option は常に付ける。
    pub(super) const fn ime_literal_keystroke(ch: char) -> Option<(u16, bool)> {
        match ch {
            '／' => Some((0x2C, false)), // ⌥/
            '［' => Some((0x1E, false)), // ⌥[ (JIS)
            '］' => Some((0x2A, false)), // ⌥] (JIS)
            _ => None,
        }
    }

    /// 全角 ASCII（U+FF01〜U+FF5E）を対応する半角文字に変換する。
    ///
    /// .yab の数字段・親指シフト面・小指シフト面のリテラル（１２…、
    /// Ａ-Ｚ、？（）等）をキーストロークで IME に渡すための写像。全角/半角の
    /// 最終形は IME の英字・記号設定に委ねる（Windows VK モードと同じ方針）。
    const fn fullwidth_to_ascii(ch: char) -> Option<char> {
        let code = ch as u32;
        if 0xFF01 <= code && code <= 0xFF5E {
            char::from_u32(code - 0xFEE0)
        } else {
            None
        }
    }

    pub struct Output {
        source: CGEventSource,
        /// 出力方式（romaji: ローマ字逆引き / kana: JIS かなストローク）
        style: OutputStyle,
        /// 出力文字 → IME に送るローマ字入力列（config の
        /// `[macos_symbol_romaji]`）。IME のローマ字テーブルに登録した独自の
        /// 入力列を使い、変換を経ずに正確な記号を未確定文字列へ入れる。
        /// 直接注入と違い変換中でも飲まれない
        symbol_romaji: std::collections::HashMap<char, String>,
        /// `Char(かな)` をローマ字キーストロークへ逆引きするためのテーブル
        /// （Windows 版 VK モードの `send_char_as_vk` と同じ方針。macOS では
        /// Unicode 直接注入だと IME が未確定文字列を持たず漢字変換不能になる）。
        kana: KanaTable,
        /// 「IME が変換中（未確定文字列あり）らしい」ヒューリスティック。
        ///
        /// macOS には composition 状態を外から観測する公開 API がないため、
        /// 自分がローマ字キーストロークを注入したら true、確定・取消キー
        /// （Enter/Escape/Tab）の通過や IME OFF の観測で false にする。
        /// 変換中の Unicode 直接注入は IME に飲まれて消えるため、その間だけ
        /// キーストロークにフォールバックする判定に使う。
        composing_hint: bool,
    }

    impl std::fmt::Debug for Output {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Output").finish_non_exhaustive()
        }
    }

    /// `Spec` から CGEvent を構築する（失敗時は警告して None）。
    impl Output {
        /// 注入ワーカースレッドを起動する。
        ///
        /// # Errors
        ///
        /// ワーカースレッドの spawn に失敗した場合。
        pub fn new(
            style: OutputStyle,
            symbol_romaji: std::collections::HashMap<char, String>,
        ) -> anyhow::Result<Self> {
            let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|()| anyhow::anyhow!("Failed to create CGEventSource"))?;
            log::info!(
                "Output style: {style:?}, symbol_romaji entries: {}",
                symbol_romaji.len()
            );
            Ok(Self {
                source,
                style,
                symbol_romaji,
                kana: KanaTable::build(),
                composing_hint: false,
            })
        }

        /// かな 1 文字を JIS かな配列のキーストロークとして送出する。
        ///
        /// 濁点・半濁点は基底かな + ゛/゜ の追い打ち。対応表に無い文字なら
        /// false を返す（呼び出し側が直接注入にフォールバック）。
        fn send_kana_strokes(&mut self, ch: char) -> bool {
            let (base, mark) = split_voicing(ch);
            let Some((keycode, shift)) = jis_kana_keycode(base) else {
                return false;
            };
            self.press_with_shift(keycode, shift);
            if let Some(mark) = mark {
                if let Some((mark_code, mark_shift)) = jis_kana_keycode(mark) {
                    self.press_with_shift(mark_code, mark_shift);
                }
            }
            log::debug!("Kana: injected '{ch}'");
            self.composing_hint = true;
            true
        }

        /// Shift ラッパー付きでキーを押して離す。
        fn press_with_shift(&self, keycode: u16, shift: bool) {
            if shift {
                self.post_key(KEYCODE_SHIFT, true, false);
            }
            self.post_press_release(keycode, shift);
            if shift {
                self.post_key(KEYCODE_SHIFT, false, false);
            }
        }

        /// 確定・取消相当の操作を観測したときに App 側から呼ばれる。
        pub const fn note_composition_break(&mut self) {
            self.composing_hint = false;
        }

        /// 単一のキーイベントを post する（tap コールバック内で同期実行）。
        fn post_key(&self, keycode: u16, down: bool, shift: bool) {
            self.post_key_with_flags(keycode, down, shift, false);
        }

        /// 修飾フラグ付きでキーイベントを post する。
        fn post_key_with_flags(&self, keycode: u16, down: bool, shift: bool, option: bool) {
            let Ok(event) = CGEvent::new_keyboard_event(self.source.clone(), keycode, down)
            else {
                warn!("Failed to create keyboard event (keycode=0x{keycode:02X})");
                return;
            };
            let mut flags = CGEventFlags::empty();
            if shift {
                flags |= CGEventFlags::CGEventFlagShift;
            }
            if option {
                flags |= CGEventFlags::CGEventFlagAlternate;
            }
            if !flags.is_empty() {
                event.set_flags(flags);
            }
            event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, INJECT_MARKER);
            event.post(CGEventTapLocation::HID);
        }

        /// キーを押して離す。
        fn post_press_release(&self, keycode: u16, shift: bool) {
            self.post_key(keycode, true, shift);
            self.post_key(keycode, false, shift);
        }

        /// Unicode 文字を CGEvent の文字列ペイロードで直接注入する。
        ///
        /// キーコードに依存しないため任意のかな文字を出力できるが、
        /// IME のかな漢字変換は経由しない（Unicode 直接注入モード用）。
        fn post_char(&self, ch: char) {
            let Ok(event) = CGEvent::new_keyboard_event(self.source.clone(), 0, true) else {
                warn!("Failed to create keyboard event for Char('{ch}')");
                return;
            };
            event.set_string(&ch.to_string());
            event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, INJECT_MARKER);
            event.post(CGEventTapLocation::HID);
            // 対応する KeyUp（文字列ペイロードなし）
            if let Ok(up) = CGEvent::new_keyboard_event(self.source.clone(), 0, false) {
                up.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, INJECT_MARKER);
                up.post(CGEventTapLocation::HID);
            }
        }

        /// ASCII 文字列をキーストロークとして送出する（IME に変換させる用途）。
        ///
        /// IME に食わせるキーストロークは composition を開始しうるため
        /// `composing_hint` を立てる。
        fn send_ascii_sequence(&mut self, s: &str, kind: &str) {
            for ch in s.chars() {
                if let Some((keycode, needs_shift)) = ascii_to_keycode(ch) {
                    if needs_shift {
                        self.post_key(KEYCODE_SHIFT, true, false);
                    }
                    self.post_press_release(keycode, needs_shift);
                    if needs_shift {
                        self.post_key(KEYCODE_SHIFT, false, false);
                    }
                } else {
                    warn!("{kind} char '{ch}' has no macOS keycode mapping, skipping");
                }
            }
            // 取りこぼし調査用（RUST_LOG=debug で注入内容を追跡できるように）
            log::debug!("{kind}: injected \"{s}\"");
            self.composing_hint = true;
        }

        /// `Char` アクション 1 文字を最適な経路で送出する。
        ///
        /// 1. かな → ローマ字キーストローク（IME が変換）
        /// 2. かな記号（。、ー「」・）→ 対応キーストローク
        /// 3. 全角 ASCII → 半角キーストローク（全角/半角は IME 設定に従う）。
        ///    ただし '／' は除外 — "/" は IME 設定で ・ に化けるため、正確な
        ///    ／ が必要（親指+2）。変換中でなければ直接注入で出す
        /// 4. 変換中（composing_hint）は直接注入が IME に飲まれるため、
        ///    キーストローク表現があればそちらへフォールバック
        /// 5. それ以外は Unicode 直接注入
        fn send_char(&mut self, ch: char) {
            // ユーザーが IME 側に登録した入力列があれば最優先で使う。
            // 変換中でも未確定文字列に正確な文字が入る唯一の経路
            if let Some(seq) = self.symbol_romaji.get(&ch) {
                let seq = seq.clone();
                self.send_ascii_sequence(&seq, "SymbolRomaji");
                return;
            }
            if self.style == OutputStyle::Kana {
                if self.send_kana_strokes(ch) {
                    return;
                }
                // かなストロークで表現できない文字（数字段・小指シフト面の
                // 英数記号）は直接注入する。かな入力モードでは ASCII
                // キーストロークがかなに化けるため、ローマ字用のフォールバックは
                // 使えない。全角リテラル（Ａ１？等）は半角にしてから注入する
                // ——「Ｓｈｉｆｔ＋キーが全角で確定する」のを避けるため
                self.post_char(fullwidth_to_ascii(ch).unwrap_or(ch));
                return;
            }
            if let Some(romaji) = self.kana.romaji_for_kana(ch) {
                let romaji = romaji.to_owned();
                self.send_ascii_sequence(&romaji, "Char");
                return;
            }
            if let Some(ascii) = kana_symbol_to_ascii(ch) {
                self.send_ascii_sequence(ascii, "Char");
                return;
            }
            // IME が変換してしまう記号は Option 修飾ストロークで正確に出す。
            // 修飾キーは実キーの押下/解放も送る（フラグを立てるだけだと
            // 修飾が押しっぱなしと解釈され、以降の注入が ⌥+key となって
            // IME に無視される。Shift 側と同じ扱いに揃える）
            if let Some((keycode, shift)) = ime_literal_keystroke(ch) {
                self.post_key(KEYCODE_OPTION, true, false);
                if shift {
                    self.post_key(KEYCODE_SHIFT, true, false);
                }
                self.post_key_with_flags(keycode, true, shift, true);
                self.post_key_with_flags(keycode, false, shift, true);
                if shift {
                    self.post_key(KEYCODE_SHIFT, false, false);
                }
                self.post_key(KEYCODE_OPTION, false, false);
                log::debug!("Char: injected '{ch}' via option-modified keystroke");
                self.composing_hint = true;
                return;
            }
            if let Some(ascii) = fullwidth_to_ascii(ch).filter(|c| ascii_to_keycode(*c).is_some()) {
                self.send_ascii_sequence(&ascii.to_string(), "Char");
                return;
            }
            self.post_char(ch);
        }

        /// `KeyAction` のリストを順に post する。
        pub fn send_keys(&mut self, actions: &[KeyAction]) {
            for action in actions {
                match action {
                    KeyAction::SpecialKey(sk) => {
                        log::debug!("SpecialKey: injected {sk:?}");
                        self.post_press_release(special_key_to_keycode(*sk), false);
                        // Enter/Escape は composition を確定・破棄する
                        if matches!(sk, SpecialKey::Enter | SpecialKey::Escape) {
                            self.composing_hint = false;
                        }
                    }
                    KeyAction::Key(vk) => self.post_key(vk.0, true, false),
                    KeyAction::KeyUp(vk) => self.post_key(vk.0, false, false),
                    KeyAction::Char(ch) => self.send_char(*ch),
                    KeyAction::Romaji(s) => {
                        // kana 方式では、ローマ字文字列もかなへ解決してストローク化する
                        // （kana 未解決の .yab エントリが ASCII 注入でかなに化けるのを防ぐ）
                        if self.style == OutputStyle::Kana {
                            if let Some(kana) = self.kana.kana_for_romaji(s) {
                                if self.send_kana_strokes(kana) {
                                    continue;
                                }
                            }
                            warn!("Romaji \"{s}\" has no kana-stroke mapping, falling back");
                        }
                        self.send_ascii_sequence(s, "Romaji");
                    }
                    KeyAction::KeySequence(s) => self.send_ascii_sequence(s, "KeySequence"),
                    KeyAction::Suppress => {}
                }
            }
        }

        /// 握りつぶしたキーを合成イベントとして再注入する
        /// （親指キー単独打鍵の英数/かな送出等）。
        pub fn reinject(&mut self, vk: VkCode, event_type: KeyEventType) {
            let down = matches!(event_type, KeyEventType::KeyDown);
            self.post_key(vk.0, down, false);
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::Output;

/// テストから `imp` 内のマッピング関数を参照するための橋渡し。
#[cfg(all(test, target_os = "macos"))]
mod imp_test {
    pub(super) use super::imp::ime_literal_keystroke as literal;
}

/// 非 macOS ビルド用スタブ（ワークスペース全体のクロスチェック用）。
#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub struct Output;

#[cfg(not(target_os = "macos"))]
impl Output {
    /// スタブ生成。
    ///
    /// # Errors
    ///
    /// スタブのため常に成功する。
    pub fn new(
        _style: OutputStyle,
        _symbol_romaji: std::collections::HashMap<char, String>,
    ) -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub fn send_keys(&mut self, actions: &[awase::types::KeyAction]) {
        for action in actions {
            log::trace!("macOS output stub: {action:?}");
        }
    }

    pub fn reinject(&mut self, vk: awase::types::VkCode, event_type: awase::types::KeyEventType) {
        log::trace!("macOS output stub: reinject 0x{:02X} {event_type:?}", vk.0);
    }

    pub fn note_composition_break(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_to_keycode_covers_yab_literal_symbols() {
        // 親指シフト面の数字段リテラル（？／～「」［］（）｛｝）が全て
        // キーストロークに落とせること（JIS 配列前提）
        for ch in ['?', '/', '~', '[', ']', '(', ')', '{', '}'] {
            assert!(ascii_to_keycode(ch).is_some(), "missing keycode for {ch:?}");
        }
    }

    #[test]
    fn ascii_to_keycode_jis_bracket_positions() {
        assert_eq!(ascii_to_keycode('['), Some((0x1E, false))); // JIS: [
        assert_eq!(ascii_to_keycode(']'), Some((0x2A, false))); // JIS: ]
        assert_eq!(ascii_to_keycode('?'), Some((0x2C, true))); // Shift+/
    }

    #[test]
    fn jis_kana_strokes_cover_all_engine_kana() {
        // NICOLA レイアウトが出力しうる全かな・記号が、かな入力モードの
        // ストローク（基底かな + 濁点/半濁点）で表現できること
        let all = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほ\
                   まみむめもやゆよらりるれろわをんぁぃぅぇぉゃゅょっ\
                   がぎぐげござじずぜぞだぢづでどばびぶべぼぱぴぷぺぽゔ\
                   ー、。・「」゛゜";
        for ch in all.chars() {
            let (base, _mark) = split_voicing(ch);
            assert!(
                jis_kana_keycode(base).is_some(),
                "missing JIS kana stroke for {ch:?} (base {base:?})"
            );
        }
    }

    #[test]
    fn ime_literal_symbols_use_jis_positions() {
        use imp_test::literal;
        // Lacaille と同じ ⌥ 付きストローク（[ ] は JIS 位置）
        assert_eq!(literal('／'), Some((0x2C, false)));
        assert_eq!(literal('［'), Some((0x1E, false)));
        assert_eq!(literal('］'), Some((0x2A, false)));
        assert_eq!(literal('あ'), None);
    }

    #[test]
    fn jis_yen_and_underscore_keycodes_match_carbon() {
        // kVK_JIS_Yen = 0x5D / kVK_JIS_Underscore = 0x5E（Events.h）。
        // 実装当初これが逆で、長音記号（ー）が出力されなかった
        assert_eq!(jis_kana_keycode('ー'), Some((0x5D, false)));
        assert_eq!(jis_kana_keycode('ろ'), Some((0x5E, false)));
        assert_eq!(ascii_to_keycode('\\'), Some((0x5D, false)));
        assert_eq!(ascii_to_keycode('_'), Some((0x5E, true)));
    }

    #[test]
    fn jis_kana_keycode_spot_checks() {
        assert_eq!(jis_kana_keycode('ね'), Some((0x2B, false))); // , キー
        assert_eq!(jis_kana_keycode('く'), Some((0x04, false))); // H キー
        assert_eq!(jis_kana_keycode('っ'), Some((0x06, true))); // Shift+Z
        assert_eq!(jis_kana_keycode('を'), Some((0x1D, true))); // Shift+0
        assert_eq!(split_voicing('が'), ('か', Some('゛')));
        assert_eq!(split_voicing('ぱ'), ('は', Some('゜')));
        assert_eq!(split_voicing('ね'), ('ね', None));
    }
}
