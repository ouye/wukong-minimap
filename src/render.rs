// This file was modified in a fork of jaskang/wukong-minimap.
//
// Upstream: https://github.com/jaskang/wukong-minimap (Apache-2.0)
// Fork:     https://github.com/Ouye/wukong-minimap
//
// Changes: maps are loaded on demand instead of all at once; the nomap
// placeholder is resized to match the map textures; replace_texture
// failures are logged.
//
// See CHANGES.md for the full record of what was changed and why.

use std::{collections::HashMap, sync::Mutex};

use crate::{
    utils::{
        image_with_bytes, image_with_file, is_in_map, load_data, load_points, MapInfo, Point, Pos2,
    },
    wukong::{self, GameState},
};
use gilrs::{GamepadId, Gilrs};
use hudhook::{
    imgui::{self, Condition, Context, WindowFlags},
    ImguiRenderLoop, RenderContext,
};
use hudhook::{
    imgui::{Image, Key},
    tracing,
};
use image::{EncodableLayout, ImageFormat, RgbaImage};

#[derive(Clone, Debug)]
pub struct MapView {
    pub center: Pos2,
    pub min_pos: Pos2,
    pub max_pos: Pos2,
    pub map_size: [f32; 2],
    pub map_full_size: [f32; 2],
    pub scale_x: f32,
    pub scale_y: f32,
}

pub struct ImageTexture {
    pub id: Option<imgui::TextureId>,
    pub image: RgbaImage,
}

impl ImageTexture {
    pub fn with_bytes(types: &[u8], format: ImageFormat) -> Self {
        Self {
            id: None,
            image: image_with_bytes(types, format),
        }
    }
}

struct Textures {
    pub map: ImageTexture,
    pub mapplayer: ImageTexture,
    pub mapwraper: ImageTexture,
    pub mainwraper: ImageTexture,
    pub tips: ImageTexture,

    // const categories = [
    //     "yaocai",
    //     "pass-route",
    //     "start",
    //     "renwu",
    //     "lingyun",
    //     "zhixian",
    //     "end",
    //     "hidden",
    //     "comment",
    // ];
    pub teleport: ImageTexture,
    // pub fan: ImageTexture,          // 起点-招魂幡
    pub boss: ImageTexture,         // boss
    pub toumu: ImageTexture,        // 头目
    pub hulu: ImageTexture,         // 葫芦
    pub jiushi: ImageTexture,       // 酒食
    pub xiandan: ImageTexture,      // 仙丹
    pub baoxiang: ImageTexture,     // 宝箱
    pub zhenwan: ImageTexture,      // 珍玩
    pub dazuo: ImageTexture,        // 打坐点
    pub cailiao: ImageTexture,      // 材料
    pub jingpo: ImageTexture,       // 精魄
    pub sandongchong: ImageTexture, // 三冬虫
    pub luojia: ImageTexture,       // 洛珈
    pub bianhua: ImageTexture,      // 变化
    pub yaojin: ImageTexture,       // 要紧事物
}

// 定义宏来简化纹理创建，仅适用于PNG格式
/// Edge length of the map textures. The images in `maps/` are authored at this
/// size and the placeholder is resized to match.
const MAP_SIZE: u32 = 2000;

macro_rules! png_texture {
    ($file:expr) => {
        ImageTexture::with_bytes(include_bytes!($file), ImageFormat::Png)
    };
}

pub struct MiniMap {
    gilrs: Mutex<Gilrs>,
    current_gamepad: Option<GamepadId>,
    textures: Textures,
    map_images: HashMap<String, RgbaImage>,
    zoom: f32,
    size: f32,
    map: Option<MapInfo>,
    maps: Vec<MapInfo>,
    points: HashMap<String, Vec<Point>>,
    game: GameState,
    is_show_main: bool,
    is_show: bool,
}

