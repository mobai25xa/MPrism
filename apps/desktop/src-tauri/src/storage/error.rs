//! Storage error types.

use std::path::PathBuf;

use thiserror::Error;

pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("数据 schema 版本过高（文件 {found}，应用支持 {supported}），已拒绝覆盖")]
    SchemaTooNew { found: u32, supported: u32 },

    #[error("不支持的数据 schema 版本（文件 {found}，应用支持 {supported}）")]
    SchemaUnsupported { found: u32, supported: u32 },

    #[error("未找到: {0}")]
    NotFound(String),

    #[error("校验失败: {0}")]
    Validation(String),

    #[error("冲突: {0}")]
    Conflict(String),

    #[error("路径不安全: {0}")]
    UnsafePath(String),

    #[error("JSON 解析失败 ({path}): {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("IO 错误 ({path}): {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("内部错误: {0}")]
    Internal(String),
}

impl StorageError {
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.into(),
            source,
        }
    }

    /// Whether this error should block the whole app (settings schema too new).
    pub fn is_blocking(&self) -> bool {
        matches!(self, Self::SchemaTooNew { .. })
    }
}
