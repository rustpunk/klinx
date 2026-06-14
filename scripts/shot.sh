#!/usr/bin/env bash
# Headless UI screenshot for klinx (wry/WebKitGTK).
#
# Renders the desktop app inside a virtual framebuffer (Xvfb) with WebKitGTK on
# software rendering, then grabs the root window with ImageMagick `import`, so a
# UI change can be eyeballed without a physical display (CI / agent sessions).
# The capture is at the X-server level, so it is renderer-agnostic. This is
# render-and-eyeball, NOT golden-image diffing. See CLAUDE.md "Headless UI
# verification" for the hover/click (xdotool) and crop recipes.
#
# Usage:
#   scripts/shot.sh [OUT_PNG] [WORKSPACE]
#     OUT_PNG    screenshot path        (default: ./klinx-shot.png)
#     WORKSPACE  --workspace dir to open (default: ./examples/pipelines)
#
# Env overrides: KLINX_BIN, KLINX_XVFB_DISPLAY, KLINX_XVFB_SIZE, KLINX_WAIT.
# Prereqs: xvfb-run (xvfb), ImageMagick (import), mesa software GL (llvmpipe),
# and a built binary (cargo build --package klinx).
set -euo pipefail

export OUT="${1:-./klinx-shot.png}"
export WS="${2:-./examples/pipelines}"
export BIN="${KLINX_BIN:-./target/debug/klinx}"
export WAIT="${KLINX_WAIT:-10}"
DISP="${KLINX_XVFB_DISPLAY:-88}"
SIZE="${KLINX_XVFB_SIZE:-1400x900x24}"

if [ ! -x "$BIN" ]; then
  echo "error: $BIN not found — run 'cargo build --package klinx' first" >&2
  exit 1
fi
for tool in xvfb-run import; do
  command -v "$tool" >/dev/null || { echo "error: missing '$tool'" >&2; exit 1; }
done

# The inner shell inherits OUT/WS/BIN/WAIT via the exported environment. Force
# X11 + software GL; WEBKIT_DISABLE_DMABUF_RENDERER is the load-bearing one
# (webkit2gtk-4.1 will not paint under Xvfb without it).
xvfb-run -n "$DISP" -s "-screen 0 $SIZE" bash -c '
  export WAYLAND_DISPLAY= GDK_BACKEND=x11 \
         WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 \
         LIBGL_ALWAYS_SOFTWARE=1 NO_AT_BRIDGE=1
  "$BIN" --workspace "$WS" >/dev/null 2>&1 &
  app=$!
  sleep "$WAIT"
  import -window root "$OUT"
  kill "$app" 2>/dev/null || true
  wait "$app" 2>/dev/null || true
'
echo "wrote $OUT"
