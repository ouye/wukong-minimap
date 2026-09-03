// This file was modified in a fork of jaskang/wukong-minimap.
//
// Upstream: https://github.com/jaskang/wukong-minimap (Apache-2.0)
// Fork:     https://github.com/Ouye/wukong-minimap
//
// Changes: package headers renamed for the newer Dumper-7 output; b1Init
// falls back to the dumped offsets when a signature scan fails and logs
// what it resolved; added a startup smoke test.
//
// See CHANGES.md for the full record of what was changed and why.

#include "stdafx.h"
#include <cstdarg>
#include "helper.hpp"
// Dumper-7 renamed the "b1-Managed" package headers; UBGUFunctionLibrary now
// lives in b1_classes.hpp and GSE_EngineFuncLib in UnrealExtent_classes.hpp.
#include "SDK/SDK/b1_classes.hpp"
#include "SDK/SDK/UnrealExtent_classes.hpp"

#include "b1sdk.h"

extern "C" __declspec(dllexport) bool toggleMouseCursor(bool show)
{
	SDK::UWorld *World = SDK::UWorld::GetWorld();
	if (!World)
	{
		printf_s("World is null\n");
		return true;
	}
	SDK::UGameplayStatics *GameplayStatics = SDK::UGameplayStatics::GetDefaultObj();
	if (!GameplayStatics)
	{
		printf_s("GameplayStatics is null\n");
		return true;
	}
	SDK::APlayerController *playerController = GameplayStatics->GetPlayerController(World, 0);
	if (!playerController)
	{
		printf_s("playerController is null\n");
		return true;
	}
	playerController->bShowMouseCursor = show ? 1 : 0;
	if (show)
	{
		SDK::UGSE_EngineFuncLib::SetInputModeUIOnly(playerController, nullptr, SDK::EMouseLockMode::DoNotLock);
	}
	else
	{
		SDK::UGSE_EngineFuncLib::SetInputModeGameOnly(playerController);
	}
	return show;
}

extern "C" __declspec(dllexport) PlayerInfo getPlayerInfo()
{
	PlayerInfo info = {
			-1.0f, // x
			-1.0f, // y
			-1.0f, // z
			0.0f,	 // angle
			0,		 // bIsLocalViewTarget
			1,		 // bShowMouseCursor
			1,		 // bIsMoveInputIgnored
			""};

	SDK::UWorld *World = SDK::UWorld::GetWorld();
	if (!World)
	{
		printf_s("World is null\n");
		return info;
	}
	SDK::UGameplayStatics *GameplayStatics = SDK::UGameplayStatics::GetDefaultObj();
	if (!GameplayStatics)
	{
		printf_s("GameplayStatics is null\n");
		return info;
	}

	SDK::ACharacter *playerCharacter = SDK::UBGUFunctionLibrary::GetPlayerCharacter(World);
	if (!playerCharacter)
	{
		printf_s("playerCharacter is null\n");
		return info;
	}

	SDK::APlayerController *playerController = GameplayStatics->GetPlayerController(World, 0);
	if (!playerController)
	{
		printf_s("playerController is null\n");
		return info;
	}

	// 获取当前关卡名称
	std::string currentLevelName(GameplayStatics->GetCurrentLevelName(World, false).ToString());
	strncpy_s(info.level, sizeof(info.level), currentLevelName.c_str(), _TRUNCATE);
	// 获取位置和角度
	SDK::FVector Location = playerCharacter->K2_GetActorLocation();
	SDK::FRotator Rotator = playerCharacter->K2_GetActorRotation();
	info.x = Location.X;
	info.y = Location.Y;
	info.z = Location.Z;

	info.angle = Rotator.Yaw;

	info.bIsLocalViewTarget = playerCharacter->bIsLocalViewTarget;

	info.bShowMouseCursor = playerController->bShowMouseCursor;

	info.bIsMoveInputIgnored = playerCharacter->IsMoveInputIgnored();

	return info;
}

// Values from the Dumper-7 dump in "5.0.0-0+++UE5+Release-5.0-b1" (2025-11).
// Only used as a fallback when the signature scan fails.
static const int32_t kDumpedGObjects     = 0x1D47ED10;
static const int32_t kDumpedAppendString = 0x0CB65030;
static const int32_t kDumpedProcessEvent = 0x0CCFA2F0;

static void b1Log(const char *fmt, ...)
{
	char buf[512];
	va_list args;
	va_start(args, fmt);
	vsnprintf_s(buf, sizeof(buf), _TRUNCATE, fmt, args);
	va_end(args);
	OutputDebugStringA(buf);
	printf_s("%s", buf);
}

