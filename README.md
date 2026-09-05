# 黑神话·悟空 - 内置实时地图（适配 1.0.20+ 分支）

![alt text](./docs/banner.png)

> **这是 [jaskang/wukong-minimap](https://github.com/jaskang/wukong-minimap) 的一个分支。**
>
> 原作者 [@jaskang](https://github.com/jaskang) 完成了这个插件的全部核心工作——注入、渲染、
> 坐标换算，以及那几百个手工采集的点位。**请先去给原仓库点个 star。**
>
> 本分支只做一件事：**让它在 2025 年 10 月 1.0.20 更新之后的游戏版本上重新可用**，
> 顺带加了两个可选功能：地图朝向模式，以及走过路线的记录。
> 小地图的玩法与视觉设计一律保持原样。
>
> 每个被修改过的文件顶部都写明了改了什么，以及上游是谁。

- 下载地址：[releases](https://github.com/Ouye/wukong-minimap/releases)
- 原项目：[jaskang/wukong-minimap](https://github.com/jaskang/wukong-minimap) · [BiliBili 演示视频](https://www.bilibili.com/video/BV1Y1KueREho/) · [Nexusmods](https://www.nexusmods.com/blackmythwukong/mods/1172)

Switch language: [English](README.en.md)

## 本分支做了什么

游戏 1.0.20 更新（2025-10-13）同时触发了两个**互相独立**的故障：

**一、插件完全不工作** —— exe 重新链接后 SDK 偏移全部失效。已重新生成 SDK，
并把全局地址改为**运行时特征码扫描**而不是编译期写死，对后续小版本更新的耐受度更高。

另外新增了走过路线的记录（按区域存盘，大小地图都会画出来），
以及小地图上的周边目标显示（敌人红点、中立灰点，目标死亡后自动消失）。

**二、小地图底图花屏** —— 这个和插件代码无关，是新版游戏的 D3D12 环境触发了
渲染库里一个一直存在的问题。触发条件经双向对照实验确认为**纹理完全不透明（alpha=255）**，
已规避。

顺带修了四个渲染库的通用 bug、把常驻内存从 368MB 降到 16MB、安装包从 96MB 降到 23MB。

## 更新日志

- v2.2（本分支）
  - 修复跨层换图时卡顿 1~3 秒（浮屠界最明显）：底图改为后台解码
  - 红点区分高度：空心=下方、实心=同层、实心加外环=上方
  - 红点区分战斗状态：已经发现你的敌人会呼吸
  - 新增地上物品显示：掉落物、采集物等，`7`
- v2.1（本分支）
  - 小地图新增周边目标显示：敌人红点 `8`、中立灰点 `Shift` + `8`
- v2.0（本分支）
  - 适配 1.0.20 之后的游戏版本
  - 修复小地图底图花屏
  - 新增地图朝向模式：`Shift` + `0`
  - 新增走过路线的记录与显示：`9` / `Shift` + `9`
  - 设置（窗口大小、比例、朝向、路线开关）自动保存，重进游戏保留
  - 路线颜色可在配置文件里自定义
  - 提示文字改用系统中文字体
  - 大地图右侧新增按键说明面板
  - 地图改为按需加载，常驻内存 368MB → 16MB
  - 地图重采样至 2000×2000，安装包 96MB → 23MB
- v1.7（上游）
  - 调整 UI，添加大量点位
- v1.6（上游）
  - 修复 AMD 显卡渲染问题，添加点位

## 按键说明

按 `Tab` 打开大地图后，右侧会列出下面这些按键，不用回来翻文档。

- `+` 放大 小地图窗口
- `-` 缩小 小地图窗口
- `Shift` + `+` 放大 小地图比例
- `Shift` + `-` 缩小 小地图比例
- `0` 显示/隐藏 地图
- `Shift` + `0` 切换地图朝向模式 **（本分支新增）**
  - 默认：地图不动，箭头随人物转向
  - 切换后：箭头锁定朝上，地图随人物转向
- `9` 开关走过的路线 **（本分支新增）**——关闭后既不显示也不记录
- `Shift` + `9` 清除全部路线 **（本分支新增）**——不可撤销，需要在 3 秒内按第二次确认
- `7` 开关地上的掉落物 / 采集物 **（本分支新增）**——默认关闭
- `8` 开关周边敌人的红点 **（本分支新增）**
- `Shift` + `8` 开关中立/友方的灰点 **（本分支新增）**——默认关闭

小地图上这些记号用两条互不干扰的通道表示：

| | 含义 |
|---|---|
| **圆形** | 角色（敌人、NPC） |
| **菱形** | 地上的东西（掉落物、采集物等） |
| **空心** | 在你**下方**一层 |
| **实心** | 和你**同层** |
| **实心 + 外环** | 在你**上方**一层 |

颜色再区分具体类别：红=敌人，灰=中立，金=掉落物，绿=采集物，白=其它可交互物。
运行时发现的土地庙，如果内置点位表里没有，会用同样的传送点图标补上。

**已经发现你的敌人，红点会缩小再弹回，循环。**小地图是用余光看的，
而余光对颜色迟钝、对运动敏感 —— 所以战斗状态用动画而不是颜色来表示。

上面这些设置（窗口大小、比例、地图朝向、路线开关）会存进 dll 同目录的
`wukong_minimap_config.json`，下次进游戏自动恢复。删掉这个文件即可恢复默认。

配置文件里还有三项颜色，只能手改：

```json
"trail_color":    "#22E0FFCC",
"enemy_color":    "#C2352BCC",
"alert_color":    "#FF4438FF",
"neutral_color":  "#C8C8C8A0",
"drop_color":     "#FFC93CEE",
"collect_color":  "#5BD86BE0",
"interact_color": "#E0E0E0B4"
```

`#RRGGBB` 或 `#RRGGBBAA`（后两位是不透明度）。改完重进游戏生效；
写错格式会退回内置颜色，并在日志里说明是哪一项。

路线按地图区域分别记录，保存在 dll 同目录的 `wukong_minimap_trails.json`，
下次启动自动载入。想备份就复制这个文件；想手动清空，删掉它即可。
每 30 秒、以及切换区域时落盘，所以游戏崩溃最多丢失最近 30 秒的路线。

## 演示截图

![alt text](./docs/demo0.png)
![alt text](./docs/demo1.png)
![alt text](./docs/demo2.png)

## 安装说明

将 `wukong-minimap.zip` 直接解压至黑神话的安装文件夹下面的 `b1\Binaries\Win64` 中
（steam 的安装文件夹可以通过右键黑神话 -> 管理 -> 浏览本地文件找到）

![alt text](./docs/install0.png)

本插件包含以下文件：

- `wukong_minimap.dll` 插件功能核心文件
- `dwmapi.dll` 加载器 - 通过代理系统功能来加载 wukong_minimap.dll
- `maps` 地图文件夹

## 使用 UE4SS 的用户

由于 ue4ss 自带的 `dwmapi.dll` 拦截了系统 api 会导致插件无法顺利加载，
我们使用 wukong-minimap 中的 dwmapi.dll 就行了。

## 卸载

删除 `wukong_minimap.dll` 文件即可

## 从源码构建

需要 Windows + MSVC（含 C++ 工具集）+ CMake + Rust（`x86_64-pc-windows-msvc`）。

```powershell
# 构建、安装到游戏目录、打包
.\build.ps1 -Package -Install "D:\Games\Steam\steamapps\common\BlackMythWukong\b1\Binaries\Win64"
```

`check_offsets.ps1` 可以在不启动游戏的情况下检查当前 exe 的特征码是否仍然有效——
**下次游戏更新后如果插件失效，先跑这个**：它会告诉你是只需重新编译，还是需要重新
dump SDK。

## 遇到问题

插件会在 dll 同目录写 `wukong_minimap.log`。需要详细日志时，启动游戏前设
`RUST_LOG=debug`。反馈问题时请附上这个文件。

## 许可与致谢

本项目继承上游的 **Apache License 2.0**。原始版权归 [@jaskang](https://github.com/jaskang) 所有。

- [jaskang/wukong-minimap](https://github.com/jaskang/wukong-minimap) — 原项目（Apache-2.0）
- [hudhook](https://github.com/veeenu/hudhook) — 注入与渲染框架（MIT，Andrea Venuta），本仓库内含经修改的 vendored 副本
- [imgui](https://github.com/ocornut/imgui)
- [Dumper-7](https://github.com/Encryqed/Dumper-7) — 生成 UE SDK

`maps/` 下的地图素材源自上游仓库，本分支仅做了重采样。
