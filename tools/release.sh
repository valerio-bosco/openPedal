#!/usr/bin/env bash
# Releases one openPedal product: runs the checks, bumps its version, regenerates its
# changelog with git-cliff, then commits `chore(release): <product> vX.Y.Z` and creates the
# annotated tag `<product>-vX.Y.Z`. It never pushes. See docs/RELEASING.md.
#
# Usage: tools/release.sh <desktop|ios> <X.Y.Z> [--dry-run] [--skip-checks]
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: tools/release.sh <desktop|ios> <X.Y.Z> [--dry-run] [--skip-checks]

  desktop   binary + web UI: version in Cargo.toml, tag desktop-vX.Y.Z, CHANGELOG.md
  ios       iPad app: version in ios/Info.plist (+ build number), tag ios-vX.Y.Z, ios/CHANGELOG.md

  --dry-run      print what would happen and change nothing (a dirty tree or another
                 branch only produces a warning)
  --skip-checks  do not run the build and test checks
EOF
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

dry_run=0
skip_checks=0
product=""
version=""
for arg in "$@"; do
  case "$arg" in
    --dry-run) dry_run=1 ;;
    --skip-checks) skip_checks=1 ;;
    -h | --help) usage; exit 0 ;;
    -*) usage >&2; echo "error: unknown option: $arg" >&2; exit 2 ;;
    *)
      if [ -z "$product" ]; then
        product="$arg"
      elif [ -z "$version" ]; then
        version="$arg"
      else
        usage >&2; echo "error: too many arguments" >&2; exit 2
      fi
      ;;
  esac
done

die() { echo "error: $*" >&2; exit 1; }
warn() { echo "warning: $*" >&2; }

# A failed precondition stops a real release; in a dry run it is only reported.
require() {
  if [ "$dry_run" -eq 1 ]; then
    warn "$1 (a real release would stop here)"
  else
    die "$1"
  fi
}

# Runs a command, or only prints it in a dry run.
step() {
  local label="$1"
  shift
  if [ "$dry_run" -eq 1 ]; then
    echo "[dry-run] $label: $*"
    return 0
  fi
  echo "==> $label"
  "$@"
}

case "$product" in
  desktop)
    prefix=desktop
    other_scope=ios
    changelog=CHANGELOG.md
    version_files=(Cargo.toml Cargo.lock CHANGELOG.md)
    ;;
  ios)
    prefix=ios
    other_scope=ui
    changelog=ios/CHANGELOG.md
    version_files=(ios/Info.plist ios/CHANGELOG.md)
    ;;
  *)
    usage >&2
    die "product must be 'desktop' or 'ios', got '${product:-nothing}'"
    ;;
esac

