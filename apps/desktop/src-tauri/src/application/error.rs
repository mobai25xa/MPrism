use mprism_protocol::{ProtocolError, ProtocolErrorKind};
use serde::Serialize;

use crate::storage::{MessageErrorRecord, StorageError};

#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            http_status: None,
            provider_request_id: None,
            retry_after_ms: None,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new("validation", message, false)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new("conflict", message, false)
    }

    pub fn cancelled() -> Self {
        Self::new("cancelled", "生成已停止", false)
    }

    pub fn to_message_record(&self) -> MessageErrorRecord {
        MessageErrorRecord {
            code: self.code.to_string(),
            message: self.message.clone(),
            retryable: self.retryable,
            http_status: self.http_status,
            provider_request_id: self.provider_request_id.clone(),
            retry_after_ms: self.retry_after_ms,
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<StorageError> for AppError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Validation(message) | StorageError::UnsafePath(message) => {
                Self::validation(message)
            }
            StorageError::NotFound(message) => Self::new("not_found", message, false),
            StorageError::Conflict(message) => Self::conflict(message),
            StorageError::SchemaTooNew { .. } | StorageError::SchemaUnsupported { .. } => {
                Self::new("storage", error.to_string(), false)
            }
            StorageError::Io { .. } | StorageError::Json { .. } => {
                Self::new("storage", "本地数据读写失败", true)
            }
            StorageError::Internal(_) => Self::new("internal", "应用内部错误", false),
        }
    }
}

impl From<ProtocolError> for AppError {
    fn from(error: ProtocolError) -> Self {
        let provider_detail = error.message.trim();
        let (code, default_message) = match error.kind {
            ProtocolErrorKind::InvalidConfiguration | ProtocolErrorKind::InvalidRequest => {
                ("validation", "模型请求配置无效")
            }
            ProtocolErrorKind::Authentication | ProtocolErrorKind::PermissionDenied => {
                ("auth", "模型服务鉴权失败，请检查 API Key")
            }
            ProtocolErrorKind::RateLimited => ("rate_limited", "模型服务请求过于频繁，请稍后重试"),
            ProtocolErrorKind::ContextLengthExceeded => {
                ("context_length", "上下文过长，请缩短内容后重试")
            }
            ProtocolErrorKind::ContentFilter => ("content_filter", "内容被模型安全策略拦截"),
            ProtocolErrorKind::ProviderUnavailable => {
                ("provider_unavailable", "模型服务暂时不可用")
            }
            ProtocolErrorKind::Timeout => ("timeout", "模型服务请求超时"),
            ProtocolErrorKind::Transport => ("transport", "无法连接模型服务"),
            ProtocolErrorKind::Unsupported => ("unsupported", "当前协议不支持该请求能力"),
            ProtocolErrorKind::Decode | ProtocolErrorKind::UnexpectedEof => {
                ("protocol", "模型服务返回了无法解析的响应")
            }
        };
        // Prefer SDK message for validation/unsupported so capability gates stay actionable.
        let message = if matches!(
            error.kind,
            ProtocolErrorKind::InvalidConfiguration
                | ProtocolErrorKind::InvalidRequest
                | ProtocolErrorKind::Unsupported
        ) && !provider_detail.is_empty()
        {
            provider_detail.to_string()
        } else {
            default_message.to_string()
        };
        Self {
            code,
            message,
            retryable: error.retryable,
            http_status: error.http_status,
            provider_request_id: error.request_id,
            retry_after_ms: error.retry_after_ms,
        }
    }
}
