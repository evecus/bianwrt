#!/bin/bash
# 打包示例插件为 .rpkg (tar.gz)
# 用法: ./make-example-plugin.sh [输出目录]  默认输出到当前目录
set -e
SRC="$(cd "$(dirname "$0")/../examples/demo-hello" && pwd)"
OUT="${1:-.}"
tar czf "$OUT/demo-hello.rpkg" -C "$SRC" manifest.toml ui/
echo "已生成: $OUT/demo-hello.rpkg"
