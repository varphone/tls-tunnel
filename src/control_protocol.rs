/// 控制通道协议 - 基于 JSON-RPC 2.0
///
/// 该模块实现了客户端与服务端之间的控制通道通信协议，
/// 使用长度前缀（4字节大端）+ JSON-RPC 2.0 格式
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC 版本（固定为 "2.0"）
    pub jsonrpc: String,

    /// 方法名
    pub method: String,

    /// 参数
    pub params: Value,

    /// 请求 ID（用于匹配响应，通知时为 None）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
}

impl JsonRpcRequest {
    /// 创建新的请求
    pub fn new(method: String, params: Value, id: u64) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method,
            params,
            id: Some(Value::Number(id.into())),
        }
    }

    /// 是否为通知
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }
}

/// JSON-RPC 2.0 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC 版本（固定为 "2.0"）
    pub jsonrpc: String,

    /// 结果（成功时）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    /// 错误（失败时）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,

    /// 请求 ID（对应请求的 ID）
    pub id: Value,
}

impl JsonRpcResponse {
    /// 创建成功响应
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// 创建错误响应
    pub fn error(id: Value, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(error),
            id,
        }
    }
}

/// JSON-RPC 2.0 错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// 错误码
    pub code: i32,

    /// 错误消息
    pub message: String,

    /// 附加数据（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// 认证请求参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticateParams {
    pub auth_key: String,
}

/// 认证响应结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticateResult {
    pub client_id: String,
}

/// 提交配置请求参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitConfigParams {
    pub proxies: Vec<crate::config::ProxyConfig>,
}

/// 提交配置响应结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitConfigResult {
    pub rejected_proxies: Vec<String>,
}

/// 控制通道方法
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlMethod {
    // 客户端 -> 服务端
    /// 认证
    Authenticate,

    /// 提交配置
    SubmitConfig,

    /// 心跳
    Heartbeat,

    // 服务端 -> 客户端
    /// 推送配置状态
    PushConfigStatus,

    /// 推送统计信息
    PushStats,
}

impl ControlMethod {
    /// 从字符串解析
    pub fn from_str(s: &str) -> anyhow::Result<Self> {
        match s {
            "authenticate" => Ok(ControlMethod::Authenticate),
            "submit_config" => Ok(ControlMethod::SubmitConfig),
            "heartbeat" => Ok(ControlMethod::Heartbeat),
            "push_config_status" => Ok(ControlMethod::PushConfigStatus),
            "push_stats" => Ok(ControlMethod::PushStats),
            _ => Err(anyhow::anyhow!("Unknown control method: {}", s)),
        }
    }

    /// 转换为字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            ControlMethod::Authenticate => "authenticate",
            ControlMethod::SubmitConfig => "submit_config",
            ControlMethod::Heartbeat => "heartbeat",
            ControlMethod::PushConfigStatus => "push_config_status",
            ControlMethod::PushStats => "push_stats",
        }
    }
}
