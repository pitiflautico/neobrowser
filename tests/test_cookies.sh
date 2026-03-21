#!/bin/bash
# NeoRender V2 — Cookie Persistence Test
NEORENDER=target/release/neorender

echo "=== Cookie Persistence Test ==="

# Step 1: Set a cookie via httpbin
echo -n "[set-cookie] "
result=$(echo -e "eval document.title\nquit" | timeout 15 $NEORENDER interact "https://httpbin.org/cookies/set/testcookie/neorender_v2" 2>&1)
echo "navigated"

# Step 2: Check cookie persists
echo -n "[read-cookie] "
result=$(echo -e "eval document.body.textContent\nquit" | timeout 15 $NEORENDER interact "https://httpbin.org/cookies" 2>&1)
if echo "$result" | grep -q "neorender_v2"; then
    echo "PASS — cookie persisted"
else
    echo "FAIL — cookie not found"
    echo "$result" | tail -3
fi
