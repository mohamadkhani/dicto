#!/usr/bin/env bash
# Build a Debian / Ubuntu (.deb) package for dicto.
#
# Usage:
#   packaging/debian/build-deb.sh <version> [binary_path] [output_dir] [arch]
#
# Example:
#   packaging/debian/build-deb.sh 0.6.2 target/release/dicto dist amd64
#
# Unlike a hand-written control file, this script derives the package's
# runtime dependencies from the binary itself:
#   - every DT_NEEDED shared library is mapped to the Debian package that
#     ships it (the build fails on any soname the table does not know, so a
#     new dependency can never be silently dropped),
#   - the libc6 minimum version is computed from the binary's required
#     GLIBC_x.y symbol versions, so the package refuses to install on
#     systems where it would crash with "version `GLIBC_2.xx' not found".
#
# The staged copy of the binary is stripped (unless --no-strip); the input
# binary is never modified.

set -euo pipefail

usage() {
  echo "Usage: $0 <version> [binary_path] [output_dir] [arch] [--no-strip]" >&2
  exit 2
}

VERSION=""
BINARY=""
OUTDIR=""
ARCH="amd64"
STRIP=yes

while [ $# -gt 0 ]; do
  case "$1" in
    --no-strip) STRIP=no ;;
    -h|--help) usage ;;
    *)
      if [ -z "$VERSION" ]; then VERSION="$1"
      elif [ -z "$BINARY" ]; then BINARY="$1"
      elif [ -z "$OUTDIR" ]; then OUTDIR="$1"
      elif [ "$ARCH" = "amd64" ]; then ARCH="$1"
      else usage
      fi
      ;;
  esac
  shift
done

[ -n "$VERSION" ] || usage
BINARY="${BINARY:-target/release/dicto}"
OUTDIR="${OUTDIR:-dist}"

# Debian versions must start with a digit; tolerate a leading 'v'.
VERSION="${VERSION#v}"
if ! [[ "$VERSION" =~ ^[0-9][A-Za-z0-9.+~-]*$ ]]; then
  echo "Error: invalid Debian version '$VERSION'" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

for tool in objdump install find; do
  command -v "$tool" >/dev/null 2>&1 || { echo "Error: '$tool' not found" >&2; exit 1; }
done
if [ ! -f "$BINARY" ]; then
  echo "Error: Binary not found at '$BINARY'" >&2
  exit 1
fi

# --------------------------------------------------------------- architecture
# Reject a mismatch between the binary's ELF machine and the requested deb
# architecture instead of naming the .deb after a guess.
ELF_FORMAT="$(objdump -f "$BINARY" | awk '/file format/ {print $4}')"
case "${ELF_FORMAT}-${ARCH}" in
  elf64-x86-64-amd64|elf64-x86-64-x86-64) ;;
  elf64-littleaarch64-arm64|elf64-bigaarch64-arm64) ;;
  *)
    echo "Error: binary format '$ELF_FORMAT' does not match requested arch '$ARCH'" >&2
    exit 1
    ;;
esac

# ------------------------------------------------------------- dependencies
# Map sonames to the Debian/Ubuntu packages that ship them. The t64 rename
# packages (Ubuntu 24.04) declare Provides for the plain names below, so
# these dependencies resolve on Debian 12/13, Ubuntu 22.04 and 24.04 alike.
declare -A SONAME_PKG=(
  [libasound.so.2]=libasound2
  [libgtk-3.so.0]=libgtk-3-0
  [libglib-2.0.so.0]=libglib2.0-0
  [libgobject-2.0.so.0]=libglib2.0-0
  [libgio-2.0.so.0]=libglib2.0-0
  [libcairo.so.2]=libcairo2
  [libcairo-gobject.so.2]=libcairo2
  [libpango-1.0.so.0]=libpango-1.0-0
  [libgdk_pixbuf-2.0.so.0]=libgdk-pixbuf-2.0-0
  [libxcb.so.1]=libxcb1
  [libxkbcommon.so.0]=libxkbcommon0
  [libxkbcommon-x11.so.0]=libxkbcommon-x11-0
  [libz.so.1]=zlib1g
  [libgcc_s.so.1]=libgcc-s1
  [libstdc++.so.6]=libstdc++6
  [libssl.so.3]=libssl3
)
# Provided by libc6 itself (or the ELF interpreter); never listed.
SKIP_SONAMES="ld-linux-x86-64.so.2 ld-linux-aarch64.so.1 libc.so.6 libm.so.6 libdl.so.2 libpthread.so.0 librt.so.1 libresolv.so.2 libutil.so.1 ld-linux.so.2"

DEPS=""
UNKNOWN=""
for soname in $(objdump -p "$BINARY" | awk '/NEEDED/ {print $2}' | sort -u); do
  skip=no
  for s in $SKIP_SONAMES; do [ "$s" = "$soname" ] && skip=yes; done
  [ "$skip" = yes ] && continue
  if [ -n "${SONAME_PKG[$soname]:-}" ]; then
    DEPS="${DEPS}${SONAME_PKG[$soname]}, "
  else
    UNKNOWN="$UNKNOWN $soname"
  fi
done
if [ -n "$UNKNOWN" ]; then
  echo "Error: binary needs soname(s) unknown to the packaging table:$UNKNOWN" >&2
  echo "Add a mapping to SONAME_PKG in $0 before shipping." >&2
  exit 1
fi

# libc6 floor from the symbol versions the binary actually requires.
GLIBC_FLOOR="$(objdump -T "$BINARY" | grep -oE 'GLIBC_2\.[0-9]+' | sort -uV | tail -n1 | cut -d_ -f2 || true)"
if [ -z "$GLIBC_FLOOR" ]; then
  echo "Error: could not determine required glibc version from the binary" >&2
  exit 1
