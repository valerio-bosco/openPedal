#!/usr/bin/env bash
# Releases one openPedal product: runs the checks, bumps its version, then commits
# `chore(release): <product> vVERSION` and creates the annotated tag `<product>-vVERSION`.
# A stable release (X.Y.Z, on master) also regenerates the changelog with git-cliff; a dev
# pre-release (X.Y.Z-dev.N, on dev) leaves it alone. It never pushes. See docs/RELEASING.md.
#
# Usage: tools/release.sh <desktop|ios> <X.Y.Z|X.Y.Z-dev.N|X.Y.Z-dev|next-dev> [--dry-run] [--skip-checks]
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: tools/release.sh <desktop|ios> <version> [--dry-run] [--skip-checks]

  desktop   binary + web UI: workspace version in Cargo.toml, tag desktop-vX.Y.Z, apps/desktop/CHANGELOG.md
  ios       iPad app: version in apps/ios/Info.plist (+ build number), tag ios-vX.Y.Z, apps/ios/CHANGELOG.md

  version   X.Y.Z          stable release, only on master (regenerates the changelog)
            X.Y.Z-dev.N    dev pre-release, only on dev (changelog untouched)
            next-dev       on dev: the next -dev.N of the upcoming version (the minor after
                           the latest stable tag, or the dev series already in progress)
            X.Y.Z-dev      on dev: the next -dev.N of X.Y.Z

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
    changelog=apps/desktop/CHANGELOG.md
    version_files=(Cargo.toml Cargo.lock)
    ;;
  ios)
    prefix=ios
    other_scope=ui
    changelog=apps/ios/CHANGELOG.md
    version_files=(apps/ios/Info.plist)
    ;;
  *)
    usage >&2
    die "product must be 'desktop' or 'ios', got '${product:-nothing}'"
    ;;
esac

num='(0|[1-9][0-9]*)'
core_re="^$num\.$num\.$num\$"
stable_tag_re="^$prefix-v$num\.$num\.$num\$"

# Version of this product in the working tree (X.Y.Z or X.Y.Z-dev.N).
current_version() {
  if [ "$product" = desktop ]; then
    python3 - "$root/Cargo.toml" <<'PY'
import re, sys
text = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r'^\[workspace\.package\]\n(?:(?!\[).*\n)*?^version = "([^"]*)"', text, re.M)
print(m.group(1) if m else "")
PY
  else
    python3 - "$root/apps/ios/Info.plist" <<'PY'
import re, sys
text = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r'<key>CFBundleShortVersionString</key>\s*<string>([^<]*)</string>', text)
print(m.group(1) if m else "")
PY
  fi
}

# Greatest X.Y.Z among the lines on stdin, or nothing.
max_core() { sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1; }

# Next -dev.N for the upcoming version $1 (X.Y.Z): one more than the highest existing N.
next_dev_for() {
  local core="$1" last
  last="$(git tag --list "$prefix-v$core-dev.*" | sed -n "s/^$prefix-v$core-dev\.\([0-9][0-9]*\)\$/\1/p" | sort -n | tail -n 1)"
  echo "$core-dev.$(( ${last:-0} + 1 ))"
}

# The upcoming stable version: continue a dev series already in progress, else the minor
# after the latest stable tag (or after the version in the tree when there is no stable tag).
upcoming_core() {
  local stable dev major minor from_tree=0
  stable="$(git tag --list "$prefix-v*" | grep -E "$stable_tag_re" | sed "s/^$prefix-v//" | max_core || true)"
  if [ -z "$stable" ]; then
    # No stable tag yet: the version in the tree stands in for it.
    stable="$(current_version | sed 's/-.*//')"
    [[ "$stable" =~ $core_re ]] || die "cannot read the current $product version"
    from_tree=1
  fi
  dev="$(git tag --list "$prefix-v*-dev.*" | sed -nE "s/^$prefix-v($num\.$num\.$num)-dev\..*/\1/p" | max_core || true)"
  # A dev series is in progress when its X.Y.Z is above the last stable one (or equal to the
  # tree's, when no stable tag exists: the iPad tree keeps X.Y.Z of the dev series).
  if [ -n "$dev" ] && { [ "$from_tree" -eq 1 ] && [ "$(printf '%s\n%s\n' "$stable" "$dev" | max_core)" = "$dev" ] ||
    [ "$dev" != "$stable" ] && [ "$(printf '%s\n%s\n' "$stable" "$dev" | max_core)" = "$dev" ]; }; then
    echo "$dev"
  else
    IFS=. read -r major minor _ <<<"$stable"
    echo "$major.$((minor + 1)).0"
  fi
}