impl MiniMap {
    pub fn new() -> Self {
        wukong::init();

        let maps: Vec<MapInfo> = load_data();
        let points: HashMap<String, Vec<Point>> = load_points();

        let textures = Textures {
            // The placeholder fixes the map texture's dimensions for the whole
            // session -- replace_texture refuses a size change -- so it has to
            // match the map images, which are 2000x2000.
            map: ImageTexture {
                id: None,
                image: image::imageops::resize(
                    &image_with_bytes(include_bytes!("../includes/nomap.webp"), ImageFormat::WebP),
                    MAP_SIZE,
                    MAP_SIZE,
                    image::imageops::FilterType::Triangle,
                ),
            },
            mapwraper: png_texture!("../includes/mapwraper.png"),
            mainwraper: png_texture!("../includes/mainwraper.png"),
            mapplayer: png_texture!("../includes/icon_player.png"),
            tips: png_texture!("../includes/tips.png"),

            teleport: png_texture!("../includes/icon_teleport.png"),
            boss: png_texture!("../includes/icon_boss.png"),
            toumu: png_texture!("../includes/icon_toumu.png"),
            hulu: png_texture!("../includes/icon_hulu.png"),
            jiushi: png_texture!("../includes/icon_jiushi.png"),
            xiandan: png_texture!("../includes/icon_xiandan.png"),
            baoxiang: png_texture!("../includes/icon_baoxiang.png"),
            zhenwan: png_texture!("../includes/icon_zhenwan.png"),
            dazuo: png_texture!("../includes/icon_dazuo.png"),
            yaojin: png_texture!("../includes/icon_yaojin.png"),
            cailiao: png_texture!("../includes/icon_cailiao.png"),
            jingpo: png_texture!("../includes/icon_jingpo.png"),
            sandongchong: png_texture!("../includes/icon_sandongchong.png"),
            luojia: png_texture!("../includes/icon_luojia.png"),
            bianhua: png_texture!("../includes/icon_bianhua.png"),
        };

        // Maps are loaded on demand in before_render, not all at once. The
        // full set is 23 images; at 2000x2000 RGBA that would be 368 MB of
        // resident memory for images the player is not looking at.
        let map_images: HashMap<String, RgbaImage> = HashMap::new();

        let gilrs = Gilrs::new().unwrap();
        Self {
            gilrs: Mutex::new(gilrs),
            current_gamepad: None,
            textures,
            map_images,
            zoom: 0.2,
            size: 0.25,
            map: None,
            maps,
            points,
            game: wukong::game_state(),
            is_show_main: false,
            is_show: true,
        }
    }

    fn update_map(&mut self) -> Option<MapInfo> {
        self.game = wukong::game_state();
        let map = self
            .maps
            .iter()
            .rev() // 从后面开始查找
            .find(|map| {
                if self.game.level == map.level
                    && self.game.x >= map.range.start[0]
                    && self.game.x <= map.range.end[0]
                    && self.game.y >= map.range.start[1]
                    && self.game.y <= map.range.end[1]
                    && self.game.z >= map.range.start[2]
                    && self.game.z <= map.range.end[2]
                {
                    return if map.areas.is_empty() {
                        true
                    } else {
                        map.areas.iter().any(|area| {
                            self.game.x >= area.start[0]
                                && self.game.x <= area.end[0]
                                && self.game.y >= area.start[1]
                                && self.game.y <= area.end[1]
                                && self.game.z >= area.start[2]
                                && self.game.z <= area.end[2]
                        })
                    };
                }
                false
            })
            .cloned();

        match (self.map.as_ref(), map) {
            (_, None) => {
                return None;
            }
            (None, Some(new_map)) => {
                return Some(new_map);
            }
            (Some(current_map), Some(new_map)) => {
                if current_map.key != new_map.key {
                    return Some(new_map);
                } else {
                    return None;
                }
            }
        }
    }

    fn get_map_view(&self, map: &MapInfo, player_pos: Pos2, zoom: f32, view_size: f32) -> MapView {
        let [x_start, y_start, _] = map.range.start;
        let [x_end, y_end, _] = map.range.end;
        let map_full_width = (x_end - x_start).abs();
        let map_full_height = (y_end - y_start).abs();
        let map_width = map_full_width * zoom;
        let map_height = map_full_height * zoom;

        let center_x = if player_pos.x - map_width / 2.0 < x_start {
            x_start + map_width / 2.0
        } else if player_pos.x + map_width / 2.0 > x_end {
            x_end - map_width / 2.0
        } else {
            player_pos.x
        };
        let center_y = if player_pos.y - map_height / 2.0 < y_start {
            y_start + map_height / 2.0
        } else if player_pos.y + map_height / 2.0 > y_end {
            y_end - map_height / 2.0
        } else {
            player_pos.y
        };

        let min_pos = Pos2::new(center_x - map_width / 2.0, center_y - map_height / 2.0);
        let max_pos = Pos2::new(center_x + map_width / 2.0, center_y + map_height / 2.0);
        MapView {
            center: Pos2::new(center_x, center_y),
            map_full_size: [map_full_width, map_full_height],
            map_size: [map_width, map_height],
            scale_x: view_size / map_width,
            scale_y: view_size / map_height,
            min_pos: min_pos,
            max_pos: max_pos,
        }
    }

