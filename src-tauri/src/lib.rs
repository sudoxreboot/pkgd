use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

// ─── types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: Option<u64>,
    pub asset_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub assets: Vec<Asset>,
    pub published_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InstalledPkg {
    pub repo: String,
    /// actual installed package / binary name (may differ from repo name for debs)
    #[serde(default)]
    pub name: String,
    pub version: String,
    /// bash script used to write "type"; accept both field names
    #[serde(alias = "type")]
    pub asset_type: String,
    pub url: String,
    /// ISO-8601 timestamp of when pkgd installed this package
    pub date: String,
    /// ISO-8601 release publish date from GitHub
    #[serde(default)]
    pub date_released: String,
    /// When true, pkgd skips update checks for this package
    #[serde(default)]
    pub locked: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Settings {
    pub priority: Vec<String>,
    pub excluded_types: Vec<String>,
    pub github_token: Option<String>,
    pub install_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            priority: vec![
                "deb".into(),
                "appimage".into(),
                "tar".into(),
                "zip".into(),
                "bin".into(),
                "apt".into(),
                "flatpak".into(),
                "snap".into(),
                "dnf".into(),
                "pacman".into(),
            ],
            excluded_types: vec!["other".into(), "win".into()],
            github_token: None,
            install_dir: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoSearchResult {
    pub full_name: String,
    pub description: String,
    pub stars: u64,
    pub url: String,
    pub has_releases: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemPkg {
    pub name: String,
    pub version: String,
    pub manager: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SysPkgResult {
    pub name: String,
    pub description: String,
    pub manager: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateStatus {
    pub repo: String,
    pub current: String,
    pub latest: String,
    pub has_update: bool,
}

// ─── helpers ──────────────────────────────────────────────────────────────────

fn pkgd_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".pkgd")
}

fn settings_path() -> std::path::PathBuf {
    pkgd_dir().join("settings.json")
}

fn db_path() -> std::path::PathBuf {
    pkgd_dir().join("installed.json")
}

fn infer_type(name: &str) -> String {
    let n = name.to_lowercase();
    // installable packages
    if n.ends_with(".deb")      { return "deb".into(); }
    if n.ends_with(".rpm")      { return "rpm".into(); }
    if n.ends_with(".appimage") { return "appimage".into(); }
    // source archives (not binary releases — classify as other before tar check)
    if (n.ends_with(".tar.gz") || n.ends_with(".tar.xz") || n.ends_with(".tar.bz2") || n.ends_with(".tgz"))
        && (n.contains(".src.") || n.contains("-src.") || n.contains("-source.") || n.contains(".source."))
    { return "other".into(); }
    if n.ends_with(".tar.gz") || n.ends_with(".tar.xz") || n.ends_with(".tar.bz2")
        || n.ends_with(".tar.zst") || n.ends_with(".tgz")
    { return "tar".into(); }
    if n.ends_with(".zip")      { return "zip".into(); }
    // macOS
    if n.ends_with(".dmg") || n.ends_with(".pkg") { return "dmg".into(); }
    // Windows executables
    if n.ends_with(".exe") || n.ends_with(".msi") || n.ends_with(".bat")
        || n.ends_with(".nupkg") || n.ends_with(".msix")
    { return "win".into(); }
    // checksums, signatures, metadata — never installable
    if n.ends_with(".sha256") || n.ends_with(".sha512") || n.ends_with(".md5")
        || n.ends_with(".sig")  || n.ends_with(".asc")  || n.ends_with(".pem")
        || n.ends_with(".txt")  || n.ends_with(".json") || n.ends_with(".yaml")
        || n.ends_with(".toml") || n.ends_with(".md")   || n.ends_with(".zsync")
        || n.ends_with(".blockmap") || n.ends_with(".p7s")
    { return "other".into(); }
    // raw binary (no or unrecognised extension)
    "bin".into()
}

// ─── inner fetch helper ───────────────────────────────────────────────────────

async fn fetch_release_inner(
    repo: &str,
    version: Option<&str>,
    token: Option<&str>,
) -> Result<Release, String> {
    let v = version.unwrap_or("latest");
    let url = if v == "latest" {
        format!("https://api.github.com/repos/{}/releases/latest", repo)
    } else {
        format!("https://api.github.com/repos/{}/releases/tags/{}", repo, v)
    };

    let client = reqwest::Client::new();
    let mut req = client
        .get(&url)
        .header("User-Agent", "pkgd/0.1.0")
        .header("Accept", "application/vnd.github+json");

    if let Some(tok) = token {
        if !tok.is_empty() {
            req = req.header("Authorization", format!("token {}", tok));
        }
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API error: {}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let tag = json["tag_name"].as_str().unwrap_or("unknown").to_string();
    let name = json["name"].as_str().unwrap_or(&tag).to_string();
    let published_at = json["published_at"].as_str().map(|s| s.to_string());

    let assets = json["assets"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let aname = a["name"].as_str()?.to_string();
                    let aurl = a["browser_download_url"].as_str()?.to_string();
                    let size = a["size"].as_u64();
                    let asset_type = infer_type(&aname);
                    Some(Asset { name: aname, url: aurl, size, asset_type })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Release { tag, name, assets, published_at })
}

// ─── tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
async fn fetch_release(
    repo: String,
    version: Option<String>,
    token: Option<String>,
) -> Result<Release, String> {
    fetch_release_inner(&repo, version.as_deref(), token.as_deref()).await
}

#[tauri::command]
async fn check_update(
    repo: String,
    current_version: String,
    token: Option<String>,
) -> Result<UpdateStatus, String> {
    let release = fetch_release_inner(&repo, None, token.as_deref()).await?;
    let has_update = release.tag != current_version && current_version != "direct";
    Ok(UpdateStatus {
        repo: repo.clone(),
        current: current_version,
        latest: release.tag,
        has_update,
    })
}

#[tauri::command]
async fn search_github(
    name: String,
    token: Option<String>,
) -> Result<Vec<RepoSearchResult>, String> {
    // simple URL-safe encoding: replace spaces, leave alphanumeric and common chars
    let encoded: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c.to_string() } else { format!("%{:02X}", c as u32) })
        .collect();

    let url = format!(
        "https://api.github.com/search/repositories?q={}+in:name&sort=stars&order=desc&per_page=12",
        encoded
    );

    let client = reqwest::Client::new();
    let mut req = client
        .get(&url)
        .header("User-Agent", "pkgd/0.1.0")
        .header("Accept", "application/vnd.github+json");

    if let Some(tok) = &token {
        if !tok.is_empty() {
            req = req.header("Authorization", format!("token {}", tok));
        }
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GitHub search API error: {}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let results = json["items"]
        .as_array()
        .map(|items| {
            items.iter().filter_map(|item| {
                let full_name = item["full_name"].as_str()?.to_string();
                let description = item["description"].as_str().unwrap_or("").to_string();
                let stars = item["stargazers_count"].as_u64().unwrap_or(0);
                let url = item["html_url"].as_str()?.to_string();
                // optimistically assume repos with stars have releases
                let has_releases = item["has_downloads"].as_bool().unwrap_or(true);
                Some(RepoSearchResult { full_name, description, stars, url, has_releases })
            }).collect()
        })
        .unwrap_or_default();

    Ok(results)
}

#[tauri::command]
async fn run_pkgd(window: tauri::Window, args: Vec<String>) -> Result<(), String> {
    let mut child = Command::new("pkgd")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to launch pkgd: {}", e))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let win1 = window.clone();
    let win2 = window.clone();

    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = win1.emit("pkgd-log", line);
        }
    });

    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let t = line.trim();
            if t.is_empty() { continue; }
            // skip wget/curl download progress bar lines (contain % + progress chars)
            if t.contains('%') && (t.contains('[') || t.contains('=') || t.contains("KB/s") || t.contains("MB/s")) {
                continue;
            }
            let _ = win2.emit("pkgd-log", &line);
        }
    });

    let status = child.wait().await.map_err(|e| e.to_string())?;
    if status.success() {
        let _ = window.emit("pkgd-done", "ok");
    } else {
        let _ = window.emit("pkgd-done", "error");
    }
    Ok(())
}

