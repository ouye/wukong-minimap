// This file is new in a fork of jaskang/wukong-minimap.
//
// Upstream: https://github.com/jaskang/wukong-minimap (Apache-2.0)
// Fork:     https://github.com/Ouye/wukong-minimap
//
// Finds a Chinese font on the machine and works out the minimal set of glyphs
// to bake, so the overlay can print Chinese without shipping a font.

use std::path::Path;

use hudhook::tracing;

/// Windows 自带的中文字体，按优先顺序。`.ttc` 是字体集合，imgui-rs 没有暴露
/// `FontNo`，只能取集合里的第 0 个 —— 对这几个文件来说正好是想要的那一款。
const CANDIDATES: &[&str] = &[
    "msyh.ttc",   // 微软雅黑
    "msyhl.ttc",  // 微软雅黑 Light
    "Deng.ttf",   // 等线
    "simhei.ttf", // 黑体
    "simsun.ttc", // 宋体
];

/// 读一款系统中文字体。都找不到就返回 None，调用方回退到内置的 ASCII 字体。
///
/// 不把字体打进 dll：中文字体动辄十几 MB，而且大多不允许随程序再分发。
pub fn load_cjk() -> Option<(&'static str, Vec<u8>)> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| String::from("C:\\Windows"));
    for name in CANDIDATES {
        let path = Path::new(&dir).join("Fonts").join(name);
        match std::fs::read(&path) {
            Ok(data) => {
                tracing::info!("font: using {} ({} KB)", path.display(), data.len() / 1024);
                return Some((name, data));
            }
            Err(_) => continue,
        }
    }
    tracing::info!("font: no Chinese system font found, falling back to ASCII");
    None
}

/// 由实际要显示的文本反推 imgui 需要的字形区间。
///
/// 排序去重后把连续码位并成区间，以 0 结尾（`FontGlyphRanges::from_slice` 的
/// 格式要求）。只烘焙用得到的那几十个字，字体图集就不会因为中文而膨胀。
///
/// 返回值必须是 `'static`，而区间是运行期算出来的，所以这里 leak 一次 —— 整个
/// 进程只会调用一次，泄漏的是几十个 u32。
pub fn glyph_ranges_for(texts: &[&str]) -> &'static [u32] {
    let mut codes: Vec<u32> = texts
        .iter()
        .flat_map(|t| t.chars())
        .map(|c| c as u32)
        .collect();
    // 可打印 ASCII：署名那行、数字、标点都要用。
    codes.extend(0x20u32..=0x7e);
    codes.sort_unstable();
    codes.dedup();

    let mut ranges: Vec<u32> = Vec::new();
    let mut i = 0;
    while i < codes.len() {
        let start = codes[i];
        let mut end = start;
        while i + 1 < codes.len() && codes[i + 1] == end + 1 {
            i += 1;
            end = codes[i];
        }
        ranges.push(start);
        ranges.push(end);
        i += 1;
    }
    ranges.push(0);

    tracing::debug!("font: {} glyphs in {} ranges", codes.len(), ranges.len() / 2);
    Box::leak(ranges.into_boxed_slice())
}
