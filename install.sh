#!/usr/bin/env bash
# pkgd installer — curl -fsSL https://raw.githubusercontent.com/sudoxreboot/pkgd/main/install.sh | bash
set -euo pipefail

REPO="sudoxreboot/pkgd"
INSTALL_DIR="/usr/local/bin"
RAW_BASE="https://raw.githubusercontent.com/${REPO}/main"

teal="\033[38;2;136;255;238m"
lilac="\033[38;2;170;170;255m"
rst="\033[0m"

info() { printf "${teal}:: ${rst}%s\n" "$*"; }
done_() { printf "${lilac}✓ ${rst}%s\n" "$*"; }

info "installing pkgd..."

if command -v curl &>/dev/null; then
  curl -fsSL "${RAW_BASE}/pkgd" -o /tmp/pkgd_install
elif command -v wget &>/dev/null; then
  wget -qO /tmp/pkgd_install "${RAW_BASE}/pkgd"
else
  echo "error: curl or wget required" >&2
  exit 1
fi

chmod +x /tmp/pkgd_install
sudo mv /tmp/pkgd_install "${INSTALL_DIR}/pkgd"

done_ "pkgd installed → ${INSTALL_DIR}/pkgd"
info  "run: pkgd --help"
