#!/usr/bin/env bash
#
# Code Complexity Analysis Report Generator
#
# This script analyzes the complexity of Rust code in this repository by running
# complexity tests and generating a summary report highlighting the most complex functions.

set -euo pipefail

# Colors for output (harmonized with Makefile)
COLOR_RESET=$(tput sgr0)
# shellcheck disable=SC2034  # Used in printf statements when formatting output
COLOR_BOLD=$(tput bold)
# shellcheck disable=SC2034  # Reserved for future output highlighting
COLOR_GREEN=$(tput setaf 2)
# shellcheck disable=SC2034  # Reserved for future output highlighting
COLOR_YELLOW=$(tput setaf 3)
COLOR_BLUE=$(tput setaf 4)
#
# What it does:
#   1. Runs cargo test complexity to execute complexity analysis tests
#   2. Filters out test framework noise and formatting artifacts
#   3. Extracts and summarizes complexity metrics from the test output
#   4. Identifies the top 3 most complex functions in two categories:
#      - Cyclomatic Complexity: Measures control flow complexity (branches, loops, conditions)
#      - Data Flow Complexity: Measures data dependency and state management complexity
#   5. Generates a readable report saved to dev/scripts/complexity_report.txt
#
# Output:
#   - Report is displayed in the terminal (via tee)
#   - Also saved to: dev/scripts/complexity_report.txt
#   - Shows full complexity analysis plus a summary of top 3 most complex functions
#
# Metrics explained:
#   - Cyclomatic Complexity: Higher values indicate more decision points (if/else, loops, matches)
#     * Lower is better (simpler control flow)
#     * Values > 10-15 may indicate functions that need refactoring
#   - Data Flow Complexity: Measures how data moves through the function
#     * Higher values indicate complex state management or data dependencies
#     * Helps identify functions with potential maintainability issues
#
# Usage:
#   ./dev/scripts/complexity_report.sh
#   cat dev/scripts/complexity_report.txt  # View the saved report
#
# Requirements:
#   - Rust toolchain (cargo)
#   - Complexity tests must be defined in tests/ directory
#   - Project must compile and tests must run successfully
#

# Get the script directory and repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
REPORT_FILE="$SCRIPT_DIR/complexity_report.txt"

cd "$PROJECT_ROOT"

printf "%bRunning complexity tests...%b\n" "$COLOR_BLUE" "$COLOR_RESET" >&2
# Capture raw output first so we can produce a meaningful report even when
# there are no matching tests or no analyzer output.
RAW_OUTPUT="$(cargo test complexity -- --nocapture 2>&1)"

printf "%s\n" "$RAW_OUTPUT" | grep -vE "(^running|^test result:|^test tests::|Finished.*test.*profile|Running unittests|Running tests/)" | sed '/^$/N;/^\n$/d' | awk '
  BEGIN {
    line_count=0
    nonempty_count=0
    ignored_file_re="(tests/other/data_flow_complexity\\.rs|tests/other/cyclomatic_complexity\\.rs)"
  }
  /^=== Cyclomatic Complexity Report ===/ { section="cyclomatic"; delete top3_cyclomatic; count_cyc=0 }
  /^=== Data Flow Complexity Report/ { section="dataflow"; delete top3_dataflow; count_df=0 }
  /^=== Top 10 Most Complex Functions ===/ && section=="cyclomatic" { in_top10=1; count_cyc=0; next }
  /^=== Top 10 Most Complex Functions ===/ && section=="dataflow" { in_top10=1; count_df=0; next }
  in_top10 && /^[0-9]+\./ && section=="cyclomatic" && count_cyc < 3 && $0 !~ ignored_file_re { top3_cyclomatic[++count_cyc]=$0 }
  in_top10 && /^[0-9]+\./ && section=="dataflow" && count_df < 3 && $0 !~ ignored_file_re { top3_dataflow[++count_df]=$0 }
  in_top10 && /^===/ { in_top10=0 }
  in_top10 && /^[0-9]+\./ && $0 ~ ignored_file_re { next }
  {
    lines[++line_count]=$0
    if ($0 !~ /^[[:space:]]*$/) nonempty_count++
  }
  END {
    if (line_count == 0 || nonempty_count == 0) {
      print "=== Complexity Report ==="
      print ""
      print "No complexity analyzer output was found."
      print ""
      print "Most likely reason:"
      print "- No tests matched '\''cargo test complexity'\''."
      print ""
      print "Tip: add tests containing '\''complexity'\'' in their name, then rerun."
      exit 0
    }

    for (i=1; i<=line_count; i++) print lines[i]
    print ""
    print "=== Evaluation Summary: Top 3 Highest Scores ==="
    print ""
    print "Cyclomatic Complexity - Top 3 Functions:"
    for (i=1; i<=3; i++) if (top3_cyclomatic[i]) print top3_cyclomatic[i]
    print ""
    print "Data Flow Complexity - Top 3 Functions:"
    for (i=1; i<=3; i++) if (top3_dataflow[i]) print top3_dataflow[i]
  }
' | tee "$REPORT_FILE"

echo "" >&2
printf "%bReport saved to: %s%b\n" "$COLOR_BLUE" "$REPORT_FILE" "$COLOR_RESET" >&2