#[tauri::command]
fn load_settings() -> Settings {
    let path = settings_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Settings::default()
    }
}

#[tauri::command]
fn save_settings(settings: Settings) -> Result<(), String> {
    let dir = pkgd_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let data = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(settings_path(), data).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn load_installed() -> Vec<InstalledPkg> {
    let path = db_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        vec![]
    }
}

#[tauri::command]
fn detect_os() -> serde_json::Value {
    let arch = std::env::consts::ARCH.to_string();
    let os = std::env::consts::OS.to_string();
    let distro = if std::path::Path::new("/etc/os-release").exists() {
        std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("ID="))
                    .map(|l| l.trim_start_matches("ID=").trim_matches('"').to_string())
            })
            .unwrap_or_else(|| "linux".into())
    } else {
        os.clone()
    };
    serde_json::json!({ "os": os, "distro": distro, "arch": arch })
}

// ─── system package commands ──────────────────────────────────────────────────

/// Packages that must never be shown to the user regardless of dpkg Priority.
/// Removing any of these would break or destroy the running system.
const SYSTEM_BLOCKLIST: &[&str] = &[
    // init / service layer
    "systemd", "systemd-sysv", "systemd-timesyncd", "systemd-resolved",
    "systemd-networkd", "sysvinit-core", "openrc",
    // IPC / device management
    "dbus", "dbus-broker", "dbus-user-session", "udev", "eudev",
    // network (core stack — not user apps)
    "network-manager", "netplan.io", "iproute2", "iptables", "nftables",
    "ifupdown", "ifupdown2",
    // bootloader
    "grub-common", "grub-pc", "grub-pc-bin", "grub-efi-amd64",
    "grub-efi-amd64-bin", "grub2-common", "grub-rescue-pc",
    // initramfs
    "initramfs-tools", "initramfs-tools-core", "dracut", "dracut-core",
    // disk / encryption / fs
    "cryptsetup", "cryptsetup-initramfs", "cryptsetup-bin",
    "lvm2", "mdadm", "dmsetup", "e2fsprogs", "dosfstools",
    "xfsprogs", "btrfs-progs", "mount",
    // auth / privilege
    "sudo", "passwd", "login", "adduser", "base-passwd",
    "polkit", "policykit-1", "libpam-runtime",
    // display server (removing kills the GUI session)
    "xserver-xorg-core", "xwayland", "xorg",
    // display / login manager
    "gdm3", "sddm", "lightdm", "xdm", "lxdm", "ly",
    // audio subsystem
    "pulseaudio", "pipewire", "wireplumber",
    // kernel helpers
    "kmod", "procps", "util-linux",
];

