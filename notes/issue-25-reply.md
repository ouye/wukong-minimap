# 给 issue #25 的回复（可直接粘贴）

> 目标 issue：https://github.com/jaskang/wukong-minimap/issues/25
> 「黑神话更新后失效了，希望大佬有空更新下」
>
> 发之前把 `Ouye` 替换成你的 GitHub 用户名。

---

我做了一份适配，能在 1.0.20 之后的版本上正常使用了，fork 在这里，
release 里有编译好的包：**https://github.com/Ouye/wukong-minimap**

排查下来，10 月 13 日那次更新其实**同时触发了两个互相独立的问题**，
所以现象才会是"装上去完全没反应"和"底图花屏"两种。

## 一、SDK 偏移失效（插件完全不工作）

exe 重新链接后全局地址整体位移，UE 类的成员偏移也变了：

| | 3 月 v1.8 时期 | 现在 |
|---|---|---|
| `GObjects` | `0x1D76CC30` | `0x1D47BB90` |
| `AppendString` | `0x0CD72A90` | `0x0CB63D40` |
| `AController::Pawn` | `0x2B8` | `0x2C8` |
| `APlayerController::bShowMouseCursor` | `0x51C` | `0x52C` |

好消息是你 11 月提交的那份新 dump（`baff527`）本身是可用的。有两点需要处理：

1. **新版 Dumper-7 把 `Offsets` 生成成了 `constexpr`**，`b1Init()` 里的
   `SDK::Offsets::GObjects = ...` 赋值编译不过。我改回了 `inline`，保留特征码扫描。
   实测那三条特征码在新版 exe 上**依然有效**，所以扫描能正确填上。

2. **`GWorld` 需要特别处理**。新 dump 给了它真实地址，而 `UWorld::GetWorld()` 里是
   `if constexpr (Offsets::GWorld != 0)` —— 会直接解引用写死的指针。
   我把它置 0，让它走 `GEngine->GameViewport->World` 这条**只依赖 GObjects**
   （由扫描得到）的路径，对后续小版本更新的耐受度高很多。

另外 `baff527` 把整个 `b1sdk/` 目录删掉了，但 `build.rs` 还写着
`cargo:rustc-link-lib=static=b1sdk`，所以**仓库在 HEAD 状态下是无法构建的**。
我从 `7375f46` 恢复了那四个文件（`b1sdk.cpp` / `b1sdk.h` / `helper.hpp` / `stdafx.h`）。

## 二、底图花屏（这个和你的代码无关）

这个卡了很久。结论是：**触发条件是纹理完全不透明（alpha=255）**，
和插件代码、纹理尺寸、上传时机、上传路径全都无关。

双向对照实验：把地图的 alpha 改成 200 → 立刻正常；把原本一直正常的
`mapwraper.png` 的 alpha 改成 255 → 立刻损坏。这也解释了为什么图标、边框、
字体图集一直没事——它们大部分区域 alpha=0。

排查过程中我先后换掉了整条上传链路（`CopyTextureRegion` → `GetCopyableFootprints`
权威布局 → CUSTOM 堆 + `WriteToSubresource`，最后一种完全绕开了上传缓冲区、
命令队列、fence 和资源屏障），**损坏形态分毫不变**；上传缓冲区回读校验也证明
进入 GPU 的字节逐字节正确。

高度怀疑与 1.0.20 集成的 **FSR4 帧生成**对"完全不透明的叠加层"的处理有关——
帧生成需要识别 UI 层来避免插值，有些实现正是用不透明度做判据。

规避方式很简单：上传前把 alpha 255 夹到 254。0.4% 的不透明度差异，肉眼不可分辨。

**这也是为什么你自己可能复现不出来** —— 同一份渲染代码在旧版游戏上完全正常。

## 三、顺带修的 hudhook bug

vendored 的 hudhook 里有三个和本项目无关的通用 bug，我一并修了
（准备单独提给上游 veeenu/hudhook）：

1. `Fence::new` 计数器初值为 0，而调用点是 `Signal(value())` → `wait()` → `incr()`，
   `wait()` 的条件是 `GetCompletedValue() < value`，首次即 `0 < 0` —— **每个 fence
   的第一次 GPU 提交从来没被等待过**
2. `Texture` 不记录资源状态，`upload_texture` 无条件发 `COPY_DEST → PIXEL_SHADER_RESOURCE`
   屏障。首次正确，之后每次 `replace_texture` 都是非法状态转换
3. `TextureHeap` 自建命令队列，纹理跨队列使用却没经由 `COMMON` 转移所有权

## 四、其它

- 地图改为按需加载（原本启动时全部 23 张解码常驻）：**368MB → 16MB**
- 地图重采样到 2000×2000：安装包 **96MB → 23MB**（小地图直径才一百多像素，4096 用不上）
- MSVC 在中文区域会因为 `b1sdk.cpp` 的中文注释 + 无 BOM UTF-8 **吞掉一整行代码**，
  加 `/utf-8` 解决
- 新增了一个可选的地图朝向模式（`Shift`+`0`，箭头锁定朝上、地图跟着转）

---

**这些改动你随便拿，需要我整理成 PR 就说一声。** 不过 fork 里的改动结构性比较强
（恢复被删的 C++ 半边、替换 vendored hudhook、重采样全部素材），
直接 review 可能比较费时间，所以我先发了 fork 让大家有个能用的版本。

感谢你做了这个插件，点位那部分的工作量是真的大。
