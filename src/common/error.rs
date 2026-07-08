#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Parse error: {0} - {1}")]
    Parse(String, String),
    #[error("Invalid data: {0}")]
    InvalidData(String), // 添加这一行
    #[error("Other error: {0}")]
    Other(String),
}

pub type ClientResult<T> = Result<T, ClientError>;
