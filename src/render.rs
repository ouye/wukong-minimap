// This file was modified in a fork of jaskang/wukong-minimap.
//
// Upstream: https://github.com/jaskang/wukong-minimap (Apache-2.0)
// Fork:     https://github.com/Ouye/wukong-minimap
//
// Changes: maps are loaded on demand instead of all at once; the nomap
// placeholder is resized to match the map textures; replace_texture
// failures are logged; added the heading-up minimap mode (Shift+0), the
// walked trail (9 / Shift+9), the nearby-target radar (7 / 8 / Shift+8),
// persisted settings, on-screen messages in Chinese where a system font
// provides it, and a fork credit line beside upstream's logo on the big map.
//
// Upstream's own logo, baked into includes/mainwraper.png, is left exactly
// as it is -- that asset is byte-identical to upstream.

use std::{collections::HashMap, path::PathBuf, sync::Mutex, time::Instant};

use crate::{
    config::Config,
    font,
    maploader::MapLoader,
    trail::{clip_polyline_to_circle, Trail},
    utils::{
        get_dll_dir, image_with_bytes, is_in_map, load_data, load_points, MapInfo,
        Point, Pos2,
    },
    wukong::{self, ActorDot, GameState},
};
use gilrs::{GamepadId, Gilrs};
use hudhook::{
    imgui::{self, Condition, Context, FontConfig, FontGlyphRanges, FontSource, WindowFlags},
    ImguiRenderLoop, RenderContext,
};
use hudhook::{
    imgui::{Image, Key},
    tracing,
};
use image::{EncodableLayout, ImageFormat, RgbaImage};

/// 某个区域没有点位时用它，省掉每帧 `unwrap_or(&vec![])` 那次空分配。
const NO_POINTS: &[Point] = &[];

// 分支署名，画在大地图左下角、上游 logo 的右侧。
//
// mainwraper.png 整张被拉伸到边长 window_size 的正方形里，所以图中固定的
// 像素坐标对应屏幕上固定的比例，与分辨率、宽高比无关。上游 logo 在 4000x4000
// 原图里占 x 150..640、y 3695..3830，即 x 0.0375..0.1600、垂直中心 0.9407。
//
// 只能用 ASCII，内置字体没有中文字形。
const FORK_CREDIT: &str = "1.0.20+ patch by Ouye@github";
/// 文字左边缘，在 logo 右边缘 0.160 之后留出底色的间距
const CREDIT_X: f32 = 0.183;
/// 文字垂直中心，与 logo 对齐
const CREDIT_Y: f32 = 0.9407;
/// 文字高度占 window_size 的比例，约为 logo 高度 0.0338 的六成
const CREDIT_PX: f32 = 0.018;
/// 字体图集的光栅化尺寸。内置 ProggyClean 只有 13px，4K 下需要放大近三倍；
/// 按 32px 烘焙、绘制时再缩小。
const FONT_ATLAS_PX: f32 = 32.0;

// ------------------------------------------------------------------ 路径 ---

/// 线宽占 window_size 的比例，下限保证低分辨率下不至于细到看不见。
const TRAIL_WIDTH: f32 = 0.0035;
/// 雷达刷新间隔。每帧扫一遍 actor 列表没必要，人也不会移动那么快。
const RADAR_INTERVAL: f32 = 0.2;
/// 记号半径占 window_size 的比例。
const DOT_RADIUS: f32 = 0.0095;
/// 高度分带的阈值（世界单位，1 单位约 1cm）。悟空的地图垂直层次很多，
/// 4 米上下基本就是"另一层"了。
const Z_BAND: f32 = 400.0;
/// 战斗中敌人呼吸一次的周期，秒。
const ALERT_PERIOD: f32 = 0.9;
/// 呼吸的半径区间，相对静止时的点半径。
/// 平均比静止的点大一圈，本身就更醒目。
const ALERT_SCALE_MIN: f32 = 0.7;
const ALERT_SCALE_MAX: f32 = 1.7;

/// 判定一个运行时发现的土地庙是否已经在静态点位表里：两个方向都在这个距离
/// 之内就算同一个。土地庙之间隔得远，放宽一点不会误判。
const MEDITATION_MATCH: f32 = 2000.0;
/// 雷达的垂直搜索上限，世界单位。竖直塔（浮屠界）在俯视投影上是个细圆筒，
/// 只按水平距离筛的话每一层的怪都会落进范围。差这么多层的目标本来也没用。
const RADAR_Z_LIMIT: f32 = 4000.0;

/// 一帧 16.7ms。任何一段超过这个毫秒数就记一条日志 —— 卡顿要靠数据定位，
/// 不能靠猜。
const SLOW_MS: f32 = 5.0;

/// 雷达搜索半径相对小地图可视半径的余量，边缘目标提前进来一点，不会突然冒出。
const RADAR_MARGIN: f32 = 1.15;

/// 提示条停留时长。
const TOAST_SECS: f32 = 3.0;
/// `Shift`+`9` 二次确认的时限。
const CLEAR_CONFIRM_SECS: f32 = 3.0;

/// 配置落盘的节流间隔。连按 `+`/`-` 时不必每一下都写盘。
const CONFIG_SAVE_DELAY: f32 = 2.0;

