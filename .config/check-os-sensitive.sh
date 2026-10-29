#!/usr/bin/env bash
# Fails when a source file branches on the OS, yet the `os-sensitive` nextest
# profile selects none of its module's tests. Pull requests run only that
# profile on macOS and Windows, so such a branch would first be tested there
# on `main`.
#
# A file's tests are the ones named after its module path: `src/io/lock.rs`
# needs a selected test named `io::lock::…`, and `lib.rs` or `main.rs` one
# named `tests::…`. Files named `tests.rs` or `*_tests.rs` are skipped. A file
# with a reason to stay out is listed in `os-sensitive-exceptions`.
#
# The check reads the list nextest gives on the host it runs on, so it proves
# that the profile names the module, not that each OS builds a test there.
#
# Arguments go to `cargo nextest list` and, with the directory it is run from,
# pick the crates to check.
set -euo pipefail

if ! command -v jq >/dev/null; then
    echo "check-os-sensitive.sh needs jq: https://jqlang.org/download/" >&2
    exit 1
fi

list=$(cargo nextest list --profile os-sensitive --message-format json "$@")
cd "$(git rev-parse --show-toplevel)"
exceptions=.config/os-sensitive-exceptions
root=$PWD

# The tests selected in each crate in scope, by its directory.
selected=$(jq -r '."rust-suites"[] | .cwd as $cwd | .testcases
    | to_entries[] | select(.value."filter-match".status == "matches")
    | "\($cwd)\t\(.key)"' <<<"$list")

# A `cfg` can span lines, so it is read on until its parentheses close.
branches_on_os() {
    awk 'cfg != "" || /cfg!?\(/ {
             cfg = cfg $0
             if (gsub(/\(/, "(", cfg) <= gsub(/\)/, ")", cfg)) {
                 if (cfg ~ /cfg!?\(.*(windows|unix|target_os)/) found = 1
                 cfg = ""
             }
         }
         END { exit !found }' "$1"
}

excepted() {
    awk -v file="$1" '$1 == file { found = 1 } END { exit !found }' "$exceptions"
}

has_selected_test() {
    awk -F '\t' -v dir="$1" -v module="$2" \
        '$1 == dir && index($2, module) == 1 { found = 1 } END { exit !found }' <<<"$selected"
}

failed=0
while IFS= read -r dir; do
    rel=${dir#"$root"/}
    while IFS= read -r file; do
        case "$file" in */tests.rs | *_tests.rs) continue ;; esac
        if ! branches_on_os "$file"; then
            if excepted "$file"; then
                echo "$file is in $exceptions but no longer branches on the OS"
                failed=1
            fi
            continue
        fi
        excepted "$file" && continue
        module=${file#"$rel/src/"}
        module=${module%.rs}
        module=${module%/mod}
        module=${module//\//::}
        case "$module" in lib | main) module=tests ;; esac
        if ! has_selected_test "$dir" "$module::"; then
            echo "$file branches on the OS, but the os-sensitive profile selects none of its tests"
            failed=1
        fi
    done < <(git ls-files "$rel/src/*.rs")
done < <(jq -r '."rust-suites"[].cwd' <<<"$list" | sort -u)

if [ "$failed" = 1 ]; then
    echo
    echo "Add the module's tests to [profile.os-sensitive] in .config/nextest.toml,"
    echo "or list the file in $exceptions with a reason."
    exit 1
fi
