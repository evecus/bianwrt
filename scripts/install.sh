#!/bin/bash
# bianwrt 部署脚本 — 在目标 Debian/Ubuntu 服务器上执行
# 用法: sudo ./install.sh [项目根目录]  (默认取脚本所在目录的上级)
set -e

ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
# 自适应两种布局: 源码树(web/dist) 与 release tar 包(web/)
if [ -f "$ROOT/web/dist/index.html" ]; then
  WEB="$ROOT/web/dist"
elif [ -f "$ROOT/web/index.html" ]; then
  WEB="$ROOT/web"
else
  WEB=""
fi
if [ -x "$ROOT/target/release/bianwrt-server" ]; then
  BIN="$ROOT/target/release/bianwrt-server"
elif [ -x "$ROOT/bianwrt-server" ]; then
  BIN="$ROOT/bianwrt-server"
else
  BIN=""
fi

[ -n "$BIN" ] || { echo "未找到 bianwrt-server 二进制（target/release/ 或 tar 包根目录）"; exit 1; }
[ -n "$WEB" ] || { echo "未找到前端构建产物 web/dist（源码需先 pnpm build）"; exit 1; }

# 依赖
for cmd in nft; do
  command -v $cmd >/dev/null || echo "[提示] 建议安装 nftables: apt install nftables"
done

install -Dm755 "$BIN" /usr/local/bin/bianwrt-server
mkdir -p /opt/bianwrt/plugins /var/lib/bianwrt
cp -r "$WEB" /opt/bianwrt/web
install -Dm644 "$ROOT/scripts/bianwrt.service" /etc/systemd/system/bianwrt.service
systemctl daemon-reload
systemctl enable --now bianwrt.service

echo
echo "bianwrt 已启动: http://<服务器IP>:8088"
echo "数据目录: /var/lib/bianwrt   插件目录: /opt/bianwrt/plugins"