extern "C" __declspec(dllexport) void b1Init()
{
	HMODULE baseModule = GetModuleHandle(NULL);
	b1Log("[b1sdk] b1Init(), base = 0x%p\n", (void *)baseModule);

	// ---- GObjects -----------------------------------------------------
	uint8_t *GObjectsScanResult = Memory::PatternScan(baseModule, "48 8B ?? ?? ?? ?? ?? 48 8B ?? ?? 48 8D ?? ?? EB ?? 33 ?? 8B ?? ?? C1 ??");
	if (GObjectsScanResult)
	{
		uintptr_t GObjectsAddr = Memory::GetAbsolute((uintptr_t)GObjectsScanResult + 0x3);
		SDK::Offsets::GObjects = (int32_t)((uintptr_t)GObjectsAddr - (uintptr_t)baseModule);
		b1Log("[b1sdk] GObjects     = 0x%X (scanned)\n", SDK::Offsets::GObjects);
	}
	else
	{
		SDK::Offsets::GObjects = kDumpedGObjects;
		b1Log("[b1sdk] GObjects     = 0x%X (SCAN FAILED, using dumped value -- expect trouble)\n", SDK::Offsets::GObjects);
	}

	// ---- FName::AppendString ------------------------------------------
	uint8_t *AppendStringScanResult = Memory::PatternScan(baseModule, "48 89 ?? ?? ?? 57 48 83 ?? ?? 80 3D ?? ?? ?? ?? 00 48 8B ?? 48 8B ?? 74 ?? 4C 8D ?? ?? ?? ?? ?? EB ?? 48 8D ?? ?? ?? ?? ?? E8 ?? ?? ?? ?? 4C ?? ?? C6 ?? ?? ?? ?? ?? 01 8B ?? 8B ?? 0F ?? ?? C1 ?? 10 89 ?? ?? ?? 89 ?? ?? ?? 48 8B ?? ?? ?? 48 ?? ?? ?? 8D ?? ?? 49 ?? ?? ?? ?? 48 8B ?? E8 ?? ?? ?? ?? 83 ?? ?? 00");
	if (AppendStringScanResult)
	{
		SDK::Offsets::AppendString = (int32_t)((uintptr_t)AppendStringScanResult - (uintptr_t)baseModule);
		b1Log("[b1sdk] AppendString = 0x%X (scanned)\n", SDK::Offsets::AppendString);
	}
	else
	{
		SDK::Offsets::AppendString = kDumpedAppendString;
		b1Log("[b1sdk] AppendString = 0x%X (SCAN FAILED, using dumped value -- expect trouble)\n", SDK::Offsets::AppendString);
	}

	// ---- ProcessEvent --------------------------------------------------
	// Not actually used by the SDK (it calls ProcessEvent through the vtable
	// via Offsets::ProcessEventIdx); kept for diagnostics / future hooking.
	uint8_t *ProcessEventScanResult = Memory::PatternScan(baseModule, "40 ?? 56 57 41 ?? 41 ?? 41 ?? 41 ?? 48 ?? ?? ?? ?? ?? ?? 48 8D ?? ?? ?? 48 89 ?? ?? ?? ?? ?? 48 8B ?? ?? ?? ?? ?? 48 ?? ?? 48 89 ?? ?? ?? ?? ?? 4D ?? ?? 48 ?? ?? 4C ?? ?? 48 ?? ??");
	if (ProcessEventScanResult)
	{
		SDK::Offsets::ProcessEvent = (int32_t)((uintptr_t)ProcessEventScanResult - (uintptr_t)baseModule);
		b1Log("[b1sdk] ProcessEvent = 0x%X (scanned)\n", SDK::Offsets::ProcessEvent);
	}
	else
	{
		SDK::Offsets::ProcessEvent = kDumpedProcessEvent;
		b1Log("[b1sdk] ProcessEvent = 0x%X (scan failed, using dumped value)\n", SDK::Offsets::ProcessEvent);
	}

	// ---- smoke test ----------------------------------------------------
	// If the member offsets in the SDK no longer match the game, this is
	// where it usually shows up first.
	SDK::UWorld *World = SDK::UWorld::GetWorld();
	b1Log("[b1sdk] UWorld::GetWorld() = 0x%p\n", (void *)World);
	if (World)
	{
		b1Log("[b1sdk] world name = %s\n", World->GetName().c_str());
	}
}
