pub type ShredResult<T> = Result<T, ShredDecodeError>;

#[derive(Debug, thiserror::Error)]
pub enum ShredDecodeError {
    #[error("failed to parse shred: {0}")]
    Parse(String),
    #[error("failed to deshred payload: {0}")]
    Deshred(String),
    #[error("deshred payload too large: {len} bytes, max {max} bytes")]
    PayloadTooLarge { len: usize, max: usize },
    #[error("failed to decode entries: {0}")]
    DecodeEntries(String),
    #[error("UDP socket error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<solana_ledger::shred::Error> for ShredDecodeError {
    fn from(error: solana_ledger::shred::Error) -> Self {
        Self::Parse(error.to_string())
    }
}

impl From<bincode::Error> for ShredDecodeError {
    fn from(error: bincode::Error) -> Self {
        Self::DecodeEntries(error.to_string())
    }
}
