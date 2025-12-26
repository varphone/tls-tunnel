use crate::config::ProxyConfig;
use crate::control_protocol::*;
use anyhow::{Context, Result};
use futures::io::{AsyncReadExt as FuturesAsyncReadExt, AsyncWriteExt as FuturesAsyncWriteExt};
use serde_json::json;
use tracing::{debug, error, info, warn};

/// 服务端控制通道事件
#[derive(Debug, Clone)]
pub enum ControlEvent {
    /// 收到认证请求
    AuthenticateRequest {
        id: serde_json::Value,
        auth_key: String,
    },

    /// 收到配置提交请求
    SubmitConfigRequest {
        id: serde_json::Value,
        proxies: Vec<ProxyConfig>,
    },

    /// 收到心跳
    Heartbeat,

    /// 连接关闭
    ConnectionClosed,
}

/// 服务端控制通道
pub struct ServerControlChannel {
    event_tx: tokio::sync::mpsc::UnboundedSender<ControlEvent>,
}

impl ServerControlChannel {
    /// 创建新的服务端控制通道
    pub fn new() -> (Self, tokio::sync::mpsc::UnboundedReceiver<ControlEvent>) {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let channel = Self { event_tx };
        (channel, event_rx)
    }

    /// 读取并处理消息（返回是否需要响应的请求）
    pub async fn read_message(
        &self,
        stream: &mut ::yamux::Stream,
    ) -> Result<Option<JsonRpcRequest>> {
        // 读取长度前缀（4字节大端序）
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                debug!("Control stream closed");
                let _ = self.event_tx.send(ControlEvent::ConnectionClosed);
                return Ok(None);
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to read message length: {}", e));
            }
        }

        let msg_len = u32::from_be_bytes(len_buf) as usize;

        // 防止过大的消息
        if msg_len > 10 * 1024 * 1024 {
            return Err(anyhow::anyhow!("Message too large: {} bytes", msg_len));
        }

        // 读取消息体
        let mut msg_buf = vec![0u8; msg_len];
        stream
            .read_exact(&mut msg_buf)
            .await
            .context("Failed to read message body")?;

        // 解析 JSON-RPC 消息
        let request: JsonRpcRequest =
            serde_json::from_slice(&msg_buf).context("Failed to parse JSON-RPC request")?;

        debug!("Received JSON-RPC request: method={}", request.method);

        // 处理请求并发送事件
        self.handle_request(&request).await?;

        // 返回需要响应的请求
        Ok(Some(request))
    }

    /// 处理 JSON-RPC 请求
    async fn handle_request(&self, request: &JsonRpcRequest) -> Result<()> {
        let method = ControlMethod::from_str(&request.method)?;

        match method {
            ControlMethod::Authenticate => {
                let params: AuthenticateParams = serde_json::from_value(request.params.clone())
                    .context("Invalid authenticate params")?;

                let id = request.id.clone().unwrap_or(serde_json::Value::Null);
                let _ = self.event_tx.send(ControlEvent::AuthenticateRequest {
                    id,
                    auth_key: params.auth_key,
                });
            }

            ControlMethod::SubmitConfig => {
                let params: SubmitConfigParams = serde_json::from_value(request.params.clone())
                    .context("Invalid submit_config params")?;

                let id = request.id.clone().unwrap_or(serde_json::Value::Null);
                let _ = self.event_tx.send(ControlEvent::SubmitConfigRequest {
                    id,
                    proxies: params.proxies,
                });
            }

            ControlMethod::Heartbeat => {
                let _ = self.event_tx.send(ControlEvent::Heartbeat);
            }

            _ => {
                warn!("Received unknown method: {}", request.method);
            }
        }

        Ok(())
    }

    /// 发送认证成功响应
    pub async fn send_auth_success(
        &self,
        stream: &mut ::yamux::Stream,
        id: serde_json::Value,
        client_id: String,
    ) -> Result<()> {
        let result = AuthenticateResult { client_id };

        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(serde_json::to_value(result)?),
            error: None,
        };

        self.send_response(stream, &response).await
    }

    /// 发送认证失败响应
    pub async fn send_auth_failure(
        &self,
        stream: &mut ::yamux::Stream,
        id: serde_json::Value,
        reason: String,
    ) -> Result<()> {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32000,
                message: reason,
                data: None,
            }),
        };

        self.send_response(stream, &response).await
    }

    /// 发送配置接受响应
    pub async fn send_config_accepted(
        &self,
        stream: &mut ::yamux::Stream,
        id: serde_json::Value,
    ) -> Result<()> {
        let result = SubmitConfigResult {
            rejected_proxies: vec![],
        };

        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(serde_json::to_value(result)?),
            error: None,
        };

        self.send_response(stream, &response).await
    }

    /// 发送配置部分拒绝响应
    pub async fn send_config_partially_rejected(
        &self,
        stream: &mut ::yamux::Stream,
        id: serde_json::Value,
        rejected_proxies: Vec<String>,
    ) -> Result<()> {
        let result = SubmitConfigResult { rejected_proxies };

        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(serde_json::to_value(result)?),
            error: None,
        };

        self.send_response(stream, &response).await
    }

    /// 发送配置拒绝响应
    pub async fn send_config_rejected(
        &self,
        stream: &mut ::yamux::Stream,
        id: serde_json::Value,
        rejected_proxies: Vec<String>,
    ) -> Result<()> {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32001,
                message: format!("All proxies rejected: {}", rejected_proxies.join(", ")),
                data: Some(json!({ "rejected_proxies": rejected_proxies })),
            }),
        };

        self.send_response(stream, &response).await
    }

    /// 发送响应
    async fn send_response(
        &self,
        stream: &mut ::yamux::Stream,
        response: &JsonRpcResponse,
    ) -> Result<()> {
        let response_json = serde_json::to_vec(response)?;
        let len_bytes = (response_json.len() as u32).to_be_bytes();

        stream.write_all(&len_bytes).await?;
        stream.write_all(&response_json).await?;
        stream.flush().await?;

        debug!("Sent JSON-RPC response: id={:?}", response.id);
        Ok(())
    }
}
