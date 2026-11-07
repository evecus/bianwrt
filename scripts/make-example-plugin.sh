#!/bin/bash
# 打包示例插件为 .rpkg (tar.gz)
# 用法: ./make-example-plugin.sh [输出目录] [插件名|all]
#   ./make-example-plugin.sh .            # 打包全部示例
#   ./make-example-plugin.sh . demo-proxy # 只打 proxy 示例
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-.}"
ONLY="${2:-all}"
mkdir -p "$OUT"

# 官方 SDK 是单一来源，打包前同步到 proxy 示例的 ui/ 并设置脚本可执行位
cp "$ROOT/sdk/bianwrt-sdk.js" "$ROOT/examples/demo-proxy/ui/bianwrt-sdk.js"
chmod +x "$ROOT/examples/demo-proxy/bin/server.py"

pack() {
  case "$1" in
    demo-hello)  tar czf "$OUT/demo-hello.rpkg"  -C "$ROOT/examples/demo-hello"  manifest.toml ui/ ;;
    demo-rootfs) tar czf "$OUT/demo-rootfs.rpkg" -C "$ROOT/examples/demo-rootfs" manifest.toml rootfs/ ui/ ;;
    demo-proxy)  tar czf "$OUT/demo-proxy.rpkg"  -C "$ROOT/examples/demo-proxy"  manifest.toml bin/ ui/ ;;
    *) echo "未知示例: $1"; exit 1 ;;
  esac
  echo "已生成: $OUT/$1.rpkg"
}

if [ "$ONLY" = "all" ]; then
  pack demo-hello
  pack demo-rootfs
  pack demo-proxy
else
  pack "$ONLY"
fi

echo
echo "安装: 登录面板 → 插件中心 → 安装 .rpkg 插件包 → 选择生成的 .rpkg"
echo "proxy 示例会占用 127.0.0.1:19090，装好后从左侧「服务」分组进入。"
