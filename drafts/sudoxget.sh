#!/bin/bash


cat > ~/.zshrc << 'EOF'
# p10k instant prompt — stays at top
if [[ -r "${XDG_CACHE_HOME:-$HOME/.cache}/p10k-instant-prompt-${(%):-%n}.zsh" ]]; then
  source "${XDG_CACHE_HOME:-$HOME/.cache}/p10k-instant-prompt-${(%):-%n}.zsh"
fi

export ZSH="$HOME/.oh-my-zsh"
ZSH_THEME="powerlevel10k/powerlevel10k"

plugins=(
  git
  fzf
  zsh-autosuggestions
  zsh-syntax-highlighting
  zsh-history-substring-search
  zsh-completions
  colored-man-pages
)

source $ZSH/oh-my-zsh.sh

# p10k config
[[ ! -f ~/.p10k.zsh ]] || source ~/.p10k.zsh

sudoxget() {
  local install=false
  local force_type=""
  local input=""

  while [[ "$1" == -* ]]; do
    case "$1" in
      -i) install=true ;;
      --deb) force_type="deb" ;;
      --appimage) force_type="appimage" ;;
      --tar) force_type="tar" ;;
      --zip) force_type="zip" ;;
      --bin) force_type="bin" ;;
    esac
    shift
  done
  input="$1"

  local repo
  if [[ "$input" == https://github.com/* ]]; then
    repo=$(echo "$input" | grep -oP 'github\.com/\K[^/]+/[^/]+')
  else
    repo="$input"
  fi

  echo "fetching latest release for ${repo}..."
  local assets=$(curl -s "https://api.github.com/repos/${repo}/releases/latest")
  local name=$(echo "$assets" | grep -oP '"tag_name": "\K[^"]*')
  local url=""

  case "$force_type" in
    deb)
      url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*amd64\.deb' | head -1)
      ;;
    appimage)
      url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*x86_64\.AppImage' | head -1)
      [[ -z "$url" ]] && url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*\.AppImage' | head -1)
      ;;
    tar)
      url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*x86_64\.tar\.gz' | head -1)
      [[ -z "$url" ]] && url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*x86_64\.tar\.xz' | head -1)
      ;;
    zip)
      url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*linux[^"]*x64[^"]*\.zip' | head -1)
      [[ -z "$url" ]] && url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*linux[^"]*\.zip' | head -1)
      ;;
    bin)
      url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*linux[^"]*' | head -1)
      ;;
    *)
      url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*amd64\.deb' | head -1)
      [[ -z "$url" ]] && url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*x86_64\.AppImage' | head -1)
      [[ -z "$url" ]] && url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*\.AppImage' | head -1)
      [[ -z "$url" ]] && url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*x86_64\.tar\.gz' | head -1)
      [[ -z "$url" ]] && url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*x86_64\.tar\.xz' | head -1)
      [[ -z "$url" ]] && url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*linux[^"]*x64[^"]*\.zip' | head -1)
      [[ -z "$url" ]] && url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*linux[^"]*\.zip' | head -1)
      [[ -z "$url" ]] && url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*linux[^"]*' | head -1)
      ;;
  esac

  if [[ -z "$url" ]]; then
    echo "no compatible asset found for ${repo}"
    return 1
  fi

  local filename=$(basename "$url")
  local appname=$(echo "$repo" | cut -d'/' -f2)
  echo "downloading ${filename} (${name})..."
  wget -q --show-progress "$url" -O "/tmp/${filename}"

  if [[ "$install" == false ]]; then
    echo "saved to /tmp/${filename}"
    return 0
  fi

  echo "installing ${filename}..."

  _sudoxget_mkdesktop() {
    local dname="$1" dexec="$2" dicon="${3:-application-x-executable}"
    mkdir -p ~/.local/share/applications
    cat > ~/.local/share/applications/${dname}.desktop << DESKTOP
[Desktop Entry]
Name=${dname}
Exec=${dexec}
Icon=${dicon}
Type=Application
Categories=Utility;
DESKTOP
    kbuildsycoca6 --noincremental 2>/dev/null || kbuildsycoca5 --noincremental 2>/dev/null
    echo "desktop entry created for ${dname}"
  }

  _sudoxget_stub() {
    local dep="$1"
    echo "stubbing missing dep: ${dep}..."
    sudo apt install -y equivs 2>/dev/null
    local tmpdir=$(mktemp -d)
    cat > "${tmpdir}/equivs-stub" << CONTROL
Section: misc
Priority: optional
Standards-Version: 3.9.2
Package: ${dep}
Version: 999.0
Description: stub for ${dep}
CONTROL
    pushd "$tmpdir" > /dev/null
    equivs-build equivs-stub 2>/dev/null
    sudo dpkg -i *.deb 2>/dev/null
    popd > /dev/null
    rm -rf "$tmpdir"
  }

  _sudoxget_resolvedeps() {
    local debfile="$1"
    echo "resolving dependencies for ${debfile}..."
    local depline=$(dpkg-deb -I "$debfile" | grep -oP 'Depends:\s*\K.*' | tr -d ' ')
    local deps=$(echo "$depline" | tr ',' '\n' | cut -d'|' -f1 | sed 's/([^)]*)//g' | tr -d ' ' | grep -v '^$')
    for dep in ${(f)deps}; do
      if ! dpkg -s "$dep" &>/dev/null; then
        echo "missing: ${dep}"
        sudo apt install -y "$dep" 2>/dev/null || _sudoxget_stub "$dep"
      fi
    done
    sudo apt --fix-broken install -y 2>/dev/null
  }

  _sudoxget_install_zip() {
    local zfile="$1" zapp="$2"
    local extract_dir=~/applications/${zapp}
    mkdir -p "$extract_dir"
    unzip -q "$zfile" -d "$extract_dir"
    local bin=$(find "$extract_dir" -maxdepth 3 -type f -executable -iname "${zapp}" | head -1)
    [[ -z "$bin" ]] && bin=$(find "$extract_dir" -maxdepth 3 -type f -executable ! -name "*.so*" ! -name "*.so.[0-9]*" | head -1)
    if [[ -n "$bin" ]]; then
      sudo ln -sf "$bin" /usr/local/bin/${zapp}
      _sudoxget_mkdesktop "$zapp" "$bin"
      echo "linked ${bin} -> /usr/local/bin/${zapp}"
    else
      echo "no executable found, contents at ${extract_dir}"
    fi
  }

  case "$filename" in
    *.deb)
      sudo apt install -y "/tmp/${filename}" 2>/dev/null
      if [[ $? -ne 0 ]]; then
        echo "initial install failed, resolving deps..."
        _sudoxget_resolvedeps "/tmp/${filename}"
        sudo apt install -y "/tmp/${filename}" 2>/dev/null
        if [[ $? -ne 0 ]]; then
          echo "deb failed after dep resolution, trying linux zip fallback..."
          local zip_url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*linux[^"]*x64[^"]*\.zip' | head -1)
          [[ -z "$zip_url" ]] && zip_url=$(echo "$assets" | grep -oP '"browser_download_url": "\K[^"]*linux[^"]*\.zip' | head -1)
          if [[ -n "$zip_url" ]]; then
            local zip_file=$(basename "$zip_url")
            echo "downloading zip fallback: ${zip_file}..."
            wget -q --show-progress "$zip_url" -O "/tmp/${zip_file}"
            _sudoxget_install_zip "/tmp/${zip_file}" "$appname"
          else
            echo "no fallback found, giving up."
            return 1
          fi
        fi
      fi
      ;;
    *.AppImage)
      mkdir -p ~/applications
      cp "/tmp/${filename}" ~/applications/
      chmod +x ~/applications/${filename}
      _sudoxget_mkdesktop "$appname" "${HOME}/applications/${filename}"
      ;;
    *.tar.gz|*.tar.xz)
      local extract_dir=~/applications/${appname}
      mkdir -p "$extract_dir"
      tar -xf "/tmp/${filename}" -C "$extract_dir" --strip-components=1
      local bin=$(find "$extract_dir" -maxdepth 2 -type f -executable ! -name "*.so*" | head -1)
      if [[ -n "$bin" ]]; then
        sudo ln -sf "$bin" /usr/local/bin/${appname}
        _sudoxget_mkdesktop "$appname" "$bin"
        echo "linked ${bin} -> /usr/local/bin/${appname}"
      fi
      ;;
    *.zip)
      _sudoxget_install_zip "/tmp/${filename}" "$appname"
      ;;
    *)
      sudo cp "/tmp/${filename}" /usr/local/bin/${appname}
      sudo chmod +x /usr/local/bin/${appname}
      ;;
  esac

  echo "done."
}
EOF
source ~/.zshrc
