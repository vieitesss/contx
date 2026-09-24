#!/usr/bin/env bash
# Opt-in local scheduler timing with real Git commands and scripted PR-bearing gh.
# The response is freshly checked on each run, but the latency is synthetic,
# NOT a measurement of GitHub's network or a <2s SLA.
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/roots"
cat > "$tmp/bin/gh" <<'SH'
#!/bin/sh
if [ "$*" != 'pr list --head topic --state all --json number,state,headRepositoryOwner,headRefName' ]; then
    echo "unexpected gh request: $*" >&2
    exit 1
fi
printf '%s\n' "$PWD" >> "$CONTX_BENCH_GH_LOG"
sleep "${CONTX_BENCH_GH_DELAY:-1.2}"
printf '%s\n' '[{"number":42,"state":"OPEN","headRepositoryOwner":{"login":"bench"},"headRefName":"topic"}]'
SH
chmod +x "$tmp/bin/gh"
for i in {0..69}; do
    repo="$tmp/roots/repo$i"
    mkdir "$repo"
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null git -C "$repo" init -q -b topic
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null git -C "$repo" \
        -c user.name=bench -c user.email=bench@example.com -c commit.gpgsign=false \
        commit -q --allow-empty -m initial
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null git -C "$repo" \
        remote add origin "https://github.com/bench/repo$i.git"
done
printf '70 distinct throwaway Git roots/remotes; valid OPEN PR JSON; gh delay=%ss; four PR workers.\n' "${CONTX_BENCH_GH_DELAY:-1.2}"
CONTX_BENCH_ROOTS="$tmp/roots" CONTX_BENCH_GH_LOG="$tmp/gh.log" \
    PATH="$tmp/bin:$PATH" cargo test --release --locked \
    tui::git::tests::candidate_to_head_and_fresh_pr_benchmark -- \
    --exact --ignored --nocapture --test-threads=1
calls=$(wc -l < "$tmp/gh.log" | tr -d ' ')
printf 'Fresh gh invocations (no cached responses): %s (expected 290)\n' "$calls"
test "$calls" -eq 290