case "$version" in
  next-dev) version="$(next_dev_for "$(upcoming_core)")" ;;
  *-dev)
    core="${version%-dev}"
    [[ "$core" =~ $core_re ]] || die "version must be X.Y.Z-dev, got '$version'"
    version="$(next_dev_for "$core")"
    ;;
esac

if [[ "$version" =~ $core_re ]]; then
  kind=stable
  core="$version"
  want_branch=master
elif [[ "$version" =~ ^$num\.$num\.$num-dev\.[1-9][0-9]*$ ]]; then
  kind=dev
  core="${version%-dev.*}"
  want_branch=dev
else
  usage >&2
  die "version must be X.Y.Z, X.Y.Z-dev.N, X.Y.Z-dev or next-dev, got '${version:-nothing}'"
fi
[ "$kind" = stable ] && version_files+=("$changelog")

tag="$prefix-v$version"
# Only stable tags delimit the changelog: -dev.N tags are ignored, so a stable release lists
# everything since the previous stable tag.
tag_pattern="$stable_tag_re"

# Preconditions.
branch="$(git rev-parse --abbrev-ref HEAD)"
if [ "$branch" != "$want_branch" ]; then
  require "a $kind release ($version) is made on $want_branch, not on '$branch'"
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

echo "Releasing $product $version ($kind) as $tag"

# 1. Checks.
if [ "$skip_checks" -eq 1 ]; then
  warn "checks skipped (--skip-checks)"
elif [ "$product" = desktop ]; then
  step "cargo test" nix develop -c cargo test --workspace
  step "clippy" nix develop -c cargo clippy --workspace --all-targets -- -D warnings
  step "web UI build" bash -c 'cd apps/desktop/ui && npm run build'
else
  step "cargo test" nix develop -c cargo test --workspace
  step "Rust core for the iPad" nix develop -c apps/ios/scripts/build-core.sh
  step "xtool build" apps/ios/scripts/xtool.sh dev build
fi

# 2. Version bump. The python snippets only print their changes in a dry run.
if [ "$product" = desktop ]; then
  python3 - "$version" "$dry_run" "$root/Cargo.toml" <<'PY'
import re, sys
version, dry, path = sys.argv[1], sys.argv[2] == "1", sys.argv[3]
text = open(path, encoding="utf-8").read()
pattern = re.compile(r'(^\[workspace\.package\]\n(?:(?!\[).*\n)*?)^(version = ")([^"]*)(")', re.M)
m = pattern.search(text)
if not m:
    sys.exit("error: no version in the [workspace.package] table of Cargo.toml")
print(f"Cargo.toml: version {m.group(3)} -> {version}")
if not dry:
    text = pattern.sub(lambda m: m.group(1) + m.group(2) + version + m.group(4), text, count=1)
    open(path, "w", encoding="utf-8").write(text)
PY
  step "refresh Cargo.lock" nix develop -c cargo update --workspace --offline
else
  # Apple wants a numeric CFBundleShortVersionString: a dev release gets its X.Y.Z and only
  # the build number tells builds apart.
  python3 - "$core" "$dry_run" "$root/apps/ios/Info.plist" <<'PY'
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
if [ "$kind" = dev ]; then
  echo "changelog: not touched for a dev pre-release"
elif [ "$dry_run" -eq 1 ]; then
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
echo "To publish (you run this): git push origin $want_branch $tag"
