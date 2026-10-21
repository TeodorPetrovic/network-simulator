#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
    echo "Usage: $0 backup <bundle.tar.gz> | restore <bundle.tar.gz>" >&2
    exit 2
}
[[ $# == 2 ]] || usage
action=$1
bundle=$2
[[ $action == backup || $action == restore ]] || usage
command -v sqlite3 >/dev/null 2>&1 || { echo "sqlite3 CLI is required" >&2; exit 1; }
command -v tar >/dev/null 2>&1 || { echo "tar is required" >&2; exit 1; }

data_home=${NETWORK_SIMULATOR_DATA_HOME:-${XDG_DATA_HOME:-${HOME:-/var/lib/network-simulator}/.local/share}/network-simulator}
db=${NETWORK_SIMULATOR_DB:-./data/network-simulator.sqlite}
image_home=${NETWORK_SIMULATOR_IMAGE_HOME:-$data_home/images}
qemu_images=${NETWORK_SIMULATOR_QEMU_IMAGES:-$image_home/qemu}
dynamips_images=${NETWORK_SIMULATOR_DYNAMIPS_IMAGES:-$image_home/dynamips}
appliances=${NETWORK_SIMULATOR_APPLIANCES:-$data_home/appliances}
service=${NETWORK_SIMULATOR_SERVICE_NAME:-network-simulator.service}

tmp_root=${XDG_CACHE_HOME:-${HOME:-/var/cache}/.cache}/network-simulator-backup
mkdir -p "$tmp_root"
work=$(mktemp -d "$tmp_root/operation.XXXXXXXX")
cleanup() { rm -rf -- "$work"; }
trap cleanup EXIT

if [[ $action == backup ]]; then
    [[ -f $db ]] || { echo "database does not exist: $db" >&2; exit 1; }
    mkdir -p "$work/payload/database" "$work/payload/images"
    sql_db=${work//\'/\'\'}
    sqlite3 "$db" ".backup '$sql_db/payload/database/network-simulator.sqlite'"
    for pair in "qemu:$qemu_images" "dynamips:$dynamips_images" "appliances:$appliances"; do
        key=${pair%%:*}
        path=${pair#*:}
        mkdir -p "$work/payload/images/$key"
        if [[ -d $path ]]; then
            cp -a -- "$path/." "$work/payload/images/$key/"
        fi
    done
    printf 'DATABASE=%s\nQEMU_IMAGES=%s\nDYNAMIPS_IMAGES=%s\nAPPLIANCES=%s\n' \
        "$db" "$qemu_images" "$dynamips_images" "$appliances" >"$work/payload/restore-paths.txt"
    mkdir -p "$(dirname "$bundle")"
    tar -czf "$bundle" -C "$work/payload" .
    echo "Backup written: $bundle"
    exit 0
fi

[[ -f $bundle ]] || { echo "backup bundle does not exist: $bundle" >&2; exit 1; }
tar -tzf "$bundle" | awk '
    /^\// || /(^|\/)\.\.($|\/)/ { bad=1 }
    END { exit bad }
' || { echo "backup bundle contains unsafe paths" >&2; exit 1; }
tar -xzf "$bundle" -C "$work"
source_db="$work/database/network-simulator.sqlite"
[[ -f $source_db ]] || { echo "backup bundle is missing database/network-simulator.sqlite" >&2; exit 1; }
integrity=$(sqlite3 "$source_db" 'PRAGMA integrity_check;')
[[ $integrity == ok ]] || { echo "backup database failed integrity_check: $integrity" >&2; exit 1; }

was_active=0
if command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet "$service"; then
    systemctl stop "$service"
    was_active=1
fi
restart_service() {
    if (( was_active )); then
        systemctl start "$service" || echo "Warning: could not restart $service" >&2
    fi
}
trap 'restart_service; cleanup' EXIT

mkdir -p "$(dirname "$db")"
sql_source=${source_db//\'/\'\'}
sqlite3 "$db" ".restore '$sql_source'"

restore_tree() {
    local source=$1 destination=$2
    mkdir -p "$destination"
    find "$destination" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
    if [[ -d $source ]]; then
        cp -a -- "$source/." "$destination/"
    fi
}
restore_tree "$work/images/qemu" "$qemu_images"
restore_tree "$work/images/dynamips" "$dynamips_images"
restore_tree "$work/images/appliances" "$appliances"
echo "Database and image directories restored from: $bundle"
