/// 客户端控制通道 - 简化版本，用于统一事件循环
///
/// 提供控制流的读写操作，但不独立运行，而是集成到主事件循环中
use crate::config::ClientFullConfig;
use crate::control_protocol::*;
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::Duration;
use tracing::{debug, warn};
use yamux::Stream as YamuxStream;

/// 控制通道事件
#[derive(Debug, Clone)]
pub enum ControlEvent {
    /// 认证成功
    AuthenticationSuccess { client_id: String },

    /// 认证失败
    AuthenticationFailed { reason: String },

    /// 配置已接受
    ConfigAccepted,

    /// 配置被部分拒绝
    ConfigPartiallyRejected { rejected_proxies: Vec<String> },

    /// 配置完全被拒绝
    ConfigRejected { rejected_proxies: Vec<String> },

    /// 连接关闭
    ConnectionClosed,
}

/// 客户端控制通道
pub struct ClientControlChannel {
    /// 配置
    config: ClientFullConfig,

    /// 事件发送器
    event_tx: mpsc::UnboundedSender<ControlEvent>,

    /// 请求 ID 计数器
    request_id: Arc<AtomicU64>,

    /// 待处理的请求（用于匹配响应）
    pending_requests: Arc<RwLock<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
}

impl ClientControlChannel {
    /// 创建新的控制通道
    pub fn new(config: ClientFullConfig) -> (Self, mpsc::UnboundedReceiver<ControlEvent>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let channel = Self {
            config,
            event_tx,
            request_id: Arc::new(AtomicU64::new(1)),
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
        };

        (channel, event_rx)
    }

