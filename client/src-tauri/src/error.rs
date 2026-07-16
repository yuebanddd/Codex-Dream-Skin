use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("订阅源地址无效：{0}")]
    InvalidSource(String),
    #[error("订阅源协议无效：{0}")]
    InvalidManifest(String),
    #[error("GitHub 请求失败：{0}")]
    Network(#[from] reqwest::Error),
    #[error("无法读取本地订阅数据：{0}")]
    Io(#[from] std::io::Error),
    #[error("订阅数据解析失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("找不到订阅源：{0}")]
    SourceNotFound(String),
}

pub type AppResult<T> = Result<T, AppError>;
