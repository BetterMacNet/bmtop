//! 把 `system_profiler` 的嵌套 JSON 整理成硬件 / 传感器页的左侧分区与右侧详情。

use bmtop_core::Language;
use bmtop_tui::DetailSection;

/// `system_profiler` 的数据类型键名对应的中文分区名。查不到就原样显示，
/// 新增数据类型时不会渲染成空白。
///
/// 英文模式不做映射，直接用 `SPHardwareDataType` 这种原名：它本来就是英文，
/// 且和 `system_profiler` 命令行里敲的完全一致，省掉一张要维护的对照表。
pub(crate) fn hardware_section_label(key: &str, language: Language) -> &str {
    if language == Language::English {
        return key;
    }
    match key {
        "SPHardwareDataType" => "硬件概览",
        "SPDisplaysDataType" => "显示与显卡",
        "SPMemoryDataType" => "内存",
        "SPStorageDataType" => "存储",
        "SPPowerDataType" => "电源与电池",
        "SPNetworkDataType" => "网络",
        "SPUSBDataType" => "USB",
        "SPThunderboltDataType" => "Thunderbolt",
        "SPBluetoothDataType" => "蓝牙",
        "SPAudioDataType" => "音频",
        "SPCameraDataType" => "摄像头",
        other => other,
    }
}

/// 硬件页：每个 `SP*DataType` 一个分区。
/// 分区展示顺序：从整机概览往外设走，比 JSON 对象的字典序好读。
const HARDWARE_SECTION_ORDER: [&str; 11] = [
    "SPHardwareDataType",
    "SPMemoryDataType",
    "SPStorageDataType",
    "SPDisplaysDataType",
    "SPPowerDataType",
    "SPNetworkDataType",
    "SPBluetoothDataType",
    "SPThunderboltDataType",
    "SPUSBDataType",
    "SPAudioDataType",
    "SPCameraDataType",
];

pub(crate) fn hardware_sections(
    sections: &serde_json::Map<String, serde_json::Value>,
    language: Language,
) -> Vec<DetailSection> {
    let rank = |key: &str| {
        HARDWARE_SECTION_ORDER
            .iter()
            .position(|known| *known == key)
            .unwrap_or(HARDWARE_SECTION_ORDER.len())
    };
    let mut keys: Vec<&String> = sections.keys().collect();
    keys.sort_by(|left, right| rank(left).cmp(&rank(right)).then(left.cmp(right)));
    keys.into_iter()
        .map(|key| {
            DetailSection::new(
                hardware_section_label(key, language),
                flatten_json(&sections[key]),
            )
        })
        .collect()
}

/// 传感器页：`SPPowerDataType` 是一个数组，每个元素按 `_name` 拆成一个分区。
const FLATTEN_KEY_COLUMNS: usize = 30;

fn scalar_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => "--".to_string(),
        other => other.to_string(),
    }
}

/// 把 `system_profiler` 的嵌套 JSON 摊平成缩进的键值文本。
/// 直接把 JSON 原样丢进面板是读不了的，这是这次要修的核心问题之一。
fn flatten_json(value: &serde_json::Value) -> String {
    let mut output = String::new();
    write_flattened(value, 0, &mut output);
    output
}

fn write_flattened(value: &serde_json::Value, depth: usize, output: &mut String) {
    let pad = "  ".repeat(depth);
    match value {
        serde_json::Value::Object(map) => {
            for (key, item) in map {
                // `_name` 已经当成分区/条目标题用掉了，正文里不再重复。
                if key == "_name" {
                    continue;
                }
                match item {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        output.push_str(&format!("{pad}{key}\n"));
                        write_flattened(item, depth + 1, output);
                    }
                    // 键比列宽还长时 `{:<width$}` 不会补空格，键和值会黏在一起
                    // （`..._maximum_capacity92%`）。所以至少留两个空格。
                    _ => {
                        let column = FLATTEN_KEY_COLUMNS.saturating_sub(pad.len());
                        let gap = column.saturating_sub(key.chars().count()).max(2);
                        output.push_str(&format!(
                            "{pad}{key}{}{}\n",
                            " ".repeat(gap),
                            scalar_text(item)
                        ));
                    }
                }
            }
        }
        serde_json::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let heading = item
                    .get("_name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("#{}", index + 1));
                output.push_str(&format!("{pad}{heading}\n"));
                write_flattened(item, depth + 1, output);
                output.push('\n');
            }
        }
        other => output.push_str(&format!("{pad}{}\n", scalar_text(other))),
    }
}
