use anyhow::{bail, Context, Result};
use std::path::{Component, Path, PathBuf};

use awase::config::AppConfig;
use awase::engine::{Engine, NicolaFsm, SpecialKeyCombos};
use awase::scanmap::KeyboardModel;
use awase::types::ModifierKey;
use awase::yab::YabLayout;

use awase_macos::vk::key_name_to_keycode;

/// kVK_JIS_Eisu（英数）— macOS 標準の IME OFF キー
const KEYCODE_EISU: u16 = 0x66;
/// kVK_JIS_Kana（かな）— macOS 標準の IME ON キー
const KEYCODE_KANA: u16 = 0x68;

/// 実行ファイルが .app バンドル内（`…/<名前>.app/Contents/MacOS/<exe>`）に
/// あれば `Contents/Resources` を返す。
///
/// 文字列部分一致ではなく親ディレクトリ構造で判定する（`Awase.APP` のような
/// 大文字表記や非 UTF-8 パスで判定を迂回されないように、大小文字非依存・
/// OsStr ベースで確認する — 2026-08-29 セキュリティレビュー第3回指摘2）。
fn bundle_resources_dir(exe: &Path) -> Option<PathBuf> {
    fn is_named(dir: &Path, name: &str) -> bool {
        dir.file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
    }
    let macos_dir = exe.parent()?;
    let contents = macos_dir.parent()?;
    let app = contents.parent()?;
    let app_ext_ok = Path::new(app.file_name()?)
        .extension()
        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("app"));
    (is_named(macos_dir, "MacOS") && is_named(contents, "Contents") && app_ext_ok)
        .then(|| contents.join("Resources"))
}

/// バンドル内のリソースを解決し、`Contents/Resources` の外への脱出を拒否する。
fn resolve_bundled_resource(resources: &Path, path: &str) -> Result<PathBuf> {
    let raw = Path::new(path);
    if raw.is_absolute()
        || raw.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("bundled resource path must stay relative: {path:?}");
    }

    let resources_meta = std::fs::symlink_metadata(resources).with_context(|| {
        format!(
            "Failed to inspect bundle resources at {}",
            resources.display()
        )
    })?;
    if resources_meta.file_type().is_symlink() || !resources_meta.is_dir() {
        bail!(
            "bundle Resources must be a real directory, not a symlink: {}",
            resources.display()
        );
    }
    let canonical_root = resources.canonicalize().with_context(|| {
        format!(
            "Failed to canonicalize bundle resources at {}",
            resources.display()
        )
    })?;
    let candidate = resources.join(raw);
    match candidate.canonicalize() {
        Ok(canonical) if canonical.starts_with(&canonical_root) => Ok(canonical),
        Ok(canonical) => bail!(
            "bundled resource escapes Contents/Resources: {} -> {}",
            candidate.display(),
            canonical.display()
        ),
        // 存在しないリソースは呼び出し側が既定値へ倒す。ただし、直前の
        // ディレクトリも canonicalize し、未作成ファイルに向かう symlink での脱出を防ぐ。
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // `canonicalize` が NotFound かつ `symlink_metadata` が成功する組み合わせは
            // dangling symlink（リンク自体は在るがリンク先が無い）に限られる。今は
            // 解決できないが、外部プロセスが後からリンク先を作れば Resources 外を
            // 読める（`exists()` / 読み込みとの競合）ため、ここで拒否する。
            if std::fs::symlink_metadata(&candidate).is_ok() {
                bail!(
                    "bundled resource is a dangling symlink: {}",
                    candidate.display()
                );
            }
            let parent = candidate.parent().unwrap_or(resources);
            let canonical_parent = parent.canonicalize().with_context(|| {
                format!(
                    "Failed to canonicalize bundled resource parent {}",
                    parent.display()
                )
            })?;
            if !canonical_parent.starts_with(&canonical_root) {
                bail!(
                    "bundled resource parent escapes Contents/Resources: {} -> {}",
                    parent.display(),
                    canonical_parent.display()
                );
            }
            Ok(candidate)
        }
        Err(e) => Err(e).with_context(|| {
            format!(
                "Failed to canonicalize bundled resource {}",
                candidate.display()
            )
        }),
    }
}

