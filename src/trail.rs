// This file is new in a fork of jaskang/wukong-minimap.
//
// Upstream: https://github.com/jaskang/wukong-minimap (Apache-2.0)
// Fork:     https://github.com/Ouye/wukong-minimap
//
// Records where the player has walked, per map area, and persists it next to
// the dll so the trail survives across sessions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hudhook::tracing;
use serde::{Deserialize, Serialize};

/// 相邻两点的最小世界距离。地图跨度约 23 万世界单位、底图 2000px，
/// 250 单位约合底图上两个像素。
const MIN_STEP: f32 = 250.0;

/// 相邻两点超过这个距离视为传送或读档，断成新的一段，
/// 否则会画出一条横穿地图的直线。
const BREAK_DIST: f32 = 8000.0;

/// 单张地图的点数上限。超出后抽稀一半、间距翻倍，记录本身不会停。
const MAX_POINTS: usize = 20000;

/// 存盘节流间隔。
const SAVE_INTERVAL: Duration = Duration::from_secs(30);

/// 换图时落盘的最小间隔。塔类关卡（浮屠界）按高度切成好几张图，跨层非常
/// 频繁，每次都序列化 + 写盘会累在渲染线程上。
const MAP_CHANGE_SAVE_INTERVAL: Duration = Duration::from_secs(5);

pub const FILE_NAME: &str = "wukong_minimap_trails.json";

fn default_step() -> f32 {
    MIN_STEP
}

#[derive(Serialize, Deserialize)]
struct Track {
    /// 本图当前的采样间距。抽稀一次翻一倍，所以要跟着数据一起存。
    #[serde(default = "default_step")]
    step: f32,
    segments: Vec<Vec<[f32; 2]>>,
}

impl Track {
    fn new() -> Self {
        Self {
            step: MIN_STEP,
            segments: Vec::new(),
        }
    }

    fn points(&self) -> usize {
        self.segments.iter().map(Vec::len).sum()
    }

    /// 超过上限后隔点抽稀，并把采样间距翻倍，避免刚抽完又立刻涨回来。
    fn trim(&mut self) {
        if self.points() <= MAX_POINTS {
            return;
        }
        self.step *= 2.0;
        for seg in &mut self.segments {
            if seg.len() < 3 {
                continue;
            }
            let last = seg[seg.len() - 1];
            let mut kept: Vec<[f32; 2]> = seg.iter().step_by(2).copied().collect();
            if kept[kept.len() - 1] != last {
                kept.push(last);
            }
            *seg = kept;
        }
        tracing::debug!("trail: decimated, step is now {}", self.step);
    }
}

#[derive(Serialize, Deserialize)]
struct TrailFile {
    version: u32,
    tracks: HashMap<String, Track>,
}

impl Default for TrailFile {
    fn default() -> Self {
        Self {
            version: 1,
            tracks: HashMap::new(),
        }
    }
}

pub struct Trail {
    file: TrailFile,
    path: PathBuf,
    dirty: bool,
    last_save: Instant,
    /// 下一个点另起一段，不要和上一个点连起来。
    ///
    /// 暂停记录、切换区域、载入存档之后都要断开：这几种情况下人是移动过的，
    /// 但中间那段没有被记录，直连就等于凭空画出一条并没走过的直线。
    break_next: bool,
}

