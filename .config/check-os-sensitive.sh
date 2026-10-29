#!/usr/bin/env bash
# Fails when a source file branches on the OS outside its tests, yet the
# `os-sensitive` nextest profile selects none of its module's tests. Pull
# requests run only that profile on macOS and Windows, so such a branch would
# first be tested there on `main`.
#
# A file's tests are the ones named after its module path: `src/io/lock.rs`
# needs a selected test named `io::lock::…`. Its own code is the part above
# its inline `mod tests {`; files named `tests.rs` or `*_tests.rs` are all test.
# A file with a reason to stay out is listed in `os-sensitive-exceptions`.
#
# Arguments go to `cargo nextest list` and, with the directory it is run from,
# pick the crates to check.
set -euo pipefail

list=$(cargo nextest list --profile os-sensitive --message-format json "$@")
cd "$(git rev-parse --show-toplevel)"
exceptions=.config/os-sensitive-exceptions
root=$PWD

# The crates in scope, by directory, and the tests selected in each.
dirs=$(jq -r '."rust-suites"[].cwd' <<<"$list" | sort -u)
selected=$(jq -r '."rust-suites"[] | .cwd as $cwd | .testcases
    | to_entries[] | select(.value."filter-match".status == "matches")
    | "\($cwd)\t\(.key)"' <<<"$list")

# A `cfg` can span lines, so it is read on until its parentheses close.
branches_on_os() {
    awk '/^[[:space:]]*mod [a-z_]*tests[[:space:]]*\{/ { exit }
         cfg != "" || /cfg!?\(/ {
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
for dir in $dirs; do
    rel=${dir#"$root"/}
    for file in $(git ls-files "$rel/src/*.rs"); do
        case $file in */tests.rs | *_tests.rs) continue ;; esac
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
        case $module in lib | main) module= ;; *) module="$module::" ;; esac
        if ! has_selected_test "$dir" "$module"; then
            echo "$file branches on the OS, but the os-sensitive profile selects none of its tests"
            failed=1
        fi
    done
done

if [ "$failed" = 1 ]; then
    echo
    echo "Add the module's tests to [profile.os-sensitive] in .config/nextest.toml,"
    echo "or list the file in $exceptions with a reason."
    exit 1
fi
