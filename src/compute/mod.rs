//! 计算项系统模块
//!
//! 提供静态计算项（无状态，如单位转换）和动态计算项（有状态，如当前圈 vs 最佳圈时间差）的注册和执行功能。
//!
//! # 架构
//!
//! - [`ComputeError`] — 计算错误类型
//! - [`ComputeContext`] — 计算上下文（帧数据 + 历史参考 + 已计算值）
//! - [`RealtimeComputeItem`] — 实时逐帧计算 trait
//! - [`BatchComputeItem`] — 批量整圈计算 trait
//! - [`ComputeRegistry`] — 计算项注册中心

use std::fmt;

pub mod context;
pub mod items;
pub mod registry;

pub use context::{ComputeContext, RealtimeComputeRequest};
pub use items::{BatchComputeItem, RealtimeComputeItem};
pub use registry::ComputeRegistry;

/// 计算项执行结果
pub type ComputeResult<T> = Result<T, ComputeError>;

/// 计算项错误类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputeError {
    /// 无有效数据（新会话、新赛道或数据损坏）
    NoValidData,
    /// 无效参考数据（例如参考圈数据格式错误）
    InvalidReferenceData,
    /// 计算过程失败
    ComputationFailed(String),
    /// 计算项未在注册表中找到
    ItemNotFound(String),
    /// 注册失败（名称无效、重复等）
    InvalidRegistration(String),
}

impl fmt::Display for ComputeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoValidData => write!(f, "无有效数据"),
            Self::InvalidReferenceData => write!(f, "无效参考数据"),
            Self::ComputationFailed(msg) => write!(f, "计算失败: {msg}"),
            Self::ItemNotFound(name) => write!(f, "计算项未找到: {name}"),
            Self::InvalidRegistration(msg) => write!(f, "注册失败: {msg}"),
        }
    }
}

impl std::error::Error for ComputeError {}

impl From<std::io::Error> for ComputeError {
    fn from(e: std::io::Error) -> Self {
        Self::ComputationFailed(e.to_string())
    }
}

impl From<crate::TelemetryError> for ComputeError {
    fn from(e: crate::TelemetryError) -> Self {
        Self::ComputationFailed(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_error_display() {
        assert_eq!(ComputeError::NoValidData.to_string(), "无有效数据");
        assert_eq!(
            ComputeError::InvalidReferenceData.to_string(),
            "无效参考数据"
        );
        assert_eq!(
            ComputeError::ComputationFailed("oops".into()).to_string(),
            "计算失败: oops"
        );
        assert_eq!(
            ComputeError::ItemNotFound("speed_mps".into()).to_string(),
            "计算项未找到: speed_mps"
        );
    }

    #[test]
    fn test_compute_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let compute_err = ComputeError::from(io_err);
        assert!(compute_err.to_string().contains("file not found"));
    }
}
