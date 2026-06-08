//! 统一 Item 标识体系
//!
//! 定义三种 item 类型（raw / calculated / system）及其标识键，
//! 用户面对统一命名 `{类型}:{路径}`，录制和 dashboard 使用同一标识。

use std::fmt;

/// Item 类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemType {
    /// 直接读取 TelemetryFrame 字段，如 `raw:controls.speed_kmh`
    Raw,
    /// 基于遥测数据计算得出，如 `calc:delta_best`
    Calculated,
    /// 系统级信息，如 `system:cpu_temp`（未来）
    System,
}

impl ItemType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemType::Raw => "raw",
            ItemType::Calculated => "calc",
            ItemType::System => "system",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "raw" => Some(ItemType::Raw),
            "calc" => Some(ItemType::Calculated),
            "system" => Some(ItemType::System),
            _ => None,
        }
    }
}

/// 统一 Item 标识键
///
/// 格式：`{类型}:{路径}`
///
/// # 示例
///
/// ```
/// use module_live_telemetry::item_key::ItemKey;
///
/// let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
/// assert_eq!(key.to_string(), "raw:controls.speed_kmh");
///
/// let key = ItemKey::parse("calc:delta_best").unwrap();
/// assert_eq!(key.to_string(), "calc:delta_best");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ItemKey {
    pub item_type: ItemType,
    pub name: String,
}

impl ItemKey {
    /// 从字符串解析 ItemKey
    ///
    /// 格式：`{raw|calc|system}:{name}`
    pub fn parse(key: &str) -> Option<Self> {
        let (prefix, name) = key.split_once(':')?;
        let item_type = ItemType::from_str(prefix)?;
        if name.is_empty() {
            return None;
        }
        Some(ItemKey {
            item_type,
            name: name.to_string(),
        })
    }

    /// 直接构造 ItemKey
    pub fn new(item_type: ItemType, name: impl Into<String>) -> Self {
        ItemKey {
            item_type,
            name: name.into(),
        }
    }

    /// 是否为 raw 类型
    pub fn is_raw(&self) -> bool {
        self.item_type == ItemType::Raw
    }

    /// 是否为 calculated 类型
    pub fn is_calculated(&self) -> bool {
        self.item_type == ItemType::Calculated
    }

    /// 是否为 system 类型
    pub fn is_system(&self) -> bool {
        self.item_type == ItemType::System
    }
}

impl fmt::Display for ItemKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.item_type.as_str(), self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_raw() {
        let key = ItemKey::parse("raw:controls.speed_kmh").unwrap();
        assert_eq!(key.item_type, ItemType::Raw);
        assert_eq!(key.name, "controls.speed_kmh");
        assert_eq!(key.to_string(), "raw:controls.speed_kmh");
    }

    #[test]
    fn test_parse_calc() {
        let key = ItemKey::parse("calc:delta_best").unwrap();
        assert_eq!(key.item_type, ItemType::Calculated);
        assert_eq!(key.name, "delta_best");
    }

    #[test]
    fn test_parse_system() {
        let key = ItemKey::parse("system:cpu_temp").unwrap();
        assert_eq!(key.item_type, ItemType::System);
        assert_eq!(key.name, "cpu_temp");
    }

    #[test]
    fn test_parse_invalid_prefix() {
        assert!(ItemKey::parse("foo:bar").is_none());
    }

    #[test]
    fn test_parse_empty_name() {
        assert!(ItemKey::parse("raw:").is_none());
    }

    #[test]
    fn test_parse_no_colon() {
        assert!(ItemKey::parse("speed_kmh").is_none());
    }

    #[test]
    fn test_is_methods() {
        assert!(ItemKey::parse("raw:x").unwrap().is_raw());
        assert!(ItemKey::parse("calc:x").unwrap().is_calculated());
        assert!(ItemKey::parse("system:x").unwrap().is_system());
        assert!(!ItemKey::parse("raw:x").unwrap().is_calculated());
    }
}
