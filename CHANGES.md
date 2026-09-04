# 本分支的改动记录

> 本文件用于满足 Apache License 2.0 第 4(b) 条：
> *"You must cause any modified files to carry prominent notices stating that You changed the files."*
>
> 上游项目：[jaskang/wukong-minimap](https://github.com/jaskang/wukong-minimap)（Apache-2.0）
> 本分支维护者：@Ouye
> 分叉基点：`baff527`（2025-11-10，"feat: 新版本 sdk"）

本分支的目的只有一个：**让插件在 2025 年 10 月 1.0.20 更新之后的《黑神话：悟空》上重新可用。**
所有改动都围绕这个目标，没有改动小地图的玩法与视觉设计。

---

## 一、为什么会失效

1.0.20 更新（2025-10-13，官方公告称有"较多的底层改动"，并集成 FSR4 帧生成）同时触发了两个**互相独立**的故障：

**故障 A：SDK 偏移全部失效** —— 插件完全不工作
游戏 exe 重新链接后，全局地址整体位移，UE 类的成员偏移也随之改变：

| | v1.8 时期（2025-03） | 本分支使用的版本 |
|---|---|---|
| `GObjects` | `0x1D76CC30` | 运行时扫描（实测 `0x1D47BB90`） |
| `AppendString` | `0x0CD72A90` | 运行时扫描（实测 `0x0CB63D40`） |
| `AController::Pawn` | `0x2B8` | `0x2C8` |
| `APlayerController::bShowMouseCursor` | `0x51C` | `0x52C` |

**故障 B：不透明纹理被破坏** —— 小地图底图花屏
这一条**与插件代码无关**。同一份渲染代码在旧版游戏上正常。经双向对照实验确认，触发条件是纹理的 **alpha 通道为 255（完全不透明）**：把地图的 alpha 改成 200 立即恢复正常，把原本正常的 `mapwraper.png` 的 alpha 改成 255 立即损坏。详见下文"hudhook 相关改动"。

---

## 二、改动清单

### 恢复并适配 C++ 半边（`b1sdk/`）

上游在 `baff527` 中删除了整个 `b1sdk/` 目录（提供 `getPlayerInfo` / `b1Init` 等接口的 C++ 静态库），仓库中只剩 Dumper-7 的产物头文件。而 `build.rs` 仍然写着 `cargo:rustc-link-lib=static=b1sdk`，**导致仓库在 HEAD 状态下无法构建**。

- 从 `7375f46` 恢复 `b1sdk.cpp` / `b1sdk.h` / `helper.hpp` / `stdafx.h`
- 将 `5.0.0-0+++UE5+Release-5.0-b1/CppSDK` 作为新 SDK 放入 `b1sdk/src/SDK`
- 包名适配：新版 Dumper-7 把 `b1MinusManaged_classes.hpp` 改名为 `b1_classes.hpp`

### SDK 偏移改为运行时解析（`b1sdk/src/SDK/SDK/Basic.hpp`）

新版 Dumper-7 把偏移生成为 `constexpr`，等于把 SDK 焊死在某一个特定 exe 上。改回 `inline` 以便 `b1Init()` 的特征码扫描在运行时覆盖：

```cpp
inline int32 GObjects     = 0x0;   // dumped: 0x1D47ED10
inline int32 AppendString = 0x0;   // dumped: 0x0CB65030
constexpr int32 GWorld    = 0x0;   // 必须保持 constexpr（用于 if constexpr）
```

`GWorld` 特别处理：新版 dump 给了它一个真实地址，而 `UWorld::GetWorld()` 里有
`if constexpr (Offsets::GWorld != 0)` —— 会直接解引用写死的指针。置 0 后走
`GEngine->GameViewport->World` 这条**只依赖 GObjects**（由特征码扫描得到）的路径，
显著提升了对后续小版本更新的耐受度。

### 构建系统（`b1sdk/CMakeLists.txt`、`build.ps1`、`setup.ps1`）

- 手写 CMakeLists 替代 cmkr（原来每次 configure 都要联网 bootstrap）
- **`/utf-8`（关键）**：`b1sdk.cpp` 含中文注释且为无 BOM 的 UTF-8。在中文区域的
  Windows 上，MSVC 按 cp936 解码源码，一条中文注释会**吞掉紧随其后的一整行代码**
  （实测吞掉了 `SDK::FVector Location = ...`，三行之后才报 C2065）。同时给四个手写
  源文件补了 UTF-8 BOM。
- 默认只编译实际用到的 5 个 SDK 翻译单元（`-DB1SDK_FULL=ON` 可全量编译）
- 用空宏桩替换 Dumper-7 的 `Assertions.inl`（21MB，会被 include 进每个翻译单元）
- `build.ps1` 一键完成 MSVC + Rust 构建、安装、打包
- `setup.ps1` 从零重建可编译的源码树

### hudhook 相关改动（`vendor/hudhook/`，MIT，Andrea Venuta）

vendored 副本，四处改动。**前三处是与本项目无关的通用 bug，值得提交上游**：

1. **`util.rs` — Fence 初值**
   所有调用点都是 `Signal(fence, value())` → `wait()` → `incr()`，而 `wait()` 的条件是
   `GetCompletedValue() < value`。初值为 0 时首次判断即为 `0 < 0`，**每个 fence 的第一次
   GPU 提交从来没有被等待过**。初值改为 1。

2. **`dx12.rs` — 重复上传时的非法资源状态**
   `Texture` 结构体不记录资源状态，`upload_texture` 无条件发出
   `COPY_DEST → PIXEL_SHADER_RESOURCE` 屏障。首次正确，此后每次 `replace_texture` 都是
   两处违规：`CopyTextureRegion` 要求目标处于 `COPY_DEST`（实际处于
   `PIXEL_SHADER_RESOURCE`），且屏障声明的 `StateBefore` 与实际不符。加入状态跟踪。

3. **`dx12.rs` — 跨队列所有权转移**
   `TextureHeap` 自建命令队列，纹理在该队列写入、在渲染队列采样。D3D12 要求跨队列
   共享的资源经由 `COMMON` 状态转移所有权。改为常驻 `COMMON` 并依赖隐式状态提升。

4. **`dx12.rs` — alpha 255 → 254（本项目专用规避，不建议提交上游）**
   见"故障 B"。排查过程中已确认**与上传路径完全无关**：先后换用
   `CopyTextureRegion`、`GetCopyableFootprints` 权威布局、CUSTOM 堆 +
   `WriteToSubresource`（完全绕开命令队列、fence、屏障、上传缓冲区），损坏形态分毫不变；
   上传缓冲区回读校验也证明进入 GPU 的字节逐字节正确。
   高度怀疑与游戏 1.0.20 集成的 FSR4 帧生成对"完全不透明的叠加层"的处理有关。
   254 与 255 相差 0.4% 不透明度，肉眼不可分辨。

   同时改用 `WriteToSubresource` + CUSTOM 堆作为纹理上传主路径（保留原路径作为回退）。

5. **`hooks/dx12.rs` — 启动竞态被记为 error**

   `Present` 比 `ExecuteCommandLists` 先被挂上，两者都就绪之前 `init_pipeline`
   无法建立管线。这是正常的启动顺序，但原代码对每一帧都打两条 `error!`。
   改为返回一个专用的 `E_NOT_READY`，该分支降为 `debug!`，其余错误照旧上报。

### 插件本体（`src/`）

- **地图按需加载**：原本启动时解码全部 23 张地图并常驻内存。改为只保留当前所在区域，
  常驻内存 **368MB → 16MB**，启动省去约 12 秒解码
- `image_with_file` 返回 `Option` 而非 `panic`（现在是游戏运行中调用，地图文件缺失
  不应导致游戏崩溃）
- `replace_texture` 的失败不再被 `let _ =` 吞掉
- 占位图 `nomap.webp` 缩放至 2000×2000 与地图尺寸一致（`replace_texture` 拒绝尺寸变化）
- 每帧、每个手柄事件的 `info!` 降为 `debug!`（`draw_nomap`、`gilrs event` 会在
  一次游戏过程中写出数 MB 日志）
- 日志默认级别改为 `error,wukong_minimap=info`：只保留错误和本插件的启动横幅，
  可用 `RUST_LOG=debug` 取回完整输出
- 启动时记录版本与仓库地址，便于确认用户反馈的日志来自哪个构建
- 大地图左下角、上游 logo 右侧增加一行分支署名。`includes/mainwraper.png`
  与上游逐字节一致，未做覆盖或改动

### 新功能：地图朝向模式（`Shift` + `0`）

原有行为（地图不动、箭头转）保留为默认。新增"箭头锁定朝上、地图随人物转向"模式。

实现要点：`add_image_rounded` 能画圆但只接受轴对齐 UV，`add_image_quad` 能任意变形
但画出来是方的。解法是**不转屏幕四边形、转采样区域**——用 72 个退化四边形拼成三角扇
维持圆形，逐顶点计算 UV：

```rust
// 屏幕偏移 d → 世界偏移 R(angle)·d/scale → uv
let wx = (dx * rot_cos - dy * rot_sin) / scale_px;
let wy = (dx * rot_sin + dy * rot_cos) / scale_px;
```

该旋转矩阵正是北向模式下箭头旋转的逆变换。UV 做了 `clamp`，因为 hudhook 的采样器是
`WRAP` 模式，人物接近地图边缘时旋转会让 UV 越界、把地图另一侧卷进来。
北向模式的代码路径未作任何改动。

### 新功能：走过的路线（`src/trail.rs`，`9` / `Shift` + `9`）

按地图区域记录玩家走过的位置，在小地图和大地图上画成折线，存盘后跨次游戏保留。

- 采样：相邻两点至少间隔 250 世界单位（底图上约 2px）。超过 8000 单位视为传送或
  读档，断成新的一段，避免画出一条横穿地图的直线
- 上限：单图 20000 点。超出后隔点抽稀、采样间距翻倍，记录本身不会停
- 存盘：dll 同目录的 `wukong_minimap_trails.json`，每 30 秒（有变化才写）以及切换
  区域时落盘。先写临时文件再改名，中途崩溃不会毁掉已有存档
- `9` 同时控制记录与显示；`Shift` + `9` 清除全部并删除存档文件，不留备份，
  需要在 3 秒内按第二次确认
- **暂停记录、切换区域、启动载入之后都要断段**：这几种情况下人是移动过的，
  但中间那段没被记录。不断开的话，恢复记录后的第一个点会直接连到暂停前的最后
  一个点，画出一条并没走过的直线 —— 看上去就像"关掉之后照样记了"
- 颜色默认青色 `#22E0FFCC`：地图底色以土黄、褐、墨绿为主，冷色最跳，也不会和
  暖色调的边框、图标撞在一起。可在配置文件里改成 `#RRGGBB` 或 `#RRGGBBAA`
- 小地图是圆的，折线不能像底图那样靠 `add_image_rounded` 裁掉，所以逐段与圆求交
  （`clip_polyline_to_circle`）：完全在内的保留，跨边界的在交点处断开，完全在外的丢弃
- 朝向模式下路径点走与图标相同的 `R(-angle)` 变换，保持与地形贴合
- 顺带加了一个屏幕提示条（开关状态、清除确认、朝向模式）。提示条窗口占满屏幕宽度：
  imgui 会把绘制裁剪到窗口矩形，窗口只有小地图那么宽的话长一点的文案会被切掉

### 按键说明面板（大地图右侧）

上游的大地图左边是点位图例，右边一直空着。按 `Tab` 打开大地图时在右侧列出全部
按键，一行一条，键名一列按最宽的对齐。

- 小地图开着时让到它下面（同一条右边距），关着时竖直居中
- 面板宽高由 `calc_text_size` 量出来，不写死 —— 中英文两套文案宽度差很多
- 和提示条一样，用一个铺满屏幕的窗口来画。imgui 会把绘制裁剪到窗口矩形，
  按内容大小去开窗口的话，量错一点点文字就被切掉了

### 设置持久化（`src/config.rs`）

小地图窗口大小、比例、地图朝向、路线开关四项存进 dll 同目录的
`wukong_minimap_config.json`，进游戏时读回来。

- 每个字段都带 `serde(default)`：旧文件、手改坏的文件、以后新增的字段都不会让
  整个读取失败，缺的用默认值
- 读进来的倍率做 clamp，和按键里的上下限一致 —— 手改成 0 会让小地图直接消失
- 落盘比较的是「上次写入的值」而不是设 dirty 标志，按 `+` 又按 `-` 回到原样
  就不会白写一次；再加 2 秒节流，连按时不会每帧写盘
- 先写临时文件再改名
- `is_show`（`0` 键的显示开关）**不**持久化：如果上次退出前藏起来了，下次进游戏
  会像插件没生效一样，不值得
- `trail_color` 只读不写：由用户手改，插件启动时解析一次。落盘时原样带过去，
  否则按一下 `+` 就会把用户改的颜色覆盖掉
- 启动时无条件落盘一次，把文件里缺的字段补齐 —— 用户打开配置文件就能看到
  全部可改项和它们当前的值，而不用去翻文档猜字段名

### 中文字体（`src/font.rs`）

原本提示文字只能用英文：项目没有加载任何字体，imgui 内置的 ProggyClean 只有
ASCII 字形。

改为在 `initialize` 里按优先级读一款系统中文字体（微软雅黑 → 等线 → 黑体 →
宋体），读不到就回退内置字体、提示文字自动切回英文。

- **不把字体打进 dll**：中文字体动辄十几 MB，而且多数不允许随程序再分发
- 只烘焙提示条实际用到的那几十个字：由文案反推码位、合并成连续区间交给
  `FontGlyphRanges::from_slice`，字体图集不会因为中文而膨胀
- `.ttc` 是字体集合，imgui-rs 没有暴露 `FontNo`，只能取第 0 个 —— 对候选的
  这几个文件来说正好是想要的那一款
- 署名那行仍然是 ASCII（是个网址）

### 素材（`maps/`）

23 张地图由 4096×4096 重采样至 2000×2000（Lanczos，webp q92）。
小地图直径仅一百余像素，原分辨率远超所需。

- 安装包体积：96.1MB → 23.2MB
- 显存占用同步下降

---

## 三、未改动的部分

小地图的布局、配色、图标、点位数据（`includes/data_points.json`）、坐标换算逻辑、
按键设计（除新增 `Shift`+`0` 外）均保持上游原样。点位数据的收集是上游作者的主要工作量，
本分支未作增删。
