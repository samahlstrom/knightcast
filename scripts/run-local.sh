#!/usr/bin/env bash
# Starts only this app and, if explicitly requested, its camera sidecar.
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(dirname -- "$script_dir")
asset_root=${KNIGHTCAST_ASSET_ROOT:-"$repo_dir/../stream-kit"}
binary=${KNIGHTCAST_BIN:-"$repo_dir/target/release/knightcast"}
python=${KNIGHTCAST_PYTHON:-"$HOME/.local/share/stream-kit/openseeface-venv/bin/python"}
camera=''
port=39539
fps=30
native_args=()
usage() {
  cat <<'EOF'
Usage: scripts/run-local.sh [--asset-root PATH] [--camera DEVICE] [--port PORT] [--fps FPS] [-- NATIVE_ARGS...]
Starts the native window. Camera capture starts ONLY with explicit --camera.
Assets default to the adjacent stream-kit directory (or KNIGHTCAST_ASSET_ROOT).
The release binary must already be built. Default camera mode: MJPG1280x720@30,
inference640x360. For other capture options, launch native_capture.py separately.
Override executables with KNIGHTCAST_BIN / KNIGHTCAST_PYTHON if needed.
EOF
}
while (($#)); do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --asset-root|--camera|--port|--fps)
      (($# >= 2)) || { printf 'Missing value for %s\n' "$1" >&2; exit 2; }
      case "$1" in
        --asset-root) asset_root=$2 ;;
        --camera) camera=$2; [[ -n "$camera" ]] || { printf 'Camera cannot be empty.\n' >&2; exit 2; } ;;
        --port) port=$2 ;;
        --fps) fps=$2 ;;
      esac
      shift 2 ;;
    --) shift; native_args=("$@"); break ;;
    *) printf 'Unknown launcher option: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done
[[ -x "$binary" ]] || { printf 'Build first: cargo build --release --locked (missing %s)\n' "$binary" >&2; exit 1; }
[[ -d "$asset_root" ]] || { printf 'Asset root does not exist: %s\n' "$asset_root" >&2; exit 1; }
if [[ -n "$camera" && ! -x "$python" ]]; then
  printf 'OpenSeeFace Python is missing: %s\n' "$python" >&2; exit 1
fi
pids=()
cleanup() {
  trap - EXIT INT TERM
  if ((${#pids[@]})); then
    kill -TERM "${pids[@]}" 2>/dev/null || true
    # Bound shutdown even if a child is blocked inside a native driver/library.
    for ((attempt=0; attempt<50; attempt++)); do
      running=0
      for pid in "${pids[@]}"; do
        if kill -0 "$pid" 2>/dev/null; then running=1; fi
      done
      ((running)) || break
      sleep .1
    done
    kill -KILL "${pids[@]}" 2>/dev/null || true
    for pid in "${pids[@]}"; do wait "$pid" 2>/dev/null || true; done
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
"$binary" --asset-root "$asset_root" --port "$port" "${native_args[@]}" &
pids+=("$!")
if [[ -n "$camera" ]]; then
  "$python" "$script_dir/native_capture.py" --camera "$camera" --port "$port" --fps "$fps" &
  pids+=("$!")
fi
# Closing the app or a failed tracker stops only the children started above.
status=0
wait -n "${pids[@]}" || status=$?
exit "$status"
