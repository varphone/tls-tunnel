# TLS Tunnel 架构设计

本文档详细说明 TLS Tunnel 的架构设计、系统拓扑、工作流程和数据转发机制。

## 系统拓扑图

完整的系统拓扑，展示所有组件的交互关系：

```mermaid
graph TB
    subgraph "公网"
        USER["👥 外部用户"]
    end
    
    subgraph "服务器端"
        SERVER["🖥️ TLS Tunnel Server<br/>Port 8443"]
        PROXY_LISTEN["📡 Proxy Listener<br/>Port 8080, 8081, ..."]
        STATS["📊 Stats Server<br/>Port 9090"]
    end
    
    subgraph "客户端网络A"
        CLIENT_A["💻 Client A<br/>连接: TLS 8443"]
        SERVICE_A["🔧 本地服务<br/>Port 3000"]
    end
    
    subgraph "客户端网络B"
        CLIENT_B["💻 Client B<br/>连接: TLS 8443"]
        SERVICE_B["🗄️ MySQL<br/>Port 3306"]
    end
    
    subgraph "客户端网络C"
        CLIENT_C["💻 Client C<br/>Visitor Mode"]
        APP_C["📱 应用<br/>连接本地3306"]
    end
    
    USER -->|HTTP 8080| PROXY_LISTEN
    PROXY_LISTEN -->|创建Yamux Stream| SERVER
    SERVER -->|多路复用| CLIENT_A
    CLIENT_A -->|连接本地| SERVICE_A
    
    SERVER -->|多路复用| CLIENT_B
    CLIENT_B -->|连接本地| SERVICE_B
    
    SERVER -->|多路复用| CLIENT_C
    CLIENT_C -->|创建Stream查询| SERVER
    SERVER -->|通过ClientB连接| SERVICE_B
    
    APP_C -->|127.0.0.1:3306| CLIENT_C
    
    SERVER -->|统计数据| STATS
    
    style SERVER fill:#4CAF50,color:#fff
    style CLIENT_A fill:#2196F3,color:#fff
    style CLIENT_B fill:#2196F3,color:#fff
    style CLIENT_C fill:#2196F3,color:#fff
    style PROXY_LISTEN fill:#FF9800,color:#fff
```

## 架构分层

```mermaid
graph TB
    subgraph "应用层"
        A["CLI 命令行"]
        B["Server 服务器"]
        C["Client 客户端"]
    end
    
    subgraph "传输层"
        D["Transport 抽象<br/>TLS / HTTP/2 / WSS"]
    end
    
    subgraph "协议层"
        E["Yamux 多路复用"]
        F["TLS 加密"]
    end
    
    subgraph "网络层"
        G["TCP Socket"]
    end
    
    A --> B
    A --> C
    B --> D
    C --> D
    D --> E
    D --> F
    E --> F
    F --> G
    
    style A fill:#e1f5ff
    style B fill:#c8e6c9
    style C fill:#f0f4c3
    style D fill:#ffe0b2
    style E fill:#f8bbd0
    style F fill:#e1bee7
    style G fill:#cfcfcf
```

## 模块依赖关系

### Server 模块结构

```mermaid
graph TB
    subgraph "Server 模块（7个子模块）"
        S1["registry<br/>类型定义和注册表"]
        S2["config<br/>配置验证"]
        S3["connection<br/>连接处理"]
        S4["yamux<br/>多路复用"]
        S5["visitor<br/>反向访问"]
        S6["stats<br/>统计服务"]
        S7["mod<br/>主函数导出"]
    end
    
    S7 --> S1
    S7 --> S2
    S7 --> S3
    S7 --> S4
    S7 --> S5
    S7 --> S6
    S2 --> S1
    S3 --> S1
    S4 --> S1
    S5 --> S1
    
    style S1 fill:#90EE90
    style S7 fill:#FFB6C1
```

### Client 模块结构

```mermaid
graph TB
    subgraph "Client 模块（5个子模块）"
        C1["config<br/>常量和环境变量"]
        C2["connection<br/>本地连接"]
        C3["visitor<br/>反向代理"]
        C4["stream<br/>流处理"]
        C5["mod<br/>主函数导出"]
    end
    
    C5 --> C1
    C5 --> C2
    C5 --> C3
    C5 --> C4
    C2 --> C1
    C3 --> C1
    C4 --> C1
    
    style C1 fill:#90EE90
    style C5 fill:#FFB6C1
```

## Proxy 模式详细流程

### 时序图

外部用户通过服务器访问客户端的本地服务：

