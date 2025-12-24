#!/bin/bash
# 生成自签名 TLS 证书的脚本

set -e

echo "正在生成 TLS 证书..."

# 检查 OpenSSL 是否可用
if ! command -v openssl &> /dev/null; then
    echo "错误: 未找到 openssl 命令"
    echo "请安装 OpenSSL"
    exit 1
fi

# 生成私钥
echo ""
echo "步骤 1/2: 生成私钥 (key.pem)..."
openssl genrsa -out key.pem 2048
echo "✓ 私钥生成成功"

# 生成自签名证书
echo ""
echo "步骤 2/2: 生成自签名证书 (cert.pem)..."
openssl req -new -x509 -key key.pem -out cert.pem -days 365 -subj "/CN=localhost"
echo "✓ 证书生成成功"

# 显示结果
echo ""
echo "证书信息:"
openssl x509 -in cert.pem -noout -subject -dates

echo ""
echo "✓ 证书生成完成！"
echo "  - 私钥文件: key.pem"
echo "  - 证书文件: cert.pem"
echo ""
echo "注意: 这是自签名证书，仅用于测试。生产环境请使用正式 CA 签发的证书。"