/// Scan for OS BASE packages — what was installed by the OS itself.
/// These are Priority=required/important/standard packages.
/// Shown ONLY behind the "use at your own risk" toggle — dangerous to remove.
#[tauri::command]
async fn scan_system_packages() -> Vec<SystemPkg> {
    let mut pkgs: Vec<SystemPkg> = Vec::new();

    if let Ok(output) = std::process::Command::new("dpkg-query")
        .args(["-W", "--showformat=${db:Status-Status}\t${Priority}\t${Package}\t${Version}\t${binary:Summary}\n"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if !line.starts_with("installed\t") { continue; }
            let rest = &line["installed\t".len()..];
            let parts: Vec<&str> = rest.splitn(4, '\t').collect();
            let priority = parts.get(0).unwrap_or(&"").trim();
            let name     = parts.get(1).unwrap_or(&"").trim();
            let version  = parts.get(2).unwrap_or(&"").trim();
            let desc     = parts.get(3).unwrap_or(&"").trim();
            if name.is_empty() { continue; }
            // ONLY show OS base packages — what the OS shipped with
            if !matches!(priority, "required" | "important" | "standard") { continue; }
            // Filter lib* to reduce noise (still hundreds otherwise)
            if name.starts_with("lib") { continue; }
            pkgs.push(SystemPkg {
                name: name.to_string(), version: version.to_string(),
                manager: "apt".to_string(), description: desc.to_string(),
            });
        }
    }

    pkgs
}

/// Helper: returns the set of apt package names the user explicitly installed
/// (not auto-pulled as dependencies). Uses `apt-mark showmanual`.
/// Falls back to an empty set if apt-mark isn't available (non-Debian systems).
fn apt_manual_packages() -> std::collections::HashSet<String> {
    if let Ok(output) = std::process::Command::new("apt-mark")
        .arg("showmanual")
        .output()
    {
        if output.status.success() {
            return String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|l| l.trim().to_lowercase())
                .filter(|l| !l.is_empty())
                .collect();
        }
    }
    std::collections::HashSet::new()
}

