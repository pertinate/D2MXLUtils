## D2MXLUtils

**D2MXLUtils** is an overlay utility for *Diablo II: Median XL* that provides drop notifications and additional helper tools.

Technologies:
- **Frontend**: `Svelte` + `Vite`
- **Desktop shell**: `Tauri`
- **Backend**: `Rust`

### Development setup

Requirements:
- `Windows 10\11`, or Linux (see below — experimental)
- `Node.js` (LTS recommended)
- `pnpm` (see `packageManager` in `package.json`)
- `Rust` toolchain and required native build tools for Tauri (see official Tauri documentation)

#### Linux (experimental)

Median XL runs under Wine/Proton; D2MXLUtils itself runs natively and attaches
to the game process directly (not via Wine), so it works regardless of which
Wine/Proton build or prefix the game uses. Beyond Tauri's own Linux
prerequisites (webkit2gtk, gtk3 — see the official Tauri docs), two things
are easy to miss and both fail silently rather than with an obvious error:

- **`gst-plugins-good`** — required for drop-notification sound playback.
  WebKitGTK plays audio via GStreamer; without this package, MP3 playback
  fails silently (missing ID3 tag demuxer and default audio sink). Install
  via your distro's package manager, e.g. `sudo pacman -S gst-plugins-good`
  on Arch/CachyOS, `sudo apt install gstreamer1.0-plugins-good` on Debian/Ubuntu.
- **`kernel.yama.ptrace_scope`** — the game-process attach/read/inject
  mechanism uses `ptrace`, which the kernel's Yama LSM restricts to a
  process's own children by default on most distros. Since D2MXLUtils isn't
  a parent of the game process, relax this for the current boot with
  `sudo sysctl kernel.yama.ptrace_scope=0`, or persist it via a file in
  `/etc/sysctl.d/`. Without this, the app fails to attach to the game at all.

Install dependencies:

```bash
pnpm install
```

Run in development:

```bash
pnpm dev         # start Vite dev server (frontend only)
pnpm tauri dev   # start the Tauri desktop app in dev mode
```

Build:

```bash
pnpm build       # build frontend
pnpm tauri build # build Tauri desktop app
```

Tauri packaging/bundling is configured under `src-tauri`; refer to Tauri docs and project scripts when adding release builds.

### Release

To create a new release:

```bash
# Bump version (choose one):
pnpm version patch   # 0.1.0 → 0.1.1 (bugfixes)
pnpm version minor   # 0.1.0 → 0.2.0 (new features)
pnpm version major   # 0.1.0 → 1.0.0 (breaking changes)

# Push with tag:
git push --follow-tags
```

This will:
1. Update version in `package.json`, `Cargo.toml`, `Cargo.lock`, and `tauri.conf.json`
2. Create a commit and git tag (e.g. `v0.2.0`)
3. Trigger GitHub Actions pipeline that builds the app and creates a GitHub Release with downloadable binaries

### Project structure (short)

- `src/` — Svelte application and styles
- `src-tauri/` — Rust code and Tauri configuration
- `docs/` — notes, plans, and additional project documents