if ! [[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  usage >&2
  die "version must be X.Y.Z (e.g. 0.2.0), got '${version:-nothing}'"
fi

tag="$prefix-v$version"
tag_pattern="$prefix-v[0-9]*"

# Preconditions.
branch="$(git rev-parse --abbrev-ref HEAD)"
if [ "$branch" != master ]; then
  require "not on master (on '$branch')"
fi
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  require "the working tree has uncommitted changes"
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  die "tag $tag already exists"
fi

if command -v git-cliff >/dev/null 2>&1; then
  cliff=(git-cliff)
else
  cliff=(nix shell nixpkgs#git-cliff -c git-cliff)
fi

# One cliff.toml serves both products. Scopes are matched on the summary line, and the
# first matching commit parser wins, so the other product's scope is skipped first.
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
config="$tmp/cliff.toml"
rule='  { message = "^[a-z]+\\('"$other_scope"'\\)!?:", skip = true },'
RULE="$rule" awk '{ print } /^commit_parsers = \[/ { print ENVIRON["RULE"] }' cliff.toml > "$config"
grep -qF "$rule" "$config" || die "could not add the scope rule to cliff.toml (no commit_parsers?)"
cliff_args=(--config "$config" --tag-pattern "$tag_pattern" --tag "$tag")

echo "Releasing $product $version as $tag"

# 1. Checks.
if [ "$skip_checks" -eq 1 ]; then
  warn "checks skipped (--skip-checks)"
elif [ "$product" = desktop ]; then
  step "cargo test" nix develop -c cargo test
  step "clippy" nix develop -c cargo clippy --all-targets -- -D warnings
  step "web UI build" bash -c 'cd ui && npm run build'
else
  step "cargo test" nix develop -c cargo test
  step "Rust core for the iPad" nix develop -c ios/scripts/build-core.sh
  step "xtool build" ios/scripts/xtool.sh dev build
fi

# 2. Version bump. The python snippets only print their changes in a dry run.
if [ "$product" = desktop ]; then
  python3 - "$version" "$dry_run" "$root/Cargo.toml" <<'PY'
import re, sys
version, dry, path = sys.argv[1], sys.argv[2] == "1", sys.argv[3]
text = open(path, encoding="utf-8").read()
pattern = re.compile(r'(^\[package\]\n(?:(?!\[).*\n)*?)^(version = ")([^"]*)(")', re.M)
m = pattern.search(text)
if not m:
    sys.exit("error: no version in the [package] table of Cargo.toml")
print(f"Cargo.toml: version {m.group(3)} -> {version}")
if not dry:
    text = pattern.sub(lambda m: m.group(1) + m.group(2) + version + m.group(4), text, count=1)
    open(path, "w", encoding="utf-8").write(text)
PY
  step "refresh Cargo.lock" nix develop -c cargo update -p openpedal --offline
else
  python3 - "$version" "$dry_run" "$root/ios/Info.plist" <<'PY'
import re, sys
version, dry, path = sys.argv[1], sys.argv[2] == "1", sys.argv[3]
text = open(path, encoding="utf-8").read()

def get(key):
    m = re.search(r'<key>' + re.escape(key) + r'</key>\s*<string>([^<]*)</string>', text)
    return m.group(1) if m else None

def put(text, key, value):
    pattern = re.compile(r'(<key>' + re.escape(key) + r'</key>\s*<string>)[^<]*(</string>)')
    if pattern.search(text):
        return pattern.sub(lambda m: m.group(1) + value + m.group(2), text, count=1)
    # Missing key: add it at the top of the root dict.
    return text.replace("<dict>\n", f"<dict>\n    <key>{key}</key>\n    <string>{value}</string>\n", 1)

old_short = get("CFBundleShortVersionString") or "unset"
old_build = get("CFBundleVersion")
if old_build is None:
    build = "1"
elif old_build.isdigit():
    build = str(int(old_build) + 1)
else:
    sys.exit(f"error: CFBundleVersion is not a number: {old_build!r}")
print(f"Info.plist: CFBundleShortVersionString {old_short} -> {version}, "
      f"CFBundleVersion {old_build or 'unset'} -> {build}")
if not dry:
    text = put(text, "CFBundleShortVersionString", version)
    text = put(text, "CFBundleVersion", build)
    open(path, "w", encoding="utf-8").write(text)
PY
fi

# 3. Changelog for this product, with the new version on the unreleased commits.
if [ "$dry_run" -eq 1 ]; then
  echo "[dry-run] changelog: git-cliff ${cliff_args[*]} --output $changelog"
  "${cliff[@]}" "${cliff_args[@]}" --output "$tmp/preview.md"
  awk '/^## \[/ { n++ } n == 1' "$tmp/preview.md" > "$tmp/section.md"
  echo "[dry-run] new section of $changelog:"
  sed -n '1,25p' "$tmp/section.md" | sed 's/^/    /'
else
  echo "==> regenerate $changelog"
  "${cliff[@]}" "${cliff_args[@]}" --output "$changelog"
fi

# 4. Release commit and annotated tag. Only the release files are staged.
step "stage release files" git add "${version_files[@]}"
step "commit" git commit -m "chore(release): $product v$version"
step "tag" git tag -a "$tag" -m "openPedal $product v$version"

if [ "$dry_run" -eq 1 ]; then
  echo "dry run: nothing was changed."
else
  echo "Created commit and tag $tag. Nothing was pushed."
fi
echo "To publish (you run this): git push origin master $tag"
