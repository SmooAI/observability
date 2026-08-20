#!/usr/bin/env node
/**
 * One version number for five SDKs.
 *
 * `packages/core/package.json` is the single source of truth — it is the only
 * manifest changesets bumps, and it is what the org already uses to version
 * every other polyglot package (`@smooai/fetch` is 3.4.1 on npm, crates.io AND
 * PyPI). Every other version-bearing file in this repo is derived from it.
 *
 * This runs in the changesets **`version`** lifecycle, NOT after publish:
 *
 *     "version": "changeset version && node scripts/sync-versions.mjs"
 *
 * That ordering is the whole point. The changesets action commits the working
 * tree after `version`, so the synced manifests land in the release commit and
 * the git tag actually carries the versions it claims. Running the sync after
 * `publish` — the pattern in the sibling repos — mutates manifests in the CI
 * workspace that are never committed, so every tag ships stale constants and
 * `cargo publish --allow-dirty` has to exist to paper over the dirt.
 *
 * `--check` verifies instead of writing, and exits non-zero on any mismatch.
 * CI runs it on every PR; see `.github/workflows/pr-checks.yml`.
 *
 * Adding a version-bearing file? Add a row to TARGETS. A row whose pattern
 * matches zero times, or more than once, is a hard error — a silently-skipped
 * target is the failure mode this script exists to prevent.
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const SOURCE = 'packages/core/package.json';

/**
 * Every version-bearing file, and the pattern locating the version within it.
 *
 * Each `pattern` must have exactly two capture groups: everything before the
 * version, and everything after. The version itself is whatever sits between
 * them, and is replaced wholesale — so the pattern pins the *declaration*, not
 * the current value, and keeps working across bumps.
 */
const TARGETS = [
    // ---- TypeScript ------------------------------------------------------
    // The constant stamped into every event's `sdk.version`. This drifted to 18
    // minor releases behind the published package before this script existed.
    { file: 'packages/core/src/client.ts', pattern: /(const SDK_VERSION = ')[^']+(';)/ },

    // ---- Rust ------------------------------------------------------------
    { file: 'rust/observability/Cargo.toml', pattern: /(\[package\][\s\S]*?\nversion = ")[^"]+(")/ },
    { file: 'rust/observability/src/client.rs', pattern: /(pub const SDK_VERSION: &str = ")[^"]+(";)/ },
    // The lockfile carries the workspace member's own version. Miss it and
    // `cargo publish --locked` fails on a lockfile that no longer matches.
    { file: 'rust/Cargo.lock', pattern: /(name = "smooai-observability"\nversion = ")[^"]+(")/ },

    // ---- Python ----------------------------------------------------------
    { file: 'python/pyproject.toml', pattern: /(\[project\][\s\S]*?\nversion = ")[^"]+(")/ },
    { file: 'python/src/smooai_observability/__init__.py', pattern: /(__version__ = ")[^"]+(")/ },
    { file: 'python/src/smooai_observability/client.py', pattern: /(SDK_VERSION = ")[^"]+(")/ },
    { file: 'python/uv.lock', pattern: /(name = "smooai-observability"\nversion = ")[^"]+(")/ },

    // ---- Go --------------------------------------------------------------
    // The core module has no manifest version — a module's version IS its git
    // tag — so its reported SDK version is the only thing to sync there. The
    // tag/version agreement is enforced separately, in publish.yml.
    { file: 'go/types.go', pattern: /(sdkVersion = ")[^"]+(")/ },
    // The fiber/gin adapters are separate modules that depend on the core by
    // its PUBLISHED path and version. These lines are what make them
    // resolvable for anyone outside this repo, so they are version-bearing in
    // the strongest sense: get them wrong and `go get` fails outright.
    { file: 'go/fiber/go.mod', pattern: /(\tgithub\.com\/SmooAI\/observability\/go v)[^\n]+(\n)/ },
    { file: 'go/gin/go.mod', pattern: /(\tgithub\.com\/SmooAI\/observability\/go v)[^\n]+(\n)/ },
    // The workspace replace is version-specific (Go rejects an all-versions
    // replace of a workspace module), so it has to track the require lines.
    { file: 'go/go.work', pattern: /(replace github\.com\/SmooAI\/observability\/go v)[^\s]+( => \.)/ },

    // ---- .NET ------------------------------------------------------------
    { file: 'dotnet/src/SmooAI.Observability/SmooAI.Observability.csproj', pattern: /(<Version>)[^<]+(<\/Version>)/ },
    { file: 'dotnet/src/SmooAI.Observability/Client.cs', pattern: /(public const string SdkVersion = ")[^"]+(")/ },
];

const check = process.argv.includes('--check');

const version = JSON.parse(readFileSync(join(ROOT, SOURCE), 'utf8')).version;
if (typeof version !== 'string' || !/^\d+\.\d+\.\d+/.test(version)) {
    console.error(`${SOURCE} has no usable version (got ${JSON.stringify(version)})`);
    process.exit(1);
}

const mismatches = [];
let written = 0;

for (const { file, pattern } of TARGETS) {
    const path = join(ROOT, file);
    const before = readFileSync(path, 'utf8');

    // A global copy of the pattern, purely to count matches. Zero matches means
    // the declaration moved or was renamed; more than one means the pattern is
    // too loose and would rewrite something unrelated. Both are bugs in TARGETS.
    const all = before.match(new RegExp(pattern.source, `${pattern.flags.replace('g', '')}g`));
    if (!all || all.length !== 1) {
        console.error(
            `${file}: pattern matched ${all?.length ?? 0} times, expected exactly 1 — fix TARGETS in ${relative(ROOT, fileURLToPath(import.meta.url))}`,
        );
        process.exit(1);
    }

    const [full, head, tail] = pattern.exec(before);
    const current = full.slice(head.length, full.length - tail.length);
    const after = before.replace(pattern, `$1${version}$2`);

    if (after === before) continue;

    if (check) {
        mismatches.push({ file, current });
        continue;
    }

    writeFileSync(path, after, 'utf8');
    written += 1;
    console.log(`  ${file}: ${current} -> ${version}`);
}

if (check) {
    if (mismatches.length > 0) {
        console.error(`\nVersion drift: ${SOURCE} says ${version}, but ${mismatches.length} file(s) disagree.\n`);
        for (const { file, current } of mismatches) {
            console.error(`  ${file}: ${current}`);
        }
        console.error(`\nRun \`node scripts/sync-versions.mjs\` and commit the result.`);
        console.error(`If this fired on a release PR, the \`version\` lifecycle hook in package.json is not running.`);
        process.exit(1);
    }
    console.log(`All ${TARGETS.length} version-bearing files agree on ${version}.`);
} else {
    console.log(written === 0 ? `All ${TARGETS.length} version-bearing files already at ${version}.` : `Synced ${written} file(s) to ${version}.`);
}