/// Helper: load the set of package names already tracked in installed.json.
fn tracked_pkg_names() -> std::collections::HashSet<String> {
    let path = db_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        let installed: Vec<InstalledPkg> = serde_json::from_str(&data).unwrap_or_default();
        installed.into_iter().map(|p| p.name.to_lowercase()).collect()
    } else {
        std::collections::HashSet::new()
    }
}

/// Scan for user-installed packages NOT part of the OS base install.
/// Shows optional/extra dpkg packages + flatpak + snap, excluding what pkgd already tracks.
/// These appear in the "detected packages" section and can be moved to pkgd managed.
#[tauri::command]
async fn scan_user_packages() -> Vec<SystemPkg> {
    let tracked = tracked_pkg_names();
    // Only show packages the user explicitly asked to install, not auto-deps
    let manual = apt_manual_packages();
    let has_manual = !manual.is_empty();
    let mut pkgs: Vec<SystemPkg> = Vec::new();

    // ── dpkg: only manually-installed, non-OS-base packages ──────────────────
    if let Ok(output) = std::process::Command::new("dpkg-query")
        .args(["-W", "--showformat=${db:Status-Status}\t${Priority}\t${Package}\t${Version}\t${binary:Summary}\n"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if !line.starts_with("installed\t") { continue; }
            let rest = &line["installed\t".len()..];
            let parts: Vec<&str> = rest.splitn(4, '\t').collect();
            let priority = parts.get(0).unwrap_or(&"").trim();
            let name     = parts.get(1).unwrap_or(&"").trim();
            let version  = parts.get(2).unwrap_or(&"").trim();
            let desc     = parts.get(3).unwrap_or(&"").trim();
            if name.is_empty() { continue; }
            // SKIP OS base — those go in scan_system_packages
            if matches!(priority, "required" | "important" | "standard") { continue; }
            // Skip auto-installed dependencies (only show what the user chose to install)
            if has_manual && !manual.contains(&name.to_lowercase()) { continue; }
            // Skip if pkgd already manages this package
            if tracked.contains(&name.to_lowercase()) { continue; }
            // Noise filters — hide libraries, dev tools, runtimes
            if SYSTEM_BLOCKLIST.contains(&name) { continue; }
            if name.starts_with("lib") { continue; }
            if name.starts_with("python3-") || name.starts_with("python-")
                || name.starts_with("ruby-") || name.starts_with("perl") { continue; }
            if name.starts_with("fonts-") || name.starts_with("linux-")
                || name.starts_with("firmware-") || name.starts_with("xserver-xorg-")
                || name.starts_with("gir1.") || name.starts_with("r-cran-")
                || name.starts_with("initramfs-") || name.starts_with("grub-")
                || name.starts_with("cryptsetup-") || name.starts_with("systemd-") { continue; }
            if name.ends_with("-dev") || name.ends_with("-doc") || name.ends_with("-dbg")
                || name.ends_with("-common") || name.ends_with("-data")
                || name.ends_with("-locale") || name.ends_with("-l10n") { continue; }
            pkgs.push(SystemPkg {
                name: name.to_string(), version: version.to_string(),
                manager: "apt".to_string(), description: desc.to_string(),
            });
        }
    }

    // ── flatpak — apps only (--app excludes runtimes/sdk deps) ───────────────
    if let Ok(output) = std::process::Command::new("flatpak")
        .args(["list", "--app", "--columns=name,version,application"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.trim().is_empty() { continue; }
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            let name = parts[0].trim();
            if tracked.contains(&name.to_lowercase()) { continue; }
            pkgs.push(SystemPkg {
                name: name.to_string(),
                version: parts.get(1).unwrap_or(&"").trim().to_string(),
                manager: "flatpak".to_string(),
                description: parts.get(2).unwrap_or(&"").trim().to_string(),
            });
        }
    }

    // ── AppImages in ~/applications not tracked by pkgd ──────────────────────
    let apps_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("applications");
    if let Ok(entries) = std::fs::read_dir(&apps_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
            if !fname.to_lowercase().ends_with(".appimage") { continue; }
            // strip .AppImage suffix for the display name
            let name = fname[..fname.len() - 9].to_string(); // len(".AppImage") == 9
            if tracked.contains(&name.to_lowercase()) { continue; }
            pkgs.push(SystemPkg {
                name: name.clone(),
                version: "local".to_string(),
                manager: "appimage".to_string(),
                description: path.to_string_lossy().to_string(),
            });
        }
    }

    // ── snap — non-core snaps not already tracked ─────────────────────────────
    if let Ok(output) = std::process::Command::new("snap").args(["list"]).output() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 { continue; }
            let name = parts[0];
            if name == "snapd" || name.starts_with("core") { continue; }
            if tracked.contains(&name.to_lowercase()) { continue; }
            pkgs.push(SystemPkg {
                name: name.to_string(), version: parts[1].to_string(),
                manager: "snap".to_string(),
                description: parts.get(3).unwrap_or(&"").to_string(),
            });
        }
    }

    pkgs
}

