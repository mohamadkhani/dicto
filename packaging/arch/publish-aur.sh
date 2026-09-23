#!/usr/bin/env bash
# Publish (or update) a dicto AUR package after a release.
#
# Called by CI with these env vars:
#   AUR_SSH_PRIVATE_KEY  — ed25519 private key whose public half is registered
#                          on the AUR account (https://aur.archlinux.org/account/ssh)
#   AUR_REPO             — AUR package repo name, e.g. "dicto" or "dicto-bin"
#   PKGBUILD_PATH        — path to the canonical PKGBUILD in this repo
#   VERSION              — bare version, e.g. 0.6.1 (no leading 'v')
#   B2SUM                — real b2sum of the released tarball
#
# This script:
#   1. clones the AUR package repo,
#   2. copies in the canonical PKGBUILD,
#   3. sets pkgver/pkgrel and injects the real b2sum of the released tarball,
#   4. regenerates .SRCINFO via makepkg (AUR rejects pushes without it),
#   5. commits and pushes over SSH using the provided deploy key.
#
# Requires makepkg (present on any Arch system; CI runs in an archlinux
# container, so no docker indirection is needed).
set -euo pipefail

: "${AUR_SSH_PRIVATE_KEY:?AUR_SSH_PRIVATE_KEY env var is required}"
: "${AUR_REPO:?AUR_REPO env var is required}"
: "${PKGBUILD_PATH:?PKGBUILD_PATH env var is required}"
: "${VERSION:?VERSION env var is required}"
: "${B2SUM:?B2SUM env var is required}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# --- SSH setup for the AUR push -------------------------------------------
# Isolated key inside the temp dir: never touch the user's ~/.ssh.
KEYFILE="${WORK}/aur_key"
printenv AUR_SSH_PRIVATE_KEY > "$KEYFILE"
chmod 600 "$KEYFILE"
# Pin AUR's real ed25519 host key (fingerprint verified via ssh-keyscan) so
# the push can't hang on a prompt and can't be silently MITM'd.
KNOWN_HOSTS="${WORK}/known_hosts"
echo "aur.archlinux.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEuBKrPzbawxA/k2g6NcyV5jmqwJ2s+zpgZGZ7tpLIcN" \
  > "$KNOWN_HOSTS"
# Force this key only — don't let an agent offer some other identity.
export GIT_SSH_COMMAND="ssh -i ${KEYFILE} -o IdentitiesOnly=yes -o UserKnownHostsFile=${KNOWN_HOSTS} -o StrictHostKeyChecking=yes"
# The commit/push below may run as the unprivileged builder user (see there);
# su doesn't inherit env, so pass the SSH command through explicitly.
export SSH_CMD="$GIT_SSH_COMMAND"

# --- Clone the AUR package repo -------------------------------------------
git clone "ssh://aur@aur.archlinux.org/${AUR_REPO}.git" "${WORK}/${AUR_REPO}"

# --- Patch the canonical PKGBUILD ------------------------------------------
PKGBUILD="${WORK}/${AUR_REPO}/PKGBUILD"
cp "${PKGBUILD_PATH}" "$PKGBUILD"

sed -i "s/^pkgver=.*/pkgver=${VERSION}/" "$PKGBUILD"
sed -i "s/^pkgrel=.*/pkgrel=1/" "$PKGBUILD"
sed -i "s/'PLACEHOLDER'/'${B2SUM}'/" "$PKGBUILD"

# --- Regenerate .SRCINFO (AUR rejects pushes without it) --------------------
# makepkg refuses to run as root ("catastrophic damage" guard) and CI
# containers run as root, so delegate to an unprivileged builder user there.
# Locally (non-root) makepkg runs directly.
cd "${WORK}/${AUR_REPO}"
if [ "$(id -u)" -eq 0 ]; then
  id builder &>/dev/null || useradd -m builder
  chown -R builder:builder "$WORK"
  su builder -s /bin/bash -c "cd '${WORK}/${AUR_REPO}' && makepkg --printsrcinfo" > .SRCINFO
else
  makepkg --printsrcinfo > .SRCINFO
fi

# --- Sanity check: real checksum, no PLACEHOLDER left ------------------------
grep -q PLACEHOLDER "$PKGBUILD" && {
  echo "::error::PKGBUILD still contains PLACEHOLDER" >&2
  exit 1
}

# --- Commit and push (idempotent: skip if nothing changed) -------------------
# Run as builder when root: the chown above made the repo builder-owned, and
# git refuses root operations on a user-owned repo ("not in a git directory").
run_git() {
  if [ "$(id -u)" -eq 0 ]; then
    su builder -s /bin/bash -c "cd '${WORK}/${AUR_REPO}' && GIT_SSH_COMMAND='${SSH_CMD}' $*"
  else
    bash -c "cd '${WORK}/${AUR_REPO}' && $*"
  fi
}

run_git "git config user.name  'dicto-ci'"
run_git "git config user.email 'ci@noreply.logi-camp.dev'"
run_git "git add PKGBUILD .SRCINFO"
if run_git "git diff --cached --quiet"; then
  echo ":: AUR package ${AUR_REPO} already at v${VERSION}, nothing to push"
else
  run_git "git commit -m 'v${VERSION}'"
  run_git "git push origin master"
  echo ":: AUR package ${AUR_REPO} updated to v${VERSION}"
fi