```mermaid
sequenceDiagram
    participant User as 外部用户
    participant Server as 服务器A<br/>Port 8443
    participant Client as 客户端B
    participant Service as 本地服务<br/>Port 3000

    User->>Server: 1. HTTP请求<br/>http://server:8080

    Server->>Client: 2. 创建yamux流<br/>(基于已建立的TLS连接)

    Client->>Service: 3. 连接本地服务<br/>127.0.0.1:3000

    Service-->>Client: 4. 服务响应

    Client-->>Server: 5. 转发响应<br/>(yamux流)

    Server-->>User: 6. 返回HTTP响应

    Note over Server,Client: 单个TLS连接支持多个并发stream<br/>(Yamux多路复用)
```

### 连接建立阶段

```mermaid
graph TD
    A["1. 客户端连接到服务器<br/>Port 8443 TLS"] --> B["2. TLS握手完成"]
    B --> C["3. 发送认证密钥<br/>auth_key"]
    C --> D{"认证检查"}
    D -->|通过| E["4. 发送代理和访问者配置<br/>JSON格式"]
    D -->|失败| Z["❌ 连接中断"]
    E --> F{"配置验证"}
    F -->|通过| G["5. 建立Yamux连接<br/>多路复用over TLS"]
    F -->|失败| Z
    G --> H["✓ 连接就绪<br/>可以处理请求"]
    style H fill:#90EE90
```

### 数据转发阶段

```mermaid
graph TD
    A["外部用户请求<br/>curl http://server:8080"] --> B["服务器接受连接<br/>Port 8080 Listener"]
    B --> C["通过已建立的Yamux连接<br/>创建新Stream"]
    C --> D["发送目标端口号<br/>Port: 3000"]
    D --> E["客户端接收Stream<br/>读取端口号"]
    E --> F["连接到本地服务<br/>127.0.0.1:3000"]
    F --> G{"连接成功?"}
    G -->|是| H["建立数据通道<br/>开始双向转发"]
    G -->|否| I["重试连接<br/>最多3次"]
    I --> J{"重试成功?"}
    J -->|是| H
    J -->|否| K["❌ 返回错误"]
    H --> L["双向转发数据<br/>User ↔ Server ↔ Client ↔ Service"]
    L --> M["连接关闭"]
    style H fill:#90EE90
    style M fill:#FFB6C6
```

## Visitor 模式详细流程

### 时序图

客户端本地应用通过隧道访问另一个客户端的服务：

```mermaid
sequenceDiagram
    participant AppC as 客户端C本地应用<br/>MySQL Client
    participant VisitorC as 客户端C<br/>Visitor Listener<br/>Port 3306
    participant Server as 服务器<br/>Proxy Registry
    participant ProxyB as 客户端B<br/>Proxy Handler
    participant ServiceB as 客户端B本地服务<br/>MySQL Server

    AppC->>VisitorC: 1. 连接<br/>127.0.0.1:3306

    VisitorC->>Server: 2. 创建yamux流<br/>请求proxy "mysql"

    Server->>ProxyB: 3. 查找并转发<br/>创建yamux流

    ProxyB->>ServiceB: 4. 连接本地服务<br/>127.0.0.1:3306

    ServiceB-->>ProxyB: 5. 连接成功

    ProxyB-->>Server: 6. 转发确认<br/>(yamux流)

    Server-->>VisitorC: 7. 转发确认<br/>(yamux流)

    VisitorC-->>AppC: 8. 连接建立

    Note over AppC,ServiceB: 9. 双向数据转发<br/>AppC ↔ Server ↔ ProxyB ↔ ServiceB<br/>(经过中间节点)

    Note over Server,ProxyB: 服务器通过ProxyRegistry查找目标proxy<br/>支持客户端间的跨域访问
```

### 连接建立阶段

```mermaid
graph TD
    A["客户端B连接到服务器"] --> B["发送Proxy配置<br/>name: mysql<br/>local_port: 3306"]
    B --> C["服务器将Proxy<br/>注册到全局Registry<br/>key: mysql,3306"]
    C --> D["客户端C连接到服务器"]
    D --> E["发送Visitor配置<br/>bind_port: 3306<br/>target: mysql"]
    E --> F["客户端C本地监听<br/>127.0.0.1:3306"]
    F --> G["✓ 系统就绪<br/>B已注册, C在监听"]
    style G fill:#90EE90
```

### 数据转发阶段