    fn get_view_uv(&self, map: &MapInfo, pos: Pos2) -> (f32, f32) {
        let [x_start, y_start, _] = map.range.start;
        let [x_end, y_end, _] = map.range.end;
        let x_offset = (pos.x - x_start) / (x_end - x_start);
        let y_offset = (pos.y - y_start) / (y_end - y_start);
        (x_offset, y_offset)
    }
    /**
     * 将游戏中玩家转换为小地图uv
     * 玩家位于小地图的中心，根据 zoom 计算出小地图的uv
     * size 小地图可视区域的比例    
     */
    fn get_map_uv(&self, map: &MapInfo, map_view: &MapView) -> ([f32; 2], [f32; 2]) {
        let [x_start, y_start, _] = map.range.start;

        let uv_x_start =
            (map_view.center.x - map_view.map_size[0] / 2.0 - x_start) / map_view.map_full_size[0];
        let uv_x_end =
            (map_view.center.x + map_view.map_size[0] / 2.0 - x_start) / map_view.map_full_size[0];
        let uv_y_start =
            (map_view.center.y - map_view.map_size[1] / 2.0 - y_start) / map_view.map_full_size[1];
        let uv_y_end =
            (map_view.center.y + map_view.map_size[1] / 2.0 - y_start) / map_view.map_full_size[1];
        ([uv_x_start, uv_y_start], [uv_x_end, uv_y_end])
    }

