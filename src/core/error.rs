//! 错误类型定义
//!
//! 提供清晰的错误类型和诊断信息，替代 Option 的模糊失败语义

use std::fmt;

/// 解析错误类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// 数据长度不足
    InsufficientData {
        expected: usize,
        actual: usize,
        context: &'static str,
    },

    /// 无效的 discriminator
    InvalidDiscriminator {
        expected: Option<[u8; 8]>,
        found: [u8; 8],
        context: &'static str,
    },

    /// Base64 解码失败
    Base64DecodeError {
        context: &'static str,
    },

    /// 账户数量不足
    InsufficientAccounts {
        expected: usize,
        actual: usize,
        context: &'static str,
    },

    /// 日志格式无效
    InvalidLogFormat {
        reason: &'static str,
    },

    /// 指令格式无效
    InvalidInstructionFormat {
        reason: &'static str,
    },

    /// 未识别的事件类型
    UnknownEventType {
        discriminator: [u8; 8],
        program: &'static str,
    },

    /// 不支持的程序
    UnsupportedProgram {
        program_id: String,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InsufficientData { expected, actual, context } => {
                write!(
                    f,
                    "数据长度不足: 需要 {} 字节，实际 {} 字节 (上下文: {})",
                    expected, actual, context
                )
            }
            ParseError::InvalidDiscriminator { expected, found, context } => {
                if let Some(exp) = expected {
                    write!(
                        f,
                        "无效的 discriminator: 期望 {:?}，发现 {:?} (上下文: {})",
                        exp, found, context
                    )
                } else {
                    write!(
                        f,
                        "无效的 discriminator: {:?} (上下文: {})",
                        found, context
                    )
                }
            }
            ParseError::Base64DecodeError { context } => {
                write!(f, "Base64 解码失败 (上下文: {})", context)
            }
            ParseError::InsufficientAccounts { expected, actual, context } => {
                write!(
                    f,
                    "账户数量不足: 需要 {} 个，实际 {} 个 (上下文: {})",
                    expected, actual, context
                )
            }
            ParseError::InvalidLogFormat { reason } => {
                write!(f, "日志格式无效: {}", reason)
            }
            ParseError::InvalidInstructionFormat { reason } => {
                write!(f, "指令格式无效: {}", reason)
            }
            ParseError::UnknownEventType { discriminator, program } => {
                write!(
                    f,
                    "未识别的事件类型: discriminator {:?} 来自程序 {}",
                    discriminator, program
                )
            }
            ParseError::UnsupportedProgram { program_id } => {
                write!(f, "不支持的程序: {}", program_id)
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// 解析结果类型别名
pub type ParseResult<T> = Result<T, ParseError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ParseError::InsufficientData {
            expected: 32,
            actual: 16,
            context: "read_pubkey",
        };
        let display = format!("{}", err);
        assert!(display.contains("32"));
        assert!(display.contains("16"));
    }

    #[test]
    fn test_error_clone() {
        let err1 = ParseError::Base64DecodeError {
            context: "PumpFun Trade",
        };
        let err2 = err1.clone();
        assert_eq!(err1, err2);
    }
}
