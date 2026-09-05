use once_cell::sync::Lazy;
use std::sync::Mutex;

// 添加静态变量来存储上一次的状态
static LAST_STATE: Lazy<Mutex<Option<GameState>>> = Lazy::new(|| Mutex::new(None));

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
struct PlayerInfo {
    x: f32,
    y: f32,
    z: f32,
    angle: f32,
    is_local_view_target: u8,
    b_show_mouse_cursor: u8,
    b_move_input_ignored: u8,
    level_name: [u8; 256],
}

// type GetGameInfoFn = unsafe extern "C" fn() -> GameInfo;

extern "C" {
    fn getPlayerInfo() -> PlayerInfo;
}

extern "C" {
    fn toggleMouseCursor(show: bool) -> bool;
}

extern "C" {
    fn b1Init() -> ();
}

/// `ActorDot::kind` 的取值，和 C++ 侧的 `DOT_KIND_*` 一一对应。
pub const KIND_HOSTILE: u8 = 0;
pub const KIND_NEUTRAL: u8 = 1;
pub const KIND_DROP: u8 = 2;
pub const KIND_COLLECT: u8 = 3;
pub const KIND_MEDITATION: u8 = 4;
pub const KIND_INTERACT: u8 = 5;

/// `ActorDot::flags` 的位。
pub const FLAG_IN_BATTLE: u8 = 0x01;

/// 玩家附近的一个目标，和 C++ 侧的 `ActorDot` 一一对应。
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ActorDot {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub kind: u8,
    pub flags: u8,
}

impl ActorDot {
    pub fn in_battle(&self) -> bool {
        self.flags & FLAG_IN_BATTLE != 0
    }
}

extern "C" {
    fn getNearbyActors(
        out: *mut ActorDot,
        max_count: i32,
        radius: f32,
        z_limit: f32,
        kind_mask: u32,
    ) -> i32;
}

/// 单次刷新最多取多少个。密集场景也就几十个，256 够用，同时给了个上限
/// 免得万一 C++ 侧数错了把内存写飞。
pub const MAX_ACTORS: usize = 256;

/// 刷新 `buf` 为玩家周围的目标：水平 `radius`、垂直 `z_limit` 之内、
/// 且被 `kind_mask` 选中的。
///
/// `kind_mask` 是 `1 << KIND_*` 的按位或；没选中的类别在 C++ 侧连一次 `IsA`
/// 都不会做。复用调用方的 Vec，避免每秒分配几次。C++ 侧出任何意外都返回 0，
/// 这里对应的就是"这一次没有数据"，不是错误。
pub fn nearby_actors(radius: f32, z_limit: f32, kind_mask: u32, buf: &mut Vec<ActorDot>) {
    if kind_mask == 0 {
        buf.clear();
        return;
    }
    buf.clear();
    buf.resize(MAX_ACTORS, ActorDot::default());
    let count =
        unsafe { getNearbyActors(buf.as_mut_ptr(), MAX_ACTORS as i32, radius, z_limit, kind_mask) };
    let count = (count.max(0) as usize).min(MAX_ACTORS);
    buf.truncate(count);
}

#[derive(Debug, Clone)]
pub struct GameState {
    pub level: String,
    pub playing: bool,
    pub show_mouse_cursor: bool,
    pub move_input_ignored: bool,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub angle: f32,
}

// [map]
// 场景ID
// 13=序章云端_MGD
// 10=黑风山
// 11=隐·旧观音禅院
// 12=黑风山-尺木间
// 20=黄风岭
// 25=隐·斯哈里国
// 24=黄风岭-藏龙洞
// 30=小西天
// 62=隐·梅山
// 92=小西天-浮屠塔
// 40=盘丝岭
// 80=隐·紫云山
// 50=火焰山
// 70=隐·壁水洞
// 98=花果山
// 61=石卵
// 31=如意画轴-六六村

pub fn init() {
    unsafe { b1Init() };
}

// 获取地图id
pub fn game_state() -> GameState {
    let info = unsafe { getPlayerInfo() };

    let level = String::from_utf8_lossy(&info.level_name)
        .trim_matches(char::from(0))
        .to_string();

    let angle = info.angle + 90.0;
    let angle = if angle < 0.0 { angle + 360.0 } else { angle };

    // 创建新的状态
    let current_state = GameState {
        playing: info.is_local_view_target == 1,
        show_mouse_cursor: info.b_show_mouse_cursor == 1,
        move_input_ignored: info.b_move_input_ignored == 1,
        level: level.clone(),
        angle,
        x: info.x,
        y: info.y,
        z: info.z,
    };
    // 检查是否需要使用上一次的状态
    let mut last_state = LAST_STATE.lock().unwrap();
    let final_state = if !level.is_empty() && info.x == 0.0 && info.y == 0.0 && info.z == 0.0 {
        // 如果有上一次的状态，使用它
        last_state.clone().unwrap_or(current_state.clone())
    } else {
        // 否则使用当前状态
        current_state.clone()
    };

    // 更新保存的状态
    *last_state = Some(final_state.clone());

    final_state
}

pub fn toggle_mouse_cursor(show: bool) {
    unsafe { toggleMouseCursor(show) };
}