    fn get_icon_offset(&self, pos: Pos2, start_offset: [f32; 2], map_view: &MapView) -> Pos2 {
        Pos2::new(
            (pos.x - map_view.min_pos.x) * map_view.scale_x + start_offset[0],
            (pos.y - map_view.min_pos.y) * map_view.scale_y + start_offset[1],
        )
    }
    fn p4_with_angle(&self, location: Pos2, angle: f32, icon_size: f32) -> [[f32; 2]; 4] {
        let rad = angle.to_radians();
        let (sin, cos) = rad.sin_cos();
        let half = icon_size / 2.0;
        // 使用一个简单的变换矩阵计算四个角点
        let transform = |dx: f32, dy: f32| {
            [
                location.x + dx * cos - dy * sin,
                location.y + dx * sin + dy * cos,
            ]
        };
        [
            transform(-half, -half), // 左上
            transform(half, -half),  // 右上
            transform(half, half),   // 右下
            transform(-half, half),  // 左下
        ]
    }
    fn render_mainmap(&mut self, ui: &imgui::Ui) {
        let [screen_width, screen_height] = ui.io().display_size;
        let window_size = screen_width.min(screen_height);
        let map_size: f32 = window_size * 0.95;
        let icon_size = screen_width.min(screen_height) * 0.03;
        let icon_size_half = icon_size / 2.0;
        let [window_offset_x, window_offset_y] = [
            (screen_width - window_size) / 2.0,
            (screen_height - window_size) / 2.0,
        ];

        ui.window("wukong-mainmap")
            .size([window_size, window_size], Condition::Always)
            .position([window_offset_x, window_offset_y], Condition::Always)
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_INPUTS
                    | WindowFlags::NO_NAV
                    | WindowFlags::NO_BACKGROUND,
            )
            .build(|| {
                ui.set_cursor_pos([0.0, 0.0]);
                let draw_list = ui.get_window_draw_list();

                if let Some(map) = self.map.as_ref() {
                    // 绘制地图
                    let map_image = self.textures.map.id.unwrap();
                    let map_offset_x = window_offset_x + (window_size - map_size) / 2.0;
                    let map_offset_y = window_offset_y + (window_size - map_size) / 2.0;
                    let map_view =
                        self.get_map_view(map, Pos2::new(self.game.x, self.game.y), 0.7, map_size);

                    let (uv_min, uv_max) = self.get_map_uv(map, &map_view);

                    draw_list
                        .add_image_rounded(
                            map_image,
                            [map_offset_x, map_offset_y],
                            [map_offset_x + map_size, map_offset_y + map_size],
                            0.0,
                        )
                        .uv_min(uv_min)
                        .uv_max(uv_max)
                        .build();

                    // 绘制地图图标
                    self.points
                        .get(map.level.as_str())
                        .unwrap_or(&vec![])
                        .iter()
                        .filter(|point| is_in_map(point, &map))
                        .for_each(|point| {
                            let icon = match point.category.as_str() {
                                "teleport" => self.textures.teleport.id,
                                "boss" => self.textures.boss.id,
                                "toumu" => self.textures.toumu.id,
                                "hulu" => self.textures.hulu.id,
                                "jiushi" => self.textures.jiushi.id,
                                "xiandan" => self.textures.xiandan.id,
                                "baoxiang" => self.textures.baoxiang.id,
                                "zhenwan" => self.textures.zhenwan.id,
                                "dazuo" => self.textures.dazuo.id,
                                "cailiao" => self.textures.cailiao.id,
                                "jingpo" => self.textures.jingpo.id,
                                "sandongchong" => self.textures.sandongchong.id,
                                "luojia" => self.textures.luojia.id,
                                "bianhua" => self.textures.bianhua.id,
                                "yaojin" => self.textures.yaojin.id,
                                _ => None,
                            };

                            if let Some(id) = icon {
                                let icon_offset = self.get_icon_offset(
                                    Pos2::new(point.x, point.y),
                                    [map_offset_x, map_offset_y],
                                    &map_view,
                                );

                                // 判断是否在可视区域内
                                let in_view = point.x > map_view.min_pos.x
                                    && point.x < map_view.max_pos.x
                                    && point.y > map_view.min_pos.y
                                    && point.y < map_view.max_pos.y;
                                if in_view {
                                    draw_list
                                        .add_image(
                                            id,
                                            [
                                                icon_offset.x - icon_size_half,
                                                icon_offset.y - icon_size_half,
                                            ],
                                            [
                                                icon_offset.x + icon_size_half,
                                                icon_offset.y + icon_size_half,
                                            ],
                                        )
                                        .build();
                                }
                            }
                        });

                    let player_offset = self.get_icon_offset(
                        Pos2::new(self.game.x, self.game.y),
                        [map_offset_x, map_offset_y],
                        &map_view,
                    );
                    // 绘制玩家角色箭头
                    let [p0, p1, p2, p3] =
                        self.p4_with_angle(player_offset, self.game.angle, icon_size * 1.5);
                    draw_list
                        .add_image_quad(self.textures.mapplayer.id.unwrap(), p0, p1, p2, p3)
                        .build();

                    // 绘制外围地图边框
                    draw_list
                        .add_image(
                            self.textures.mainwraper.id.unwrap(),
                            [window_offset_x, window_offset_y],
                            [window_offset_x + window_size, window_offset_y + window_size],
                        )
                        .build();
                    ui.set_cursor_pos([40.0, map_size - 40.0]);
                    // ui.text(format!("{:?}", self.game));
                } else {
                    tracing::info!("draw_nomap");
                }
            });