/// Scan auto-installed apt packages (dependencies of user packages).
/// Applies the same noise filters as scan_user_packages.
/// These are shown in the separate "dependencies" section below detected packages.
#[tauri::command]
async fn scan_dep_packages() -> Vec<SystemPkg> {
    let tracked = tracked_pkg_names();

    // Get the set of auto-installed packages from apt-mark
    let auto_set: std::collections::HashSet<String> = {
        if let Ok(out) = std::process::Command::new("apt-mark").arg("showauto").output() {
            if out.status.success() {
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(|l| l.trim().to_lowercase())
                    .filter(|l| !l.is_empty())
                    .collect()
            } else {
                return vec![]; // apt-mark not available — nothing to show
            }
        } else {
            return vec![];
        }
    };

    let mut pkgs: Vec<SystemPkg> = Vec::new();

    if let Ok(output) = std::process::Command::new("dpkg-query")
        .args(["-W", "--showformat=${db:Status-Status}\t${Priority}\t${Package}\t${Version}\t${binary:Summary}\n"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if !line.starts_with("installed\t") { continue; }
            let rest = &line["installed\t".len()..];
            let parts: Vec<&str> = rest.splitn(4, '\t').collect();
            let priority = parts.get(0).unwrap_or(&"").trim();
            let name     = parts.get(1).unwrap_or(&"").trim();
            let version  = parts.get(2).unwrap_or(&"").trim();
            let desc     = parts.get(3).unwrap_or(&"").trim();
            if name.is_empty() { continue; }
            // OS base goes in scan_system_packages
            if matches!(priority, "required" | "important" | "standard") { continue; }
            // Only auto-installed (dependency) packages
            if !auto_set.contains(&name.to_lowercase()) { continue; }
            // Skip if pkgd already manages this
            if tracked.contains(&name.to_lowercase()) { continue; }
            // Same noise filters as user packages
            if SYSTEM_BLOCKLIST.contains(&name) { continue; }
            if name.starts_with("lib") { continue; }
            if name.starts_with("python3-") || name.starts_with("python-")
                || name.starts_with("ruby-") || name.starts_with("perl") { continue; }
            if name.starts_with("fonts-") || name.starts_with("linux-")
                || name.starts_with("firmware-") || name.starts_with("xserver-xorg-")
                || name.starts_with("gir1.") || name.starts_with("r-cran-")
                || name.starts_with("initramfs-") || name.starts_with("grub-")
                || name.starts_with("cryptsetup-") || name.starts_with("systemd-") { continue; }
            if name.ends_with("-dev") || name.ends_with("-doc") || name.ends_with("-dbg")
                || name.ends_with("-common") || name.ends_with("-data")
                || name.ends_with("-locale") || name.ends_with("-l10n") { continue; }
            pkgs.push(SystemPkg {
                name: name.to_string(), version: version.to_string(),
                manager: "apt".to_string(), description: desc.to_string(),
            });
        }
    }

    pkgs
}

/// Launch a system-managed package (apt binary, flatpak app, snap, or AppImage).
#[tauri::command]
async fn launch_sys_pkg(window: tauri::Window, name: String, manager: String) -> Result<(), String> {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));

    let (prog, args): (String, Vec<String>) = match manager.as_str() {
        "flatpak" => ("flatpak".into(), vec!["run".into(), name.clone()]),
        "snap"    => (name.clone(), vec![]),
        "appimage" => {
            let path = home.join("applications").join(format!("{}.AppImage", name));
            (path.to_string_lossy().into(), vec![])
        }
        // apt / deb / unknown: try to find a .desktop Exec= or fall back to binary name
        _ => {
            // Try to find the desktop file for this package and get its Exec line
            let exec = find_desktop_exec(&name);
            (exec.unwrap_or_else(|| name.clone()), vec![])
        }
    };

    // Spawn detached so the GUI doesn't wait
    std::process::Command::new(&prog)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not launch {}: {}", prog, e))?;

    let _ = window.emit("pkgd-log", format!("   launched {}", name));
    let _ = window.emit("pkgd-done", "ok");
    Ok(())
}

