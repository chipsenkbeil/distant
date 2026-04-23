#!/bin/bash
# Removes all distant FileProvider domains from macOS.
#
# Clears provider-level domain registrations (Domains.plist), per-domain
# databases (UUID dirs in Application Support/FileProvider), group container
# metadata, and restarts fileproviderd to flush FPFS state.
#
# The FPFS kernel-level CloudStorage ghost directories may persist until
# reboot, but they will be gone from Finder's sidebar immediately.
#
# Usage: bash scripts/clean-fp-domains.sh

set -euo pipefail

FP_APP_SUPPORT="$HOME/Library/Application Support/FileProvider"
PROVIDER_DIR="$FP_APP_SUPPORT/dev.distant.file-provider"
TEST_PROVIDER_DIR="$FP_APP_SUPPORT/dev.distant.test.file-provider"

echo "Removing all distant FileProvider domains..."

# Step 1: Clear Domains.plist (unregister all domains from the provider)
for pdir in "$PROVIDER_DIR" "$TEST_PROVIDER_DIR"; do
    plist="$pdir/Domains.plist"
    if [ -f "$plist" ]; then
        count=$(plutil -p "$plist" 2>/dev/null | grep -c "dev\.distant\." || true)
        # Overwrite with empty plist dictionary
        plutil -create xml1 /tmp/_empty_fp.plist 2>/dev/null || true
        cp /tmp/_empty_fp.plist "$plist"
        rm -f /tmp/_empty_fp.plist
        echo "Cleared $count domains from $plist"
    fi
done

# Step 2: Remove per-domain data directories under the provider dir
for pdir in "$PROVIDER_DIR" "$TEST_PROVIDER_DIR"; do
    if [ -d "$pdir" ]; then
        count=$(ls "$pdir" 2>/dev/null | grep -c "^dev\.distant\." || true)
        rm -rf "$pdir"/dev.distant.* 2>/dev/null
        echo "Removed $count per-domain data dirs from $pdir"
    fi
done

# Step 3: Remove UUID-based domain databases mapped to Distant
# fileproviderctl dump shows which UUIDs have com.apple.file-provider-domain-id
# xattrs pointing to dev.distant.file-provider/*
removed_uuids=0
for uuid_dir in "$FP_APP_SUPPORT"/[0-9A-F]*-*-*-*-[0-9A-F]*; do
    [ -d "$uuid_dir" ] || continue
    uuid=$(basename "$uuid_dir")
    domain_id=$(xattr -p com.apple.file-provider-domain-id "$uuid_dir" 2>/dev/null || true)
    if echo "$domain_id" | grep -q "^dev\.distant"; then
        rm -rf "$uuid_dir"
        removed_uuids=$((removed_uuids + 1))
    fi
done
echo "Removed $removed_uuids UUID domain databases"

# Step 4: Clean group container domain metadata
for group_dir in \
    "$HOME/Library/Group Containers/39C6AGD73Z.group.dev.distant/domains" \
    "$HOME/Library/Group Containers/group.dev.distant.test/domains"; do
    if [ -d "$group_dir" ]; then
        count=$(ls "$group_dir" 2>/dev/null | wc -l | tr -d ' ')
        rm -rf "$group_dir"/* 2>/dev/null
        echo "Cleaned $count domain metadata files from $group_dir"
    fi
done

# Step 5: Disable extension, kill fileproviderd, re-enable extension
# Disabling first ensures fileproviderd drops all FPFS state for the provider
pluginkit -e ignore -i dev.distant.file-provider 2>/dev/null && echo "Disabled extension" || true
sleep 1
killall fileproviderd 2>/dev/null && echo "Killed fileproviderd" || true
sleep 2
pluginkit -e use -i dev.distant.file-provider 2>/dev/null && echo "Re-enabled extension" || true
sleep 1

# Step 6: Report
remaining=$(ls ~/Library/CloudStorage/ 2>/dev/null | grep -c Distant || true)
if [ "$remaining" -gt 0 ]; then
    echo ""
    echo "$remaining FPFS ghost entries remain in ~/Library/CloudStorage/"
    echo "These are kernel-level artifacts that will disappear after reboot."
    echo "They are already gone from Finder's sidebar."
else
    echo "All CloudStorage entries removed."
fi
echo "Done"