// 提示条文案。找得到系统中文字体就用中文，否则回退到英文（内置字体只有 ASCII）。
// 这里列出的中文字符决定了要烘焙进字体图集的字形集合，见 `font::glyph_ranges_for`。
const MSG_TRAIL_ON: (&str, &str) = ("路线记录：开", "Trail ON");
const MSG_TRAIL_OFF: (&str, &str) = ("路线记录：关", "Trail OFF");
const MSG_TRAIL_CONFIRM: (&str, &str) = (
    "再按一次 Shift+9 清除全部路线",
    "Press Shift+9 again to clear the trail",
);
const MSG_TRAIL_CLEARED: (&str, &str) = ("路线已清除", "Trail cleared");
const MSG_HEADING_UP: (&str, &str) = ("地图朝向：跟随人物", "Map: heading up");
const MSG_NORTH_UP: (&str, &str) = ("地图朝向：正北朝上", "Map: north up");
const MSG_POINTS: (&str, &str) = ("个点", "points");
const MSG_ENEMY_ON: (&str, &str) = ("敌人红点：开", "Enemy dots ON");
const MSG_ENEMY_OFF: (&str, &str) = ("敌人红点：关", "Enemy dots OFF");
const MSG_NEUTRAL_ON: (&str, &str) = ("其他灰点：开", "Neutral dots ON");
const MSG_NEUTRAL_OFF: (&str, &str) = ("其他灰点：关", "Neutral dots OFF");
const MSG_ITEMS_ON: (&str, &str) = ("物品显示：开", "Items ON");
const MSG_ITEMS_OFF: (&str, &str) = ("物品显示：关", "Items OFF");

