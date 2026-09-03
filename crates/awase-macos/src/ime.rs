//! macOS IME 検出 (TISCopyCurrentKeyboardInputSource)
//!
//! `kTISPropertyInputSourceID` から現在の入力ソース ID を取得し、
//! 日本語 IME のかな入力モードかどうかを判定する。
//!
//! 代表的な InputSourceID:
//! - `com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese` — 日本語IM ひらがな
//! - `com.apple.inputmethod.Kotoeri.RomajiTyping.Roman` — 日本語IM 英字
//! - `com.google.inputmethod.Japanese.base` — Google 日本語入力 ひらがな
//! - `com.google.inputmethod.Japanese.Roman` — Google 日本語入力 英数
//! - `com.apple.keylayout.ABC` — 英語キーボードレイアウト（IME なし）

#[cfg(target_os = "macos")]
mod imp {
    // TIS (Text Input Sources) API の呼び出しに必要
    #![allow(unsafe_code)]

    use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
    use core_foundation::base::{CFRelease, TCFType};
    use core_foundation::dictionary::CFDictionaryRef;
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;
    use std::ptr;
    use std::time::{Duration, Instant};

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> *mut c_void;
        fn TISGetInputSourceProperty(source: *mut c_void, key: CFStringRef) -> *mut c_void;
        fn TISCreateInputSourceList(
            properties: CFDictionaryRef,
            include_all_installed: bool,
        ) -> CFArrayRef;
        fn TISSelectInputSource(source: *mut c_void) -> i32;
        static kTISPropertyInputSourceID: CFStringRef;
    }

    /// input source ポインタから InputSourceID を取り出す（Get ルール）。
    unsafe fn input_source_id(source: *mut c_void) -> Option<String> {
        let id_ref = TISGetInputSourceProperty(source, kTISPropertyInputSourceID);
        if id_ref.is_null() {
            None
        } else {
            Some(CFString::wrap_under_get_rule(id_ref.cast()).to_string())
        }
    }

    /// 現在の入力ソース ID を取得する。
    fn current_input_source_id() -> Option<String> {
        unsafe {
            let source = TISCopyCurrentKeyboardInputSource();
            if source.is_null() {
                return None;
            }
            let id = input_source_id(source);
            // Copy ルール: input source は呼び出し側が解放する
            CFRelease(source.cast_const());
            id
        }
    }

    /// ID が日本語 IM のいずれかのモードかどうか。
    ///
    /// ATOK の英字モード ID は "Japanese" を含まない可能性があるため、
    /// 既知ベンダー（justsystems/atok）も判定に含める。
    fn is_japanese_im(id: &str) -> bool {
        id.contains("inputmethod")
            && (id.contains("Japanese") || id.contains("Kotoeri") || id.contains("atok"))
    }

    /// ID がひらがな系の日本語 IME モードかどうか（ON 側の選択対象）。
    fn is_japanese_kana_mode(id: &str) -> bool {
        is_japanese_im(id)
            && !id.contains("Roman")
            && !id.contains("Katakana")
            && !id.contains("Eiji")
    }

    /// 入力ソース ID から IME 状態を判定する（TIS 観測から切り離した純粋部分）。
    ///
    /// - `Some(true)`: IME ON（ひらがな・カタカナモード等）
    /// - `Some(false)`: IME OFF（日本語 IM の英字モード、または IME なしレイアウト）
    /// - `None`: 判定不可（日本語 IM でもキーボードレイアウトでもない）
    fn classify_input_source(id: &str) -> Option<bool> {
        if is_japanese_im(id) {
            // "…Japanese" / "…Japanese.Katakana" は ON、
            // "…Roman" / "…FullWidthRoman" / "…HalfWidthEiji"（英字モード）は OFF
            Some(!(id.contains("Roman") || id.contains("Eiji")))
        } else if id.contains("keylayout") {
            Some(false)
        } else {
            None
        }
    }

    /// ID の末尾セグメントを除いた IM ファミリ prefix を返す。
    ///
    /// かな/英字などのモード違いで同じ値になる。例:
    ///   com.justsystems.inputmethod.atok34.Japanese → …atok34
    ///   com.justsystems.inputmethod.atok34.Roman    → …atok34
    ///   com.google.inputmethod.Japanese.base        → ….Japanese
    fn im_family_prefix(id: &str) -> Option<&str> {
        id.rsplit_once('.').map(|(prefix, _)| prefix)
    }

    /// 候補 ID が同じ IM ファミリに属するか（セグメント境界厳密）。
    ///
    /// 素の `starts_with` だと `…Japanese` に `…JapaneseEvil.…` のような
    /// 兄弟 ID まで一致してしまう（2026-08-29 セキュリティレビュー第3回指摘3）。
    fn in_family(candidate: &str, prefix: &str) -> bool {
        candidate == prefix
            || candidate
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('.'))
    }

    /// 有効な入力ソースから述語に合う最初のものを選択する。
    fn select_input_source_matching(pred: impl Fn(&str) -> bool) -> bool {
        unsafe {
            let list = TISCreateInputSourceList(ptr::null(), false);
            if list.is_null() {
                return false;
            }
            let mut selected = false;
            let count = CFArrayGetCount(list.cast());
            for i in 0..count {
                let source = CFArrayGetValueAtIndex(list.cast(), i).cast_mut();
                if source.is_null() {
                    continue;
                }
                if input_source_id(source).is_some_and(|id| pred(&id)) {
                    selected = TISSelectInputSource(source) == 0;
                    if selected {
                        break;
                    }
                }
            }
            CFRelease(list.cast());
            selected
        }
    }

    /// IME 切替キー送出後、TIS 観測が追いつくまで期待値を優先する猶予時間。
    /// これを過ぎても観測が一致しなければ「切替は失敗した」と見なす。
    ///
    /// 入力ソースの切替は非同期で数十〜数百 ms かかるため、英数/かな 直後の
    /// 打鍵は観測ベースだと旧状態で判定されて素通り/誤変換する
    /// （英字モード→かな→即 k で「き」でなく k が出る問題）。
    ///
    /// 実測（2026-09-02、ATOK atok36、英数⇄かな 往復 102 回、BUG-101）:
    /// 「かな」キー送出から TIS が新入力ソースを報告するまで p50 ~60ms /
    /// p90 ~160ms / **max 208ms**（2 回の計測で max 191ms → 208ms と伸びた）。
    /// OFF 側は max 122ms。実測最大 208ms + マージン 92ms = 300ms とする。
    ///
    /// これは失敗検出の期限でもあるため、長すぎると害がある: 物理「かな」キー
    /// を ATOK が取りこぼした場合、この時間だけ出力が止まってから張り直しが
    /// 走る（500ms 版では体感 0.5 秒の停止、250ms 版では実測 328ms）。
    /// 逆に短すぎると、正当に遅いだけの切替を失敗と誤判定して、その間の打鍵が
    /// 生キーで素通しされる。250ms でも実測上の誤爆は無かったが、max との
    /// マージンが 42ms しか無かったため 300ms を採る。
    const EXPECTATION_GRACE: Duration = Duration::from_millis(300);

    /// TIS が切替済みを報告してから、注入を再開してよいと見なすまでの猶予。
    ///
    /// `TISCopyCurrentKeyboardInputSource` はプロセス横断のグローバル状態で、
    /// フォアグラウンドアプリのテキスト入力コンテキストが新しい入力ソースへ
    /// 差し替わるより **先に** 切替済みを報告する。観測と同時に注入を再開すると
    /// この隙間でローマ字が旧入力ソース（英数）にリテラル解釈され、「きょう」が
    /// `kilyou` として出る。観測を確認しても settle 分は保留を続ける。
    ///
    /// この隙間の長さは外から観測できない（アプリの入力コンテキストがいつ
    /// 新 IME に繋がったかを問い合わせる API が無い）ため、値は症状の有無で
    /// しか検証できない。**足りないときの症状は 2 種類ある**:
    ///
    /// - 全く足りない（切替直後に注入）: 旧入力ソースが解釈してローマ字が
    ///   リテラルで出る（「きょう」→ `kilyou`）
    /// - 少し足りない: 新入力ソースはアクティブだがコンポジションコンテキストが
    ///   未接続で、キーストロークがどこにも入らず**消える**（VS Code で実測）
    ///
    /// 後者は「出力が化ける」より気づきにくいので、値を詰めるときは
    /// リテラルが消えたことだけを根拠にしないこと。
    ///
    /// `AWASE_MACOS_SETTLE_MS` で上書きできる（この値を実機で詰めるための
    /// 診断用。BUG-101）。
    const OBSERVATION_SETTLE: Duration = Duration::from_millis(50);

    /// 実効 settle 値。`AWASE_MACOS_SETTLE_MS` があればそれを使う。
    fn observation_settle() -> Duration {
        static SETTLE: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
        *SETTLE.get_or_init(|| {
            let Some(ms) = std::env::var("AWASE_MACOS_SETTLE_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
            else {
                return OBSERVATION_SETTLE;
            };
            log::info!("OBSERVATION_SETTLE overridden to {ms}ms (AWASE_MACOS_SETTLE_MS)");
            Duration::from_millis(ms)
        })
    }

    /// 切替期待の状態機械。TIS 観測から切り離した純粋部分（テスト対象）。
    #[derive(Debug, Clone, Copy)]
    struct SwitchExpectation {
        /// 切替キー/`TISSelectInputSource` の直後に立てた期待状態
        expected: bool,
        /// 期待を立てた時刻（`EXPECTATION_GRACE` の起点）
        started: Instant,
        /// 観測が初めて期待と一致した時刻（`OBSERVATION_SETTLE` の起点）
        confirmed: Option<Instant>,
    }

    /// `SwitchExpectation::resolve` の結果。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Expectation {
        /// 観測はまだ追いついていない。期待値を `is_ime_on` の答えとして使う
        Hold(bool),
        /// 観測が追いついた。settle 待ちのため保留（`is_switch_pending`）は続ける
        Settling,
        /// 期待を解除する（settle 完了、または観測されないまま猶予切れ）
        Clear,
    }

    impl SwitchExpectation {
        const fn new(expected: bool, now: Instant) -> Self {
            Self {
                expected,
                started: now,
                confirmed: None,
            }
        }

        fn resolve(&mut self, observed: Option<bool>, now: Instant) -> Expectation {
            if observed == Some(self.expected) {
                let confirmed = *self.confirmed.get_or_insert(now);
                if now.duration_since(confirmed) >= observation_settle() {
                    Expectation::Clear
                } else {
                    Expectation::Settling
                }
            } else if now.duration_since(self.started) > EXPECTATION_GRACE {
                // 観測されないまま猶予切れ。呼び出し側は「切替に失敗した」と
                // 見なせるよう、期待値ではなく生の観測へ戻す
                Expectation::Clear
            } else {
                Expectation::Hold(self.expected)
            }
        }
    }

    /// macOS IME 検出。
    ///
    /// - 最後に観測した日本語 IME（かなモード）の入力ソース ID を記憶し、
    ///   `set_ime_on` での復元先として使う。
    /// - IME 切替キーの直後は「まもなく切り替わる」期待値を観測より優先する
    ///   （`expect_ime_on` / `EXPECTATION_GRACE`）。
    ///
    /// 単一 thread（main の CFRunLoop）でのみ使う前提で `RefCell` を持つ。
    #[derive(Debug)]
    pub struct ImeDetector {
        /// 最後に観測した日本語 IME かなモードの ID（ON 復元先の第一候補）
        last_japanese_id: std::cell::RefCell<Option<String>>,
        /// 最後に観測した日本語 IM のファミリ prefix（英数モードでも更新。
        /// 英数モードで起動して ON する場合の復元先の確定に使う —
        /// 2026-08-29 セキュリティレビュー第2回指摘）
        last_japanese_prefix: std::cell::RefCell<Option<String>>,
        /// IME 切替キー送出後の期待状態（settle 完了か猶予超過で解除）
        pending: std::cell::RefCell<Option<SwitchExpectation>>,
        /// 直近に観測した入力ソース ID（種別を問わない）。遷移をログへ残す
        /// ためだけに持つ — `last_japanese_id` はかなモードの ID しか記録
        /// しないため、ATOK Roman → ABC keylayout のような日本語 IM の外へ
        /// 出る遷移が誰の直後に起きたのか追えなかった（BUG-101 の追跡課題）。
        last_observed_id: std::cell::RefCell<Option<String>>,
        /// 最後に観測した「IME OFF を意味する入力ソース」の ID。ABC keylayout も
        /// ATOK の英字モードも、ユーザーが使っている方をそのまま記憶する。
        ///
        /// ON 側（`last_japanese_id`）と対称にするためのもの。`select_for(false)` が
        /// 「同ファミリの Roman/Eiji を優先し、無ければ keylayout」という優先順位で
        /// 選ぶと、ABC を使っている環境が ATOK の英字モードへ勝手に移り、macOS が
        /// それを記憶して以後ずっと変わってしまう（BUG-104。実際に BUG-102 の
        /// 張り直し経路がこれを起こした）。優先順位を押し付けず、観測した実物を
        /// 復元する。
        last_off_id: std::cell::RefCell<Option<String>>,
        /// 観測されないまま猶予切れした切替の目標状態。呼び出し側が
        /// `take_failed_switch` で一度だけ受け取り、張り直しに使う（BUG-102）。
        failed_switch: std::cell::Cell<Option<bool>>,
        /// 直近の切替が観測で確認された時刻。`pending` は settle 完了で消えるので
        /// 別に持つ — 「切替の何 ms 後に注入したか」をログに残し、
        /// `OBSERVATION_SETTLE` を実測で詰めるために使う（BUG-101）。
        last_confirmed_at: std::cell::Cell<Option<Instant>>,
    }

    impl ImeDetector {
        #[must_use]
        pub fn new() -> Self {
            log::info!("IME detector: TISCopyCurrentKeyboardInputSource");
            let detector = Self {
                last_japanese_id: std::cell::RefCell::new(None),
                last_japanese_prefix: std::cell::RefCell::new(None),
                pending: std::cell::RefCell::new(None),
                last_observed_id: std::cell::RefCell::new(None),
                last_off_id: std::cell::RefCell::new(None),
                failed_switch: std::cell::Cell::new(None),
                last_confirmed_at: std::cell::Cell::new(None),
            };
            // 起動時点の入力ソースを観測しておく（最初の打鍵前に
            // set_ime_on(true) が呼ばれても復元先が分かるように）
            let _ = detector.is_ime_on();
            detector
        }

        /// IME 切替キー（英数/かな）が OS に届いた直後に呼び、期待状態を立てる。
        pub fn expect_ime_on(&self, open: bool) {
            *self.pending.borrow_mut() = Some(SwitchExpectation::new(open, Instant::now()));
        }

        /// IME 切替がまだ落ち着いていない（切替中 or settle 待ち）かどうか。
        ///
        /// この間に注入したキーストロークは旧入力ソースで解釈されて
        /// リテラルの "wo" 等になるため、呼び出し側は送出を遅延させる。
        #[must_use]
        pub fn is_switch_pending(&self) -> bool {
            // 観測を取り込んで pending の解除判定を進めてから判定する
            let _ = self.is_ime_on();
            self.pending.borrow().is_some()
        }

        /// 保留中の切替期待の目標状態（`is_switch_pending` が true の間だけ Some）。
        ///
        /// 観測は進めない — 呼び出し側は直前に `is_switch_pending` を呼んでいる
        /// 前提で、保留を始めた時点の期待を記録するために使う。
        #[must_use]
        pub fn pending_open(&self) -> Option<bool> {
            self.pending.borrow().as_ref().map(|exp| exp.expected)
        }

        /// 観測されないまま猶予切れした切替を一度だけ受け取る（BUG-102）。
        ///
        /// 切替キーが IME に届かなかったということなので、呼び出し側は
        /// `set_ime_on` で張り直す。判定を進めるため、先に `is_ime_on` を
        /// 呼んでから取り出すこと。
        pub fn take_failed_switch(&self) -> Option<bool> {
            self.failed_switch.take()
        }

        /// 直近の切替が観測で確認されてからの経過時間（BUG-101 の実測用）。
        ///
        /// 「切替の N ms 後に注入した打鍵が消えた／化けた」を突き合わせて
        /// `OBSERVATION_SETTLE` を詰めるために使う。
        #[must_use]
        pub fn since_switch_confirmed(&self) -> Option<Duration> {
            self.last_confirmed_at.get().map(|at| at.elapsed())
        }

        /// 現在の IME 状態を問い合わせる
        /// - Some(true): IME ON (ひらがな・カタカナモード等)
        /// - Some(false): IME OFF (英数モード・IME なしレイアウト)
        /// - None: 検出不可
        #[must_use]
        pub fn is_ime_on(&self) -> Option<bool> {
            let observed = self.observe_ime_on();

            let mut pending = self.pending.borrow_mut();
            let Some(exp) = pending.as_mut() else {
                return observed;
            };
            let was_confirmed = exp.confirmed.is_some();
            let outcome = exp.resolve(observed, Instant::now());
            let expected = exp.expected;
            if !was_confirmed {
                if let Some(confirmed) = exp.confirmed {
                    self.last_confirmed_at.set(Some(confirmed));
                    // settle 定数の実測用（BUG-101）。切替キー送出から TIS が
                    // 新入力ソースを報告するまでの実時間を残す
                    log::debug!(
                        "IME switch observed: open={expected} after {}ms, \
                         holding output {}ms for settle",
                        confirmed.duration_since(exp.started).as_millis(),
                        observation_settle().as_millis(),
                    );
                }
            }
            match outcome {
                Expectation::Hold(expected) => Some(expected),
                Expectation::Settling => observed,
                Expectation::Clear => {
                    if observed != Some(expected) {
                        log::warn!(
                            "IME switch to open={expected} not observed within {}ms; \
                             injected output would be read by the old input source",
                            EXPECTATION_GRACE.as_millis(),
                        );
                        self.failed_switch.set(Some(expected));
                    }
                    *pending = None;
                    observed
                }
            }
        }

        /// TIS 観測のみで IME 状態を判定する（期待値を適用しない）。
        fn observe_ime_on(&self) -> Option<bool> {
            let id = current_input_source_id()?;
            {
                let mut last = self.last_observed_id.borrow_mut();
                if last.as_deref() != Some(id.as_str()) {
                    log::debug!(
                        "input source: {} -> {id}",
                        last.as_deref().unwrap_or("(unknown)")
                    );
                    *last = Some(id.clone());
                }
            }
            if is_japanese_im(&id) {
                // ユーザーが実際に使っている日本語 IM を記憶する
                // （ATOK / Google 日本語入力 / 日本語IM の区別を保つため。
                // 述語ベースの選択だとリスト先頭の OS 標準 IME に化ける）。
                // ファミリ prefix は英数モードでも更新する — 英数モードで
                // 起動すると kana ID が未観測のままになるため。
                if let Some(prefix) = im_family_prefix(&id) {
                    *self.last_japanese_prefix.borrow_mut() = Some(prefix.to_string());
                }
                if is_japanese_kana_mode(&id) {
                    let mut last = self.last_japanese_id.borrow_mut();
                    if last.as_deref() != Some(&id) {
                        log::debug!("IME observed: {id}");
                        *last = Some(id.clone());
                    }
                }
            }
            let observed = classify_input_source(&id);
            // OFF 側も ON 側と対称に、ユーザーが実際に使っている入力ソースを
            // 記憶する（BUG-104）。`select_for(false)` が優先順位で選ぶと、
            // ABC を使っている環境が ATOK の英字モードへ勝手に移る
            if observed == Some(false) {
                let mut last = self.last_off_id.borrow_mut();
                if last.as_deref() != Some(id.as_str()) {
                    log::debug!("IME off-source observed: {id}");
                    *last = Some(id);
                }
            }
            observed
        }

        /// 日本語 IME がアクティブかどうか（英数モードも含む）
        ///
        /// ON 期待（IME 切替キー直後）の間は true を返す — `compute_state` は
        /// `is_japanese_ime` を `ime_on` より先に評価するため、ABC keylayout
        /// からの切替中にここが false だと期待値ブリッジが無効化される。
        #[must_use]
        pub fn is_japanese_layout(&self) -> bool {
            if let Some(exp) = *self.pending.borrow() {
                if exp.expected && exp.started.elapsed() <= EXPECTATION_GRACE {
                    return true;
                }
            }
            current_input_source_id().is_none_or(|id| is_japanese_im(&id))
        }

        /// IME の ON/OFF を強制する（`ImeEffect::SetOpen` の実装）。
        ///
        /// - ON: 最後に使っていた日本語 IME のかなモードを復元。かなモード
        ///   未観測ならファミリ prefix 一致に限って選択する。ファミリも不明なら
        ///   失敗させる — 複数 IME 環境でリスト先頭の別 IME に切り替えるのは
        ///   ユーザー意図に反し、入力内容を扱うコンポーネントの選択としても
        ///   危険なため（2026-08-29 セキュリティレビュー第2回指摘）
        /// - OFF: 同じ IM ファミリの英字モード（`….Roman` / `…Eiji`）を優先し、
        ///   無ければ keylayout（ABC 等）
        ///
        /// 対象の入力ソースが見つからない/選択に失敗した場合は false。
        pub fn set_ime_on(&self, open: bool) -> bool {
            let selected = self.select_for(open);
            if selected {
                // TISSelectInputSource も反映まで observation lag があるため
                // 期待を立てる（失敗時に立てると偽の ON/OFF 判定が猶予いっぱい
                // 残るので、成功時のみ）
                self.expect_ime_on(open);
            }
            selected
        }

        fn select_for(&self, open: bool) -> bool {
            let last = self.last_japanese_id.borrow().clone();
            let prefix = self.last_japanese_prefix.borrow().clone();
            if open {
                if let Some(ref id) = last {
                    if select_input_source_matching(|c| c == id) {
                        return true;
                    }
                    log::warn!("IME restore failed for {id}, trying family prefix");
                }
                if let Some(ref prefix) = prefix {
                    if select_input_source_matching(|c| {
                        in_family(c, prefix) && is_japanese_kana_mode(c)
                    }) {
                        return true;
                    }
                }
                log::warn!(
                    "IME set_open(true): no Japanese IME observed yet; refusing to pick \
                     an arbitrary input source"
                );
                false
            } else {
                // まずユーザーが実際に使っている OFF 側の入力ソースを厳密に復元する
                // （`last_off_id` の doc 参照。ON 側の `last_japanese_id` と対称）
                let last_off = self.last_off_id.borrow().clone();
                if let Some(ref id) = last_off {
                    if select_input_source_matching(|c| c == id) {
                        return true;
                    }
                    log::warn!("IME off-source restore failed for {id}, trying fallbacks");
                }
                // 未観測（英字モードで起動した等）のときだけ優先順位に頼る
                if let Some(ref prefix) = prefix {
                    // 例: …atok34.Japanese → …atok34.Roman / …atok34.…Eiji
                    if select_input_source_matching(|c| {
                        in_family(c, prefix)
                            && (c.contains("Roman") || c.contains("Eiji"))
                            && !c.contains("FullWidth")
                    }) {
                        return true;
                    }
                }
                select_input_source_matching(|id| id.contains("keylayout"))
            }
        }
    }

    impl Default for ImeDetector {
        fn default() -> Self {
            Self::new()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn kana_mode_matches_hiragana_ids_of_known_imes() {
            assert!(is_japanese_kana_mode(
                "com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese"
            ));
            assert!(is_japanese_kana_mode(
                "com.google.inputmethod.Japanese.base"
            ));
            assert!(is_japanese_kana_mode(
                "com.justsystems.inputmethod.atok34.Japanese"
            ));
        }

        #[test]
        fn kana_mode_rejects_roman_katakana_and_keylayouts() {
            assert!(!is_japanese_kana_mode(
                "com.apple.inputmethod.Kotoeri.RomajiTyping.Roman"
            ));
            assert!(!is_japanese_kana_mode(
                "com.google.inputmethod.Japanese.Roman"
            ));
            assert!(!is_japanese_kana_mode(
                "com.google.inputmethod.Japanese.Katakana"
            ));
            assert!(!is_japanese_kana_mode(
                "com.justsystems.inputmethod.atok34.Japanese.HalfWidthEiji"
            ));
            assert!(!is_japanese_kana_mode("com.apple.keylayout.ABC"));
        }

        #[test]
        fn japanese_im_covers_atok_roman_mode_without_japanese_substring() {
            assert!(is_japanese_im("com.justsystems.inputmethod.atok34.Roman"));
            assert!(!is_japanese_im("com.apple.keylayout.ABC"));
        }

        #[test]
        fn in_family_requires_segment_boundary() {
            let prefix = "com.google.inputmethod.Japanese";
            assert!(in_family("com.google.inputmethod.Japanese", prefix));
            assert!(in_family("com.google.inputmethod.Japanese.Roman", prefix));
            // 兄弟 ID（prefix + 追加文字）はセグメント境界で拒否する
            assert!(!in_family(
                "com.google.inputmethod.JapaneseEvil.base",
                prefix
            ));
        }

        const MS: Duration = Duration::from_millis(1);

        /// 期待を立てた時刻から `after` 経過した時点で resolve する。
        fn at(exp: &mut SwitchExpectation, observed: Option<bool>, after: Duration) -> Expectation {
            exp.resolve(observed, exp.started + after)
        }

        #[test]
        fn expectation_holds_until_the_observation_catches_up() {
            let mut exp = SwitchExpectation::new(true, Instant::now());
            assert_eq!(at(&mut exp, Some(false), 10 * MS), Expectation::Hold(true));
            assert_eq!(at(&mut exp, None, 30 * MS), Expectation::Hold(true));
        }

        /// BUG-101 の回帰: TIS が切替済みを報告した瞬間に保留を解いてはならない。
        /// ここが `Clear` に戻ると、アプリの入力コンテキストが新入力ソースへ
        /// 繋がる前にローマ字が流れ、「きょう」が `kilyou` になる。
        #[test]
        fn expectation_keeps_holding_output_through_the_settle_window() {
            let mut exp = SwitchExpectation::new(true, Instant::now());
            assert_eq!(at(&mut exp, Some(true), 40 * MS), Expectation::Settling);
            let almost = (40 * MS + observation_settle()).saturating_sub(MS);
            assert_eq!(at(&mut exp, Some(true), almost), Expectation::Settling);
        }

        #[test]
        fn expectation_clears_once_the_settle_window_elapses() {
            let mut exp = SwitchExpectation::new(true, Instant::now());
            assert_eq!(at(&mut exp, Some(true), 40 * MS), Expectation::Settling);
            let settled = 40 * MS + observation_settle();
            assert_eq!(at(&mut exp, Some(true), settled), Expectation::Clear);
        }

        /// settle の起点は「期待を立てた時刻」ではなく「観測が一致した時刻」。
        /// 起点を取り違えると、観測が遅れたケースで settle が丸ごと消える。
        #[test]
        fn settle_window_starts_at_the_observation_not_the_expectation() {
            let mut exp = SwitchExpectation::new(true, Instant::now());
            let late = EXPECTATION_GRACE.saturating_sub(10 * MS);
            assert_eq!(at(&mut exp, Some(true), late), Expectation::Settling);
            assert_eq!(
                at(
                    &mut exp,
                    Some(true),
                    (late + observation_settle()).saturating_sub(MS)
                ),
                Expectation::Settling,
            );
        }

        /// 猶予切れは「切替が終わった」ではなく「諦めた」。呼び出し側が
        /// 観測と期待の不一致で失敗を検出できるよう、期待値へ倒さず解除する。
        #[test]
        fn expectation_gives_up_after_the_grace_without_any_observation() {
            let mut exp = SwitchExpectation::new(true, Instant::now());
            assert_eq!(
                at(&mut exp, Some(false), EXPECTATION_GRACE + MS),
                Expectation::Clear
            );
            assert!(exp.confirmed.is_none());
        }

        /// BUG-104 の一部: ATOK の英字モードも ABC keylayout も等しく
        /// 「OFF を意味する入力ソース」として分類される。ここが片方だけだと
        /// `last_off_id` に記録されず、`select_for(false)` が優先順位で
        /// 選び直してユーザーの入力ソースを勝手に移してしまう。
        #[test]
        fn both_atok_roman_and_abc_classify_as_ime_off() {
            assert_eq!(
                classify_input_source("com.justsystems.inputmethod.atok36.Roman"),
                Some(false)
            );
            assert_eq!(
                classify_input_source("com.apple.keylayout.ABC"),
                Some(false)
            );
            assert_eq!(
                classify_input_source("com.justsystems.inputmethod.atok36.Japanese.HalfWidthEiji"),
                Some(false)
            );
            assert_eq!(
                classify_input_source("com.justsystems.inputmethod.atok36.Japanese"),
                Some(true)
            );
            // 日本語 IM でもキーボードレイアウトでもない（中国語 IM 等）
            assert_eq!(
                classify_input_source("com.apple.inputmethod.SCIM.ITABC"),
                None
            );
        }

        #[test]
        fn family_prefix_is_mode_invariant() {
            assert_eq!(
                im_family_prefix("com.justsystems.inputmethod.atok34.Japanese"),
                im_family_prefix("com.justsystems.inputmethod.atok34.Roman"),
            );
            assert_eq!(
                im_family_prefix("com.google.inputmethod.Japanese.base"),
                Some("com.google.inputmethod.Japanese"),
            );
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::ImeDetector;

/// 非 macOS ビルド用スタブ。
#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub struct ImeDetector;

#[cfg(not(target_os = "macos"))]
impl ImeDetector {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn is_ime_on(&self) -> Option<bool> {
        None
    }

    #[must_use]
    pub fn is_japanese_layout(&self) -> bool {
        true
    }

    pub fn set_ime_on(&self, _open: bool) -> bool {
        false
    }

    pub fn expect_ime_on(&self, _open: bool) {}

    #[must_use]
    pub fn is_switch_pending(&self) -> bool {
        false
    }

    #[must_use]
    pub fn pending_open(&self) -> Option<bool> {
        None
    }

    pub fn take_failed_switch(&self) -> Option<bool> {
        None
    }

    #[must_use]
    pub fn since_switch_confirmed(&self) -> Option<std::time::Duration> {
        None
    }
}

#[cfg(not(target_os = "macos"))]
impl Default for ImeDetector {
    fn default() -> Self {
        Self::new()
    }
}
