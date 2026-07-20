# Codex Usage Overlay

A lightweight Windows overlay for live Codex allowance percentages, reset times,
daily and lifetime token activity, and additional credits. It uses the installed
Codex CLI's supported `app-server` interface and never reads or copies Codex
authentication files.

## What it shows

- Every allowance window returned for the main Codex quota bucket
- Percentage remaining and a live reset countdown
- Today's and lifetime token activity in the expanded view
- Credit and spend-control information when the account provides it
- A visible stale state while the local app-server reconnects

Included-plan quotas are deliberately labelled as **allowance remaining**. Codex
does not expose an absolute token capacity for those windows, so this app never
claims to know an exact number of tokens left.

## Requirements

- Windows 10 or Windows 11, x64
- An installed Codex CLI. The app checks PATH, common user install folders, and
  VS Code/Cursor extension installs, or its path can be selected in the app
- A ChatGPT account signed into Codex
- Node.js 20 or newer and Rust 1.77.2 or newer to build from source

## Development

```powershell
npm.cmd install
npm.cmd run tauri dev
```

The overlay starts in the top-right corner, stays above normal windows, and can
be dragged by its header. Closing it hides it to the tray. Use the tray's **Quit**
command to terminate the overlay and its private app-server child process.

On first launch, Start with Windows is enabled. It can be disabled from the
expanded overlay or tray menu.

### Build the Windows executable

```powershell
npm.cmd run build:exe
```

This runs the test suite, builds the release app, creates the NSIS installer,
and copies both outputs into `artifacts\`:

- `Codex Usage Overlay.exe` — standalone release executable
- `Codex Usage Overlay_<version>_x64-setup.exe` — Windows installer

Use `powershell -NoProfile -ExecutionPolicy Bypass -File
scripts\build-executable.ps1 -SkipTests` when rebuilding after tests have
already passed.

## Verification

```powershell
npm.cmd test
npm.cmd run build
cargo test --manifest-path src-tauri\Cargo.toml
cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings
npm.cmd run tauri build
```

An opt-in live smoke uses the existing Codex login but prints only response-shape
counts, never raw usage:

```powershell
npm.cmd run test:live-codex
```

## Protocol maintenance

The app uses stable app-server methods and runtime capability checks. Generate a
fresh TypeScript protocol snapshot after upgrading Codex:

```powershell
npm.cmd run protocol:generate
```

Generated files are intentionally ignored; `codex-protocol.lock.json` records the
Codex version against which the protocol was last reviewed.

## Troubleshooting

- **Codex CLI not found:** The app checks PATH, `%USERPROFILE%\.vscode\extensions`,
  VS Code Insiders/Cursor extension folders, and common Codex/OpenAI install
  folders. If Codex was installed after the app started, use **Use Codex from
  PATH** to retry or choose the installed `codex.exe` directly.
- **Sign-in required:** Select **Sign in with Codex**. Authentication remains
  managed by Codex in the system browser.
- **Reconnecting:** The last in-memory snapshot remains visible and marked stale.
  Use **Refresh now** after network access returns.
- **No 5-hour row:** The app maps Codex's primary rate-limit window to the
  five-hour tracker, including responses where the server omits the duration
  metadata. If primary is absent entirely, Codex has not provided a valid
  five-hour percentage to display.