/// 大地图右侧那一列按键说明。左键名是 ASCII，右边的说明跟着字体走。
const HELP_TITLE: (&str, &str) = ("按键说明", "Controls");
const HELP_ROWS: &[(&str, (&str, &str))] = &[
    ("Tab", ("大地图", "Big map")),
    ("0", ("显示 / 隐藏小地图", "Show / hide")),
    ("+ / -", ("小地图窗口大小", "Window size")),
    ("Shift  +/-", ("地图缩放比例", "Map scale")),
    ("Shift  0", ("地图朝向模式", "Map orientation")),
    ("9", ("路线记录开关", "Trail on / off")),
    ("Shift  9", ("清除全部路线", "Clear the trail")),
    ("8", ("敌人红点开关", "Enemy dots")),
    ("Shift  8", ("其他灰点开关", "Neutral dots")),
    ("7", ("掉落物 / 采集物", "Items on the ground")),
];

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
    /// 底图的后台解码器。换图不再阻塞渲染线程。
    map_loader: MapLoader,
    zoom: f32,
    size: f32,
    /// 玩家当前实际所在的区域。每帧判定，立刻跟进 —— 路线记录和雷达半径
    /// 都要用它。
    map: Option<MapInfo>,
    /// 屏幕上正在显示的那张图。底图解码好之前一直是上一张：显示的地形和
    /// UV 换算始终对得上，只是慢一步，比整个游戏卡住一两秒强得多。
    shown: Option<MapInfo>,
    maps: Vec<MapInfo>,
    points: HashMap<String, Vec<Point>>,
    game: GameState,
    is_show_main: bool,
    is_show: bool,
    /// Heading-up mode: the arrow stays pointing up and the map turns under it.
    /// Off (the default) is north-up: the map is fixed and the arrow turns.
    rotate_map: bool,
    /// 走过的路线。始终常驻，由 `trail_enabled` 决定记不记、画不画。
    trail: Trail,
    trail_enabled: bool,
    /// `Shift`+`9` 按下的时刻。清除是不可逆的，所以要在时限内按第二次。
    trail_clear_armed: Option<Instant>,
    /// 屏幕提示。
    toast: Option<(String, Instant)>,
    /// 字体图集里有中文字形。没有的话提示条回退成英文。
    cjk: bool,
    /// 路线颜色。启动时从配置解析一次，`config.trail_color` 是它的来源。
    trail_color: [f32; 4],
    /// 小地图上的红点/灰点。
    show_enemies: bool,
    show_neutrals: bool,
    show_items: bool,
    enemy_color: [f32; 4],
    alert_color: [f32; 4],
    neutral_color: [f32; 4],
    drop_color: [f32; 4],
    collect_color: [f32; 4],
    interact_color: [f32; 4],
    /// 上次刷新拿到的目标，按 RADAR_INTERVAL 节流刷新。
    actors: Vec<ActorDot>,
    actors_at: Instant,
    /// 上次落盘的设置，用来判断有没有改动。
    config: Config,
    config_path: PathBuf,
    config_saved_at: Instant,
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


        let dll_dir = get_dll_dir();
        let config = Config::load(&dll_dir);
        // 把补全过的配置写回去，文件里就总是带着全部字段和它们当前的值，
        // 用户打开就知道有哪些能改。
        config.save(&Config::path(&dll_dir));

        let gilrs = Gilrs::new().unwrap();
        Self {
            gilrs: Mutex::new(gilrs),
            current_gamepad: None,
            textures,
            // 底图按需解码，不是启动时全部读进来。整套 23 张在 2000×2000
            // RGBA 下是 368 MB，而玩家同时只看得到一张。
            map_loader: MapLoader::new(),
            zoom: config.zoom,
            size: config.size,
            map: None,
            shown: None,
            maps,
            points,
            game: wukong::game_state(),
            is_show_main: false,
            is_show: true,
            rotate_map: config.rotate_map,
            trail: Trail::load(&dll_dir),
            trail_enabled: config.trail_enabled,
            trail_clear_armed: None,
            toast: None,
            cjk: false,
            trail_color: config.trail_color_rgba(),
            show_enemies: config.show_enemies,
            show_neutrals: config.show_neutrals,
            show_items: config.show_items,
            enemy_color: config.enemy_color_rgba(),
            alert_color: config.alert_color_rgba(),
            neutral_color: config.neutral_color_rgba(),
            drop_color: config.drop_color_rgba(),
            collect_color: config.collect_color_rgba(),
            interact_color: config.interact_color_rgba(),
            actors: Vec::new(),
            actors_at: Instant::now(),
            config_path: Config::path(&dll_dir),
            config,
            config_saved_at: Instant::now(),
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
            });

        // 这里每帧都跑。MapInfo 里有三个 String 和一个 Vec，原来无条件
        // `.cloned()` 等于每帧白扔四次分配 —— 只有真的换图时才需要那份拷贝。
        match (self.map.as_ref(), map) {
            (_, None) => None,
            (None, Some(new_map)) => Some(new_map.clone()),
            (Some(current_map), Some(new_map)) => {
                if current_map.key != new_map.key {
                    Some(new_map.clone())
                } else {
                    None
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

                if let Some(map) = self.shown.as_ref() {
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

                    // 已走路线，画在底图之上、图标之下。大地图是方的，交给
                    // imgui 的裁剪矩形即可。
                    if self.trail_enabled {
                        let thickness = (window_size * TRAIL_WIDTH).max(1.5);
                        draw_list.with_clip_rect(
                            [map_offset_x, map_offset_y],
                            [map_offset_x + map_size, map_offset_y + map_size],
                            || {
                                for seg in self.trail.segments(map.key.as_str()) {
                                    let pts: Vec<[f32; 2]> = seg
                                        .iter()
                                        .map(|p| {
                                            let o = self.get_icon_offset(
                                                Pos2::new(p[0], p[1]),
                                                [map_offset_x, map_offset_y],
                                                &map_view,
                                            );
                                            [o.x, o.y]
                                        })
                                        .collect();
                                    if pts.len() >= 2 {
                                        draw_list
                                            .add_polyline(pts, self.trail_color)
                                            .thickness(thickness)
                                            .build();
                                    }
                                }
                            },
                        );
                    }

                    // 绘制地图图标
                    self.points
                        .get(map.level.as_str())
                        .map(Vec::as_slice)
                        .unwrap_or(NO_POINTS)
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

                    // 分支署名，坐标见文件顶部的 CREDIT_* 常量
                    let credit_px = window_size * CREDIT_PX;
                    ui.set_window_font_scale(credit_px / FONT_ATLAS_PX);
                    let text_size = ui.calc_text_size(FORK_CREDIT);
                    let text_pos = [
                        window_offset_x + window_size * CREDIT_X,
                        window_offset_y + window_size * CREDIT_Y - text_size[1] / 2.0,
                    ];
                    let pad_x = credit_px * 0.55;
                    let pad_y = credit_px * 0.28;
                    draw_list
                        .add_rect(
                            [text_pos[0] - pad_x, text_pos[1] - pad_y],
                            [
                                text_pos[0] + text_size[0] + pad_x,
                                text_pos[1] + text_size[1] + pad_y,
                            ],
                            [0.0, 0.0, 0.0, 0.55],
                        )
                        .filled(true)
                        .rounding((text_size[1] + pad_y * 2.0) / 2.0)
                        .build();
                    draw_list.add_text(text_pos, [1.0, 1.0, 1.0, 0.85], FORK_CREDIT);
                    ui.set_window_font_scale(1.0);

                    ui.set_cursor_pos([40.0, map_size - 40.0]);
                    // ui.text(format!("{:?}", self.game));
                } else {
                    tracing::debug!("draw_nomap");
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

                if let Some(map) = self.shown.as_ref() {
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

                    // Quantities the heading-up path needs. `scale_px` is
                    // pixels per world unit; `rot_*` rotate between screen and
                    // world space.
                    let center_px = Pos2::new(
                        map_offset_x + map_size / 2.0,
                        map_offset_y + map_size / 2.0,
                    );
                    let player = Pos2::new(self.game.x, self.game.y);
                    let [x_start, y_start, _] = map.range.start;
                    let [x_end, y_end, _] = map.range.end;
                    let full_w = (x_end - x_start).abs();
                    let full_h = (y_end - y_start).abs();
                    let scale_px = map_size / (full_w * self.zoom);
                    let (rot_sin, rot_cos) = self.game.angle.to_radians().sin_cos();

                    if self.rotate_map {
                        // Heading-up. The screen quad stays put and the sampled
                        // region turns instead, so the minimap keeps its round
                        // shape: a triangle fan out of degenerate image quads,
                        // with each vertex's uv computed individually.
                        //
                        // A screen offset d maps to the world offset
                        // R(angle) * d / scale, which is the inverse of the
                        // rotation applied to the arrow in north-up mode.
                        //
                        // uvs are clamped because hudhook's sampler is set to
                        // WRAP -- without this the far edge of the map bleeds in
                        // once the player gets near a border.
                        let to_uv = |dx: f32, dy: f32| {
                            let wx = (dx * rot_cos - dy * rot_sin) / scale_px;
                            let wy = (dx * rot_sin + dy * rot_cos) / scale_px;
                            [
                                ((player.x + wx - x_start) / full_w).clamp(0.0, 1.0),
                                ((player.y + wy - y_start) / full_h).clamp(0.0, 1.0),
                            ]
                        };

                        const SEGMENTS: usize = 72;
                        let radius = map_size / 2.0;
                        let uv_center = to_uv(0.0, 0.0);
                        let step = std::f32::consts::TAU / SEGMENTS as f32;
                        for i in 0..SEGMENTS {
                            let (s0, c0) = (i as f32 * step).sin_cos();
                            let (s1, c1) = ((i + 1) as f32 * step).sin_cos();
                            let (d0x, d0y) = (c0 * radius, s0 * radius);
                            let (d1x, d1y) = (c1 * radius, s1 * radius);
                            let a = [center_px.x + d0x, center_px.y + d0y];
                            let b = [center_px.x + d1x, center_px.y + d1y];
                            let uv_a = to_uv(d0x, d0y);
                            let uv_b = to_uv(d1x, d1y);
                            draw_list
                                .add_image_quad(
                                    map_image,
                                    [center_px.x, center_px.y],
                                    a,
                                    b,
                                    b,
                                )
                                .uv(uv_center, uv_a, uv_b, uv_b)
                                .build();
                        }
                    } else {
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
                    }

                    // 世界坐标 -> 屏幕坐标。路线和周边目标共用同一套变换。
                    let radius = map_size / 2.0;
                    let to_screen = |p: &[f32; 2]| -> [f32; 2] {
                        if self.rotate_map {
                            let wx = (p[0] - player.x) * scale_px;
                            let wy = (p[1] - player.y) * scale_px;
                            [
                                center_px.x + wx * rot_cos + wy * rot_sin,
                                center_px.y - wx * rot_sin + wy * rot_cos,
                            ]
                        } else {
                            let o = self.get_icon_offset(
                                Pos2::new(p[0], p[1]),
                                [map_offset_x, map_offset_y],
                                &map_view,
                            );
                            [o.x, o.y]
                        }
                    };

                    // 已走路线。小地图是圆的，折线得自己按圆裁剪。
                    if self.trail_enabled {
                        let thickness = (window_size * TRAIL_WIDTH).max(1.5);

                        // 先在世界坐标里粗筛。一张图上的路线点长时间游玩后能有
                        // 上万个，而小地图一次只看得到很小一块 —— 整段都在视野
                        // 外就直接跳过，省掉逐点变换和那次 Vec 分配。判定只有
                        // 两次减法两次乘法，比变换本身便宜得多。
                        let view_r = radius / scale_px * 1.2;
                        let view_r2 = view_r * view_r;
                        let near = |p: &[f32; 2]| {
                            let dx = p[0] - player.x;
                            let dy = p[1] - player.y;
                            dx * dx + dy * dy <= view_r2
                        };

                        for seg in self.trail.segments(map.key.as_str()) {
                            if !seg.iter().any(|p| near(p)) {
                                continue;
                            }
                            let pts: Vec<[f32; 2]> = seg.iter().map(|p| to_screen(p)).collect();
                            let runs = clip_polyline_to_circle(
                                &pts,
                                [center_px.x, center_px.y],
                                radius,
                            );
                            for run in runs {
                                draw_list
                                    .add_polyline(run, self.trail_color)
                                    .thickness(thickness)
                                    .build();
                            }
                        }
                    }

                    // 周边目标。画在路线之上、点位图标之下 —— 图标是静态信息，
                    // 这些是当下的，不该被盖住，但也不该盖掉传送点。
                    //
                    // 两条互不干扰的视觉通道：
                    //   形状 = 是什么   圆 = 角色，菱形 = 地上的东西
                    //   填充 = 高度差   空心 = 在你下方，实心 = 同层，
                    //                   实心加外环 = 在你上方
                    // 颜色再区分具体类别，以及敌人有没有发现你。视觉重量随高度
                    // 递增，一眼能看出够不够得着。
                    if !self.actors.is_empty() {
                        let dot_r = (window_size * DOT_RADIUS).max(1.8);
                        // 呼吸到最大、又在上方一层的话，外环会到 2 倍半径，
                        // 裁剪半径要留够，否则贴边的目标被切掉半圈。
                        let limit = radius - dot_r * ALERT_SCALE_MAX * 2.0;
                        let player_z = self.game.z;

                        // 已经发现你的敌人：点自己缩小再弹回，循环。
                        //
                        // 颜色差在这个尺寸上读不出来 —— 那是个绝对差异，没有
                        // 参照就分辨不了，而两种状态的红点几乎不会同时出现。
                        // 运动不一样：小地图是用余光看的，而余光对颜色和形状
                        // 迟钝、对变化极其敏感。
                        //
                        // 只动半径、不动填充：空心/实心已经用来表示高度了，
                        // 让它跟着呼吸的话，战斗中的怪会有一半时间看起来像在
                        // 你楼下 —— 而这些恰恰是最需要知道同不同层的那些。
                        //
                        // 所有战斗中的敌人共用一个相位，同步呼吸是一个整体
                        // 信号，比各自随机闪更容易被捕捉到。
                        let phase = (ui.time() as f32) / ALERT_PERIOD * std::f32::consts::TAU;
                        let alert_r = dot_r
                            * (ALERT_SCALE_MIN
                                + (ALERT_SCALE_MAX - ALERT_SCALE_MIN) * (0.5 + 0.5 * phase.sin()));

                        let draw_marker = |p: [f32; 2], r: f32, color: [f32; 4], dz: f32, diamond: bool| {
                            let below = dz < -Z_BAND;
                            let above = dz > Z_BAND;
                            let edge = (r * 0.6).max(1.0);

                            if diamond {
                                let d = r * 1.3;
                                let mut pts = vec![
                                    [p[0], p[1] - d],
                                    [p[0] + d, p[1]],
                                    [p[0], p[1] + d],
                                    [p[0] - d, p[1]],
                                ];
                                if below {
                                    // 描边要自己闭合，add_polyline 不填充时是开口的。
                                    pts.push(pts[0]);
                                    draw_list.add_polyline(pts, color).thickness(edge).build();
                                } else {
                                    draw_list.add_polyline(pts, color).filled(true).build();
                                }
                            } else if below {
                                draw_list
                                    .add_circle(p, r, color)
                                    .thickness(edge)
                                    .num_segments(12)
                                    .build();
                            } else {
                                draw_list
                                    .add_circle(p, r, color)
                                    .filled(true)
                                    .num_segments(12)
                                    .build();
                            }

                            if above {
                                draw_list
                                    .add_circle(p, r * 2.0, color)
                                    .thickness((r * 0.4).max(0.9))
                                    .num_segments(16)
                                    .build();
                            }
                        };

                        for actor in &self.actors {
                            let p = to_screen(&[actor.x, actor.y]);
                            let dx = p[0] - center_px.x;
                            let dy = p[1] - center_px.y;
                            if dx * dx + dy * dy > limit * limit {
                                continue;
                            }
                            let dz = actor.z - player_z;

                            match actor.kind {
                                wukong::KIND_HOSTILE => {
                                    if actor.in_battle() {
                                        draw_marker(p, alert_r, self.alert_color, dz, false);
                                    } else {
                                        draw_marker(p, dot_r, self.enemy_color, dz, false);
                                    }
                                }
                                wukong::KIND_NEUTRAL => {
                                    draw_marker(p, dot_r, self.neutral_color, dz, false)
                                }
                                wukong::KIND_DROP => {
                                    draw_marker(p, dot_r, self.drop_color, dz, true)
                                }
                                wukong::KIND_COLLECT => {
                                    draw_marker(p, dot_r, self.collect_color, dz, true)
                                }
                                wukong::KIND_MEDITATION => {
                                    // 静态点位表里已经有的土地庙不重复画；漏掉的
                                    // 用同一个传送点图标补上，看起来才是一套的。
                                    if !self.has_static_teleport(map, actor.x, actor.y) {
                                        if let Some(id) = self.textures.teleport.id {
                                            let half = icon_size_half * 0.85;
                                            draw_list
                                                .add_image(
                                                    id,
                                                    [p[0] - half, p[1] - half],
                                                    [p[0] + half, p[1] + half],
                                                )
                                                .build();
                                        }
                                    }
                                }
                                _ => draw_marker(p, dot_r, self.interact_color, dz, true),
                            }
                        }
                    }

                    // 绘制地图图标
                    self.points
                        .get(map.level.as_str())
                        .map(Vec::as_slice)
                        .unwrap_or(NO_POINTS)
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

                                let icon_offset = if self.rotate_map {
                                    // World offset -> R(-angle) -> screen.
                                    let wx = (point.x - player.x) * scale_px;
                                    let wy = (point.y - player.y) * scale_px;
                                    Pos2::new(
                                        center_px.x + wx * rot_cos + wy * rot_sin,
                                        center_px.y - wx * rot_sin + wy * rot_cos,
                                    )
                                } else {
                                    self.get_icon_offset(
                                        Pos2::new(point.x, point.y),
                                        [map_offset_x, map_offset_y],
                                        &map_view,
                                    )
                                };
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

                    // 绘制玩家角色箭头（旋转模式下箭头锁定，由地图转）
                    let arrow_angle = if self.rotate_map { 0.0 } else { self.game.angle };
                    let [p0, p1, p2, p3] =
                        self.p4_with_angle(center, arrow_angle, icon_size * 1.5);
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
                    tracing::debug!("draw_nomap");
                }
            });
    }
    fn render(&mut self, ui: &imgui::Ui) {
        if ui.is_key_pressed_no_repeat(Key::Minus) && !ui.is_key_down(Key::LeftShift) {
            self.size = (self.size - 0.05).max(0.15);
            tracing::debug!("size: {}", self.size);
        }
        if ui.is_key_pressed_no_repeat(Key::Equal) && !ui.is_key_down(Key::LeftShift) {
            self.size = (self.size + 0.05).min(0.5);
            tracing::debug!("size: {}", self.size);
        }
        if ui.is_key_pressed_no_repeat(Key::Minus) && ui.is_key_down(Key::LeftShift) {
            self.zoom = (self.zoom - 0.05).max(0.15);
            tracing::debug!("zoom: {}", self.zoom);
        }
        if ui.is_key_pressed_no_repeat(Key::Equal) && ui.is_key_down(Key::LeftShift) {
            self.zoom = (self.zoom + 0.05).min(0.5);
            tracing::debug!("zoom: {}", self.zoom);
        }
        if ui.is_key_pressed_no_repeat(Key::Alpha0) {
            if ui.is_key_down(Key::LeftShift) {
                self.rotate_map = !self.rotate_map;
                tracing::debug!("rotate_map: {}", self.rotate_map);
                let msg = if self.rotate_map {
                    self.msg(MSG_HEADING_UP)
                } else {
                    self.msg(MSG_NORTH_UP)
                };
                self.toast = Some((String::from(msg), Instant::now()));
            } else {
                self.is_show = !self.is_show;
            }
        }
        if ui.is_key_pressed_no_repeat(Key::Alpha7) {
            self.show_items = !self.show_items;
            tracing::info!("radar: items={}", self.show_items);
            let msg = if self.show_items {
                MSG_ITEMS_ON
            } else {
                MSG_ITEMS_OFF
            };
            self.toast = Some((String::from(self.msg(msg)), Instant::now()));
        }
        if ui.is_key_pressed_no_repeat(Key::Alpha8) {
            let msg = if ui.is_key_down(Key::LeftShift) {
                self.show_neutrals = !self.show_neutrals;
                if self.show_neutrals {
                    MSG_NEUTRAL_ON
                } else {
                    MSG_NEUTRAL_OFF
                }
            } else {
                self.show_enemies = !self.show_enemies;
                if self.show_enemies {
                    MSG_ENEMY_ON
                } else {
                    MSG_ENEMY_OFF
                }
            };
            tracing::info!(
                "radar: enemies={} neutrals={}",
                self.show_enemies,
                self.show_neutrals
            );
            self.toast = Some((String::from(self.msg(msg)), Instant::now()));
        }
        if ui.is_key_pressed_no_repeat(Key::Alpha9) {
            if ui.is_key_down(Key::LeftShift) {
                // 清除不可逆，要求在时限内按第二次。
                let armed = self
                    .trail_clear_armed
                    .map(|t| t.elapsed().as_secs_f32() <= CLEAR_CONFIRM_SECS)
                    .unwrap_or(false);
                if armed {
                    self.trail_clear_armed = None;
                    self.trail.clear();
                    self.toast = Some((String::from(self.msg(MSG_TRAIL_CLEARED)), Instant::now()));
                } else {
                    self.trail_clear_armed = Some(Instant::now());
                    self.toast =
                        Some((String::from(self.msg(MSG_TRAIL_CONFIRM)), Instant::now()));
                }
            } else {
                self.trail_enabled = !self.trail_enabled;
                // 暂停期间人是会走动的，但那段没被记录。不断开的话，恢复记录后
                // 的第一个点会直接连到暂停前的最后一个点，画出一条并没走过的直线。
                self.trail.cut();
                if !self.trail_enabled {
                    self.trail.save();
                }
                tracing::info!("trail_enabled: {}", self.trail_enabled);
                let msg = if self.trail_enabled {
                    format!(
                        "{}  ({} {})",
                        self.msg(MSG_TRAIL_ON),
                        self.trail.total_points(),
                        self.msg(MSG_POINTS)
                    )
                } else {
                    String::from(self.msg(MSG_TRAIL_OFF))
                };
                self.toast = Some((msg, Instant::now()));
            }
        }
        if ui.is_key_pressed_no_repeat(Key::Tab) {
            self.is_show_main = !self.is_show_main;
            // wukong::toggle_mouse_cursor(self.is_show_main);
        }
        if let Ok(mut gilrs) = self.gilrs.lock() {
            // Examine new events
            while let Some(gilrs::Event { id, event, .. }) = gilrs.next_event() {
                self.current_gamepad = Some(id);
                tracing::debug!("gilrs event from {}: {:?}", id, event);
                if let gilrs::EventType::ButtonPressed(button, code) = event {
                    let gamepad = gilrs.gamepad(id);
                    if gamepad.is_pressed(gilrs::Button::RightTrigger) {
                        match button {
                            gilrs::Button::DPadDown => {
                                self.is_show_main = !self.is_show_main;
                                // wukong::toggle_mouse_cursor(self.is_show_main);
                                tracing::debug!("gamepad: toggle main map");
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
                self.render_help(ui);
            }
            if self.is_show {
                self.render_minimap(ui);
            }
            self.render_toast(ui);
        }
    }

    /// 这个坐标附近是否已经有一个静态传送点。
    ///
    /// 运行时能发现所有土地庙，但 `data_points.json` 里手工采的那些已经画出来了，
    /// 重复画两个图标反而更乱。只补表里漏掉的。
    fn has_static_teleport(&self, map: &MapInfo, x: f32, y: f32) -> bool {
        self.points
            .get(map.level.as_str())
            .map(|points| {
                points.iter().any(|point| {
                    point.category == "teleport"
                        && (point.x - x).abs() < MEDITATION_MATCH
                        && (point.y - y).abs() < MEDITATION_MATCH
                })
            })
            .unwrap_or(false)
    }

    /// 按 RADAR_INTERVAL 节流刷新周边目标。
    ///
    /// 搜索半径跟着小地图当前的可视范围走：比例调小了就少扫一些，没必要把
    /// 半张图的 actor 都过一遍。
    fn refresh_radar(&mut self) {
        let mask = self.radar_mask();
        if mask == 0 {
            self.actors.clear();
            return;
        }
        if !self.game.playing {
            return;
        }
        if self.actors_at.elapsed().as_secs_f32() < RADAR_INTERVAL {
            return;
        }
        self.actors_at = Instant::now();

        let radius = match self.map.as_ref() {
            Some(map) => {
                let full_w = (map.range.end[0] - map.range.start[0]).abs();
                full_w * self.zoom / 2.0 * RADAR_MARGIN
            }
            None => return,
        };
        wukong::nearby_actors(radius, RADAR_Z_LIMIT, mask, &mut self.actors);

        // IsUnitInBattle 在自动化测试的辅助库里，正式版未必留着。这条能直接
        // 看出它到底有没有生效：一直是 0 就说明拿不到战斗状态。
        let in_battle = self.actors.iter().filter(|a| a.in_battle()).count();
        tracing::debug!(
            "radar: {} dots, {} in battle",
            self.actors.len(),
            in_battle
        );
    }

    /// 要收集哪些类别。关掉的类别在 C++ 侧连一次 `IsA` 都不会做。
    fn radar_mask(&self) -> u32 {
        let mut mask = 0u32;
        if self.show_enemies {
            mask |= 1 << wukong::KIND_HOSTILE;
        }
        if self.show_neutrals {
            mask |= 1 << wukong::KIND_NEUTRAL;
        }
        if self.show_items {
            mask |= (1 << wukong::KIND_DROP)
                | (1 << wukong::KIND_COLLECT)
                | (1 << wukong::KIND_MEDITATION)
                | (1 << wukong::KIND_INTERACT);
        }
        mask
    }

    /// 把按键改动过的设置写回配置文件。
    ///
    /// 比较的是「上次落盘的值」而不是设一个 dirty 标志，这样按 `+` 又按 `-`
    /// 回到原样就不会白写一次。节流是为了连按时不要每帧都落盘。
    fn sync_config(&mut self) {
        let current = Config {
            size: self.size,
            zoom: self.zoom,
            rotate_map: self.rotate_map,
            trail_enabled: self.trail_enabled,
            show_enemies: self.show_enemies,
            show_neutrals: self.show_neutrals,
            show_items: self.show_items,
            // 颜色只读不写，原样带过去，否则这里会把用户手改的值覆盖掉。
            trail_color: self.config.trail_color.clone(),
            enemy_color: self.config.enemy_color.clone(),
            alert_color: self.config.alert_color.clone(),
            neutral_color: self.config.neutral_color.clone(),
            drop_color: self.config.drop_color.clone(),
            collect_color: self.config.collect_color.clone(),
            interact_color: self.config.interact_color.clone(),
        };
        if current == self.config {
            return;
        }
        if self.config_saved_at.elapsed().as_secs_f32() < CONFIG_SAVE_DELAY {
            return;
        }
        self.config = current;
        self.config_saved_at = Instant::now();
        self.config.save(&self.config_path);
    }

    /// 有中文字形就用中文，否则用英文。
    fn msg(&self, pair: (&'static str, &'static str)) -> &'static str {
        if self.cjk {
            pair.0
        } else {
            pair.1
        }
    }

    /// 大地图右侧的按键说明。大地图左边是上游原有的图例，右边这一列是本分支加的。
    ///
    /// 和提示条一样用一个铺满屏幕的窗口来画：imgui 会把绘制裁剪到窗口矩形，
    /// 窗口按内容大小去开的话，量错一点点文字就被切掉了。
    fn render_help(&self, ui: &imgui::Ui) {
        let [screen_width, screen_height] = ui.io().display_size;
        let short_side = screen_width.min(screen_height);
        let px = (short_side * 0.017).max(12.0);
        let line_h = px * 1.6;
        let pad = px * 0.9;
        let gap = px * 1.2;

        ui.window("wukong-minimap-help")
            .size([screen_width, screen_height], Condition::Always)
            .position([0.0, 0.0], Condition::Always)
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_INPUTS
                    | WindowFlags::NO_NAV
                    | WindowFlags::NO_BACKGROUND,
            )
            .build(|| {
                let draw_list = ui.get_window_draw_list();
                ui.set_window_font_scale(px / FONT_ATLAS_PX);

                let title = self.msg(HELP_TITLE);
                let rows: Vec<(&str, &str)> = HELP_ROWS
                    .iter()
                    .map(|(key, desc)| (*key, self.msg(*desc)))
                    .collect();

                // 键名一列按最宽的对齐，说明从同一个 x 开始。
                let key_w = rows
                    .iter()
                    .map(|(k, _)| ui.calc_text_size(*k)[0])
                    .fold(0.0f32, f32::max);
                let desc_w = rows
                    .iter()
                    .map(|(_, d)| ui.calc_text_size(*d)[0])
                    .fold(0.0f32, f32::max);
                let content_w = (key_w + gap + desc_w).max(ui.calc_text_size(title)[0]);
                let panel_w = content_w + pad * 2.0;
                let panel_h = pad * 2.0 + line_h * (rows.len() as f32 + 1.4);

                // 贴屏幕右缘，和小地图同一条边距；小地图开着的时候让到它下面，
                // 关着就竖直居中。
                let x = (screen_width - panel_w - 10.0).max(8.0);
                let y = if self.is_show {
                    10.0 + short_side * self.size + short_side * 0.055
                } else {
                    (screen_height - panel_h) / 2.0
                };

                draw_list
                    .add_rect([x, y], [x + panel_w, y + panel_h], [0.0, 0.0, 0.0, 0.55])
                    .filled(true)
                    .rounding(px * 0.5)
                    .build();

                let text_x = x + pad;
                draw_list.add_text([text_x, y + pad], [1.0, 0.86, 0.55, 0.95], title);

                let mut row_y = y + pad + line_h * 1.4;
                for (key, desc) in &rows {
                    draw_list.add_text([text_x, row_y], [1.0, 1.0, 1.0, 0.7], *key);
                    draw_list.add_text(
                        [text_x + key_w + gap, row_y],
                        [1.0, 1.0, 1.0, 0.95],
                        *desc,
                    );
                    row_y += line_h;
                }

                ui.set_window_font_scale(1.0);
            });
    }

    /// 屏幕提示，画在小地图正下方。
    ///
    /// 窗口占满屏幕宽度而不是只有小地图那么宽：imgui 会把绘制裁剪到窗口矩形，
    /// 窗口一窄，长一点的文案就被切掉了。药丸本身仍以小地图中心对齐，只在快要
    /// 超出屏幕时才让开。
    fn render_toast(&self, ui: &imgui::Ui) {
        let Some((msg, at)) = self.toast.as_ref() else {
            return;
        };
        if at.elapsed().as_secs_f32() > TOAST_SECS {
            return;
        }

        let [screen_width, screen_height] = ui.io().display_size;
        let short_side = screen_width.min(screen_height);
        let minimap_w = short_side * self.size;
        let minimap_center_x = screen_width - minimap_w / 2.0 - 10.0;
        let box_y = 10.0 + minimap_w + 6.0;
        let px = (short_side * 0.016).max(12.0);

        ui.window("wukong-minimap-toast")
            .size([screen_width, px * 2.5], Condition::Always)
            .position([0.0, box_y], Condition::Always)
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_INPUTS
                    | WindowFlags::NO_NAV
                    | WindowFlags::NO_BACKGROUND,
            )
            .build(|| {
                let draw_list = ui.get_window_draw_list();
                ui.set_window_font_scale(px / FONT_ATLAS_PX);
                let size = ui.calc_text_size(msg.as_str());
                let pad = px * 0.5;
                let pill_w = size[0] + pad * 2.0;
                let max_x = (screen_width - pill_w - 8.0).max(8.0);
                let x = (minimap_center_x - pill_w / 2.0 + pad).clamp(8.0 + pad, max_x + pad);
                let pos = [x, box_y + px * 0.5];
                draw_list
                    .add_rect(
                        [pos[0] - pad, pos[1] - pad * 0.5],
                        [pos[0] + size[0] + pad, pos[1] + size[1] + pad * 0.5],
                        [0.0, 0.0, 0.0, 0.6],
                    )
                    .filled(true)
                    .rounding((size[1] + pad) / 2.0)
                    .build();
                draw_list.add_text(pos, [1.0, 1.0, 1.0, 0.92], msg.as_str());
                ui.set_window_font_scale(1.0);
            });
    }
}

impl ImguiRenderLoop for MiniMap {
    fn initialize<'a>(&'a mut self, ctx: &mut Context, render_context: &'a mut dyn RenderContext) {
        let io = ctx.io_mut();
        io.mouse_draw_cursor = false;
        // 字体图集。不建的话 imgui 会自己塞一个 13px 的 ProggyClean，在高分辨率
        // 下要放大好几倍，糊得明显；而且它只有 ASCII。
        //
        // 优先读一款系统中文字体（不打进 dll：中文字体十几 MB，而且多数不允许
        // 随程序再分发），只烘焙提示条实际用到的那几十个字。读不到就退回内置
        // 字体，提示条随之切成英文。
        self.cjk = match font::load_cjk() {
            Some((_, data)) => {
                // 只烘焙真正会显示的字。以后改中文文案，记得让新字也走到这里，
                // 否则新字会变成方块。
                let mut texts = vec![
                    MSG_TRAIL_ON.0,
                    MSG_TRAIL_OFF.0,
                    MSG_TRAIL_CONFIRM.0,
                    MSG_TRAIL_CLEARED.0,
                    MSG_HEADING_UP.0,
                    MSG_NORTH_UP.0,
                    MSG_POINTS.0,
                    MSG_ENEMY_ON.0,
                    MSG_ENEMY_OFF.0,
                    MSG_NEUTRAL_ON.0,
                    MSG_NEUTRAL_OFF.0,
                    MSG_ITEMS_ON.0,
                    MSG_ITEMS_OFF.0,
                    HELP_TITLE.0,
                ];
                texts.extend(HELP_ROWS.iter().map(|(_, desc)| desc.0));
                let ranges = font::glyph_ranges_for(&texts);
                ctx.fonts().add_font(&[FontSource::TtfData {
                    data: &data,
                    size_pixels: FONT_ATLAS_PX,
                    config: Some(FontConfig {
                        glyph_ranges: FontGlyphRanges::from_slice(ranges),
                        ..FontConfig::default()
                    }),
                }]);
                true
            }
            None => {
                ctx.fonts().add_font(&[FontSource::DefaultFontData {
                    config: Some(FontConfig {
                        size_pixels: FONT_ATLAS_PX,
                        ..FontConfig::default()
                    }),
                }]);
                false
            }
        };

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
        let t_start = Instant::now();

        // 逻辑区域立刻跟进：路线要记到正确的那张图上，雷达半径也跟着走。
        if let Some(map) = self.update_map() {
            tracing::debug!("update map: {} at {:?}", map.key, (self.game.x, self.game.y));
            self.map = Some(map);
            // 换图时落盘，免得刚走完的一段因为崩溃丢掉。塔类关卡跨层很频繁，
            // 所以这里是节流版本，不是每次都真的写。
            self.trail.save_on_map_change();
            // 再次进入同一个区域时，落脚点未必挨着上次离开的地方。
            self.trail.cut();
        }
        let t_map = t_start.elapsed();

        // 底图换成后台解码。解码一张 2000×2000 的 webp 要几百毫秒到一两秒，
        // 而 before_render 跑在渲染线程上 —— 原来同步做这件事，每次跨层整个
        // 画面就停住。小西天按高度切成 5 张图，在浮屠界的圆筒里跑上跑下会
        // 反复触发。
        //
        // 现在解码交给工作线程，图没好之前继续显示上一张（`shown` 不动，
        // 地形和 UV 换算始终自洽），好了再上传纹理、一次切过去。
        let t_upload_start = Instant::now();
        self.map_loader.poll();
        let want = self.map.as_ref().map(|map| map.key.clone());
        if let Some(key) = want {
            if self.shown.as_ref().map(|m| m.key.as_str()) != Some(key.as_str()) {
                if let Some(image) = self.map_loader.get(&key) {
                    match render_context.replace_texture(
                        self.textures.map.id.unwrap(),
                        image.as_bytes(),
                        image.width(),
                        image.height(),
                    ) {
                        Ok(()) => self.shown = self.map.clone(),
                        Err(e) => tracing::error!("replace_texture failed for {key}: {e:?}"),
                    }
                }
            }
        }
        let t_upload = t_upload_start.elapsed();

        let t_trail_start = Instant::now();
        if self.trail_enabled && self.game.playing {
            if let Some(key) = self.map.as_ref().map(|m| m.key.clone()) {
                self.trail.record(&key, self.game.x, self.game.y);
            }
        }
        self.trail.save_if_due();
        self.sync_config();
        let t_trail = t_trail_start.elapsed();

        let t_radar_start = Instant::now();
        self.refresh_radar();
        let t_radar = t_radar_start.elapsed();

        // 哪一段拖慢了帧，日志里直接能看到。默认级别就记，不用开 debug ——
        // 出现卡顿时用户手上就有数据。
        let ms = |d: std::time::Duration| d.as_secs_f32() * 1000.0;
        let total = ms(t_start.elapsed());
        if total >= SLOW_MS {
            tracing::info!(
                "slow before_render: {total:.1} ms (map {:.1}, texture {:.1}, trail {:.1}, radar {:.1})",
                ms(t_map),
                ms(t_upload),
                ms(t_trail),
                ms(t_radar),
            );
        }
    }
    fn render(&mut self, ui: &mut imgui::Ui) {
        let started = Instant::now();
        self.render(ui);
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        if ms >= SLOW_MS {
            tracing::info!("slow render: {ms:.1} ms");
        }
        // ui.show_demo_window(&mut true);
    }
}
