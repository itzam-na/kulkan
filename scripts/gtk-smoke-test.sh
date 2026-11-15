#!/usr/bin/env bash
set -euo pipefail

script_path=$(realpath "$0")
repo_root=$(cd "$(dirname "$script_path")/.." && pwd)
stage=${1:-}

find_smoke_servers() {
  local command_file pid executable command_line server_socket
  server_socket="$KULKAN_SMOKE_ROOT/runtime/org.kulkan.Kulkan/server.sock"
  for command_file in /proc/[0-9]*/cmdline; do
    [[ -r "$command_file" ]] || continue
    pid=${command_file#/proc/}
    pid=${pid%/cmdline}
    executable=$(readlink "/proc/$pid/exe" 2>/dev/null || true)
    [[ "$executable" == "$repo_root/target/debug/kulkan-server" ]] || continue
    command_line=$(tr '\0' ' ' <"$command_file" 2>/dev/null || true)
    [[ "$command_line" == *"$server_socket"* ]] && printf '%s\n' "$pid"
  done
}

wait_for_app_exit() {
  local state
  for _ in {1..100}; do
    state=$(ps -o stat= -p "$app_pid" 2>/dev/null || true)
    if [[ -z "$state" || "$state" == Z* ]]; then
      wait "$app_pid" 2>/dev/null || true
      app_pid=
      return 0
    fi
    sleep 0.1
  done
  return 1
}

stop_smoke_servers() {
  local pid
  while read -r pid; do
    [[ -n "$pid" ]] && kill "$pid" 2>/dev/null || true
  done < <(find_smoke_servers)
}

if [[ "$stage" == "--inside" ]]; then
  "$repo_root/target/debug/kulkan" >"$KULKAN_SMOKE_ROOT/kulkan.log" 2>&1 &
  app_pid=$!
  cleanup() {
    local window_id
    window_id=$(xdotool search --onlyvisible --name "Kulkán" 2>/dev/null | head -n 1 || true)
    if [[ -n "$window_id" ]]; then
      xdotool windowclose "$window_id" 2>/dev/null || true
    fi
    if ! wait_for_app_exit; then
      kill "$app_pid" 2>/dev/null || true
      wait "$app_pid" 2>/dev/null || true
      app_pid=
    fi
    stop_smoke_servers
  }
  trap cleanup EXIT

  if ! python3 "$repo_root/scripts/gtk_smoke_test.py"; then
    cat "$KULKAN_SMOKE_ROOT/kulkan.log" >&2
    exit 1
  fi
  window_id=$(xdotool search --onlyvisible --name "Kulkán" | head -n 1)
  xdotool windowclose "$window_id"
  if ! wait_for_app_exit; then
    echo "Kulkán did not exit after the smoke test closed its window" >&2
    exit 1
  fi
  if [[ -n "$(find_smoke_servers)" ]]; then
    echo "Kulkán left its child server running after the smoke test" >&2
    exit 1
  fi
  exit 0
fi

if [[ "$stage" == "--weston" ]]; then
  weston \
    --backend=headless \
    --renderer=pixman \
    --xwayland \
    --socket="$WAYLAND_DISPLAY" \
    --width=1280 \
    --height=800 \
    --idle-time=0 \
    --no-config \
    -- "$script_path" --inside
  exit $?
fi

for tool in cargo dbus-run-session weston xdotool import tesseract python3; do
  command -v "$tool" >/dev/null || {
    echo "GTK smoke test requires $tool" >&2
    exit 2
  }
done

cargo build --workspace --locked

smoke_root=$(mktemp -d "${TMPDIR:-/tmp}/kulkan-gtk-smoke.XXXXXX")
mkdir -p \
  "$smoke_root/runtime" \
  "$smoke_root/home" \
  "$smoke_root/data" \
  "$smoke_root/config" \
  "$smoke_root/cache"
chmod 700 "$smoke_root/runtime"
export KULKAN_SMOKE_ROOT="$smoke_root"
export HOME="$smoke_root/home"
export XDG_RUNTIME_DIR="$smoke_root/runtime"
export XDG_DATA_HOME="$smoke_root/data"
export XDG_CONFIG_HOME="$smoke_root/config"
export XDG_CACHE_HOME="$smoke_root/cache"
export WAYLAND_DISPLAY=wayland-kulkan-smoke
export GDK_BACKEND=x11
export GSETTINGS_BACKEND=memory
export GTK_A11Y=none
export GIO_USE_VFS=local

set +e
dbus-run-session -- "$script_path" --weston >"$smoke_root/weston.log" 2>&1
result=$?
set -e
if (( result != 0 )); then
  cat "$smoke_root/weston.log" >&2
else
  echo "GTK Library workspace smoke test passed"
fi
rm -rf -- "$smoke_root"
exit "$result"
