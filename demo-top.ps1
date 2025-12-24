# Top Command Demo Script
# This script demonstrates how to use the tls-tunnel top command

Write-Host "=== TLS Tunnel Top Command Demo ===" -ForegroundColor Cyan
Write-Host ""

# Check if server is running
Write-Host "Checking if stats server is accessible..." -ForegroundColor Yellow
try {
    $response = Invoke-WebRequest -Uri "http://localhost:9090/stats" -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
    Write-Host "✓ Stats server is running on port 9090" -ForegroundColor Green
    Write-Host ""
    
    # Show current stats
    Write-Host "Current statistics (JSON):" -ForegroundColor Yellow
    $stats = $response.Content | ConvertFrom-Json
    $stats | ConvertTo-Json -Depth 3
    Write-Host ""
    
    # Show top command usage
    Write-Host "Starting interactive top command..." -ForegroundColor Yellow
    Write-Host "Controls:" -ForegroundColor Cyan
    Write-Host "  - Press 'q' or 'Esc' to quit" -ForegroundColor White
    Write-Host "  - Press 'r' to refresh manually" -ForegroundColor White
    Write-Host "  - Auto-refresh every 2 seconds (configurable with --interval)" -ForegroundColor White
    Write-Host ""
    Start-Sleep -Seconds 2
    
    # Launch top command
    & ".\target\release\tls-tunnel.exe" top --url "http://localhost:9090" --interval 2
    
} catch {
    Write-Host "✗ Stats server is not accessible" -ForegroundColor Red
    Write-Host ""
    Write-Host "To start the server with stats enabled:" -ForegroundColor Yellow
    Write-Host "  tls-tunnel server --config test-server-with-stats.toml -v" -ForegroundColor White
    Write-Host ""
    Write-Host "Make sure the server is running and stats_port is configured." -ForegroundColor Yellow
    exit 1
}