impl Trail {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE_NAME);
        let file = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<TrailFile>(&text) {
                Ok(file) => file,
                Err(e) => {
                    // 宁可从空的开始，也不要因为一个坏文件让插件起不来。
                    tracing::error!("trail: {} is unreadable ({e}), starting empty", path.display());
                    TrailFile::default()
                }
            },
            Err(_) => TrailFile::default(),
        };

        let total: usize = file.tracks.values().map(Track::points).sum();
        if total > 0 {
            tracing::info!("trail: loaded {total} points from {}", path.display());
        }

        Self {
            file,
            path,
            dirty: false,
            last_save: Instant::now(),
            // 上次退出到这次进游戏之间，人多半已经不在原地了。
            break_next: true,
        }
    }

    /// 让下一个记录点另起一段。
    pub fn cut(&mut self) {
        self.break_next = true;
    }

    pub fn record(&mut self, map_key: &str, x: f32, y: f32) {
        let cut = std::mem::take(&mut self.break_next);

        let track = self
            .file
            .tracks
            .entry(map_key.to_string())
            .or_insert_with(Track::new);

        let p = [x, y];
        let step2 = track.step * track.step;

        if cut {
            track.segments.push(vec![p]);
            track.trim();
            self.dirty = true;
            return;
        }

        match track.segments.last_mut() {
            Some(seg) if !seg.is_empty() => {
                let last = seg[seg.len() - 1];
                let d2 = (p[0] - last[0]).powi(2) + (p[1] - last[1]).powi(2);
                if d2 < step2 {
                    return;
                }
                if d2 > BREAK_DIST * BREAK_DIST {
                    track.segments.push(vec![p]);
                } else {
                    seg.push(p);
                }
            }
            _ => track.segments.push(vec![p]),
        }

        track.trim();
        self.dirty = true;
    }

    pub fn segments(&self, map_key: &str) -> &[Vec<[f32; 2]>] {
        self.file
            .tracks
            .get(map_key)
            .map(|t| t.segments.as_slice())
            .unwrap_or(&[])
    }

    pub fn total_points(&self) -> usize {
        self.file.tracks.values().map(Track::points).sum()
    }

    /// 清空全部路径并删除存档文件。没有备份，按下去就是没了。
    pub fn clear(&mut self) {
        self.file.tracks.clear();
        self.dirty = false;
        match std::fs::remove_file(&self.path) {
            Ok(()) => tracing::info!("trail: cleared, removed {}", self.path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!("trail: cleared")
            }
            Err(e) => tracing::error!("trail: could not remove {}: {e}", self.path.display()),
        }
    }

    /// 换图时落盘。比常规节流积极，但不是每次都真的写 —— 见
    /// `MAP_CHANGE_SAVE_INTERVAL`。
    pub fn save_on_map_change(&mut self) {
        if self.dirty && self.last_save.elapsed() >= MAP_CHANGE_SAVE_INTERVAL {
            self.save();
        }
    }

    pub fn save_if_due(&mut self) {
        if self.dirty && self.last_save.elapsed() >= SAVE_INTERVAL {
            self.save();
        }
    }

    /// 先写临时文件再改名，中途崩溃不会毁掉已有的存档。
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        self.last_save = Instant::now();

        let text = match serde_json::to_string(&self.file) {
            Ok(text) => text,
            Err(e) => {
                tracing::error!("trail: could not serialize: {e}");
                return;
            }
        };

        let tmp = self.path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, text) {
            tracing::error!("trail: could not write {}: {e}", tmp.display());
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            tracing::error!("trail: could not replace {}: {e}", self.path.display());
            return;
        }
        self.dirty = false;
        tracing::debug!("trail: saved {} points", self.total_points());
    }
}

/// 把一条屏幕坐标折线裁剪成若干条完全落在圆内的折线。
///
/// 小地图是圆的，折线不像图片那样能靠 `add_image_rounded` 裁掉，所以逐段和圆
/// 求交：完全在内的原样保留，跨越边界的在交点处断开，完全在外的丢弃。
pub fn clip_polyline_to_circle(
    pts: &[[f32; 2]],
    center: [f32; 2],
    radius: f32,
) -> Vec<Vec<[f32; 2]>> {
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut run: Vec<[f32; 2]> = Vec::new();
    let r2 = radius * radius;
    let inside =
        |p: [f32; 2]| (p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) <= r2;

    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (ia, ib) = (inside(a), inside(b));

        if ia && ib {
            if run.is_empty() {
                run.push(a);
            }
            run.push(b);
            continue;
        }

        // |a + t(b - a) - center|² = radius², 解 t ∈ [0, 1]
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let (fx, fy) = (a[0] - center[0], a[1] - center[1]);
        let qa = dx * dx + dy * dy;
        if qa <= f32::EPSILON {
            continue;
        }
        let qb = 2.0 * (fx * dx + fy * dy);
        let qc = fx * fx + fy * fy - r2;
        let disc = qb * qb - 4.0 * qa * qc;

        if disc < 0.0 {
            if run.len() >= 2 {
                out.push(std::mem::take(&mut run));
            } else {
                run.clear();
            }
            continue;
        }

        let sq = disc.sqrt();
        let t0 = ((-qb - sq) / (2.0 * qa)).clamp(0.0, 1.0);
        let t1 = ((-qb + sq) / (2.0 * qa)).clamp(0.0, 1.0);
        if t1 <= t0 {
            if run.len() >= 2 {
                out.push(std::mem::take(&mut run));
            } else {
                run.clear();
            }
            continue;
        }

        let at = |t: f32| [a[0] + dx * t, a[1] + dy * t];

        if ia {
            // 出圆
            if run.is_empty() {
                run.push(a);
            }
            run.push(at(t1));
            out.push(std::mem::take(&mut run));
        } else if ib {
            // 入圆
            if run.len() >= 2 {
                out.push(std::mem::take(&mut run));
            } else {
                run.clear();
            }
            run.push(at(t0));
            run.push(b);
        } else {
            // 两端都在外，但穿过了圆
            if run.len() >= 2 {
                out.push(std::mem::take(&mut run));
            } else {
                run.clear();
            }
            out.push(vec![at(t0), at(t1)]);
        }
    }

    if run.len() >= 2 {
        out.push(run);
    }
    out
}