/// リソース（config.toml / layout）を解決する。
///
/// .app バンドル内から実行されている場合は署名対象の `Contents/Resources` に
/// 固定し、カレントディレクトリへはフォールバックしない — Accessibility 権限を
/// 持つプロセスが、起動ディレクトリに置かれた署名対象外の config/layout を
/// 読み込むのを防ぐため（2026-08-29 セキュリティレビュー指摘1）。
///
/// バンドル外の解決はビルド種別で分ける（同レビュー第4回指摘2）:
/// - 開発ビルド: `paths::resolve_relative_to_exe`（exe 隣接 → ワークスペース
///   ルート → CWD）。`cargo run` の利便性を優先
/// - リリースビルド: exe 隣接のみ（fail closed）。共通解決器はパス中の任意の
///   `target` ディレクトリをワークスペースとみなすため、偽の `target/release/`
///   配下に置かれたバイナリに外部設定を読ませられる。`current_exe()` 取得
///   不能時も CWD には落とさず、存在しないパスを返して既定値動作にする。
fn resolve_resource(path: &str) -> Result<PathBuf> {
    let raw = Path::new(path);
    let exe = std::env::current_exe().ok();
    if let Some(resources) = exe.as_deref().and_then(bundle_resources_dir) {
        return resolve_bundled_resource(&resources, path);
    }
    if raw.is_absolute() {
        return Ok(raw.to_path_buf());
    }
    #[cfg(debug_assertions)]
    {
        Ok(awase::paths::resolve_relative_to_exe(path))
    }
    #[cfg(not(debug_assertions))]
    {
        Ok(exe.as_deref().and_then(Path::parent).map_or_else(
            // /var/empty は root 所有の空ディレクトリ（macOS 標準）。
            // 「確実に存在しない相対リソース」の錨として使い、呼び出し側の
            // 既定値（デフォルト設定・空レイアウト警告）へ倒す
            || Path::new("/var/empty").join(path),
            |dir| dir.join(path),
        ))
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use std::fs;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "awase_macos_resource_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bundle_detection_is_structural_and_case_insensitive() {
        let dir = |p: &str| bundle_resources_dir(Path::new(p));
        assert_eq!(
            dir("/Applications/Awase.app/Contents/MacOS/awase"),
            Some(PathBuf::from("/Applications/Awase.app/Contents/Resources")),
        );
        // 大文字表記でも迂回できない
        assert_eq!(
            dir("/tmp/Awase.APP/CONTENTS/MACOS/awase"),
            Some(PathBuf::from("/tmp/Awase.APP/CONTENTS/Resources")),
        );
        // バンドル構造でなければ None
        assert_eq!(dir("/usr/local/bin/awase"), None);
        assert_eq!(dir("/tmp/Awase.app/MacOS/awase"), None);
        assert_eq!(dir("/tmp/NotBundle/Contents/MacOS/awase"), None);
    }

    #[test]
    fn bundled_resource_accepts_existing_file_inside_resources() {
        let root = unique_temp_dir("inside");
        let resources = root.join("Contents/Resources");
        fs::create_dir_all(resources.join("layout")).unwrap();
        fs::write(resources.join("layout/nicola.yab"), "").unwrap();

        let resolved = resolve_bundled_resource(&resources, "layout/nicola.yab").unwrap();
        assert_eq!(
            resolved,
            resources.join("layout/nicola.yab").canonicalize().unwrap()
        );
        let _ = fs::remove_dir_all(root);
    }

    /// 最終要素そのものが dangling symlink のケース。`canonicalize` は NotFound を
    /// 返すため親ディレクトリ検査だけでは通ってしまうが、外部プロセスがあとから
    /// リンク先を作れば Resources 外を読める。
    #[cfg(unix)]
    #[test]
    fn bundled_resource_rejects_dangling_symlink_as_the_final_component() {
        use std::os::unix::fs::symlink;

        let root = unique_temp_dir("dangling_leaf");
        let resources = root.join("Contents/Resources");
        fs::create_dir_all(resources.join("layout")).unwrap();
        // リンク先はまだ存在しない（＝canonicalize は NotFound）
        symlink(
            root.join("not-created-yet.yab"),
            resources.join("layout/default.yab"),
        )
        .unwrap();

        assert!(resolve_bundled_resource(&resources, "layout/default.yab").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bundled_resource_rejects_absolute_path_and_parent_traversal() {
        let root = unique_temp_dir("lexical_escape");
        let resources = root.join("Contents/Resources");
        fs::create_dir_all(&resources).unwrap();

        assert!(resolve_bundled_resource(&resources, "/tmp/evil.yab").is_err());
        assert!(resolve_bundled_resource(&resources, "layout/../../evil.yab").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn bundled_resource_rejects_outward_symlink() {
        use std::os::unix::fs::symlink;

        let root = unique_temp_dir("symlink_escape");
        let resources = root.join("Contents/Resources");
        let outside = root.join("outside");
        fs::create_dir_all(&resources).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("evil.yab"), "").unwrap();
        symlink(&outside, resources.join("layout")).unwrap();

        assert!(resolve_bundled_resource(&resources, "layout/evil.yab").is_err());
        assert!(resolve_bundled_resource(&resources, "layout/not-created.yab").is_err());
        let _ = fs::remove_dir_all(root);
    }
}

/// config の `[macos_symbol_romaji]` を「出力 1 文字 → 入力列」へ変換する。
/// キーが 1 文字でないエントリは警告して捨てる。
fn parse_symbol_romaji(
    table: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<char, String> {
    table
        .iter()
        .filter_map(|(k, v)| {
            let mut chars = k.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else {
                log::warn!("macos_symbol_romaji: key \"{k}\" must be a single character");
                return None;
            };
            Some((ch, v.clone()))
        })
        .collect()
}

fn warn_if_key_content_logging_enabled() {
    if awase_macos::diagnostics::key_content_enabled() {
        log::warn!(
            "{}=1: diagnostic logs may contain physical key codes and typed text; \
             do not share them without review",
            awase_macos::diagnostics::KEY_CONTENT_ENV
        );
    }
}

fn main() -> Result<()> {
    // 1. Initialize logging
    // ms 精度: IME 切替の settle 窓（数十 ms）と打鍵の前後関係を読むには
    // 既定の秒精度では足りない（BUG-101 の調査で判明）
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    warn_if_key_content_logging_enabled();

    log::info!("awase-macos starting");

    // 2. Load config
    let config_path = resolve_resource("config.toml")?;
    let config = if config_path.exists() {
        log::info!("Loading config from: {}", config_path.display());
        AppConfig::load(&config_path)?
    } else {
        log::warn!("config.toml not found, using defaults");
        let toml_str = "[general]";
        toml::from_str(toml_str).context("Failed to create default config")?
    };
    let (config, warnings) = config.validate();
    for w in &warnings {
        log::warn!("Config: {w}");
    }

    // 3. Resolve key names to macOS keycodes
    let left_thumb = key_name_to_keycode(&config.general.left_thumb_key)
        .with_context(|| format!("Unknown left thumb key: {}", config.general.left_thumb_key))?;
    let right_thumb = key_name_to_keycode(&config.general.right_thumb_key).with_context(|| {
        format!(
            "Unknown right thumb key: {}",
            config.general.right_thumb_key
        )
    })?;

    // 4. Set thumb keycodes for hook classification
    awase_macos::hook::set_thumb_keycodes(left_thumb, right_thumb);

    // 5. Load .yab layout
    // .yab は JIS 物理位置ベースのため Jis 固定（keyboard_model 設定は 2026-07-06 撤去）
    let keyboard_model = KeyboardModel::Jis;

    let layout_rel = Path::new(&config.general.layouts_dir).join(&config.general.default_layout);
    let layout_path = resolve_resource(&layout_rel.to_string_lossy())?;
    let mut layout = if layout_path.exists() {
        let content = std::fs::read_to_string(&layout_path)?;
        YabLayout::parse(&content, keyboard_model)?.resolve_kana()
    } else {
        log::warn!(
            "Layout file not found: {}, using empty layout",
            layout_path.display()
        );
        YabLayout::parse("", keyboard_model)?
    };

    // NICOLA 規格では最下段 / 位置の単打は ・ だが、共有 .yab は Windows 版の
    // 既存出力（Unicode モードで ／）を変えないよう ／ のまま維持されている。
    // macOS はローマ字キーストローク出力の都合で単打面のみ ・ に置き換える
    // （"/" キーストロークを IME が ・ に変換する。親指シフト面の ／ は
    // 直接注入で正確に出すため、面ごとに文字を分ける必要がある —
    // output.rs の ime_renders_differently 参照）
    for value in layout.normal.values_mut() {
        if matches!(value, awase::yab::YabValue::Literal(s) if s == "／") {
            *value = awase::yab::YabValue::Literal("・".to_string());
        }
    }

    // 6. Build Engine (NicolaFsm + InputTracker + empty ImeSyncKeys/SpecialKeyCombos)
    let mut fsm = NicolaFsm::new(
        layout,
        left_thumb,
        right_thumb,
        config.general.simultaneous_threshold_ms,
        config.general.confirm_mode,
        config.general.speculative_delay_ms,
    );
    // 親指キー自体が Shift（macOS keycode 0x38/0x3C）に割り当てられている場合、
    // 親指押下だけで Shift レベルが立つため複合面を無効化する（Windows/Linux 側と
    // 同じ判定方針。magic number を `hook::classify_modifier` 呼び出しに置き換え
    // 重複を解消、2026-08-20 独立レビューで指摘）。
    fsm.set_thumb_shift_faces_enabled(
        awase_macos::hook::classify_modifier(left_thumb.0) != Some(ModifierKey::Shift)
            && awase_macos::hook::classify_modifier(right_thumb.0) != Some(ModifierKey::Shift),
    );
    // 親指キーが 英数/かな（kVK_JIS_Eisu / kVK_JIS_Kana）そのものなら、非活性化
    // フラッシュで捨てずに送出させる。macOS では親指キーが IME 切替キーを兼ねる
    // ため、捨てるとユーザーの切替操作が消える（BUG-103）。Space 等の文字キーを
    // 割り当てた構成では false のままにする — 生キー送出の誤注入を防ぐ抑制が
    // 本来の目的どおり効くべきなので（`thumb_keys_are_ime_switch` の doc 参照）
    let is_switch_key = |kc: u16| matches!(kc, KEYCODE_EISU | KEYCODE_KANA);
    fsm.set_thumb_keys_are_ime_switch(is_switch_key(left_thumb.0) && is_switch_key(right_thumb.0));

    // [keys] のコンボ設定を macOS keycode に解決して Engine に渡す
    let parse_combos = |keys: &[String], label: &str| {
        let parsed: Vec<_> = keys
            .iter()
            .filter_map(|s| {
                let combo = awase_macos::vk::parse_key_combo(s);
                if combo.is_none() {
                    log::warn!("keys.{label}: cannot parse combo \"{s}\" on macOS, ignoring");
                }
                combo
            })
            .collect();
        log::info!("keys.{label}: {keys:?} ({} parsed)", parsed.len());
        parsed
    };
    let engine = Engine::new(
        fsm,
        SpecialKeyCombos {
            engine_on: parse_combos(&config.keys.engine_on, "engine_on"),
            engine_off: parse_combos(&config.keys.engine_off, "engine_off"),
            ime_on: parse_combos(&config.keys.ime_on, "ime_on"),
            ime_off: parse_combos(&config.keys.ime_off, "ime_off"),
            ime_toggle: parse_combos(&config.keys.ime_toggle, "ime_toggle"),
        },
    );

    // 7. Run platform event loop
    let poll_interval =
        std::time::Duration::from_millis(u64::from(config.general.ime_poll_interval_ms.max(100)));
    let output_style = if config.general.macos_output_style == "kana" {
        awase_macos::output::OutputStyle::Kana
    } else {
        awase_macos::output::OutputStyle::Romaji
    };
    let symbol_romaji = parse_symbol_romaji(&config.macos_symbol_romaji);
    run_event_loop(
        engine,
        &config.general.default_layout,
        poll_interval,
        output_style,
        symbol_romaji,
    )
}

#[cfg(target_os = "macos")]
fn run_event_loop(
    engine: Engine,
    layout_name: &str,
    poll_interval: std::time::Duration,
    output_style: awase_macos::output::OutputStyle,
    symbol_romaji: std::collections::HashMap<char, String>,
) -> Result<()> {
    use std::cell::RefCell;
    use std::rc::Rc;

    if !awase_macos::hook::check_accessibility_permission() {
        anyhow::bail!(
            "Accessibility permission is not granted. \
             Enable this app in System Settings > Privacy & Security > Accessibility, \
             then restart."
        );
    }

    let output = awase_macos::output::Output::new(output_style, symbol_romaji)?;

    // メニューバー常駐（NSApplication 初期化後に作ること）
    awase_macos::event_loop::init_nsapp();
    let tray = awase_macos::tray::SystemTray::new();
    tray.set_layout_name(layout_name);

    let app = Rc::new(RefCell::new(app::App::new(engine, output, tray)));

    log::info!("awase-macos running (menu bar icon: あ). Quit from the menu or Ctrl+C.");
    let mut event_loop = awase_macos::event_loop::EventLoop::new();
    event_loop.run(app, poll_interval)
}

#[cfg(not(target_os = "macos"))]
fn run_event_loop(
    _engine: Engine,
    _layout_name: &str,
    _poll_interval: std::time::Duration,
    _output_style: awase_macos::output::OutputStyle,
    _symbol_romaji: std::collections::HashMap<char, String>,
) -> Result<()> {
    log::warn!("awase-macos event loop is only available on macOS");
    Ok(())
}

#[cfg(target_os = "macos")]
mod app {
    use std::time::Instant;

    use awase::engine::{
        Decision, Effect, Engine, EngineCommand, ImeEffect, InputContext, InputEffect,
        InputModeState, ModifierState, SetOpenOrigin, TimerEffect, UiEffect,
    };
    use awase::types::{
        KeyClassification, KeyEventType, ModifierKey, RawKeyEvent, ScanCode, Timestamp, VkCode,
    };
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventType, EventField};

    use awase_macos::event_loop::{LoopHandler, MenuAction, TapAction, Timers};
    use awase_macos::hook;
    use awase_macos::ime::ImeDetector;
    use awase_macos::output::{Output, INJECT_MARKER};
    use awase_macos::tray::SystemTray;

    use super::{KEYCODE_EISU, KEYCODE_KANA};

    /// IME 切替待ちの出力フラッシュ用タイマー ID。
    /// Engine の TimerEffect ID（小さい連番）と衝突しない値を使う。
    const FLUSH_TIMER_ID: usize = usize::MAX;
    /// フラッシュ再確認の間隔
    const FLUSH_RETRY: std::time::Duration = std::time::Duration::from_millis(20);
    /// 保留キューの上限（DoS 耐性。通常の切替待ちで溜まるのは数打鍵）
    const DEFER_CAP: usize = 128;
    /// 観測されない切替を `TISSelectInputSource` で張り直す上限回数。
    ///
    /// 実測では張り直しは毎回 ~20ms で成功しているので 1 回で足りるが、
    /// `EXPECTATION_GRACE` を実測最大 +59ms まで詰めた分、期限の空振りが
    /// キュー破棄（＝入力消失）に直結しないよう 2 回まで許す（BUG-101）。
    const MAX_SWITCH_REASSERTS: u8 = 2;

    /// この外部パススルーキーがフォーカスを動かしうるか（＝保留出力の宛先が
    /// 変わりうるか）。
    ///
    /// PID が同じでも Tab / Cmd+L 等でフォーカス先の control は変わりうるため、
    /// そのまま送ると保留文字が別の入力欄へ入る。一方で **パススルー全部を対象に
    /// すると害の方が大きい**: Backspace や矢印はフォーカスを移さないので破棄の
    /// 利得が無く、保留中の文字を捨てたうえで Backspace を通すと「その前の文字」
    /// が消える。対象はフォーカスを動かしうるものだけに絞る —
    /// 既存の composition 切れ目（Enter / keypad Enter / Escape / Tab、
    /// `on_cg_event` の `note_composition_break` と同じ集合）と、修飾キー同時押し
    /// （Cmd+L 等のショートカット）。Shift は単独でフォーカスを動かさないので除く
    /// （Shift+Tab は Tab 側で捕まる）。
    ///
    /// **修飾キー自身の KeyDown は対象外**（`is_modifier_key`）。`modifiers` は
    /// この判定より前に `update()` 済みなので、Ctrl 単独を押しただけでも
    /// `is_os_modifier_held()` が真になる。区別しないと、Ctrl を押して離すだけで
    /// 保留中の入力が消える。
    ///
    /// 判定はキーの性質だけで決め、キューの状態は見ない。「このイベント自身が
    /// 積んだ出力」も破棄対象に含める必要があるため — 保留中の文字はこのキーより
    /// 前に打たれたもので、本来は旧 control へ出すべきもの。切替未完了で出せない
    /// 以上、新しい control へ遅れて注入するより捨てる方が正しい。
    ///
    /// awase 自身の注入イベントはこの判定より前に return するため対象外。
    const fn passthrough_moves_focus(
        action: TapAction,
        keycode: u16,
        event_type: KeyEventType,
        modifiers: ModifierState,
        is_modifier_key: bool,
    ) -> bool {
        if !matches!(action, TapAction::Pass)
            || !matches!(event_type, KeyEventType::KeyDown)
            || is_modifier_key
        {
            return false;
        }
        matches!(keycode, 0x24 | 0x4C | 0x35 | 0x30) || modifiers.is_os_modifier_held()
    }

    /// IME 切替キー（英数/かな）の送出アクションかどうか。
    const fn is_ime_switch_action(action: &awase::types::KeyAction) -> bool {
        matches!(
            action,
            awase::types::KeyAction::Key(vk) | awase::types::KeyAction::KeyUp(vk)
                if matches!(vk.0, KEYCODE_EISU | KEYCODE_KANA)
        )
    }

    /// 前面アプリの PID を返す（保留出力の誤送出防止用）。
    fn frontmost_pid() -> Option<i32> {
        let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication()?;
        Some(app.processIdentifier())
    }

    /// 起動時点からの経過マイクロ秒を返す
    fn now_timestamp() -> Timestamp {
        use std::sync::OnceLock;
        static BASELINE: OnceLock<Instant> = OnceLock::new();
        let baseline = BASELINE.get_or_init(Instant::now);
        u64::try_from(baseline.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// Engine・出力・タイマーを束ねるアプリケーション状態。
    ///
    /// CFRunLoop（単一スレッド）上で tap コールバックとタイマーの両方から
    /// `RefCell` 経由で呼ばれる。
    pub struct App {
        engine: Engine,
        output: Output,
        timers: Timers,
        ime: ImeDetector,
        tray: SystemTray,
        modifiers: ModifierState,
        left_thumb_down: Option<Timestamp>,
        right_thumb_down: Option<Timestamp>,
        /// IME 切替が観測確認されるまで保留する出力アクション。
        ///
        /// 期待値ブリッジで Engine は切替直後の打鍵も変換するが、注入した
        /// ローマ字キーストローク自体が入力ソース切替の完了とレースすると
        /// 旧ソース（ABC 等）で解釈されてリテラル "wo" などになる。切替確認
        /// まで溜めて `FLUSH_TIMER_ID` のタイマーでフラッシュする。
        deferred_keys: Vec<awase::types::KeyAction>,
        /// 最初に保留した時点の前面アプリ PID。フラッシュ時に前面アプリが
        /// 変わっていたら破棄する — 保留中に Cmd+Tab 等でフォーカスが移ると、
        /// 入力文字や Backspace が別アプリへ送出されてしまうため
        /// （2026-08-29 セキュリティレビュー第3回指摘1）。
        deferred_focus_pid: Option<i32>,
        /// 保留を始めた時点で待っていた IME 状態。`is_switch_pending` は
        /// 「切替が完了した」と「猶予切れで諦めた」を区別しないため、
        /// フラッシュ直前に観測がこれと一致するかを確かめる（BUG-101）。
        deferred_expect_on: Option<bool>,
        /// 保留を始めた時刻（フラッシュまでの実待ち時間をログに残す）
        deferred_since: Option<Instant>,
        /// 観測されない切替を `TISSelectInputSource` で張り直した回数
        deferred_switch_reasserts: u8,
        /// 保留キューが空のまま失敗した切替を張り直した回数（BUG-102）。
        /// 切替キーを新たに観測するたびに 0 に戻す
        switch_recoveries: u8,
    }

    impl App {
        pub fn new(engine: Engine, output: Output, tray: SystemTray) -> Self {
            let mut app = Self {
                engine,
                output,
                timers: Timers::new(),
                ime: ImeDetector::new(),
                tray,
                modifiers: ModifierState::default(),
                left_thumb_down: None,
                right_thumb_down: None,
                deferred_keys: Vec::new(),
                deferred_focus_pid: None,
                deferred_expect_on: None,
                deferred_since: None,
                deferred_switch_reasserts: 0,
                switch_recoveries: 0,
            };
            // トレイ表示を実際の activation 状態に合わせる。既定値のままだと
            // 起動時に IME が OFF でも「あ」と出る（状態が変化しないため
            // UiEffect が飛ばず、ポーリングでも補正されない）
            let active = app.engine.compute_active(&app.make_ctx());
            app.tray.set_enabled(active);
            app
        }

        /// IME 切替待ちが解消していれば保留出力をフラッシュする。
        ///
        /// 保留開始時・送出時の両方で前面アプリ PID が取得でき、かつ一致した
        /// 場合のみ送出する（fail closed — どちらかが取得不能でも破棄。
        /// 2026-08-29 セキュリティレビュー第4回指摘1）。
        fn maybe_flush_deferred(&mut self) {
            if self.deferred_keys.is_empty() || self.ime.is_switch_pending() {
                return;
            }
            // 猶予切れで期待を諦めた場合も is_switch_pending は false になる。
            // 観測が期待に届いていなければ IME は旧入力ソースのままで、ここで
            // 送出するとローマ字がリテラルで漏れる（「きょう」→ "kilyou"）。
            // 実測では物理「かな」キーを ATOK が取りこぼす（英数 切替の完了前に
            // かな を打つと落ちる）ケースが主因で、`TISSelectInputSource` での
            // 張り直しは ~20ms で必ず成功していた。上限まで張り直して待ち直し、
            // それでも駄目なら破棄する
            if let Some(expected) = self.deferred_expect_on {
                if self.ime.is_ime_on() != Some(expected) {
                    if self.deferred_switch_reasserts < MAX_SWITCH_REASSERTS
                        && self.ime.set_ime_on(expected)
                    {
                        self.deferred_switch_reasserts += 1;
                        log::warn!(
                            "IME switch to open={expected} not observed; re-asserting it and \
                             holding {} deferred key action(s)",
                            self.deferred_keys.len()
                        );
                        self.timers.set(FLUSH_TIMER_ID, FLUSH_RETRY);
                        return;
                    }
                    self.discard_deferred("IME switch to the expected state never took effect");
                    return;
                }
            }
            let keys = std::mem::take(&mut self.deferred_keys);
            let expected_pid = self.deferred_focus_pid.take();
            let held = self.deferred_since.take().map(|since| since.elapsed());
            self.deferred_expect_on = None;
            self.deferred_switch_reasserts = 0;
            let same_focus = matches!(
                (expected_pid, frontmost_pid()),
                (Some(expected), Some(current)) if expected == current
            );
            if !same_focus {
                log::warn!(
                    "Discarding {} deferred key action(s): frontmost app changed or \
                     unknown during IME switch",
                    keys.len()
                );
                return;
            }
            // settle 定数の実測用（BUG-101）。保留開始から実際に送出するまでの
            // 実待ち時間を残す
            log::debug!(
                "Flushing {} deferred key action(s) {}ms after the switch was expected",
                keys.len(),
                held.unwrap_or_default().as_millis(),
            );
            self.output.send_keys(&keys);
        }

        /// この切替アクションが、より後に押された反対側の親指キーに
        /// 追い越されているか（BUG-103 追補）。
        ///
        /// 両親指が押されている間だけ判定する。押下時刻の新しい方がユーザーの
        /// 最終意図なので、古い方の切替キーを送ると結果が逆になる。
        fn is_superseded_switch(&self, actions: &[awase::types::KeyAction]) -> bool {
            let (Some(left_at), Some(right_at)) = (self.left_thumb_down, self.right_thumb_down)
            else {
                return false;
            };
            actions.iter().any(|action| match action {
                awase::types::KeyAction::Key(vk) => match vk.0 {
                    KEYCODE_EISU => right_at > left_at,
                    KEYCODE_KANA => left_at > right_at,
                    _ => false,
                },
                _ => false,
            })
        }

        /// 観測されないまま猶予切れした切替を張り直す（BUG-102）。
        ///
        /// 保留キューがあるときは `maybe_flush_deferred` が回数制限付きで面倒を
        /// 見るので、ここは「切替キーが効かなかったが、まだ何も打鍵していない」
        /// 経路だけを拾う。放置すると engine が非活性のまま次の打鍵が生キーで
        /// 素通しされる（実測: 「かな」が効かず生 `k` が出た）。
        ///
        /// 打鍵イベントの処理より先に呼ぶこと。ここで期待を張り直しておけば、
        /// その打鍵自体も `ime_on=true` として consume され保留に載る。
        fn recover_failed_switch(&mut self) {
            // 猶予切れの判定を進めてから失敗を取り出す
            let _ = self.ime.is_ime_on();
            let Some(expected) = self.ime.take_failed_switch() else {
                return;
            };
            if !self.deferred_keys.is_empty() || self.switch_recoveries >= MAX_SWITCH_REASSERTS {
                return;
            }
            if self.ime.set_ime_on(expected) {
                self.switch_recoveries += 1;
                log::warn!("IME switch to open={expected} not observed; re-asserting it");
            }
        }

        /// 保留出力を破棄する（クリック等でフォーカス・キャレットが動いた場合）。
        fn discard_deferred(&mut self, reason: &str) {
            if !self.deferred_keys.is_empty() {
                log::warn!(
                    "Discarding {} deferred key action(s): {reason}",
                    self.deferred_keys.len()
                );
                self.deferred_keys.clear();
            }
            self.deferred_focus_pid = None;
            self.deferred_expect_on = None;
            self.deferred_since = None;
            self.deferred_switch_reasserts = 0;
        }

        fn make_ctx(&self) -> InputContext {
            InputContext {
                // IME 検出不能なとき（不明なレイアウト等）は ON と仮定する
                ime_on: self.ime.is_ime_on().unwrap_or(true),
                // macOS の日本語 IME はローマ字入力が既定。JIS かな入力の観測は
                // 未実装のため Linux 実装と同じく ObservedRomaji 固定とする
                input_mode: InputModeState::ObservedRomaji,
                is_japanese_ime: self.ime.is_japanese_layout(),
                composing: false, // macOS では composition 検出未実装
                modifiers: self.modifiers,
                left_thumb_down: self.left_thumb_down,
                right_thumb_down: self.right_thumb_down,
            }
        }

        fn run_effects(&mut self, effects: &[Effect]) {
            for effect in effects {
                match effect {
                    Effect::Input(InputEffect::SendKeys(actions)) => {
                        // 英数/かな の生 VK 送出は IME 切替そのものの引き金なので、
                        // 保留してはいけない。保留すると後続の物理切替キーに
                        // 追い越されて順序が逆転し、IME が意図と逆の状態に落ちる
                        // （英数→かな と続けて打つと 英数 の注入が後着し、直後の
                        // 打鍵がリテラルで漏れる事例を実測）
                        if actions.iter().any(is_ime_switch_action) {
                            // 保留から遅れて出てきた切替キーは、その間に押された
                            // 反対側の親指に追い越される（BUG-103 追補: 英数 →
                            // 42ms 後に かな と打つと、flush された 英数 が かな の
                            // 後に着地して IME が意図と逆の OFF に落ちた）。
                            // 後から押された方をユーザーの最終意図とみなして捨てる
                            if self.is_superseded_switch(actions) {
                                if awase_macos::diagnostics::key_content_enabled() {
                                    log::debug!(
                                        "Dropping superseded IME switch action(s): {actions:?}"
                                    );
                                } else {
                                    log::debug!(
                                        "Dropping {} superseded IME switch action(s)",
                                        actions.len()
                                    );
                                }
                                continue;
                            }
                            for action in actions {
                                if let awase::types::KeyAction::Key(vk) = action {
                                    self.expect_ime_from_key(vk.0);
                                }
                            }
                            self.output.send_keys(actions);
                            continue;
                        }
                        // IME 切替中は旧入力ソースで解釈されてしまうため保留する
                        // （既に保留がある場合も順序維持のため追記する）
                        if self.ime.is_switch_pending() || !self.deferred_keys.is_empty() {
                            if self.deferred_keys.is_empty() {
                                self.deferred_focus_pid = frontmost_pid();
                                self.deferred_expect_on = self.ime.pending_open();
                                self.deferred_since = Some(Instant::now());
                            }
                            if self.deferred_keys.len() + actions.len() > DEFER_CAP {
                                log::warn!(
                                    "Deferred key queue over {DEFER_CAP} actions, \
                                     dropping new output"
                                );
                            } else {
                                self.deferred_keys.extend(actions.iter().cloned());
                            }
                            self.timers.set(FLUSH_TIMER_ID, FLUSH_RETRY);
                        } else {
                            // settle を実測で詰めるため、切替からの経過を残す
                            // （消えた／化けた打鍵がここの何 ms かを突き合わせる）
                            if let Some(since) = self.ime.since_switch_confirmed() {
                                log::debug!(
                                    "sending {} action(s) {}ms after the switch settled",
                                    actions.len(),
                                    since.as_millis(),
                                );
                            }
                            self.output.send_keys(actions);
                        }
                    }
                    Effect::Input(InputEffect::ReinjectKey(ev)) => {
                        if matches!(ev.event_type, KeyEventType::KeyDown) {
                            self.expect_ime_from_key(ev.vk_code.0);
                        }
                        self.output.reinject(ev.vk_code, ev.event_type);
                    }
                    Effect::Timer(TimerEffect::Set { id, duration }) => {
                        self.timers.set(*id, *duration);
                    }
                    Effect::Timer(TimerEffect::Kill(id)) => self.timers.kill(*id),
                    Effect::Ime(ImeEffect::SetOpen { open, origin }) => match origin {
                        // 明示的なユーザー操作（ime_on/off/toggle コンボ等）のみ実行する
                        SetOpenOrigin::ExplicitUserAction => {
                            if self.ime.set_ime_on(*open) {
                                log::debug!("IME set_open({open}) via TISSelectInputSource");
                            } else {
                                log::warn!("IME set_open({open}) failed: no matching input source");
                            }
                        }
                        // ActivationSync は activation 遷移の echo（SetOpenOrigin の doc
                        // 参照）。macOS では ctx.ime_on が毎イベント TIS 観測で得た
                        // 実状態そのものなので、echo を TISSelectInputSource で実行する
                        // と OS/IME 自身の切替と競合する（ATOK が OS 標準 IME に
                        // 化ける等）。観測駆動の macOS では無視するのが正しい。
                        SetOpenOrigin::ActivationSync => {
                            log::trace!("IME set_open({open}) echo (ActivationSync) ignored");
                        }
                    },
                    Effect::Ui(UiEffect::EngineStateChanged { enabled, .. }) => {
                        self.tray.set_enabled(*enabled);
                    }
                }
            }
        }

        fn apply_decision(&mut self, decision: Decision) -> TapAction {
            match decision {
                Decision::PassThrough => TapAction::Pass,
                Decision::PassThroughWith { effects } => {
                    self.run_effects(&effects);
                    TapAction::Pass
                }
                Decision::Consume { effects } => {
                    self.run_effects(&effects);
                    TapAction::Consume
                }
            }
        }

        /// 英数/かな キーが OS に届く時点で IME 状態の期待値を立てる。
        ///
        /// 入力ソース切替は非同期のため、TIS 観測が追いつく前の打鍵が
        /// 旧状態で判定される（英字モード→かな→即 k で k が素通りする）
        /// のを防ぐ。
        ///
        /// 期待値を立てた直後に activation の再評価も走らせる。ポーリング
        /// （既定 500ms）任せだと、切替後に打鍵を止めた場合にトレイ表示が
        /// 最大 0.5 秒古いまま残るため。
        fn expect_ime_from_key(&mut self, keycode: u16) {
            match keycode {
                KEYCODE_EISU => self.ime.expect_ime_on(false),
                KEYCODE_KANA => self.ime.expect_ime_on(true),
                _ => return,
            }
            // 新しい切替キーを観測したので、張り直しの回数制限を仕切り直す
            self.switch_recoveries = 0;
            let ctx = self.make_ctx();
            let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
            let _ = self.apply_decision(decision);
        }

        /// FlagsChanged イベントから修飾キーの押下/解放を求める。
        fn flags_changed_event_type(
            keycode: u16,
            event: &CGEvent,
        ) -> Option<(ModifierKey, KeyEventType)> {
            let mk = hook::classify_modifier(keycode)?;
            let flags = event.get_flags();
            let bit = match mk {
                ModifierKey::Shift => CGEventFlags::CGEventFlagShift,
                ModifierKey::Ctrl => CGEventFlags::CGEventFlagControl,
                ModifierKey::Alt => CGEventFlags::CGEventFlagAlternate,
                ModifierKey::Meta => CGEventFlags::CGEventFlagCommand,
            };
            let event_type = if flags.contains(bit) {
                KeyEventType::KeyDown
            } else {
                KeyEventType::KeyUp
            };
            Some((mk, event_type))
        }
    }

    impl LoopHandler for App {
        fn on_cg_event(&mut self, etype: CGEventType, event: &CGEvent) -> TapAction {
            // 切替待ちの保留出力があれば、後続イベント処理の前に順序を保って流す
            self.maybe_flush_deferred();
            // 効かなかった切替キーの張り直しも、この打鍵を判定する前に済ませる
            self.recover_failed_switch();

            // クリックは IME の未確定文字列を確定させる（composing ヒントの
            // 主要なクリア漏れだった。Enter を打たない確定スタイルへの対応）。
            // 同一アプリ内でもキャレットが動いた可能性があるため保留出力も破棄する
            if matches!(
                etype,
                CGEventType::LeftMouseDown
                    | CGEventType::RightMouseDown
                    | CGEventType::OtherMouseDown
            ) {
                self.output.note_composition_break();
                self.discard_deferred("mouse click during IME switch");
                return TapAction::Pass;
            }

            let keycode =
                u16::try_from(event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE))
                    .unwrap_or(u16::MAX);

            // 自分自身の注入イベントは Engine に通さず素通しする
            // （親指キー単独打鍵の再注入もここを通って OS の IME 切替に届く）
            if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == INJECT_MARKER {
                // 注入イベントが OS のイベントストリームに実在した証跡
                // （CGEventPost 後の消失と IME 側での無視を切り分ける）
                if awase_macos::diagnostics::key_content_enabled() {
                    log::debug!("inj-tap 0x{keycode:02X} {etype:?}");
                } else {
                    log::debug!("inj-tap {etype:?}");
                }
                if matches!(etype, CGEventType::KeyDown) {
                    self.expect_ime_from_key(keycode);
                }
                return TapAction::Pass;
            }

            let event_type = match etype {
                CGEventType::KeyDown => KeyEventType::KeyDown,
                CGEventType::KeyUp => KeyEventType::KeyUp,
                CGEventType::FlagsChanged => {
                    // 修飾キーはフラグ遷移から down/up を合成する
                    match Self::flags_changed_event_type(keycode, event) {
                        Some((_, et)) => et,
                        None => return TapAction::Pass, // CapsLock 等は素通し
                    }
                }
                _ => return TapAction::Pass,
            };

            let (key_classification, physical_pos) = hook::classify_key(keycode);
            // 修飾キー自身かどうか（`passthrough_moves_focus` の doc 参照）
            let is_modifier_key = hook::classify_modifier(keycode).is_some();
            let is_down = matches!(event_type, KeyEventType::KeyDown);

            let raw = RawKeyEvent {
                vk_code: VkCode(keycode),
                scan_code: ScanCode(u32::from(keycode)),
                event_type,
                extra_info: 0,
                timestamp: now_timestamp(),
                key_classification,
                physical_pos,
                ime_relevance: hook::classify_ime_relevance(keycode),
                modifier_key: hook::classify_modifier(keycode),
                modifier_snapshot: self.modifiers,
                // CGEventTap では他プロセス注入の確実な識別手段がないため false 固定
                injected: false,
            };

            self.modifiers.update(&raw);

            // auto-repeat KeyDown では最初のタイムスタンプを上書きしない
            // （Linux/Windows 実装と同じセマンティクス。上書きすると
            // `left_thumb_consumed` との比較で「消費済み」が剥がれる）。
            match key_classification {
                KeyClassification::LeftThumb => {
                    self.left_thumb_down = if is_down {
                        self.left_thumb_down.or(Some(raw.timestamp))
                    } else {
                        None
                    };
                }
                KeyClassification::RightThumb => {
                    self.right_thumb_down = if is_down {
                        self.right_thumb_down.or(Some(raw.timestamp))
                    } else {
                        None
                    };
                }
                KeyClassification::Char | KeyClassification::Passthrough => {}
            }

            let ctx = self.make_ctx();

            // 確定/取消キー（Enter・keypad Enter・Escape・Tab）の通過と IME OFF は
            // composition の切れ目とみなす（Output::composing_hint の doc 参照）
            if (is_down && matches!(keycode, 0x24 | 0x4C | 0x35 | 0x30)) || !ctx.ime_on {
                self.output.note_composition_break();
            }

            let decision = self.engine.on_input(raw, &ctx);
            // 取りこぼし調査用: 物理イベントと判定の対応を RUST_LOG=debug で追える
            let decision_name = match &decision {
                Decision::PassThrough => "pass",
                Decision::PassThroughWith { .. } => "pass+fx",
                Decision::Consume { .. } => "consume",
            };
            if awase_macos::diagnostics::key_content_enabled() {
                log::debug!(
                    "phys 0x{keycode:02X} {event_type:?} {key_classification:?} -> {decision_name}"
                );
            } else {
                log::debug!("phys {event_type:?} {key_classification:?} -> {decision_name}");
            }
            let action = self.apply_decision(decision);

            // 保留開始時と同じ PID でも、Tab / Cmd+L 等のパススルー操作で focused
            // control は変わりうる。新しい control へ保留文字を誤注入しないよう、
            // 外部イベントを通す時点で入力コンテキストを失効させる。
            // `apply_decision` の **後** に判定するのは、このイベント自身が
            // `PassThroughWith(SendKeys(..))` で積んだ出力も宛先を失うため
            // （activation 遷移フラッシュや bypass 経由の `flush_pending` が
            // この形で SendKeys を返す）。
            if passthrough_moves_focus(action, keycode, event_type, self.modifiers, is_modifier_key)
            {
                self.discard_deferred("focus-moving passthrough key during IME switch");
            }

            // 物理の英数/かな キーが素通しで OS に届く場合（Engine 非活性時や
            // 親指キー以外に設定されている場合）も IME 切替の期待を立てる
            if action == TapAction::Pass && is_down {
                self.expect_ime_from_key(keycode);
            }
            action
        }

        fn on_timer_fired(&mut self, id: usize) {
            self.timers.fired(id);
            if id == FLUSH_TIMER_ID {
                if self.ime.is_switch_pending() {
                    // まだ切替中: 再確認をスケジュール（期待の猶予超過で
                    // is_switch_pending が false になるため無限には続かない）
                    self.timers.set(FLUSH_TIMER_ID, FLUSH_RETRY);
                } else {
                    self.maybe_flush_deferred();
                }
                return;
            }
            let ctx = self.make_ctx();
            let decision = self.engine.on_timeout(id, &ctx);
            // タイムアウトには「現在のイベント」が無いため Pass/Consume は無意味
            let _ = self.apply_decision(decision);
        }

        fn on_menu_action(&mut self, action: MenuAction) {
            match action {
                MenuAction::ToggleEngine => {
                    let ctx = self.make_ctx();
                    let decision = self.engine.on_command(EngineCommand::ToggleEngine, &ctx);
                    // メニュー操作にも「現在のイベント」は無い
                    let _ = self.apply_decision(decision);
                }
                MenuAction::ToggleLoginItem => {
                    let _ = awase_macos::login_item::toggle();
                    self.tray.sync_login_item();
                }
            }
        }

        fn on_poll(&mut self) {
            self.maybe_flush_deferred();
            // 打鍵が来なくても IME が戻るように、ポーリングでも張り直す
            self.recover_failed_switch();
            // activation 遷移の検知はキーイベント経由でしか起きないため、
            // IME 切替後に打鍵が無いとトレイ表示が古いまま残る。RefreshState で
            // 遷移チェックだけを走らせ、UiEffect でトレイを追随させる。
            let ctx = self.make_ctx();
            let decision = self.engine.on_command(EngineCommand::RefreshState, &ctx);
            let _ = self.apply_decision(decision);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{passthrough_moves_focus, KeyEventType, ModifierState, TapAction};

        const TAB: u16 = 0x30;
        const ENTER: u16 = 0x24;
        const KEYPAD_ENTER: u16 = 0x4C;
        const ESCAPE: u16 = 0x35;
        const BACKSPACE: u16 = 0x33;
        const LEFT_ARROW: u16 = 0x7B;
        const L: u16 = 0x25;
        /// kVK_Control（修飾キー自身）
        const CONTROL: u16 = 0x3B;

        fn moves_focus(keycode: u16, modifiers: ModifierState, is_modifier_key: bool) -> bool {
            passthrough_moves_focus(
                TapAction::Pass,
                keycode,
                KeyEventType::KeyDown,
                modifiers,
                is_modifier_key,
            )
        }

        fn held(modifier: fn(&mut ModifierState)) -> ModifierState {
            let mut m = ModifierState::default();
            modifier(&mut m);
            m
        }

        #[test]
        fn focus_moving_keys_invalidate_the_deferred_queue() {
            for key in [TAB, ENTER, KEYPAD_ENTER, ESCAPE] {
                assert!(
                    moves_focus(key, ModifierState::default(), false),
                    "{key:#x}"
                );
            }
            // Cmd+L / Ctrl+L / Option+L 等のショートカットは別 control へ飛ばす
            for set in [
                (|m: &mut ModifierState| m.win = true) as fn(&mut ModifierState),
                |m: &mut ModifierState| m.ctrl = true,
                |m: &mut ModifierState| m.alt = true,
            ] {
                assert!(moves_focus(L, held(set), false));
            }
        }

        /// フォーカスを移さない編集キーで破棄すると、保留中の文字が消えたうえで
        /// Backspace が「その前の文字」を消す。利得が無く実害だけがある。
        #[test]
        fn plain_editing_keys_keep_the_deferred_queue() {
            assert!(!moves_focus(BACKSPACE, ModifierState::default(), false));
            assert!(!moves_focus(LEFT_ARROW, ModifierState::default(), false));
            // Shift 単独はフォーカスを動かさない（Shift+Tab は Tab 側で捕まる）
            assert!(!moves_focus(BACKSPACE, held(|m| m.shift = true), false));
        }

        /// `modifiers` は判定前に `update()` 済みなので、Ctrl 自身の KeyDown でも
        /// `is_os_modifier_held()` は真になる。区別しないと Ctrl を押して離すだけで
        /// 保留中の入力が消える。
        #[test]
        fn a_bare_modifier_key_press_keeps_the_deferred_queue() {
            assert!(!moves_focus(CONTROL, held(|m| m.ctrl = true), true));
        }

        #[test]
        fn consumed_keys_and_key_up_never_move_focus() {
            assert!(!passthrough_moves_focus(
                TapAction::Consume,
                TAB,
                KeyEventType::KeyDown,
                ModifierState::default(),
                false
            ));
            assert!(!passthrough_moves_focus(
                TapAction::Pass,
                TAB,
                KeyEventType::KeyUp,
                held(|m| m.win = true),
                false
            ));
        }
    }
}
