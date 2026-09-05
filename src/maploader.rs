// This file is new in a fork of jaskang/wukong-minimap.
//
// Upstream: https://github.com/jaskang/wukong-minimap (Apache-2.0)
// Fork:     https://github.com/Ouye/wukong-minimap
//
// Decodes map images on a worker thread and keeps a few of them around, so
// crossing a map boundary never blocks the render thread.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use hudhook::tracing;
use image::RgbaImage;

use crate::utils::image_with_file;

/// 缓存几张解码好的底图。每张 2000×2000 RGBA 常驻 16 MB。
///
/// 之所以不是 1：小西天（浮屠界那座塔）在 `data.json` 里按高度被切成 5 张图，
/// 在圆筒里跑上跑下会反复跨越分界线。只留一张的话每次跨层都要重新解码整张
/// webp；留 4 张，来回走基本全是命中。
const CACHE_LEN: usize = 4;

type Loaded = (String, Option<RgbaImage>);

/// 底图的后台解码器 + 最近使用缓存。
///
/// 解码一张 2000×2000 的 webp 要几百毫秒到一两秒。原来这件事发生在
/// `before_render` 里，而它跑在渲染线程上（Present 钩子），于是每次换图整个
/// 游戏画面就停住。现在交给工作线程，主线程只在解码完成后做一次纹理上传。
pub struct MapLoader {
    /// 最近用过的排在前面。
    cache: VecDeque<(String, Arc<RgbaImage>)>,
    /// 后台正在解码的那张。同一时刻只解一张，避免连续跨层时线程堆积。
    inflight: Option<String>,
    /// 已经想要、但要等 `inflight` 结束才发出去的那张。只保留最新的一个 ——
    /// 中间那些一闪而过的层没必要解。
    queued: Option<String>,
    /// 解码失败过的，不再重试。否则 `get` 每帧都会重新派一个线程去解同一张
    /// 打不开的图 —— 文件缺失或损坏时会变成每秒几十个线程。
    failed: Vec<String>,
    tx: Sender<Loaded>,
    /// `Receiver` 不是 `Sync`，而 hudhook 要求整个 render loop 是
    /// `Send + Sync`（`gilrs` 当初也是因为这个被套了 `Mutex`）。方法都拿
    /// `&mut self`，所以实际取用走 `get_mut()`，不会真的上锁。
    rx: Mutex<Receiver<Loaded>>,
}

impl MapLoader {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            cache: VecDeque::new(),
            inflight: None,
            queued: None,
            failed: Vec::new(),
            tx,
            rx: Mutex::new(rx),
        }
    }

    /// 取一张底图。缓存里有就立刻返回，没有就安排后台解码并返回 None ——
    /// 调用方继续显示上一张，等好了再换。
    pub fn get(&mut self, key: &str) -> Option<Arc<RgbaImage>> {
        if let Some(pos) = self.cache.iter().position(|(k, _)| k == key) {
            let entry = self.cache.remove(pos).expect("position() 刚给的下标");
            let image = Arc::clone(&entry.1);
            self.cache.push_front(entry);
            return Some(image);
        }
        self.request(key);
        None
    }

    /// 收后台送回来的结果。每帧调一次。
    pub fn poll(&mut self) {
        // 先全部收下来再处理：直接在 try_recv 的循环里改 self 会和 rx 的
        // 借用打架。通常一个都没有，Vec::new() 也不分配。
        let mut done: Vec<Loaded> = Vec::new();
        {
            // 锁只可能被自己持有，中毒了也无所谓 —— 里面就是个 Receiver。
            let rx = self.rx.get_mut().unwrap_or_else(|e| e.into_inner());
            while let Ok(message) = rx.try_recv() {
                done.push(message);
            }
        }

        for (key, image) in done {
            if self.inflight.as_deref() == Some(key.as_str()) {
                self.inflight = None;
            }
            match image {
                Some(image) => {
                    tracing::debug!("map: decoded {key}");
                    self.insert(key, image);
                }
                // 解码失败已经在 image_with_file 里记过日志了。
                None => {
                    if !self.failed.iter().any(|k| k == &key) {
                        self.failed.push(key);
                    }
                }
            }
        }

        if self.inflight.is_none() {
            if let Some(key) = self.queued.take() {
                self.spawn(key);
            }
        }
    }

    fn request(&mut self, key: &str) {
        if self.inflight.as_deref() == Some(key) {
            return;
        }
        if self.failed.iter().any(|k| k == key) {
            return;
        }
        if self.inflight.is_some() {
            self.queued = Some(key.to_string());
            return;
        }
        self.spawn(key.to_string());
    }

    fn spawn(&mut self, key: String) {
        tracing::debug!("map: decoding {key} in the background");
        self.inflight = Some(key.clone());
        let tx = self.tx.clone();
        // 解码线程跑完就结束。发送失败只可能是插件正在卸载，忽略即可。
        if let Err(e) = thread::Builder::new()
            .name(String::from("wukong-minimap-decode"))
            .spawn(move || {
                let image = image_with_file(&key).map(|mut image| {
                    // 顺手把 alpha 255 夹到 254。渲染器那边本来就要做这件事
                    // （见 vendor/hudhook 里的 opaque-texture 规避），在这里
                    // 做等于把一次 16 MB 的拷贝从渲染线程挪到了工作线程。
                    for px in image.chunks_exact_mut(4) {
                        if px[3] == 255 {
                            px[3] = 254;
                        }
                    }
                    image
                });
                let _ = tx.send((key, image));
            })
        {
            tracing::error!("map: could not spawn the decode thread: {e}");
            self.inflight = None;
        }
    }

    fn insert(&mut self, key: String, image: RgbaImage) {
        self.cache.push_front((key, Arc::new(image)));
        while self.cache.len() > CACHE_LEN {
            if let Some((dropped, _)) = self.cache.pop_back() {
                tracing::debug!("map: evicted {dropped}");
            }
        }
    }
}