fi
DEPS="libc6 (>= ${GLIBC_FLOOR}), ${DEPS%, }"
DEPS="${DEPS%, }"

echo ":: Derived Depends: ${DEPS}"

case "$GLIBC_FLOOR" in
  2.3[3-9]|2.[4-9][0-9])
    echo ":: WARNING: binary requires glibc >= ${GLIBC_FLOOR}." >&2
    echo "::   Older stable releases (Debian 12 = 2.36, Ubuntu 22.04 = 2.35," >&2
    echo "::   Ubuntu 24.04 = 2.39) cannot run it; apt will refuse there." >&2
    echo "::   To widen compatibility, build in a Debian-based container." >&2
    ;;
esac

# ------------------------------------------------------------------ staging
mkdir -p "$OUTDIR"
OUTDIR="$(cd "$OUTDIR" && pwd)"
DEB_NAME="dicto_${VERSION}_${ARCH}.deb"
DEB_PATH="${OUTDIR}/${DEB_NAME}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

PKG="${WORK}/pkg"
mkdir -p "${PKG}/DEBIAN" \
         "${PKG}/usr/bin" \
         "${PKG}/usr/share/applications" \
         "${PKG}/usr/share/icons/hicolor/scalable/apps" \
         "${PKG}/usr/share/man/man1" \
         "${PKG}/usr/share/doc/dicto"

install -m 755 "$BINARY" "${PKG}/usr/bin/dicto"
if [ "$STRIP" = yes ] && command -v strip >/dev/null 2>&1; then
  strip --strip-unneeded "${PKG}/usr/bin/dicto"
fi

install -m 644 "${SCRIPT_DIR}/control" "${PKG}/DEBIAN/control"
sed -i -e "s/^Version:.*/Version: ${VERSION}/" \
       -e "s/^Architecture:.*/Architecture: ${ARCH}/" \
       "${PKG}/DEBIAN/control"

# Depends is computed from the binary (see above) and must not be hand-edited
# in the template; insert it above Description, where dpkg-shlibdeps puts it.
sed -i "/^Description:/i\\
Depends: ${DEPS}" "${PKG}/DEBIAN/control"

# Refresh desktop database and icon cache through dpkg triggers when the
# owning packages are installed; a no-op otherwise (no fragile postinst).
cat > "${PKG}/DEBIAN/triggers" <<'EOF'
activate-noawait /usr/share/applications
activate-noawait /usr/share/icons/hicolor
EOF

install -m 644 "${REPO_ROOT}/packaging/arch/dicto.desktop" \
  "${PKG}/usr/share/applications/dicto.desktop"
install -m 644 "${REPO_ROOT}/assets/icon.svg" \
  "${PKG}/usr/share/icons/hicolor/scalable/apps/dicto.svg"

gzip -n -9 -c "${SCRIPT_DIR}/dicto.1" > "${PKG}/usr/share/man/man1/dicto.1.gz"

MAINTAINER="$(awk -F': ' '/^Maintainer:/ {print $2}' "${PKG}/DEBIAN/control")"
cat > "${WORK}/changelog" <<EOF
dicto (${VERSION}) unstable; urgency=medium

  * Release ${VERSION}. See
    https://github.com/logi-camp/dicto/releases/tag/v${VERSION}
    for the changelog.

 -- ${MAINTAINER}  $(date -R -u)
EOF
gzip -n -9 -c "${WORK}/changelog" > "${PKG}/usr/share/doc/dicto/changelog.Debian.gz"

{
  echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
  echo "Upstream-Name: dicto"
  echo "Source: $(awk -F': ' '/^Homepage:/ {print $2}' "${PKG}/DEBIAN/control")"
  echo
  echo "Files: *"
  echo "Copyright: Mohammadreza Khani"
  echo "License: AGPL-3.0-or-later"
  echo
  cat "${REPO_ROOT}/LICENSE"
} > "${PKG}/usr/share/doc/dicto/copyright"
if [ -f "${REPO_ROOT}/README.md" ]; then
  install -m 644 "${REPO_ROOT}/README.md" "${PKG}/usr/share/doc/dicto/README.md"
fi

# Installed-Size (KB) is computed from the staged tree.
INSTALLED_SIZE="$(du -sk "${PKG}/usr" | awk '{print $1}')"
sed -i "/^Description:/i\\
Installed-Size: ${INSTALLED_SIZE}" "${PKG}/DEBIAN/control"

(
  cd "$PKG"
  find usr -type f -exec md5sum {} +
) > "${PKG}/DEBIAN/md5sums"
chmod 644 "${PKG}/DEBIAN/md5sums"

# -------------------------------------------------------------------- build
if command -v dpkg-deb >/dev/null 2>&1; then
  echo ":: Building .deb using dpkg-deb..."
  dpkg-deb --build --root-owner-group "$PKG" "$DEB_PATH"
else
  echo ":: dpkg-deb not found; assembling .deb with ar/tar..."
  echo "2.0" > "${WORK}/debian-binary"
  tar --owner=0 --group=0 --numeric-owner -czf "${WORK}/control.tar.gz" -C "${PKG}/DEBIAN" .
  tar --owner=0 --group=0 --numeric-owner -czf "${WORK}/data.tar.gz" -C "$PKG" usr
  ( cd "$WORK" && ar -rc "$DEB_PATH" debian-binary control.tar.gz data.tar.gz )
fi

echo "Successfully built: ${DEB_PATH}"
if command -v dpkg-deb >/dev/null 2>&1; then
  dpkg-deb -I "$DEB_PATH"
  dpkg-deb -c "$DEB_PATH"
fi
