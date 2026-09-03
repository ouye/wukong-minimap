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

### 插件本体（`src/`）

- **地图按需加载**：原本启动时解码全部 23 张地图并常驻内存。改为只保留当前所在区域，
  常驻内存 **368MB → 16MB**，启动省去约 12 秒解码
- `image_with_file` 返回 `Option` 而非 `panic`（现在是游戏运行中调用，地图文件缺失
  不应导致游戏崩溃）
- `replace_texture` 的失败不再被 `let _ =` 吞掉
- 占位图 `nomap.webp` 缩放至 2000×2000 与地图尺寸一致（`replace_texture` 拒绝尺寸变化）
- 日志默认级别 `debug` → `info`，可用 `RUST_LOG` 覆盖

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
