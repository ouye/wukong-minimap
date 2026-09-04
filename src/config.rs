// This file is new in a fork of jaskang/wukong-minimap.
//
// Upstream: https://github.com/jaskang/wukong-minimap (Apache-2.0)
// Fork:     https://github.com/Ouye/wukong-minimap
//
// Remembers the settings the player changes with the hotkeys, so they survive
// a restart.
//
// See CHANGES.md for the full record of what was changed and why.

use std::path::{Path, PathBuf};

use hudhook::tracing;
use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "wukong_minimap_config.json";

/// 路线的内置颜色：青色，八成不透明。地图底色以土黄、褐、墨绿为主，
/// 冷色在上面最跳，也不会和暖色调的边框、图标撞在一起。
pub const DEFAULT_TRAIL_COLOR: &str = "#22E0FFCC";

fn default_size() -> f32 {
    0.25
}
fn default_zoom() -> f32 {
    0.2
}
fn default_trail() -> bool {
    true
}

/// 每个字段都带 `serde(default)`，这样旧的配置文件、手改坏的文件、
/// 以后新增的字段都不会让读取整个失败。
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_size")]
    pub size: f32,
    #[serde(default = "default_zoom")]
    pub zoom: f32,
    #[serde(default)]
    pub rotate_map: bool,
    #[serde(default = "default_trail")]
    pub trail_enabled: bool,
    /// 路线颜色，`#RRGGBB` 或 `#RRGGBBAA`。
    ///
    /// 留空或者写错格式都退回内置颜色。启动时读一次，改完要重进游戏。
    #[serde(default)]
    pub trail_color: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            size: default_size(),
            zoom: default_zoom(),
            rotate_map: false,
            trail_enabled: default_trail(),
            trail_color: Some(String::from(DEFAULT_TRAIL_COLOR)),
        }
    }
}

impl Config {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }

    pub fn load(dir: &Path) -> Self {
        let path = Self::path(dir);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => return Self::default(),
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(mut cfg) => {
                // 补上文件里没有的字段，这样启动时那次落盘会把完整的一份写回去，
                // 用户打开配置文件就能看到所有可改的项和它们当前的值。
                if cfg.trail_color.is_none() {
                    cfg.trail_color = Some(String::from(DEFAULT_TRAIL_COLOR));
                }
                tracing::info!(
                    "config: size={:.2} zoom={:.2} rotate_map={} trail={} color={}",
                    cfg.size,
                    cfg.zoom,
                    cfg.rotate_map,
                    cfg.trail_enabled,
                    cfg.trail_color.as_deref().unwrap_or("-")
                );
                cfg.clamped()
            }
            Err(e) => {
                tracing::error!("config: {} is unreadable ({e}), using defaults", path.display());
                Self::default()
            }
        }
    }

    /// 手改过的文件可能把倍率写成 0 或者负数，那样小地图会直接消失。
    /// 范围和按键里的上下限保持一致。
    fn clamped(mut self) -> Self {
        self.size = self.size.clamp(0.15, 0.5);
        self.zoom = self.zoom.clamp(0.15, 0.5);
        self
    }

    /// 解析出来的路线颜色。没写或者写错就用内置的，并且说明为什么。
    pub fn trail_color_rgba(&self) -> [f32; 4] {
        let fallback = || {
            parse_hex_rgba(DEFAULT_TRAIL_COLOR).expect("the built-in colour is a valid hex")
        };
        match self.trail_color.as_deref() {
            None => fallback(),
            Some(text) => match parse_hex_rgba(text) {
                Some(rgba) => rgba,
                None => {
                    tracing::error!(
                        "config: trail_color \"{text}\" is not #RRGGBB or #RRGGBBAA, \
                         using {DEFAULT_TRAIL_COLOR}"
                    );
                    fallback()
                }
            },
        }
    }

    /// 先写临时文件再改名，中途崩溃不会毁掉已有的配置。
    pub fn save(&self, path: &Path) {
        let text = match serde_json::to_string_pretty(self) {
            Ok(text) => text,
            Err(e) => {
                tracing::error!("config: could not serialize: {e}");
                return;
            }
        };
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, text) {
            tracing::error!("config: could not write {}: {e}", tmp.display());
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            tracing::error!("config: could not replace {}: {e}", path.display());
            return;
        }
        tracing::debug!("config: saved");
    }
}

/// `#RRGGBB` / `#RRGGBBAA` 解析成 imgui 要的 0..1 四元组。`#` 可省。
/// 六位形式的透明度按不透明处理。
pub fn parse_hex_rgba(text: &str) -> Option<[f32; 4]> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let [r, g, b, a] = if hex.len() == 6 {
        [(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff, 0xff]
    } else {
        [
            (value >> 24) & 0xff,
            (value >> 16) & 0xff,
            (value >> 8) & 0xff,
            value & 0xff,
        ]
    };
    Some([
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ])
}
