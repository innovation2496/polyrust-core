#!/bin/bash
# WebSocket Market Channel Smoke Test Assertion Script
# Usage: ./smoke_ws_assert.sh <log_file> <jsonl_file> <pid>
# Exit 0 = PASS, Exit 1 = FAIL

set -e

LOG_FILE="${1:-data/regression_15min.log}"
JSONL_FILE="${2:-data/ws_regression_15min.jsonl}"
PID="${3:-}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "=== WebSocket Market Channel Smoke Test Assertion ==="
echo "Log file: $LOG_FILE"
echo "JSONL file: $JSONL_FILE"
echo ""

FAIL=0

# 1. Parse last progress line
echo "--- Checking Progress Log ---"
LAST_LINE=$(tail -1 "$LOG_FILE" | grep -oP 'Progress:.*' || echo "")

if [ -z "$LAST_LINE" ]; then
    echo -e "${RED}FAIL: Could not parse progress line${NC}"
    FAIL=1
else
    echo "Last progress: $LAST_LINE"

    # Extract unknown count
    UNKNOWN=$(echo "$LAST_LINE" | grep -oP 'unknown=\K\d+' || echo "-1")
    RECONNECTS=$(echo "$LAST_LINE" | grep -oP 'reconnects=\K\d+' || echo "-1")
    TOTAL=$(echo "$LAST_LINE" | grep -oP 'total=\K\d+' || echo "-1")

    echo "  unknown=$UNKNOWN, reconnects=$RECONNECTS, total=$TOTAL"

    # Assert unknown == 0
    if [ "$UNKNOWN" != "0" ]; then
        echo -e "${RED}FAIL: unknown=$UNKNOWN (expected 0)${NC}"
        FAIL=1
    else
        echo -e "${GREEN}PASS: unknown=0${NC}"
    fi

    # Assert reconnects == 0
    if [ "$RECONNECTS" != "0" ]; then
        echo -e "${RED}FAIL: reconnects=$RECONNECTS (expected 0)${NC}"
        FAIL=1
    else
        echo -e "${GREEN}PASS: reconnects=0${NC}"
    fi

    # Assert total > 0
    if [ "$TOTAL" -le "0" ] 2>/dev/null; then
        echo -e "${RED}FAIL: total=$TOTAL (expected > 0)${NC}"
        FAIL=1
    else
        echo -e "${GREEN}PASS: total=$TOTAL${NC}"
    fi
fi

# 2. Check file growth
echo ""
echo "--- Checking File Growth ---"
if [ -f "$JSONL_FILE" ]; then
    LINES_BEFORE=$(wc -l < "$JSONL_FILE")
    sleep 5
    LINES_AFTER=$(wc -l < "$JSONL_FILE")
    GROWTH=$((LINES_AFTER - LINES_BEFORE))

    echo "Lines before: $LINES_BEFORE, after: $LINES_AFTER, growth: $GROWTH"

    if [ "$GROWTH" -le "0" ]; then
        echo -e "${YELLOW}WARN: No file growth in 5 seconds (may be expected if market ended)${NC}"
    else
        echo -e "${GREEN}PASS: File growing (+$GROWTH lines in 5s)${NC}"
    fi
else
    echo -e "${RED}FAIL: JSONL file not found${NC}"
    FAIL=1
fi

# 3. Check RSS if PID provided
echo ""
echo "--- Checking Resource Usage ---"
if [ -n "$PID" ] && ps -p "$PID" > /dev/null 2>&1; then
    RSS=$(ps -p "$PID" -o rss= | tr -d ' ')
    echo "RSS: ${RSS}KB"

    # Simple check: RSS should be less than 100MB for this workload
    if [ "$RSS" -gt "102400" ]; then
        echo -e "${YELLOW}WARN: RSS=${RSS}KB seems high (>100MB)${NC}"
    else
        echo -e "${GREEN}PASS: RSS=${RSS}KB (reasonable)${NC}"
    fi
else
    echo "No PID provided or process not running, skipping RSS check"
fi

# 4. Summary
echo ""
echo "=== Summary ==="
if [ "$FAIL" -eq "0" ]; then
    echo -e "${GREEN}PASS: All assertions passed${NC}"
    exit 0
else
    echo -e "${RED}FAIL: One or more assertions failed${NC}"
    exit 1
fi
