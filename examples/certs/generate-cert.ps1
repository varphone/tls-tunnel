#!/usr/bin/env pwsh
# 生成自签名 TLS 证书的脚本

Write-Host "正在生成 TLS 证书..." -ForegroundColor Green

# 检查 OpenSSL 是否可用
if (-not (Get-Command openssl -ErrorAction SilentlyContinue)) {
    Write-Host "错误: 未找到 openssl 命令" -ForegroundColor Red
    Write-Host "请安装 OpenSSL 或使用 Git Bash" -ForegroundColor Yellow
    exit 1
}

# 生成私钥
Write-Host "`n步骤 1/2: 生成私钥 (key.pem)..." -ForegroundColor Cyan
openssl genrsa -out key.pem 2048

if ($LASTEXITCODE -ne 0) {
    Write-Host "错误: 生成私钥失败" -ForegroundColor Red
    exit 1
}

Write-Host "✓ 私钥生成成功" -ForegroundColor Green

# 生成自签名证书
Write-Host "`n步骤 2/2: 生成自签名证书 (cert.pem)..." -ForegroundColor Cyan
openssl req -new -x509 -key key.pem -out cert.pem -days 365 -subj "/CN=localhost"

if ($LASTEXITCODE -ne 0) {
    Write-Host "错误: 生成证书失败" -ForegroundColor Red
    exit 1
}

Write-Host "✓ 证书生成成功" -ForegroundColor Green

# 显示结果
Write-Host "`n证书信息:" -ForegroundColor Yellow
openssl x509 -in cert.pem -noout -subject -dates

Write-Host "`n✓ 证书生成完成！" -ForegroundColor Green
Write-Host "  - 私钥文件: key.pem" -ForegroundColor Cyan
Write-Host "  - 证书文件: cert.pem" -ForegroundColor Cyan
Write-Host "`n注意: 这是自签名证书，仅用于测试。生产环境请使用正式 CA 签发的证书。" -ForegroundColor Yellow