/// Search .desktop files in standard locations to find the Exec= for a package name.
fn find_desktop_exec(pkg_name: &str) -> Option<String> {
    let dirs = [
        "/usr/share/applications",
        "/usr/local/share/applications",
    ];
    for dir in &dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "desktop").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        // Check if this desktop file belongs to our package
                        let fname = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
                        if !fname.contains(&pkg_name.to_lowercase()) { continue; }
                        // Extract Exec= line
                        for line in content.lines() {
                            if let Some(exec) = line.strip_prefix("Exec=") {
                                // Strip %U %F etc args
                                let cmd = exec.split_whitespace()
                                    .next()
                                    .unwrap_or(exec)
                                    .to_string();
                                return Some(cmd);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Return the installed dependencies of a given package name.
/// Uses `apt-cache depends --installed` for apt packages.
#[tauri::command]
async fn get_pkg_deps(name: String, manager: String) -> Vec<String> {
    let mut deps: Vec<String> = Vec::new();
    match manager.as_str() {
        "apt" | "deb" => {
            if let Ok(out) = std::process::Command::new("apt-cache")
                .args(["depends", "--installed", &name])
                .output()
            {
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    let line = line.trim();
                    // lines look like: "  Depends: libfoo" or "  Recommends: libbar"
                    if let Some(rest) = line.strip_prefix("Depends: ") {
                        let dep = rest.trim().trim_start_matches('<').trim_end_matches('>');
                        if !dep.is_empty() { deps.push(dep.to_string()); }
                    }
                }
            }
        }
        "flatpak" => {
            if let Ok(out) = std::process::Command::new("flatpak")
                .args(["info", "--show-runtime", &name])
                .output()
            {
                let runtime = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !runtime.is_empty() { deps.push(format!("runtime: {}", runtime)); }
            }
        }
        _ => {}
    }
    deps
}

/// Search all available package managers for packages matching the query.
/// Checks apt, dnf, pacman, flatpak, snap — whichever are installed.
#[tauri::command]
async fn search_sys_pkgmgr(query: String) -> Vec<SysPkgResult> {
    let mut results: Vec<SysPkgResult> = Vec::new();

    // ── apt-cache (Debian/Ubuntu) ─────────────────────────────────────────────
    if let Ok(output) = std::process::Command::new("apt-cache")
        .args(["search", &query])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut count = 0;
        for line in stdout.lines() {
            if count >= 8 { break; }
            if let Some(idx) = line.find(" - ") {
                let name = line[..idx].trim();
                if name.starts_with("lib") { continue; }
                if name.ends_with("-dev") || name.ends_with("-doc") || name.ends_with("-dbg") { continue; }
                results.push(SysPkgResult {
                    name: name.to_string(),
                    description: line[idx+3..].trim().to_string(),
                    manager: "apt".to_string(),
                });
                count += 1;
            }
        }
    }

    // ── dnf (Fedora/RHEL/CentOS) ──────────────────────────────────────────────
    if let Ok(output) = std::process::Command::new("dnf")
        .args(["search", &query])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut count = 0;
        for line in stdout.lines() {
            if count >= 6 { break; }
            // skip section headers (===) and empty lines
            if line.starts_with('=') || line.trim().is_empty() { continue; }
            if let Some(idx) = line.find(" : ") {
                let raw_name = line[..idx].trim();
                // strip .arch suffix (vim.x86_64 → vim)
                let name = raw_name.split('.').next().unwrap_or(raw_name);
                if name.starts_with("lib") { continue; }
                if name.ends_with("-devel") { continue; }
                results.push(SysPkgResult {
                    name: name.to_string(),
                    description: line[idx+3..].trim().to_string(),
                    manager: "dnf".to_string(),
                });
                count += 1;
            }
        }
    }

    // ── pacman (Arch/Manjaro) ─────────────────────────────────────────────────
    if let Ok(output) = std::process::Command::new("pacman")
        .args(["-Ss", &query])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        let mut i = 0;
        let mut count = 0;
        while i < lines.len() && count < 6 {
            let line = lines[i];
            // pkg line: "repo/pkgname version [flags]", desc is next indented line
            if let Some(slash) = line.find('/') {
                let after = &line[slash+1..];
                let name = after.split_whitespace().next().unwrap_or("").trim();
                if !name.is_empty() && !name.starts_with("lib") {
                    let desc = if i + 1 < lines.len() {
                        lines[i+1].trim().to_string()
                    } else {
                        String::new()
                    };
                    results.push(SysPkgResult {
                        name: name.to_string(),
                        description: desc,
                        manager: "pacman".to_string(),
                    });
                    count += 1;
                }
                i += 2;
            } else {
                i += 1;
            }
        }
    }

    // ── flatpak search ────────────────────────────────────────────────────────
    if let Ok(output) = std::process::Command::new("flatpak")
        .args(["search", "--columns=name,description", &query])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().take(5) {
            if line.trim().is_empty() { continue; }
            let parts: Vec<&str> = line.splitn(2, '\t').collect();
            let name = parts[0].trim();
            if name.is_empty() { continue; }
            results.push(SysPkgResult {
                name: name.to_string(),
                description: parts.get(1).unwrap_or(&"").trim().to_string(),
                manager: "flatpak".to_string(),
            });
        }
    }

    // ── snap find ─────────────────────────────────────────────────────────────
    if let Ok(output) = std::process::Command::new("snap")
        .args(["find", &query])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().skip(1).take(5) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() { continue; }
            let name = parts[0];
            if name.is_empty() { continue; }
            let desc = if parts.len() > 3 { parts[3..].join(" ") } else { String::new() };
            results.push(SysPkgResult {
                name: name.to_string(),
                description: desc,
                manager: "snap".to_string(),
            });
        }
    }

    results
}

