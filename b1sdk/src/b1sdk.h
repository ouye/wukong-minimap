#pragma once

// 使用纯 C 风格结构体，避免使用 std::string
extern "C" struct PlayerInfo
{
  float x;
  float y;
  float z;
  float angle;
  uint8_t bIsLocalViewTarget;
  uint8_t bShowMouseCursor;
  uint8_t bIsMoveInputIgnored;
  char level[256]; // 固定大小的字符数组替代 std::string
};

// ActorDot::kind 的取值，和 Rust 侧的 DotKind 一一对应。
#define DOT_KIND_HOSTILE 0    // 敌对角色
#define DOT_KIND_NEUTRAL 1    // 中立或友方角色
#define DOT_KIND_DROP 2       // 掉落物
#define DOT_KIND_COLLECT 3    // 采集物
#define DOT_KIND_MEDITATION 4 // 土地庙
#define DOT_KIND_INTERACT 5   // 其它可交互物

// ActorDot::flags 的位。
#define DOT_FLAG_IN_BATTLE 0x01 // 该角色处于战斗状态（已经发现玩家）

// 玩家附近的一个目标。坐标和 PlayerInfo 同一套世界坐标系。
extern "C" struct ActorDot
{
  float x;
  float y;
  float z;
  uint8_t kind;
  uint8_t flags;
};

// 使用 __cdecl 调用约定
extern "C" __declspec(dllexport) void b1Init(void);
extern "C" __declspec(dllexport) bool toggleMouseCursor(bool show);
extern "C" __declspec(dllexport) PlayerInfo getPlayerInfo(void);
// 收集玩家周围 radius（世界单位）内的目标，写进 out，返回写入的个数。
//
// 距离判定分开两个方向：radius 管水平，zLimit 管垂直。竖直塔（浮屠界）在
// 俯视投影上是一个细圆筒，只按水平距离筛的话每一层的 actor 都会落进范围里。
//
// kindMask 是 (1 << DOT_KIND_*) 的按位或，只收集要的类别 —— 关掉的类别一次
// IsA 都不会做。出任何意外都返回 0，不会抛异常，也不会让调用方看到半截数据。
extern "C" __declspec(dllexport) int32_t getNearbyActors(ActorDot *out, int32_t maxCount, float radius, float zLimit, uint32_t kindMask);
