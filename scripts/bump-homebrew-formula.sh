#!/bin/sh
# 把 Homebrew tap 里的 formula 指向某个已发布的 tag。
#
# 用法：scripts/bump-homebrew-formula.sh v0.2.0
#
# tap 是另一个仓库，release.yml 里 github.token 只对本仓库有权限，所以要一个
# 对 tap 有 Contents:write 的 token，从 GH_TOKEN 读。
# 本地试跑：DRY_RUN=1 scripts/bump-homebrew-formula.sh v0.2.0（只打印 diff，不推）
set -eu

tag=${1:-}
if [ -z "$tag" ]; then
    echo "usage: $0 <tag>   e.g. $0 v0.2.0" >&2
    exit 64
fi

: "${SOURCE_REPO:=BetterMacNet/bmtop}"
: "${TAP_REPO:=BetterMacNet/homebrew-tap}"
: "${FORMULA_PATH:=Formula/bmtop.rb}"
: "${DRY_RUN:=0}"

version=${tag#v}
tarball="https://github.com/$SOURCE_REPO/archive/refs/tags/$tag.tar.gz"

if [ "$DRY_RUN" != "1" ] && [ -z "${GH_TOKEN:-}" ]; then
    echo "GH_TOKEN is empty: need a token with Contents:write on $TAP_REPO" >&2
    exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# tag 刚推上去，GitHub 生成源码 tarball 有几秒延迟，重试而不是直接失败。
attempt=1
while :; do
    if curl -fsSL --retry 3 -o "$work/source.tar.gz" "$tarball"; then
        break
    fi
    if [ "$attempt" -ge 5 ]; then
        echo "source tarball still missing after $attempt attempts: $tarball" >&2
        exit 1
    fi
    echo "tarball not ready yet (attempt $attempt), waiting 10s"
    attempt=$((attempt + 1))
    sleep 10
done

# 校验拿到的确实是那个 tag 的源码，而不是一个 404 页面或空文件。
if ! tar tzf "$work/source.tar.gz" | head -1 | grep -q "^bmtop-$version/"; then
    echo "tarball does not look like bmtop $version source" >&2
    exit 1
fi
# macOS 有 shasum，Linux runner 上 sha256sum 更保准，两个都认。
if command -v shasum >/dev/null 2>&1; then
    sha=$(shasum -a 256 "$work/source.tar.gz" | cut -d' ' -f1)
else
    sha=$(sha256sum "$work/source.tar.gz" | cut -d' ' -f1)
fi
echo "$tag source sha256: $sha"

if [ "$DRY_RUN" = "1" ] && [ -z "${GH_TOKEN:-}" ]; then
    clone_url="https://github.com/$TAP_REPO.git"
else
    clone_url="https://x-access-token:$GH_TOKEN@github.com/$TAP_REPO.git"
fi
git clone --depth 1 --quiet "$clone_url" "$work/tap"

# 用 python 而不是 sed：改完要能断言两个字段各命中一次，
# formula 结构一变就报错，而不是悄悄什么都没改还当成功。
python3 - "$work/tap/$FORMULA_PATH" "$tarball" "$sha" <<'PY'
import pathlib
import re
import sys

path, url, sha = sys.argv[1], sys.argv[2], sys.argv[3]
formula = pathlib.Path(path)
text = formula.read_text()
text, urls = re.subn(r'^(\s*url\s+)"[^"]*"', lambda m: m.group(1) + '"%s"' % url, text, count=1, flags=re.M)
text, shas = re.subn(r'^(\s*sha256\s+)"[^"]*"', lambda m: m.group(1) + '"%s"' % sha, text, count=1, flags=re.M)
if urls != 1 or shas != 1:
    sys.exit("formula shape changed: url matches=%d sha256 matches=%d" % (urls, shas))
formula.write_text(text)
PY

# 交叉验证：新的 tag 与 sha 真的落进文件了。
grep -q "$tag.tar.gz" "$work/tap/$FORMULA_PATH"
grep -q "$sha" "$work/tap/$FORMULA_PATH"

cd "$work/tap"
if git diff --quiet -- "$FORMULA_PATH"; then
    echo "formula already points at $tag, nothing to do"
    exit 0
fi
git --no-pager diff -- "$FORMULA_PATH"

if [ "$DRY_RUN" = "1" ]; then
    echo "DRY_RUN=1: not committing or pushing"
    exit 0
fi

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add "$FORMULA_PATH"
git commit -q -m "feat: bmtop $version" \
    -m "https://github.com/$SOURCE_REPO/releases/tag/$tag"
git push --quiet origin HEAD
echo "pushed formula bump for $tag to $TAP_REPO"
