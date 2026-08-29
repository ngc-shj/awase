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
        '_' => Some((0x5D, true)),  // Shift+ろ (kVK_JIS_Underscore)
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
        '|' => Some((0x5E, true)),  // Shift+¥ (JIS)
        '\\' => Some((0x5E, false)), // kVK_JIS_Yen（JIS では ¥、IME 経由で ￥/＼）
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
        'ー' => (0x5E, false), // ¥
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
        'ろ' => (0x5D, false), // ろ
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
            _ => None,
        }
    }

    /// IME のキーストローク変換がリテラルと**別の文字**になる全角記号。
    ///
    /// 例: `/` は ・ に、`[` `]` は 「 」 に変換される（ATOK/MS-IME 既定）。
    /// これらは非変換中なら Unicode 直接注入で正確に出し、変換中
    /// （直接注入が IME に飲まれる）のみキーストロークにフォールバックする。
    const fn ime_renders_differently(ch: char) -> bool {
        matches!(ch, '／' | '［' | '］')
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

    /// 注入ワーカーへ渡すイベント仕様。
    ///
    /// CGEvent の構築と post はワーカースレッドが行う（`CGEventSource` は
    /// ワーカーが専有）。
    enum Spec {
        Key {
            keycode: u16,
            down: bool,
            shift: bool,
        },
        /// Unicode 直接注入（keycode 0 + 文字列ペイロードの KeyDown）
        CharDown(char),
        /// `CharDown` に対応する KeyUp
        CharUp,
    }

    /// 注入イベント間の待機。
    ///
    /// 間隔ゼロで連射すると**バースト先頭のキーストロークが失われる**
    /// （`ku`→k・`ne`→n・`to`→t・`lyo`→l の欠落を実測。Session 化・実タイム
    /// スタンプ・suppression 無効化でも解消せず）。CGEventPost 系ツールの
    /// 既知の弱点で、espanso 等と同様にペーシングで回避する。
    const INJECT_GAP: std::time::Duration = std::time::Duration::from_millis(2);

    /// CGEventPost によるキー出力。
    ///
    /// 注入イベントはすべて `INJECT_MARKER` 付きで、専用ワーカースレッドから
    /// `INJECT_GAP` の間隔を空けて Session タップ位置に post する（tap
    /// コールバックをブロックせずにペーシングするため）。キーストロークは
    /// IME を含む通常の入力パイプラインを通る。
    pub struct Output {
        tx: std::sync::mpsc::Sender<Spec>,
        /// 出力方式（romaji: ローマ字逆引き / kana: JIS かなストローク）
        style: OutputStyle,
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
    fn build_event(source: &CGEventSource, spec: &Spec) -> Option<CGEvent> {
        match *spec {
            Spec::Key {
                keycode,
                down,
                shift,
            } => {
                let event = CGEvent::new_keyboard_event(source.clone(), keycode, down)
                    .map_err(|()| {
                        warn!("Failed to create keyboard event (keycode=0x{keycode:02X})");
                    })
                    .ok()?;
                if shift {
                    event.set_flags(CGEventFlags::CGEventFlagShift);
                }
                Some(event)
            }
            Spec::CharDown(ch) => {
                let event = CGEvent::new_keyboard_event(source.clone(), 0, true)
                    .map_err(|()| warn!("Failed to create keyboard event for Char('{ch}')"))
                    .ok()?;
                event.set_string(&ch.to_string());
                Some(event)
            }
            Spec::CharUp => CGEvent::new_keyboard_event(source.clone(), 0, false).ok(),
        }
    }

    /// ワーカースレッド: `Spec` を受け取り、ペーシングしながら post する。
    fn injection_worker(rx: &std::sync::mpsc::Receiver<Spec>) {
        let Ok(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else {
            log::error!("injection worker: failed to create CGEventSource");
            return;
        };
        // 合成イベント送出後、同ソース外のイベントを短時間抑制するレガシー動作
        // （既定 ~250ms）を無効化する
        #[allow(unsafe_code)] // CGEventSource の未バインド API 呼び出し
        unsafe {
            use foreign_types::ForeignType;
            extern "C" {
                fn CGEventSourceSetLocalEventsSuppressionInterval(
                    source: *mut core_graphics::sys::CGEventSource,
                    seconds: f64,
                );
            }
            CGEventSourceSetLocalEventsSuppressionInterval(source.as_ptr(), 0.0);
        }

        while let Ok(spec) = rx.recv() {
            let Some(event) = build_event(&source, &spec) else {
                continue;
            };
            event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, INJECT_MARKER);
            // timestamp=0 のまま送ると時刻整合処理で捨てられる余地があるため
            // 実時刻を付与する。post 先は物理入力の合流点を避けて Session 層
            #[allow(unsafe_code)] // CGEvent の未バインド API 呼び出しに必要
            unsafe {
                use foreign_types::ForeignType;
                extern "C" {
                    fn CGEventSetTimestamp(
                        event: *mut core_graphics::sys::CGEvent,
                        timestamp: u64,
                    );
                    fn mach_absolute_time() -> u64;
                }
                CGEventSetTimestamp(event.as_ptr(), mach_absolute_time());
            }
            event.post(CGEventTapLocation::Session);
            std::thread::sleep(INJECT_GAP);
        }
    }

    impl Output {
        /// 注入ワーカースレッドを起動する。
        ///
        /// # Errors
        ///
        /// ワーカースレッドの spawn に失敗した場合。
        pub fn new(style: OutputStyle) -> anyhow::Result<Self> {
            let (tx, rx) = std::sync::mpsc::channel::<Spec>();
            std::thread::Builder::new()
                .name("awase-inject".to_string())
                .spawn(move || injection_worker(&rx))
                .map_err(|e| anyhow::anyhow!("Failed to spawn injection worker: {e}"))?;
            log::info!("Output style: {style:?}");
            Ok(Self {
                tx,
                style,
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

        /// 単一のキーイベントをワーカー経由で post する。
        fn post_key(&self, keycode: u16, down: bool, shift: bool) {
            let _ = self.tx.send(Spec::Key {
                keycode,
                down,
                shift,
            });
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
            let _ = self.tx.send(Spec::CharDown(ch));
            let _ = self.tx.send(Spec::CharUp);
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
            if self.style == OutputStyle::Kana {
                if self.send_kana_strokes(ch) {
                    return;
                }
                // かなストロークで表現できない文字（数字・英字・記号）は直接注入。
                // かな入力モードでは ASCII キーストロークがかなに化けるため
                // ローマ字用のキーストロークフォールバックは使えない
                self.post_char(ch);
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
            let keystroke = fullwidth_to_ascii(ch).filter(|c| ascii_to_keycode(*c).is_some());
            if let Some(ascii) = keystroke {
                if !ime_renders_differently(ch) || self.composing_hint {
                    self.send_ascii_sequence(&ascii.to_string(), "Char");
                    return;
                }
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
    pub fn new(_style: OutputStyle) -> anyhow::Result<Self> {
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
