# awase on macOS — packaging and autostart

## Build and install the app bundle

```sh
./packaging/macos/make-app.sh
./packaging/macos/install-app.sh
```

`install-app.sh` installs to `/Applications` when that directory is writable
(admin accounts have group write on it, so no `sudo` is involved) and to
`~/Applications` otherwise — a standard account without admin rights can
install without help. Override with an argument or `AWASE_INSTALL_DIR`:

```sh
./packaging/macos/install-app.sh ~/Applications
AWASE_INSTALL_DIR=~/Applications ./packaging/macos/install-app.sh
```

The Accessibility grant follows the bundle's signature and bundle ID, not its
directory, so a `~/Applications` install behaves the same. The paths below
spell out `/Applications`; substitute your destination if you moved it.

Do not `cp -R` over an existing `Awase.app`: cp merges old and
new bundle contents, which breaks the code signature, and the stale TCC entry
makes the Accessibility toggle in System Settings a no-op. The install script
quits the running instance, replaces the bundle wholesale, and runs
`tccutil reset Accessibility com.github.cuzic.awase` so the next launch
prompts for a fresh grant.

The bundle embeds `config.toml` and `layout/` under `Contents/Resources/`
(the binary also finds them next to the executable or in the current
directory, in that order of precedence — see `resolve_resource` in
`crates/awase-macos/src/main.rs`).

## Grant permissions (first launch)

1. Launch `/Applications/Awase.app` once; macOS shows the Accessibility prompt.
2. System Settings > Privacy & Security > **Accessibility** — enable Awase.
3. If the event tap still fails, also enable Awase under **Input Monitoring**.
4. Relaunch the app. An 「あ」 icon appears in the menu bar.

**Note on rebuilds:** the default ad-hoc signature changes on every rebuild,
so macOS silently drops the Accessibility grant — reinstall with
`install-app.sh` (which resets the stale TCC entry) and grant again. For a
stable identity that survives rebuilds, create a self-signed code-signing
certificate (Keychain Access > Certificate Assistant, type "Code Signing",
e.g. named `awase-codesign`) and build with:

```sh
CODESIGN_IDENTITY=awase-codesign ./packaging/macos/make-app.sh
```

For distributing binaries to others, sign with a Developer ID Application
certificate, enable the Hardened Runtime, and notarize — an app holding
Accessibility access should have a verifiable publisher (see
[TN3127](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements/)).

## Start at login

Pick **one** of the following:

- **Login Items** (simplest): System Settings > General > Login Items >
  add `Awase.app`.
- **LaunchAgent** (restarts on crash, logs to `~/Library/Logs/Awase/`):

  ```sh
  mkdir -p ~/Library/Logs/Awase && chmod 700 ~/Library/Logs/Awase
  APP_DIR=/Applications  # or ~/Applications, wherever install-app.sh put it
  sed -e "s|USERNAME|$USER|g" -e "s|/Applications/Awase.app|$APP_DIR/Awase.app|g" \
    packaging/macos/com.github.cuzic.awase.plist \
    > ~/Library/LaunchAgents/com.github.cuzic.awase.plist
  launchctl load ~/Library/LaunchAgents/com.github.cuzic.awase.plist
  ```

  Unload with `launchctl unload ~/Library/LaunchAgents/com.github.cuzic.awase.plist`.

## Confirm modes

`wait` (default) and `speculative` are verified on macOS with ATOK —
speculative's Backspace-and-replace works against the IME preedit. The
difference is hard to perceive (wait's worst-case delay is the
simultaneity threshold, ~100 ms); pick with `confirm_mode` in config.toml.
`two_phase` and the adaptive modes are untested on macOS.

## Notes

- Quit from the menu bar icon (awase を終了) or `launchctl unload`.
- Thumb keys default to 英数 (left) / かな (right); the IME must be in
  romaji-input hiragana mode for kana-kanji conversion to work.
- Secure input fields (password boxes) bypass the event tap by OS design.
