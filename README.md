# pkgd

**universal release installer for linux (and macos)**  
by [sudoxreboot](https://github.com/sudoxreboot)

---

## what it is

pkgd is a shell command + gui app that installs software directly from github releases, gitlab releases, or direct urls — automatically detecting your os/distro/arch and picking the right package type. no hunting release pages, no manual wget, no version pinning.

---

## features

- **os-aware** — detects debian/ubuntu/fedora/arch/macos and picks the right package format automatically
- **arch-aware** — handles x86_64, arm64, armhf
- **multi-source** — github, gitlab, direct urls
- **asset type support** — `.deb`, `.rpm`, `.AppImage`, `.tar.gz`, `.tar.xz`, `.zip`, raw binaries, `.dmg`
- **dep resolution** — for debs: tries apt first, then stubs unavailable packages with `equivs` (handles broken deps on newer ubuntu like `libgdk-pixbuf2.0-0`)
- **smart fallback** — if deb fails, falls back to zip/appimage automatically
- **desktop entries** — creates `.desktop` files and refreshes plasma/gnome app cache on `-i`
- **gui** — tauri-based gui with asset priority picker, live install log, and source browser
- **any shell** — standalone bash script, works in zsh/bash/fish/any shell
- **curl installable** — one line to get pkgd itself

---

## install pkgd

**curl one-liner (recommended)**
```bash
curl -fsSL https://raw.githubusercontent.com/sudoxreboot/pkgd/main/install.sh | bash
```

**git clone**
```bash
git clone https://github.com/sudoxreboot/pkgd.git
cd pkgd
sudo cp pkgd /usr/local/bin/pkgd
sudo chmod +x /usr/local/bin/pkgd
```

**install pkgd with pkgd** (once you have it)
```bash
pkgd -i sudoxreboot/pkgd
```

---

## cli usage

```
pkgd [options] <repo|url>
```

| option | description |
|---|---|
| `-i` | install after download |
| `--deb` | force deb package |
| `--rpm` | force rpm package |
| `--appimage` | force appimage |
| `--tar` | force tar archive |
| `--zip` | force zip archive |
| `--bin` | force raw binary |
| `--list` | list all available assets without downloading |
| `--version` | show version |
| `-h` / `--help` | show help |

**examples**
```bash
# just download to /tmp
pkgd balena-io/etcher

# download and install
pkgd -i balena-io/etcher

# full github url
pkgd -i https://github.com/cli/cli

# gitlab
pkgd -i https://gitlab.com/inkscape/inkscape

# direct url
pkgd -i https://example.com/myapp-linux-x64.tar.gz

# force a specific asset type
pkgd -i --appimage balena-io/etcher

# list all assets for a release
pkgd --list jesseduffield/lazydocker
```

---

## automatic package priority by os

| distro | priority order |
|---|---|
| debian / ubuntu / kubuntu / mint / pop | deb → appimage → tar → zip → bin |
| fedora / rhel / centos / rocky / opensuse | rpm → appimage → tar → zip → bin |
| arch / manjaro / endeavouros | appimage → tar → zip → bin |
| macos | dmg → zip → tar → bin |
| unknown | appimage → tar → zip → bin |

---

## dep resolution (debian-based)

when a `.deb` install fails due to missing dependencies:

1. runs `apt --fix-broken install`
2. parses the deb's dependency list
3. tries `apt install` for each missing dep
4. if a dep is genuinely unavailable on the current distro version (e.g. `libgdk-pixbuf2.0-0` on ubuntu 24.04), stubs it with an `equivs` dummy package
5. retries install
6. if still failing, falls back to the linux zip asset if one exists

---

## gui

the pkgd gui is a [tauri](https://tauri.app) app (rust + webview).

**features**
- search/paste any github, gitlab, or direct url
- browse all available assets for a release
- drag to reorder asset type priority per-install
- live install log with color output
- os detection display
- install history

**design**
- `#1f1f1f` background with lilac grid overlay
- `#88ffee` primary text
- `#aaaaff` secondary text
- jetbrains mono + syne

**build the gui**
```bash
# prereqs: rust, node 18+
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
npm install
npm run tauri build
```

**run in dev**
```bash
npm run tauri dev
```

---

## repo structure

```
pkgd/
├── pkgd                      # standalone shell command
├── install.sh                # curl one-liner installer
├── package.json
├── vite.config.js
├── src/                      # tauri frontend (html/css/js)
│   └── index.html
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   └── src/
│       ├── main.rs
│       └── lib.rs
└── README.md
```

---

## roadmap

- [ ] `pkgd update` — check and update already-installed pkgd-managed apps
- [ ] `pkgd list` — show installed pkgd-managed packages
- [ ] `pkgd remove` — uninstall
- [ ] aur helper integration for arch
- [ ] flatpak ref support
- [ ] zshrc function bundled for shell-native usage
- [ ] gui: saved favorites / quick-install list
- [ ] gui: auto-update checker for managed packages
- [ ] windows support (winget/exe fallback)

---

## license

mit — do whatever you want with it.  
built by [sudoxreboot](https://github.com/sudoxreboot) | [sudoxreboot.studio](https://sudoxreboot.com)