    /// 从控制流读取一条消息
    /// 自动处理响应消息，返回请求/通知消息
    pub async fn read_message(
        &mut self,
        stream: &mut YamuxStream,
    ) -> Result<Option<JsonRpcRequest>> {
        use futures::AsyncReadExt;

        loop {
            // 读取长度前缀
            let mut len_buf = [0u8; 4];
            match stream.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(e) => return Err(e.into()),
            }

            let msg_len = u32::from_be_bytes(len_buf) as usize;

            if msg_len > 10 * 1024 * 1024 {
                return Err(anyhow::anyhow!("Message too large: {} bytes", msg_len));
            }

            // 读取消息内容
            let mut msg_buf = vec![0u8; msg_len];
            stream.read_exact(&mut msg_buf).await?;

            // 先尝试解析为响应
            if let Ok(response) = serde_json::from_slice::<JsonRpcResponse>(&msg_buf) {
                self.handle_response(response).await?;
                continue; // 继续读取下一条消息
            }

            // 否则解析为请求（通知）
            let request: JsonRpcRequest =
                serde_json::from_slice(&msg_buf).context("Failed to parse JSON-RPC message")?;

            return Ok(Some(request));
        }
    }

    /// 处理响应消息
    async fn handle_response(&mut self, response: JsonRpcResponse) -> Result<()> {
        // 从 Value 中提取 u64
        let request_id = if let Value::Number(n) = &response.id {
            n.as_u64().unwrap_or(0)
        } else {
            warn!("Invalid response ID type: {:?}", response.id);
            return Ok(());
        };

        // 查找并移除待处理的请求
        let sender = {
            let mut pending = self.pending_requests.write().await;
            pending.remove(&request_id)
        };

        if let Some(sender) = sender {
            let _ = sender.send(response);
        } else {
            warn!("Received response for unknown request ID: {}", request_id);
        }

        Ok(())
    }

    /// 处理通知消息（服务端推送）
    pub async fn handle_notification(&mut self, request: JsonRpcRequest) -> Result<()> {
        debug!("Received notification: {}", request.method);

        match request.method.as_str() {
            "config_status_push" => {
                debug!("Config status push params: {:?}", request.params);
            }
            "stats_push" => {
                debug!("Stats push params: {:?}", request.params);
            }
            _ => {
                warn!("Unknown notification method: {}", request.method);
            }
        }

        Ok(())
    }

    /// 发送认证请求
    pub async fn send_authenticate(&mut self, stream: &mut YamuxStream) -> Result<()> {
        use futures::AsyncWriteExt;

        let request_id = self.request_id.fetch_add(1, Ordering::SeqCst);

        let params = AuthenticateParams {
            auth_key: self.config.client.auth_key.clone(),
        };

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "authenticate".to_string(),
            params: serde_json::to_value(params)?,
            id: Some(Value::Number(request_id.into())),
        };

        let data = serde_json::to_vec(&request)?;
        let len = data.len() as u32;

        stream.write_all(&len.to_be_bytes()).await?;
        stream.write_all(&data).await?;
        stream.flush().await?;

        debug!("Sent authentication request");

        // 注册待处理的请求
        let (response_tx, response_rx) = oneshot::channel();
        self.pending_requests
            .write()
            .await
            .insert(request_id, response_tx);

        // 等待响应
        tokio::spawn({
            let event_tx = self.event_tx.clone();
            let pending = self.pending_requests.clone();
            async move {
                match tokio::time::timeout(Duration::from_secs(10), response_rx).await {
                    Ok(Ok(response)) => {
                        if let Some(result) = response.result {
                            if let Ok(auth_result) =
                                serde_json::from_value::<AuthenticateResult>(result)
                            {
                                let _ = event_tx.send(ControlEvent::AuthenticationSuccess {
                                    client_id: auth_result.client_id,
                                });
                            }
                        } else if let Some(error) = response.error {
                            let _ = event_tx.send(ControlEvent::AuthenticationFailed {
                                reason: error.message,
                            });
                        }
                    }
                    Ok(Err(_)) => {
                        let _ = event_tx.send(ControlEvent::AuthenticationFailed {
                            reason: "Response channel closed".to_string(),
                        });
                    }
                    Err(_) => {
                        let _ = event_tx.send(ControlEvent::AuthenticationFailed {
                            reason: "Timeout waiting for authentication response".to_string(),
                        });
                        pending.write().await.remove(&request_id);
                    }
                }
            }
        });

        Ok(())
    }

    /// 发送配置提交请求
    pub async fn send_submit_config(&mut self, stream: &mut YamuxStream) -> Result<()> {
        use futures::AsyncWriteExt;

        let request_id = self.request_id.fetch_add(1, Ordering::SeqCst);

        let params = SubmitConfigParams {
            proxies: self.config.proxies.clone(),
        };

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "submit_config".to_string(),
            params: serde_json::to_value(params)?,
            id: Some(Value::Number(request_id.into())),
        };

        let data = serde_json::to_vec(&request)?;
        let len = data.len() as u32;

        stream.write_all(&len.to_be_bytes()).await?;
        stream.write_all(&data).await?;
        stream.flush().await?;

        debug!("Sent config submission request");

        // 注册待处理的请求
        let (response_tx, response_rx) = oneshot::channel();
        self.pending_requests
            .write()
            .await
            .insert(request_id, response_tx);

        // 等待响应
        tokio::spawn({
            let event_tx = self.event_tx.clone();
            let pending = self.pending_requests.clone();
            async move {
                match tokio::time::timeout(Duration::from_secs(10), response_rx).await {
                    Ok(Ok(response)) => {
                        if let Some(result) = response.result {
                            if let Ok(config_result) =
                                serde_json::from_value::<SubmitConfigResult>(result)
                            {
                                if config_result.rejected_proxies.is_empty() {
                                    let _ = event_tx.send(ControlEvent::ConfigAccepted);
                                } else {
                                    let _ = event_tx.send(ControlEvent::ConfigPartiallyRejected {
                                        rejected_proxies: config_result.rejected_proxies,
                                    });
                                }
                            }
                        } else if let Some(error) = response.error {
                            // 从错误数据中提取 rejected_proxies
                            let rejected_proxies = if let Some(data) = error.data {
                                if let Ok(rejected) = serde_json::from_value::<Vec<String>>(
                                    data.get("rejected_proxies")
                                        .cloned()
                                        .unwrap_or(Value::Array(vec![])),
                                ) {
                                    rejected
                                } else {
                                    vec![error.message.clone()]
                                }
                            } else {
                                vec![error.message.clone()]
                            };

                            let _ =
                                event_tx.send(ControlEvent::ConfigRejected { rejected_proxies });
                        }
                    }
                    Ok(Err(_)) => {
                        let _ = event_tx.send(ControlEvent::ConfigRejected {
                            rejected_proxies: vec!["Response channel closed".to_string()],
                        });
                    }
                    Err(_) => {
                        let _ = event_tx.send(ControlEvent::ConfigRejected {
                            rejected_proxies: vec![
                                "Timeout waiting for config response".to_string()
                            ],
                        });
                        pending.write().await.remove(&request_id);
                    }
                }
            }
        });

        Ok(())
    }

    /// 发送心跳通知
    pub async fn send_heartbeat(&mut self, stream: &mut YamuxStream) -> Result<()> {
        use futures::AsyncWriteExt;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "heartbeat".to_string(),
            params: Value::Null,
            id: None, // 通知，无需响应
        };

        let data = serde_json::to_vec(&request)?;
        let len = data.len() as u32;

        stream.write_all(&len.to_be_bytes()).await?;
        stream.write_all(&data).await?;
        stream.flush().await?;

        Ok(())
    }
}
