#!/usr/bin/env bash
# save_baselines.sh — copy Criterion JSON results into benches/baselines/ for
# committing and later comparison with critcmp.
#
# Usage:
#   scripts/save_baselines.sh [label]
#
# If [label] is not provided, the current git branch name is used.
#
# After running benchmarks with `cargo bench`, call this script to snapshot
# the results. Commit the baselines/ directory so CI can compare future runs.
#
# Comparing against a saved baseline:
#   cargo install critcmp
#   critcmp main candidate --threshold 5
#
# This reports all benchmarks that regressed or improved by more than 5%.
# Use this in CI to gate merges on performance regressions.
#
# Generating HTML reports (requires gnuplot):
#   cargo bench
#   open target/criterion/report/index.html
#
# The HTML report contains violin plots, PDF distributions, and iteration time
# graphs for every benchmark group and individual benchmark.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CRITERION_DIR="$REPO_ROOT/target/criterion"
BASELINES_DIR="$REPO_ROOT/benches/baselines"

# Determine the label for this baseline snapshot.
if [[ $# -ge 1 ]]; then
    LABEL="$1"
else
    LABEL="$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unnamed")"
fi

DEST="$BASELINES_DIR/$LABEL"

if [[ ! -d "$CRITERION_DIR" ]]; then
    echo "ERROR: $CRITERION_DIR does not exist. Run 'cargo bench' first." >&2
    exit 1
fi

echo "Saving baselines from $CRITERION_DIR → $DEST"
mkdir -p "$DEST"

# Copy only the JSON estimates files (not HTML/SVG — those are large and
# already gitignored via target/).
find "$CRITERION_DIR" -name "estimates.json" | while read -r f; do
    # Reconstruct the relative path under target/criterion/.
    rel="${f#$CRITERION_DIR/}"
    dest_file="$DEST/$rel"
    mkdir -p "$(dirname "$dest_file")"
    cp "$f" "$dest_file"
done

echo "Saved $(find "$DEST" -name 'estimates.json' | wc -l) benchmark estimates."
echo ""
echo "To compare this baseline against another:"
echo "  critcmp $LABEL <other-label> --threshold 5"
echo ""
echo "Commit the baselines directory to make this the reference for CI:"
echo "  git add benches/baselines/$LABEL"
echo "  git commit -m 'perf: update $LABEL benchmarks baseline'"