/// Returns which package managers are available on this system.
/// Used by the frontend to filter the priority list and settings to only show relevant options.
#[tauri::command]
fn detect_pkg_managers() -> Vec<String> {
    let mut mgrs = Vec::new();
    let checks: &[(&str, &[&str])] = &[
        ("dpkg",    &["/usr/bin/dpkg"]),
        ("apt",     &["/usr/bin/apt", "/usr/bin/apt-cache"]),
        ("rpm",     &["/usr/bin/rpm"]),
        ("dnf",     &["/usr/bin/dnf"]),
        ("pacman",  &["/usr/bin/pacman"]),
        ("flatpak", &["/usr/bin/flatpak", "/usr/local/bin/flatpak"]),
        ("snap",    &["/usr/bin/snap"]),
    ];
    for (name, paths) in checks {
        if paths.iter().any(|p| std::path::Path::new(p).exists()) {
            mgrs.push(name.to_string());
        }
    }
    mgrs
}

/// Toggle the version-lock flag for a package in installed.json.
#[tauri::command]
fn toggle_pkg_lock(repo: String) -> Result<bool, String> {
    let path = db_path();
    let mut pkgs: Vec<InstalledPkg> = if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else { return Err("installed.json not found".into()); };
    let mut new_state = false;
    for pkg in &mut pkgs {
        if pkg.repo == repo {
            pkg.locked = !pkg.locked;
            new_state = pkg.locked;
            break;
        }
    }
    std::fs::create_dir_all(pkgd_dir()).map_err(|e| e.to_string())?;
    let data = serde_json::to_string_pretty(&pkgs).map_err(|e| e.to_string())?;
    std::fs::write(path, data).map_err(|e| e.to_string())?;
    Ok(new_state)
}

/// Remove a package entry from installed.json by repo key.
#[tauri::command]
fn remove_installed(repo: String) -> Result<(), String> {
    let path = db_path();
    let pkgs: Vec<InstalledPkg> = if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        return Ok(()); // nothing to remove
    };
    let filtered: Vec<InstalledPkg> = pkgs.into_iter().filter(|p| p.repo != repo).collect();
    std::fs::create_dir_all(pkgd_dir()).map_err(|e| e.to_string())?;
    let data = serde_json::to_string_pretty(&filtered).map_err(|e| e.to_string())?;
    std::fs::write(path, data).map_err(|e| e.to_string())?;
    Ok(())
}

