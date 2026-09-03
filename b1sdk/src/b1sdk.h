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

// 使用 __cdecl 调用约定
extern "C" __declspec(dllexport) void b1Init(void);
extern "C" __declspec(dllexport) bool toggleMouseCursor(bool show);
extern "C" __declspec(dllexport) PlayerInfo getPlayerInfo(void);