#!/bin/bash
# bianwrt 部署脚本 — 在目标 Debian/Ubuntu 服务器上执行
# 前端资源已内嵌进二进制，部署只需一个可执行文件
# 用法: sudo ./install.sh [目录]  (默认取脚本所在目录的上级)
set -e

ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
if [ -x "$ROOT/target/release/bianwrt-server" ]; then
  BIN="$ROOT/target/release/bianwrt-server"
elif [ -x "$ROOT/bianwrt-server" ]; then
  BIN="$ROOT/bianwrt-server"
else
  echo "未找到 bianwrt-server 二进制（target/release/ 或当前目录）"; exit 1
fi

# 依赖
for cmd in nft; do
  command -v $cmd >/dev/null || echo "[提示] 建议安装 nftables: apt install nftables"
done

install -Dm755 "$BIN" /usr/local/bin/bianwrt-server
mkdir -p /etc/bianwrt/data /etc/bianwrt/plugins /etc/bianwrt/conf
install -Dm644 "$ROOT/scripts/bianwrt.service" /etc/systemd/system/bianwrt.service
systemctl daemon-reload
systemctl enable --now bianwrt.service

echo
echo "bianwrt 已启动: http://<服务器IP>:8088（前端资源内嵌于二进制）"
echo "工作目录: /etc/bianwrt（data=数据 plugins=插件 conf=插件配置，可用 BIANWRT_WORK_DIR 指定）"