/// Add or update a package entry in installed.json (used to track sys-manager installs).
#[tauri::command]
fn record_installed(pkg: InstalledPkg) -> Result<(), String> {
    let path = db_path();
    let mut pkgs: Vec<InstalledPkg> = if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        vec![]
    };
    // replace existing entry for the same repo
    pkgs.retain(|p| p.repo != pkg.repo);
    pkgs.push(pkg);
    std::fs::create_dir_all(pkgd_dir()).map_err(|e| e.to_string())?;
    let data = serde_json::to_string_pretty(&pkgs).map_err(|e| e.to_string())?;
    std::fs::write(path, data).map_err(|e| e.to_string())?;
    Ok(())
}

/// Install a system package via its manager, streaming output to the GUI.
/// Returns true if the install succeeded, false if it failed.
#[tauri::command]
async fn run_sys_install(window: tauri::Window, name: String, manager: String) -> Result<bool, String> {
    let (prog, args): (&str, Vec<String>) = match manager.as_str() {
        "apt" => ("sudo", vec!["-n".into(), "apt-get".into(), "install".into(), "-y".into(), name.clone()]),
        "dnf" => ("sudo", vec!["-n".into(), "dnf".into(), "install".into(), "-y".into(), name.clone()]),
        "pacman" => ("sudo", vec!["-n".into(), "pacman".into(), "-S".into(), "--noconfirm".into(), name.clone()]),
        "flatpak" => ("flatpak", vec!["install".into(), "-y".into(), "--noninteractive".into(), name.clone()]),
        "snap" => ("sudo", vec!["-n".into(), "snap".into(), "install".into(), name.clone()]),
        _ => return Err(format!("unknown manager: {}", manager)),
    };

    let mut child = Command::new(prog)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to launch {} install: {}", manager, e))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let win1 = window.clone();
    let win2 = window.clone();

    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = win1.emit("pkgd-log", line);
        }
    });
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let t = line.trim();
            if t.is_empty() { continue; }
            let _ = win2.emit("pkgd-log", &line);
        }
    });

    let status = child.wait().await.map_err(|e| e.to_string())?;
    let ok = status.success();
    let _ = window.emit("pkgd-done", if ok { "ok" } else { "error" });
    Ok(ok)
}

/// Remove a system package via its manager, streaming output to the GUI.
#[tauri::command]
async fn run_sys_remove(window: tauri::Window, name: String, manager: String) -> Result<(), String> {
    let (prog, args): (&str, Vec<String>) = match manager.as_str() {
        "apt" => ("sudo", vec!["-n".into(), "apt-get".into(), "remove".into(), "-y".into(), name.clone()]),
        "dnf" => ("sudo", vec!["-n".into(), "dnf".into(), "remove".into(), "-y".into(), name.clone()]),
        "pacman" => ("sudo", vec!["-n".into(), "pacman".into(), "-R".into(), "--noconfirm".into(), name.clone()]),
        "flatpak" => ("flatpak", vec!["uninstall".into(), "-y".into(), "--noninteractive".into(), name.clone()]),
        "snap" => ("sudo", vec!["-n".into(), "snap".into(), "remove".into(), name.clone()]),
        _ => return Err(format!("unknown manager: {}", manager)),
    };

    let mut child = Command::new(prog)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to launch {} remove: {}", manager, e))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let win1 = window.clone();
    let win2 = window.clone();

    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = win1.emit("pkgd-log", line);
        }
    });
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let t = line.trim();
            if t.is_empty() { continue; }
            let _ = win2.emit("pkgd-log", &line);
        }
    });

    let status = child.wait().await.map_err(|e| e.to_string())?;
    if status.success() {
        let _ = window.emit("pkgd-done", "ok");
    } else {
        let _ = window.emit("pkgd-done", "error");
    }
    Ok(())
}

// ─── app entry ────────────────────────────────────────────────────────────────

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            fetch_release,
            check_update,
            search_github,
            run_pkgd,
            load_settings,
            save_settings,
            load_installed,
            detect_os,
            detect_pkg_managers,
            toggle_pkg_lock,
            remove_installed,
            record_installed,
            scan_system_packages,
            scan_user_packages,
            scan_dep_packages,
            get_pkg_deps,
            launch_sys_pkg,
            search_sys_pkgmgr,
            run_sys_install,
            run_sys_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error running pkgd");
}
