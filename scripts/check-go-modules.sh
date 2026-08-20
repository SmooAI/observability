#!/usr/bin/env bash
#
# The two things that make a Go module in this repo unresolvable for anyone
# outside it — both invisible from inside, because inside the repo everything
# builds.
#
# Run from `go/`. Called by BOTH the PR lane and the publish gate: the PR lane
# so a broken module is caught before merge, the publish gate so it is caught
# before a tag exists. One script, two callers, so the two cannot drift.
set -uo pipefail

MODULES=(. fiber gin)
ROOT_PATH="github.com/SmooAI/observability/go"
status=0

# 1. No `replace` directives in a published module.
#
# Go honours `replace` only in the MAIN module's go.mod. A `replace … => ../` in
# a published module is therefore invisible to every consumer: `go get
# …/go/fiber@vX` resolves the core at whatever the require line names, ignores
# the replace, and fails. Both adapters shipped exactly that shape, requiring a
# `v0.0.0` that has never existed.
#
# Local builds get the sibling source from `go/go.work`, which is not published.
for m in "${MODULES[@]}"; do
    [ -f "$m/go.mod" ] || continue
    if go mod edit -json "$m/go.mod" | grep -qE '"Replace": *\['; then
        echo "::error::$m/go.mod has a replace directive. Consumers ignore it, so the module will not resolve. Put the override in go/go.work instead."
        status=1
    fi
done

# 2. Each go.mod declares the module path Go derives from its directory.
#
# A mismatch means the module resolves nothing at all — and nothing inside the
# repo notices, because the workspace resolves by disk path.
for m in "${MODULES[@]}"; do
    [ -f "$m/go.mod" ] || continue
    if [ "$m" = "." ]; then expected="$ROOT_PATH"; else expected="$ROOT_PATH/$m"; fi
    # Read the module line straight out of go.mod. `go list -m` reports every
    # module in the workspace, not the one in this directory.
    actual=$(awk '$1 == "module" { print $2; exit }' "$m/go.mod")
    printf '  %-6s %s\n' "$m" "$actual"
    if [ "$actual" != "$expected" ]; then
        echo "::error::$m/go.mod module path '$actual' != expected '$expected'"
        status=1
    fi
done

# 3. Majors ≥ 2 need a /vN suffix on the module path.
#
# Go requires it, and without it `go get …@v2.x` resolves nothing. Today every
# module here is 0.x so the suffix must be ABSENT; this check flips over
# automatically at the 2.0 bump rather than waiting to be remembered.
# Read the version without a JSON parser so this stays runnable from any lane,
# including the Go one, with nothing but coreutils on PATH.
version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' ../packages/core/package.json | head -1)
major="${version%%.*}"
for m in "${MODULES[@]}"; do
    [ -f "$m/go.mod" ] || continue
    path=$(awk '$1 == "module" { print $2; exit }' "$m/go.mod")
    if [ "$major" -ge 2 ]; then
        if [ "${path##*/}" != "v$major" ]; then
            echo "::error::version is $version but $m/go.mod path '$path' has no /v$major suffix — Go will not resolve it"
            status=1
        fi
    elif [[ "${path##*/}" =~ ^v[0-9]+$ ]]; then
        echo "::error::version is $version (major 0/1) but $m/go.mod path '$path' carries a /vN suffix"
        status=1
    fi
done

if [ "$status" -eq 0 ]; then
    echo "Go modules are publishable: no replace directives, paths match directories, /vN suffix matches major $major."
fi
exit "$status"
