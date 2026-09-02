#!/usr/bin/env bash
# Install dist/Awase.app with a clean TCC slate.
#
# Destination: the first argument, else $AWASE_INSTALL_DIR, else /Applications
# when it is writable — admin accounts have group write on it, so no sudo is
# needed — falling back to ~/Applications for standard accounts.
#
# Why not plain `cp -R`: copying over an existing bundle MERGES old and new
# contents, which breaks the code signature — and the TCC (Accessibility)
# entry keeps pointing at the previous signature, so toggling the checkbox
# in System Settings has no effect on the new binary. The reliable sequence
# is: quit, remove, copy whole, reset the stale TCC entry, re-grant.
set -euo pipefail

cd "$(dirname "$0")/../.."

APP=dist/Awase.app

if [[ -n ${1:-} ]]; then
  INSTALL_DIR=$1
elif [[ -n ${AWASE_INSTALL_DIR:-} ]]; then
  INSTALL_DIR=$AWASE_INSTALL_DIR
elif [[ -w /Applications ]]; then
  INSTALL_DIR=/Applications
else
  INSTALL_DIR=$HOME/Applications
fi

if [[ ! -d "$APP" ]]; then
  echo "error: $APP not found — run ./packaging/macos/make-app.sh first" >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR" 2>/dev/null || true
if [[ ! -d "$INSTALL_DIR" || ! -w "$INSTALL_DIR" ]]; then
  echo "error: cannot write to $INSTALL_DIR — pick another directory, e.g." >&2
  echo "  $0 \"\$HOME/Applications\"" >&2
  exit 1
fi
# Absolute and slash-normalized, so the duplicate-install check below compares
# against the same spelling the caller sees in the messages.
INSTALL_DIR=$(cd "$INSTALL_DIR" && pwd)
DEST=$INSTALL_DIR/Awase.app

# Quit a running instance so we don't replace a busy binary
osascript -e 'quit app "awase"' 2>/dev/null || true

rm -rf "$DEST"
ditto "$APP" "$DEST"

# Drop the Accessibility entry tied to the old signature. Without this the
# System Settings toggle operates on a dead entry. Ignore failure (first
# install has no entry yet).
tccutil reset Accessibility com.github.cuzic.awase || true

echo "Installed $DEST"
echo "Launch it (open $DEST) and grant Accessibility when prompted."
echo "Tip: build with CODESIGN_IDENTITY=<self-signed cert> to keep the"
echo "grant across rebuilds and skip the re-grant entirely."

# A copy left in the other location keeps its own TCC entry, and Login Items /
# LaunchAgent may still point at it — the stale build would keep running.
for other in /Applications/Awase.app "$HOME/Applications/Awase.app"; do
  if [[ $other != "$DEST" && -d $other ]]; then
    echo "warning: another install exists at $other — remove it, or check that" >&2
    echo "         Login Items / LaunchAgent point at $DEST" >&2
  fi
done
