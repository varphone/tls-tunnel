# Statistics Feature

## Overview

The TLS Tunnel server now includes a built-in HTTP statistics server that provides real-time monitoring of proxy connections and traffic.

## Features

- **Real-time Statistics**: View active connections, total connections, bytes sent/received for each proxy
- **HTML Dashboard**: Beautiful web interface with auto-refresh (5 seconds)
- **JSON API**: RESTful API endpoint for programmatic access
- **Per-Proxy Metrics**: Individual statistics for each configured proxy
- **Uptime Tracking**: Monitor how long each proxy has been running

## Configuration

Add the `stats_port` option to your server configuration:

```toml
[server]
bind_addr = "0.0.0.0"
bind_port = 3080
auth_key = "your-secret-key"
# ... other config ...

# Enable statistics server on port 9090
stats_port = 9090
```

If `stats_port` is not configured or commented out, the statistics server will not start.

## Usage

### HTML Dashboard

Access the statistics dashboard in your browser:

```
http://server-ip:9090/
```

The dashboard displays:
- Total number of proxies
- Total active connections across all proxies
- Total connections made
- Per-proxy statistics table with:
  - Proxy name
  - Published address and port
  - Client port
  - Active connections
  - Total connections
  - Bytes sent/received (formatted)
  - Uptime

The page auto-refreshes every 5 seconds.

### JSON API

Get statistics in JSON format:

```
http://server-ip:9090/stats
```

Example response:

```json
[
  {
    "name": "web",
    "publish_addr": "0.0.0.0",
    "publish_port": 8888,
    "local_port": 80,
    "total_connections": 42,
    "active_connections": 3,
    "bytes_sent": 1048576,
    "bytes_received": 524288,
    "start_time": 1700000000
  }
]
```

### Command Line Tools

You can use `curl` to fetch statistics:

```bash
# Get JSON statistics
curl http://server-ip:9090/stats

# Pretty-print with jq
curl -s http://server-ip:9090/stats | jq .

# Get HTML dashboard
curl http://server-ip:9090/
```

### Built-in Top Command

The `tls-tunnel` binary includes a built-in `top` command for real-time monitoring in your terminal:

```bash
# View real-time statistics with interactive terminal UI
tls-tunnel top --url http://localhost:9090

# Custom refresh interval (default: 2 seconds)
tls-tunnel top --url http://localhost:9090 --interval 5

# Short form
tls-tunnel top -u http://localhost:9090 -i 5
```

The `top` command provides:
- **Real-time Dashboard**: Beautiful terminal UI powered by ratatui
- **Auto-refresh**: Configurable refresh interval (default 2 seconds)
- **Interactive Controls**: 
  - `q` or `Esc`: Quit
  - `r`: Manual refresh
- **Formatted Display**: Human-readable bytes and durations
- **Color Coding**: Active connections highlighted in green

For detailed `top` command usage, see [TOP_USAGE.md](TOP_USAGE.md).

## Security Considerations

⚠️ **Important Security Notes:**

1. **No Authentication**: The statistics endpoint does not require authentication
2. **No Encryption**: HTTP traffic is not encrypted
3. **Firewall Protection**: Configure firewall rules to restrict access to trusted IPs only
4. **Local Access**: For production, consider binding to `127.0.0.1` and use SSH tunneling:

```bash
# SSH tunnel to access stats server remotely
ssh -L 9090:localhost:9090 user@server-ip

# Then access via http://localhost:9090
```

5. **Alternative**: Use a reverse proxy (like Nginx) to add authentication and HTTPS

## Metrics Explained

### Per-Proxy Metrics

- **Proxy Name**: Unique identifier for the proxy
- **Published Address**: Server address where the proxy listens
- **Published Port**: Server port where external connections are accepted
- **Client Port**: Port on the client machine where traffic is forwarded
- **Active Connections**: Currently open connections through this proxy
- **Total Connections**: Cumulative number of connections since proxy started
- **Bytes Sent**: Total data sent to client (formatted: B, KB, MB, GB, TB)
- **Bytes Received**: Total data received from client (formatted)
- **Uptime**: Time since proxy was registered (format: days, hours, minutes, seconds)

### Global Metrics

- **Total Proxies**: Number of active proxies currently registered
- **Total Active Connections**: Sum of active connections across all proxies
- **Total Connections**: Sum of all connections across all proxies

## Testing

To test the statistics feature:

1. Start the server with stats enabled:
   ```bash
   ./tls-tunnel server --config test-server-with-stats.toml
   ```

2. In another terminal, start a client:
   ```bash
   ./tls-tunnel client --config test-client.toml
   ```

3. Open your browser:
   ```
   http://localhost:9090
   ```

4. Make some connections to the proxied ports to see statistics update:
   ```bash
   # Example: connect to published port
   curl http://localhost:8888
   ```

5. Watch the statistics update in real-time (auto-refresh every 5 seconds)

## Troubleshooting

### Statistics server not starting

- Check if `stats_port` is configured in server config
- Verify the port is not already in use:
  ```bash
  # Linux/Mac
  lsof -i :9090
  
  # Windows
  netstat -ano | findstr :9090
  ```
- Check server logs for error messages

### Cannot access statistics page

- Verify server is running and listening on stats port
- Check firewall rules
- Ensure you're using the correct IP address
- Try accessing from localhost first: `http://localhost:9090`

### Statistics show zero

- Ensure client is connected
- Make sure there's actual traffic through the proxies
- Check if proxies are correctly configured
- Verify client authentication is successful

## Example Configuration Files

See the example configurations with stats enabled:

- `examples/standalone-server.toml` - Direct mode with stats (uncomment `stats_port`)
- `examples/proxied-server.toml` - Reverse proxy mode with stats (uncomment `stats_port`)
- `test-server-with-stats.toml` - Minimal test configuration with stats enabled

## Integration with Monitoring Systems

The JSON API can be integrated with monitoring systems:

### Prometheus

Create a script to export metrics:

```bash
#!/bin/bash
# prometheus_exporter.sh
curl -s http://localhost:9090/stats | jq -r '.[] | 
  "tls_tunnel_active_connections{proxy=\"\(.name)\"} \(.active_connections)\n" +
  "tls_tunnel_total_connections{proxy=\"\(.name)\"} \(.total_connections)\n" +
  "tls_tunnel_bytes_sent{proxy=\"\(.name)\"} \(.bytes_sent)\n" +
  "tls_tunnel_bytes_received{proxy=\"\(.name)\"} \(.bytes_received)"'
```

### Custom Monitoring

Parse the JSON endpoint in your preferred language:

```python
import requests
import json

response = requests.get('http://server-ip:9090/stats')
stats = response.json()

for proxy in stats:
    print(f"Proxy {proxy['name']}: {proxy['active_connections']} active connections")
```

## Future Enhancements

Potential improvements for the statistics feature:

- [ ] Add authentication to stats endpoint
- [ ] Support HTTPS for stats server
- [ ] Historical data and graphs
- [ ] Prometheus metrics endpoint
- [ ] WebSocket for real-time updates
- [ ] Per-connection details
- [ ] Bandwidth rate limiting visualization
- [ ] Alert thresholds configuration
