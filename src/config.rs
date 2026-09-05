// This file is new in a fork of jaskang/wukong-minimap.
//
// Upstream: https://github.com/jaskang/wukong-minimap (Apache-2.0)
// Fork:     https://github.com/Ouye/wukong-minimap
//
// Remembers the settings the player changes with the hotkeys, so they survive
// a restart.

use std::path::{Path, PathBuf};

use hudhook::tracing;
use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "wukong_minimap_config.json";

/// 路线的内置颜色：青色，八成不透明。地图底色以土黄、褐、墨绿为主，
/// 冷色在上面最跳，也不会和暖色调的边框、图标撞在一起。
pub const DEFAULT_TRAIL_COLOR: &str = "#22E0FFCC";

/// 还没发现玩家的敌人。暗一档，和"已经冲过来了"区分开。
pub const DEFAULT_ENEMY_COLOR: &str = "#C2352BCC";

/// 已经进入战斗的敌人。亮红，第一眼就该看到。
pub const DEFAULT_ALERT_COLOR: &str = "#FF4438FF";

/// 中立/友方的灰点。刻意压暗，让红点先跳出来。
pub const DEFAULT_NEUTRAL_COLOR: &str = "#C8C8C8A0";

/// 掉落物。暖金色 —— 打完一场架掉在地上没捡的，最想找的就是这个。
pub const DEFAULT_DROP_COLOR: &str = "#FFC93CEE";

/// 采集物（药材、材料）。绿色。
pub const DEFAULT_COLLECT_COLOR: &str = "#5BD86BE0";

/// 其它可交互物。中性白，不抢眼。
pub const DEFAULT_INTERACT_COLOR: &str = "#E0E0E0B4";

fn default_size() -> f32 {
    0.25
}
fn default_zoom() -> f32 {
    0.2
}
fn default_trail() -> bool {
    true
}
fn default_true() -> bool {
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
    /// 小地图上显示敌对目标的红点。
    #[serde(default = "default_true")]
    pub show_enemies: bool,
    /// 小地图上显示中立/友方的灰点。默认关：多数时候是噪音。
    #[serde(default)]
    pub show_neutrals: bool,
    /// 红点颜色，格式同 `trail_color`。
    #[serde(default)]
    pub enemy_color: Option<String>,
    /// 灰点颜色，格式同 `trail_color`。
    #[serde(default)]
    pub neutral_color: Option<String>,
    /// 小地图上显示掉落物、采集物等可交互物。
    #[serde(default)]
    pub show_items: bool,
    /// 已进入战斗的敌人颜色。
    #[serde(default)]
    pub alert_color: Option<String>,
    /// 掉落物颜色。
    #[serde(default)]
    pub drop_color: Option<String>,
    /// 采集物颜色。
    #[serde(default)]
    pub collect_color: Option<String>,
    /// 其它可交互物颜色。
    #[serde(default)]
    pub interact_color: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            size: default_size(),
            zoom: default_zoom(),
            rotate_map: false,
            trail_enabled: default_trail(),
            trail_color: Some(String::from(DEFAULT_TRAIL_COLOR)),
            show_enemies: true,
            show_neutrals: false,
            enemy_color: Some(String::from(DEFAULT_ENEMY_COLOR)),
            neutral_color: Some(String::from(DEFAULT_NEUTRAL_COLOR)),
            show_items: false,
            alert_color: Some(String::from(DEFAULT_ALERT_COLOR)),
            drop_color: Some(String::from(DEFAULT_DROP_COLOR)),
            collect_color: Some(String::from(DEFAULT_COLLECT_COLOR)),
            interact_color: Some(String::from(DEFAULT_INTERACT_COLOR)),
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
                if cfg.enemy_color.is_none() {
                    cfg.enemy_color = Some(String::from(DEFAULT_ENEMY_COLOR));
                }
                if cfg.neutral_color.is_none() {
                    cfg.neutral_color = Some(String::from(DEFAULT_NEUTRAL_COLOR));
                }
                if cfg.alert_color.is_none() {
                    cfg.alert_color = Some(String::from(DEFAULT_ALERT_COLOR));
                }
                if cfg.drop_color.is_none() {
                    cfg.drop_color = Some(String::from(DEFAULT_DROP_COLOR));
                }
                if cfg.collect_color.is_none() {
                    cfg.collect_color = Some(String::from(DEFAULT_COLLECT_COLOR));
                }
                if cfg.interact_color.is_none() {
                    cfg.interact_color = Some(String::from(DEFAULT_INTERACT_COLOR));
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

    pub fn trail_color_rgba(&self) -> [f32; 4] {
        resolve_color("trail_color", self.trail_color.as_deref(), DEFAULT_TRAIL_COLOR)
    }

    pub fn enemy_color_rgba(&self) -> [f32; 4] {
        resolve_color("enemy_color", self.enemy_color.as_deref(), DEFAULT_ENEMY_COLOR)
    }

    pub fn neutral_color_rgba(&self) -> [f32; 4] {
        resolve_color(
            "neutral_color",
            self.neutral_color.as_deref(),
            DEFAULT_NEUTRAL_COLOR,
        )
    }

    pub fn alert_color_rgba(&self) -> [f32; 4] {
        resolve_color("alert_color", self.alert_color.as_deref(), DEFAULT_ALERT_COLOR)
    }

    pub fn drop_color_rgba(&self) -> [f32; 4] {
        resolve_color("drop_color", self.drop_color.as_deref(), DEFAULT_DROP_COLOR)
    }

    pub fn collect_color_rgba(&self) -> [f32; 4] {
        resolve_color(
            "collect_color",
            self.collect_color.as_deref(),
            DEFAULT_COLLECT_COLOR,
        )
    }

    pub fn interact_color_rgba(&self) -> [f32; 4] {
        resolve_color(
            "interact_color",
            self.interact_color.as_deref(),
            DEFAULT_INTERACT_COLOR,
        )
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

/// 解析一项颜色配置。没写或者写错就用内置的，并说明是哪一项、哪个值。
fn resolve_color(field: &str, text: Option<&str>, default_hex: &str) -> [f32; 4] {
    let fallback = || parse_hex_rgba(default_hex).expect("a built-in colour is valid hex");
    match text {
        None => fallback(),
        Some(text) => match parse_hex_rgba(text) {
            Some(rgba) => rgba,
            None => {
                tracing::error!(
                    "config: {field} \"{text}\" is not #RRGGBB or #RRGGBBAA, using {default_hex}"
                );
                fallback()
            }
        },
    }
}