        let tips_width = window_size * 0.1825;
        let tips_height = window_size;
        let tips_offset_x = window_offset_x - tips_width;
        let tips_offset_y = window_offset_y;
        ui.window("wukong-mainmap-tips")
            .size([tips_width, tips_height], Condition::Always)
            .position([tips_offset_x, tips_offset_y], Condition::Always)
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_INPUTS
                    | WindowFlags::NO_NAV
                    | WindowFlags::NO_BACKGROUND,
            )
            .build(|| {
                let draw_list = ui.get_window_draw_list();
                draw_list
                    .add_image(
                        self.textures.tips.id.unwrap(),
                        [tips_offset_x, tips_offset_y],
                        [tips_offset_x + tips_width, tips_offset_y + tips_height],
                    )
                    .build();
            });
    }
    fn render_minimap(&mut self, ui: &imgui::Ui) {
        let [screen_width, screen_height] = ui.io().display_size;
        let window_size = screen_width.min(screen_height) * self.size;
        let map_size: f32 = window_size * 0.947;
        let icon_size = screen_width.min(screen_height) * 0.025;
        let icon_size_half = icon_size / 2.0;
        let [window_offset_x, window_offset_y] = [screen_width - window_size - 10.0, 10.0];
        let center = Pos2::new(
            window_offset_x + window_size / 2.0,
            window_offset_y + window_size / 2.0,
        );

        ui.window("wukong-minimap")
            .size([window_size, window_size], Condition::Always)
            .position([window_offset_x, window_offset_y], Condition::Always)
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_INPUTS
                    | WindowFlags::NO_NAV
                    | WindowFlags::NO_BACKGROUND,
            )
            .build(|| {
                ui.set_cursor_pos([0.0, 0.0]);
                let draw_list = ui.get_window_draw_list();

                if let Some(map) = self.map.as_ref() {
                    // 绘制地图
                    let map_image = self.textures.map.id.unwrap();
                    let map_offset_x = window_offset_x + (window_size - map_size) / 2.0;
                    let map_offset_y = window_offset_y + (window_size - map_size) / 2.0;
                    let map_view = self.get_map_view(
                        map,
                        Pos2::new(self.game.x, self.game.y),
                        self.zoom,
                        map_size,
                    );

                    let (uv_min, uv_max) = self.get_map_uv(map, &map_view);
                    draw_list
                        .add_image_rounded(
                            map_image,
                            [map_offset_x, map_offset_y],
                            [map_offset_x + map_size, map_offset_y + map_size],
                            map_size / 2.0,
                        )
                        .uv_min(uv_min)
                        .uv_max(uv_max)
                        .build();

                    // 绘制地图图标
                    self.points
                        .get(map.level.as_str())
                        .unwrap_or(&vec![])
                        .iter()
                        .filter(|point| is_in_map(point, &map))
                        .for_each(|point| {
                            let icon = match point.category.as_str() {
                                "teleport" => self.textures.teleport.id,
                                "boss" => self.textures.boss.id,
                                "toumu" => self.textures.toumu.id,
                                "hulu" => self.textures.hulu.id,
                                "jiushi" => self.textures.jiushi.id,
                                "xiandan" => self.textures.xiandan.id,
                                "baoxiang" => self.textures.baoxiang.id,
                                "zhenwan" => self.textures.zhenwan.id,
                                "dazuo" => self.textures.dazuo.id,
                                "cailiao" => self.textures.cailiao.id,
                                "jingpo" => self.textures.jingpo.id,
                                "sandongchong" => self.textures.sandongchong.id,
                                "luojia" => self.textures.luojia.id,
                                "bianhua" => self.textures.bianhua.id,
                                "yaojin" => self.textures.yaojin.id,
                                _ => None,
                            };

                            if let Some(id) = icon {
                                let center_offset_x = map_offset_x + map_size / 2.0;
                                let center_offset_y = map_offset_y + map_size / 2.0;

                                let icon_offset = self.get_icon_offset(
                                    Pos2::new(point.x, point.y),
                                    [map_offset_x, map_offset_y],
                                    &map_view,
                                );
                                // 判断是否在可视区域内, icon_pos 和 center 之间的距离小于 map_size / 2 - icon_size_half
                                let distance = ((icon_offset.x - center_offset_x).powi(2)
                                    + (icon_offset.y - center_offset_y).powi(2))
                                .sqrt();
                                if distance <= map_size / 2.0 - icon_size_half {
                                    draw_list
                                        .add_image(
                                            id,
                                            [
                                                icon_offset.x - icon_size_half,
                                                icon_offset.y - icon_size_half,
                                            ],
                                            [
                                                icon_offset.x + icon_size_half,
                                                icon_offset.y + icon_size_half,
                                            ],
                                        )
                                        .build();
                                }
                            }
                        });

                    // 绘制玩家角色箭头
                    let [p0, p1, p2, p3] =
                        self.p4_with_angle(center, self.game.angle, icon_size * 1.5);
                    draw_list
                        .add_image_quad(self.textures.mapplayer.id.unwrap(), p0, p1, p2, p3)
                        .build();

                    // 绘制外围地图边框
                    draw_list
                        .add_image(
                            self.textures.mapwraper.id.unwrap(),
                            [window_offset_x, window_offset_y],
                            [window_offset_x + window_size, window_offset_y + window_size],
                        )
                        .build();
                } else {
                    tracing::info!("draw_nomap");
                }
            });
    }
    fn render(&mut self, ui: &imgui::Ui) {
        if ui.is_key_pressed_no_repeat(Key::Minus) && !ui.is_key_down(Key::LeftShift) {
            self.size = (self.size - 0.05).max(0.15);
            tracing::info!("size: {}", self.size);
        }
        if ui.is_key_pressed_no_repeat(Key::Equal) && !ui.is_key_down(Key::LeftShift) {
            self.size = (self.size + 0.05).min(0.5);
            tracing::info!("size: {}", self.size);
        }
        if ui.is_key_pressed_no_repeat(Key::Minus) && ui.is_key_down(Key::LeftShift) {
            self.zoom = (self.zoom - 0.05).max(0.15);
            tracing::info!("zoom: {}", self.zoom);
        }
        if ui.is_key_pressed_no_repeat(Key::Equal) && ui.is_key_down(Key::LeftShift) {
            self.zoom = (self.zoom + 0.05).min(0.5);
            tracing::info!("zoom: {}", self.zoom);
        }
        if ui.is_key_pressed_no_repeat(Key::Alpha0) {
            self.is_show = !self.is_show;
        }
        if ui.is_key_pressed_no_repeat(Key::Tab) {
            self.is_show_main = !self.is_show_main;
            // wukong::toggle_mouse_cursor(self.is_show_main);
        }
        if let Ok(mut gilrs) = self.gilrs.lock() {
            // Examine new events
            while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
                self.current_gamepad = Some(id);
                tracing::info!("gilrs event from {}: {:?}", id, event);
                if let gilrs::EventType::ButtonPressed(button, code) = event {
                    let gamepad = gilrs.gamepad(id);
                    if gamepad.is_pressed(gilrs::Button::RightTrigger) {
                        match button {
                            gilrs::Button::DPadDown => {
                                self.is_show_main = !self.is_show_main;
                                // wukong::toggle_mouse_cursor(self.is_show_main);
                                tracing::info!("Button West is pressed");
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if self.game.playing {
            if self.is_show_main {
                self.render_mainmap(ui);
            }
            if self.is_show {
                self.render_minimap(ui);
            }
        }
    }
}

impl ImguiRenderLoop for MiniMap {
    fn initialize<'a>(&'a mut self, ctx: &mut Context, render_context: &'a mut dyn RenderContext) {
        let io = ctx.io_mut();
        io.mouse_draw_cursor = false;
        let style = ctx.style_mut();
        style.window_rounding = 10.0;
        style.window_padding = [0.0, 0.0];
        // text red
        // style.colors[imgui::StyleColor::Text as usize] = [0.0, 0.0, 1.0, 1.0];

        // 定义宏来简化纹理加载
        macro_rules! load_textures {
            ($($texture:ident),*) => {
                $(
                    self.textures.$texture.id = Some(
                        render_context
                            .load_texture(
                                self.textures.$texture.image.as_bytes(),
                                self.textures.$texture.image.width(),
                                self.textures.$texture.image.height(),
                            )
                            .unwrap(),
                    );
                )*
            }
        }

        // 使用宏加载所有纹理
        load_textures!(
            map,
            mapplayer,
            mapwraper,
            mainwraper,
            tips,
            teleport,
            boss,
            toumu,
            hulu,
            jiushi,
            xiandan,
            baoxiang,
            zhenwan,
            dazuo,
            cailiao,
            jingpo,
            sandongchong,
            luojia,
            bianhua,
            yaojin
        );
    }
    fn before_render<'a>(
        &'a mut self,
        _ctx: &mut Context,
        render_context: &'a mut dyn RenderContext,
    ) {
        let map = self.update_map();
        if let Some(map) = map {
            tracing::info!("update map: {} at {:?}", map.key, (self.game.x, self.game.y));

            // Load on demand and keep only the current map resident.
            if !self.map_images.contains_key(map.key.as_str()) {
                self.map_images.clear();
                if let Some(img) = image_with_file(map.key.as_str()) {
                    self.map_images.insert(map.key.clone(), img);
                }
            }

            if let Some(map_image) = self.map_images.get(map.key.as_str()) {
                if let Err(e) = render_context.replace_texture(
                    self.textures.map.id.unwrap(),
                    map_image.as_bytes(),
                    map_image.width(),
                    map_image.height(),
                ) {
                    tracing::error!("replace_texture failed for {}: {e:?}", map.key);
                }
            }
            self.map = Some(map);
        }
    }
    fn render(&mut self, ui: &mut imgui::Ui) {
        self.render(ui);
        // ui.show_demo_window(&mut true);
    }
}
