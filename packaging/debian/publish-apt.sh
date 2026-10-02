#!/usr/bin/env bash
# Publish (or update) the GitHub Pages APT repository after a release.
#
# Unlike a flat "deb ... ./" root (Packages index at the repo top), this
# uses the standard dists/ + pool/ layout so the sources line reads
# `deb [signed-by=...] <url> stable main`, matches what apt expects of any
# normal repository, and can grow multiple suites/components later.
#
# Called by CI with these env vars:
#   GITHUB_TOKEN         — token allowed to push to the gh-pages branch
#   GITHUB_REPOSITORY    — e.g. logi-camp/dicto (falls back to git remote)
#   VERSION              — bare version, e.g. 0.6.2
#   DEB_PACKAGE          — path to the built .deb package
#   APT_GPG_PRIVATE_KEY  — ASCII-armored private GPG key (REQUIRED: an
#                          unsigned repo would leave users with install
#                          instructions that 404 or fail apt's signature
#                          check)
set -euo pipefail

: "${VERSION:?VERSION env var is required}"
: "${DEB_PACKAGE:?DEB_PACKAGE env var is required}"
: "${APT_GPG_PRIVATE_KEY:?APT_GPG_PRIVATE_KEY env var is required (signing is mandatory)}"

if [ ! -f "$DEB_PACKAGE" ]; then
  echo "Error: Deb package not found at '$DEB_PACKAGE'" >&2
  exit 1
fi

PKG_NAME="dicto"
SUITE="stable"
COMPONENT="main"
ARCH="amd64"
PAGES_BRANCH="gh-pages"

if [ -n "${GITHUB_REPOSITORY:-}" ]; then
  OWNER="${GITHUB_REPOSITORY%%/*}"
  REPO="${GITHUB_REPOSITORY#*/}"
else
  REMOTE_URL="$(git -C "$(dirname "$0")/../.." remote get-url origin)"
  OWNER_REPO="${REMOTE_URL#*github.com[:/]}"
  OWNER_REPO="${OWNER_REPO%.git}"
  OWNER="${OWNER_REPO%%/*}"
  REPO="${OWNER_REPO#*/}"
fi
PAGES_URL="https://${OWNER}.github.io/${REPO}"

for tool in dpkg-scanpackages gpg sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "Error: '$tool' not found (Debian/Ubuntu: apt-get install -y dpkg-dev gnupg)" >&2
    exit 1
  }
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

REPO_DIR="${WORK}/apt-repo"
REMOTE_URL="https://x-access-token:${GITHUB_TOKEN}@github.com/${OWNER}/${REPO}.git"

echo ":: Fetching ${PAGES_BRANCH} branch..."
if git clone --depth 1 --branch "$PAGES_BRANCH" "$REMOTE_URL" "$REPO_DIR" 2>/dev/null; then
  echo ":: Existing ${PAGES_BRANCH} branch found."
else
  echo ":: ${PAGES_BRANCH} branch does not exist yet. Initializing orphan branch..."
  mkdir -p "$REPO_DIR"
  cd "$REPO_DIR"
  git init -b "$PAGES_BRANCH"
  git remote add origin "$REMOTE_URL"
fi
cd "$REPO_DIR"

# The .deb lands in the pool; old versions stay so users can pin them.
install -D -m 644 "$DEB_PACKAGE" "pool/main/${PKG_NAME:0:1}/${PKG_NAME}/$(basename "$DEB_PACKAGE")"

echo ":: Generating Packages index..."
mkdir -p "dists/${SUITE}/${COMPONENT}/binary-${ARCH}"
dpkg-scanpackages --multiversion pool /dev/null > "dists/${SUITE}/${COMPONENT}/binary-${ARCH}/Packages"
gzip -9n -c "dists/${SUITE}/${COMPONENT}/binary-${ARCH}/Packages" \
  > "dists/${SUITE}/${COMPONENT}/binary-${ARCH}/Packages.gz"