```mermaid
graph TD
    A["本地应用连接<br/>mysql -h 127.0.0.1 -P 3306"] --> B["客户端C Visitor Listener<br/>接受连接"]
    B --> C["创建Yamux Stream<br/>到服务器"]
    C --> D["发送目标信息<br/>proxy_name: mysql<br/>publish_port: 3306"]
    D --> E["服务器查询Registry<br/>查找mysql对应的ProxyRegistration"]
    E --> F{"找到目标?"}
    F -->|是| G["通过客户端B的连接<br/>创建新Yamux Stream"]
    F -->|否| H["❌ 返回错误<br/>proxy not found"]
    G --> I["客户端B接收Stream"]
    I --> J["连接到本地MySQL<br/>127.0.0.1:3306"]
    J --> K{"连接成功?"}
    K -->|是| L["发送确认给服务器<br/>Server → Client C"]
    K -->|否| M["❌ 返回错误<br/>connection failed"]
    L --> N["建立完整数据通道<br/>AppC ↔ VisitorC ↔ Server ↔ ProxyB ↔ ServiceB"]
    N --> O["执行SQL操作"]
    O --> P["连接关闭"]
    style L fill:#90EE90
    style N fill:#87CEEB
```

## Proxy Registry 机制

```mermaid
graph LR
    A["多个客户端连接"] --> B["各自发送Proxy配置"]
    B --> C["服务器构建Registry"]
    C --> D["key: proxy_name,publish_port<br/>value: ProxyRegistration"]
    D --> E["ProxyRegistration包含:<br/>stream_tx: 创建stream的通道<br/>proxy_info: 代理信息"]
    E --> F["Visitor查询时<br/>快速查找目标Proxy"]
    F --> G["通过stream_tx<br/>请求创建新stream"]
    style F fill:#FFE4B5
    style G fill:#FFE4B5
```

### Registry 设计特点

1. **Key 结构** - `(proxy_name, publish_port)`
   - 支持同一 proxy 在不同端口的多次发布
   - 快速查找目标 proxy

2. **Value 结构** - `ProxyRegistration`
   - `stream_tx`: 用于向客户端请求创建新的 yamux stream
   - `proxy_info`: 代理的详细信息（名称、地址、端口等）

3. **查询性能**
   - O(1) 时间复杂度的查找
   - 支持并发查询和更新

4. **生命周期**
   - 客户端连接时注册所有代理
   - 客户端断开时自动清理相关代理

## 关键设计特点

### 1. 多路复用 (Yamux)

- **单一连接**：每个客户端只需一条 TLS 连接到服务器
- **并发流**：单个连接支持数百个并发的独立数据流
- **性能优势**：避免了重复的 TLS 握手开销

### 2. 动态配置

- **服务器无配置**：服务器不需要预先配置代理列表
- **客户端提供**：客户端连接时动态告知服务器自己的代理信息
- **灵活性**：支持动态添加/删除代理，无需重启服务器

### 3. 双向通信

- **Proxy 模式**：从公网访问内网（传统反向代理）
- **Visitor 模式**：内网客户端互相访问（反向访问）
- **同时支持**：单个客户端可同时作为 Proxy 和 Visitor

### 4. 连接复用

**Proxy 模式**：
- 根据代理类型决定是否复用连接
- TCP 类型：不复用，每个连接独立
- HTTP/1.1 类型：支持连接复用，减少连接数
- HTTP/2.0 类型：强制单连接多路复用

**Visitor 模式**：
- 每个 visitor 请求使用独立的 stream
- 多个 visitor 共享同一条 TLS 连接
- 支持并发的 visitor 连接

### 5. 错误处理与重试

- **本地连接失败**：自动重试（默认 3 次）
- **认证失败**：立即断开，拒绝连接
- **配置校验失败**：返回详细错误信息
- **通信异常**：自动重连机制

## 性能特性

| 特性 | 说明 |
|------|------|
| **连接数** | 每个客户端仅需 1 条 TLS 连接 |
| **并发流** | 单个连接支持数百个并发 stream |
| **握手成本** | 摊销到所有流，大幅降低开销 |
| **内存占用** | O(client_count + active_streams) |
| **吞吐量** | 受限于网络带宽，不受连接数限制 |
| **延迟** | 相对于直连，增加一层中转延迟 |

## 扩展性考虑

1. **水平扩展**：部署多个服务器，使用负载均衡
2. **客户端扩展**：单服务器可支持数千个客户端连接
3. **流扩展**：每个连接可支持数百个并发流
4. **协议扩展**：支持多种传输协议（TLS/HTTP2/WSS）

## 参考资源

- [Yamux 多路复用协议](https://github.com/hashicorp/yamux)
- [Rustls TLS 实现](https://github.com/rustls/rustls)
- [Tokio 异步运行时](https://tokio.rs)
