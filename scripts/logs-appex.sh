#!/usr/bin/env bash
# logs-appex.sh — Quick viewer for the distant FileProvider appex logs.
#
# Usage:
#   scripts/logs-appex.sh              # Show last 5 minutes of unified logs + latest log file
#   scripts/logs-appex.sh --minutes 10 # Show last 10 minutes
#   scripts/logs-appex.sh --follow     # Stream live logs (log stream)
#   scripts/logs-appex.sh --crashes    # List recent crash reports

set -euo pipefail

MINUTES=5
MODE="show"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --minutes|-m)
            MINUTES="$2"
            shift 2
            ;;
        --follow|-f)
            MODE="follow"
            shift
            ;;
        --crashes|-c)
            MODE="crashes"
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [--minutes N] [--follow] [--crashes]"
            echo ""
            echo "  --minutes N, -m N   Show last N minutes of logs (default: 5)"
            echo "  --follow, -f        Stream live logs"
            echo "  --crashes, -c       List recent crash reports"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

APPEX_LOG_DIR="$HOME/Library/Group Containers/39C6AGD73Z.group.dev.distant/logs"
APPEX_LOG_DIR_LEGACY="$HOME/Library/Containers/dev.distant.file-provider/Data/tmp"
CRASH_DIR="$HOME/Library/Logs/DiagnosticReports"

case "$MODE" in
    show)
        echo "=== Unified log (last ${MINUTES}m, process=distant) ==="
        log show --predicate 'process == "distant"' --last "${MINUTES}m" --style compact 2>/dev/null || true
        echo ""

        echo "=== Latest appex log file ==="
        LATEST=""
        for dir in "$APPEX_LOG_DIR" "$APPEX_LOG_DIR_LEGACY"; do
            if [[ -d "$dir" ]]; then
                CANDIDATE=$(find "$dir" -name '*.log' -type f 2>/dev/null | sort -r | head -1)
                if [[ -n "$CANDIDATE" ]]; then
                    LATEST="$CANDIDATE"
                    break
                fi
            fi
        done
        if [[ -n "$LATEST" ]]; then
            echo "($LATEST)"
            cat "$LATEST"
        else
            echo "(no log files found in $APPEX_LOG_DIR or $APPEX_LOG_DIR_LEGACY)"
        fi
        ;;

    follow)
        echo "=== Streaming live logs (process=distant) — Ctrl-C to stop ==="
        log stream --predicate 'process == "distant"' --style compact
        ;;

    crashes)
        echo "=== Recent crash reports matching 'distant' ==="
        if [[ -d "$CRASH_DIR" ]]; then
            find "$CRASH_DIR" -name '*distant*' -type f -mtime -7 2>/dev/null | sort -r | head -20
            COUNT=$(find "$CRASH_DIR" -name '*distant*' -type f -mtime -7 2>/dev/null | wc -l | tr -d ' ')
            echo "(${COUNT} crash reports in last 7 days)"
        else
            echo "(directory not found: $CRASH_DIR)"
        fi
        ;;
esac