# Release file with the full checksum suite apt expects.
cd "dists/${SUITE}"
DATE_UTC="$(date -Ru)"
{
  cat <<EOF
Origin: ${OWNER}
Label: ${REPO}
Suite: ${SUITE}
Codename: ${SUITE}
Architectures: ${ARCH}
Components: ${COMPONENT}
Description: APT repository for ${REPO}
Date: ${DATE_UTC}
EOF
  for algo in MD5Sum SHA1 SHA256 SHA512; do
    echo "${algo}:"
    for f in "${COMPONENT}/binary-${ARCH}/Packages" "${COMPONENT}/binary-${ARCH}/Packages.gz"; do
      case "$algo" in
        MD5Sum)  hash="$(md5sum "$f" | awk '{print $1}')" ;;
        SHA1)    hash="$(sha1sum "$f" | awk '{print $1}')" ;;
        SHA256)  hash="$(sha256sum "$f" | awk '{print $1}')" ;;
        SHA512)  hash="$(sha512sum "$f" | awk '{print $1}')" ;;
      esac
      size="$(stat -c%s "$f")"
      echo " ${hash} ${size} ${f}"
    done
  done
} > Release

echo ":: Signing Release with GPG..."
export GNUPGHOME="${WORK}/gnupg"
mkdir -p "$GNUPGHOME"
chmod 700 "$GNUPGHOME"
echo "$APT_GPG_PRIVATE_KEY" | gpg --batch --import

KEY_ID="$(gpg --list-secret-keys --with-colons | awk -F: '/^sec/ {print $5; exit}')"
if [ -z "$KEY_ID" ]; then
  echo "Error: no secret key found in APT_GPG_PRIVATE_KEY" >&2
  exit 1
fi

gpg --batch --yes --pinentry-mode loopback --default-key "$KEY_ID" --clearsign -o InRelease Release
gpg --batch --yes --pinentry-mode loopback --default-key "$KEY_ID" --armor --detach-sign -o Release.gpg Release
cd ../..

# Binary (not armored) keyring so users can curl it straight into
# /etc/apt/keyrings without a gpg --dearmor step.
gpg --batch --yes --export "$KEY_ID" > "${REPO}-archive-keyring.gpg"

cat <<EOF > index.html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${REPO} APT Repository</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; max-width: 800px; margin: 40px auto; padding: 0 20px; line-height: 1.6; color: #24292f; }
    pre { background: #f6f8fa; padding: 16px; border-radius: 6px; overflow-x: auto; font-size: 14px; }
    code { font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace; }
    h1 { border-bottom: 1px solid #d0d7de; padding-bottom: 8px; }
    a { color: #0969da; text-decoration: none; }
    a:hover { text-decoration: underline; }
  </style>
</head>
<body>
  <h1>${REPO} APT Repository</h1>
  <p>Official Debian &amp; Ubuntu APT repository for <a href="https://github.com/${OWNER}/${REPO}">${REPO}</a>.</p>

  <h2>Installation on Debian / Ubuntu</h2>
  <pre><code># 1. Add the repository signing key
sudo mkdir -p /etc/apt/keyrings
curl -fsSL ${PAGES_URL}/${REPO}-archive-keyring.gpg | sudo tee /etc/apt/keyrings/${REPO}-archive-keyring.gpg &gt; /dev/null

# 2. Add the repository
echo "deb [signed-by=/etc/apt/keyrings/${REPO}-archive-keyring.gpg] ${PAGES_URL} ${SUITE} ${COMPONENT}" | sudo tee /etc/apt/sources.list.d/${REPO}.list

# 3. Install
sudo apt update
sudo apt install ${PKG_NAME}</code></pre>

  <h2>Upgrade</h2>
  <pre><code>sudo apt update &amp;&amp; sudo apt upgrade</code></pre>
</body>
</html>
EOF

echo ":: Committing and pushing to ${PAGES_BRANCH}..."
git config user.name "${REPO}-ci"
git config user.email "ci@noreply.${REPO}"
git add -A
if git diff --cached --quiet; then
  echo ":: APT repository already up to date, nothing to push."
else
  git commit -m "Deploy v${VERSION} to APT repository"
  git push origin "$PAGES_BRANCH"
  echo ":: Successfully published v${VERSION} to ${PAGES_BRANCH} APT repository (${PAGES_URL})."
fi
