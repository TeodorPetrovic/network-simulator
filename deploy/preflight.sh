#!/usr/bin/env bash
set -Eeuo pipefail

failures=0
warnings=0

ok()   { printf 'PASS  %s\n' "$*"; }
warn() { printf 'WARN  %s\n' "$*"; warnings=$((warnings + 1)); }
fail() { printf 'FAIL  %s\n' "$*"; failures=$((failures + 1)); }

resolve_runtime() {
    local variable=$1 program=$2 override=${!1-} data_home=${NETWORK_SIMULATOR_DATA_HOME:-}
    if [[ -n "$override" ]]; then
        printf '%s\n' "$override"
    elif command -v "$program" >/dev/null 2>&1; then
        command -v "$program"
    else
        if [[ -z "$data_home" ]]; then
            data_home="${XDG_DATA_HOME:-${HOME:-/var/lib/network-simulator}/.local/share}/network-simulator"
        fi
        printf '%s\n' "$data_home/bin/$program"
    fi
}

resolved_path=
check_executable() {
    local variable=$1 program=$2
    resolved_path=$(resolve_runtime "$variable" "$program")
    if [[ -x "$resolved_path" ]]; then
        ok "$program executable: $resolved_path"
    else
        fail "$program is missing or not executable (set $variable or install it on PATH); checked $resolved_path"
        resolved_path=
    fi
}

docker=${NETWORK_SIMULATOR_DOCKER:-$(command -v docker || true)}
if [[ -z "$docker" || ! -x "$docker" ]]; then
    fail "docker executable is missing (set NETWORK_SIMULATOR_DOCKER)"
elif "$docker" info >/dev/null 2>&1; then
    ok "Docker CLI and daemon are available"
else
    fail "Docker daemon is unavailable to $(id -un); check the docker group/socket"
fi

check_executable NETWORK_SIMULATOR_QEMU qemu-system-x86_64
qemu=$resolved_path
check_executable NETWORK_SIMULATOR_QEMU_IMG qemu-img
qemu_img=$resolved_path
check_executable NETWORK_SIMULATOR_VPCS vpcs
vpcs=$resolved_path
check_executable NETWORK_SIMULATOR_UBRIDGE ubridge
ubridge=$resolved_path
check_executable NETWORK_SIMULATOR_DYNAMIPS dynamips
dynamips=$resolved_path

if [[ -n "$qemu" ]]; then
    if "$qemu" --version 2>&1 | head -1 | grep -Eq 'QEMU emulator version [0-9]'; then
        ok "QEMU reports a version"
    else
        fail "QEMU version probe failed: $qemu --version"
    fi
fi
if [[ -n "$qemu_img" ]]; then
    if "$qemu_img" --version 2>&1 | head -1 | grep -Eq 'qemu-img version [0-9]'; then
        ok "qemu-img reports a version"
    else
        fail "qemu-img version probe failed: $qemu_img --version"
    fi
fi
if [[ -n "$vpcs" ]]; then
    if "$vpcs" -v >/dev/null 2>&1 || "$vpcs" -h >/dev/null 2>&1; then
        ok "VPCS executable responds to a version/help probe"
    else
        warn "VPCS probe returned a nonzero status; the executable is present"
    fi
fi
if [[ -n "$ubridge" ]]; then
    if "$ubridge" -v >/dev/null 2>&1 || "$ubridge" -h >/dev/null 2>&1; then
        ok "uBridge executable responds to a version/help probe"
    else
        warn "uBridge probe returned a nonzero status; the executable is present"
    fi
fi
if [[ -n "$dynamips" ]]; then
    version_output=$("$dynamips" -P none 2>&1 || true)
    if printf '%s\n' "$version_output" | python3 -c '
import re, sys
text = sys.stdin.read()
m = re.search(r"version\s+(\d+)\.(\d+)\.(\d+)", text, re.I)
sys.exit(0 if m and tuple(map(int, m.groups())) >= (0, 2, 23) else 1)
'; then
        detected=$(printf '%s\n' "$version_output" | grep -Eio 'version[[:space:]]+[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)
        ok "Dynamips $detected meets minimum version 0.2.23"
    else
        fail "Dynamips version probe did not report version 0.2.23 or newer"
    fi
fi

if [[ -c /dev/kvm && -r /dev/kvm && -w /dev/kvm ]]; then
    ok "KVM is present and accessible to $(id -un)"
elif [[ ${PREFLIGHT_REQUIRE_KVM:-1} == 1 ]]; then
    fail "KVM is required but /dev/kvm is missing or inaccessible; add the service account to kvm"
else
    warn "KVM is unavailable; QEMU will use software emulation"
fi

if command -v ip >/dev/null 2>&1 && ip link show "${NETWORK_SIMULATOR_NAT_INTERFACE:-virbr0}" >/dev/null 2>&1; then
    ok "libvirt NAT interface ${NETWORK_SIMULATOR_NAT_INTERFACE:-virbr0} exists"
else
    fail "libvirt NAT interface ${NETWORK_SIMULATOR_NAT_INTERFACE:-virbr0} is missing; start libvirt's default network"
fi

data_dir=${NETWORK_SIMULATOR_DATA_HOME:-/var/lib/network-simulator}
min_free_gb=${PREFLIGHT_MIN_FREE_GB:-20}
if [[ -d "$data_dir" ]]; then
    disk_dir=$data_dir
else
    disk_dir=$(dirname "${NETWORK_SIMULATOR_DB:-/var/lib/network-simulator/network-simulator.sqlite}")
fi
if [[ -e "$disk_dir" ]]; then
    free_kb=$(df -Pk "$disk_dir" | awk 'NR==2 {print $4}')
else
    free_kb=$(df -Pk "$(dirname "$disk_dir")" | awk 'NR==2 {print $4}')
fi
if [[ "$free_kb" =~ ^[0-9]+$ ]] && (( free_kb >= min_free_gb * 1024 * 1024 )); then
    ok "at least ${min_free_gb} GiB is free for projects, images, and runtime state"
else
    fail "need ${min_free_gb} GiB free at $disk_dir (set PREFLIGHT_MIN_FREE_GB to adjust)"
fi

bind=${NETWORK_SIMULATOR_BIND:-127.0.0.1:3080}
bind_port=${bind##*:}
extra_ports=${NETWORK_SIMULATOR_PREFLIGHT_PORTS:-}
ports="$bind_port${extra_ports:+,$extra_ports}"
if python3 - "$bind" "$ports" <<'PY'
import socket, sys
bind = sys.argv[1]
host, port_text = bind.rsplit(":", 1)
host = host.strip("[]") or "0.0.0.0"
for port_text in sys.argv[2].split(","):
    if not port_text.strip():
        continue
    port = int(port_text)
    family = socket.AF_INET6 if ":" in host else socket.AF_INET
    address = (host, port, 0, 0) if family == socket.AF_INET6 else (host, port)
    sock = socket.socket(family, socket.SOCK_STREAM)
    try:
        sock.bind(address)
    except OSError as exc:
        print(f"port {host}:{port} is unavailable: {exc}", file=sys.stderr)
        sys.exit(1)
    finally:
        sock.close()
PY
then
    ok "controller and configured preflight TCP ports are available ($ports)"
else
    fail "one or more controller/preflight TCP ports are already in use ($ports)"
fi

printf '\nPreflight finished: %d failure(s), %d warning(s).\n' "$failures" "$warnings"
(( failures == 0 ))
