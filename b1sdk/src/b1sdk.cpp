// This file was modified in a fork of jaskang/wukong-minimap.
//
// Upstream: https://github.com/jaskang/wukong-minimap (Apache-2.0)
// Fork:     https://github.com/Ouye/wukong-minimap
//
// Changes: package headers renamed for the newer Dumper-7 output; b1Init
// falls back to the dumped offsets when a signature scan fails and logs
// what it resolved; added a startup smoke test; added getNearbyActors, which
// collects nearby characters and interactables for the minimap radar.

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


// ------------------------------------------------------------------ 雷达 ---
//
// 遍历已加载关卡的 actor 列表，挑出玩家附近值得画在小地图上的东西。
//
// 角色的继承链（来自 SDK dump）：
//     ACharacter -> ABGUCharacter -> ABGUCharacterCS -> ABGU_CharacterAI
//                                                        -> ABGUPlayerCharacterCS
// 可交互物的：
//     ABGUInteractiveActorBase -> ABGUDropItemActor      掉落物
//                              -> ABGUCollectionBase     采集物
//                              -> ABGUMeditationPointBase 土地庙
//                              （其余归入 DOT_KIND_INTERACT）
//
// 三个刻意的选择：
//
// 1. 直接读 ULevel::Actors，不用 UGameplayStatics::GetAllActorsOfClass。后者每次
//    调用都会通过 ProcessEvent 返回一个新的 TArray，而 Dumper-7 的 TArray 没有
//    析构函数 —— 每秒调几次，一路漏下去。直接遍历零分配。
//
// 2. 坐标读 RootComponent->RelativeLocation，不调 K2_GetActorLocation()。后者是
//    UFunction，每个 actor 一次 ProcessEvent；根组件没有父级，它的相对坐标就是
//    世界坐标。
//
// 3. 先按 kindMask 和距离筛，再调 BGUIsUnitDead / BGUIsEnemyTeam / IsUnitInBattle。
//    这三个才是 ProcessEvent，只对进了范围的少数几个调用。
//
// 关卡流式加载和 GC 随时可能让某个 actor 指针失效，所以整个遍历包在 SEH 里
// （CMakeLists 里开了 /EHa），出事就返回 0，宁可这一帧不画也不能崩游戏。

namespace
{
	SDK::UClass *g_AICharClass = nullptr;
	SDK::UClass *g_PlayerCharClass = nullptr;
	SDK::UClass *g_InteractiveClass = nullptr;
	SDK::UClass *g_DropItemClass = nullptr;
	SDK::UClass *g_CollectionClass = nullptr;
	SDK::UClass *g_MeditationClass = nullptr;

	SDK::UObject *g_FuncLibCDO = nullptr;
	SDK::UFunction *g_IsUnitDead = nullptr;
	SDK::UFunction *g_IsEnemyTeam = nullptr;

	SDK::UObject *g_TestLibCDO = nullptr;
	SDK::UFunction *g_IsUnitInBattle = nullptr;

	bool g_RadarResolved = false;
	bool g_CharsUsable = false;
	bool g_ItemsUsable = false;

	// 参数布局照抄 Dumper-7 生成的 Params 结构，见 *_parameters.hpp。
	struct Params_OneActor
	{
		SDK::AActor *Unit;   // 0x00
		uint8_t ReturnValue; // 0x08 (bit 0)
	};

	struct Params_TwoActors
	{
		SDK::AActor *SelfUnit;  // 0x00
		SDK::AActor *OtherUnit; // 0x08
		uint8_t ReturnValue;    // 0x10 (bit 0)
	};

	void resolveRadarSymbols()
	{
		if (g_RadarResolved)
			return;
		g_RadarResolved = true;

		g_AICharClass = SDK::UObject::FindClassFast("BGU_CharacterAI");
		g_PlayerCharClass = SDK::UObject::FindClassFast("BGUPlayerCharacterCS");
		g_InteractiveClass = SDK::UObject::FindClassFast("BGUInteractiveActorBase");
		g_DropItemClass = SDK::UObject::FindClassFast("BGUDropItemActor");
		g_CollectionClass = SDK::UObject::FindClassFast("BGUCollectionBase");
		g_MeditationClass = SDK::UObject::FindClassFast("BGUMeditationPointBase");

		if (SDK::UClass *funcLib = SDK::UObject::FindClassFast("BGUFunctionLibraryCS"))
		{
			g_FuncLibCDO = funcLib->DefaultObject;
			g_IsUnitDead = funcLib->GetFunction("BGUFunctionLibraryCS", "BGUIsUnitDead");
			g_IsEnemyTeam = funcLib->GetFunction("BGUFunctionLibraryCS", "BGUIsEnemyTeam");
		}

		// 战斗状态在自动化测试的辅助库里，不是正经游戏逻辑用的接口。
		// 拿不到就退化成"不知道有没有发现你"，不影响其它部分。
		if (SDK::UClass *testLib = SDK::UObject::FindClassFast("AutoTestHelperLib"))
		{
			g_TestLibCDO = testLib->DefaultObject;
			g_IsUnitInBattle = testLib->GetFunction("AutoTestHelperLib", "IsUnitInBattle");
		}

		g_CharsUsable = g_AICharClass && g_PlayerCharClass && g_FuncLibCDO && g_IsUnitDead;
		g_ItemsUsable = g_InteractiveClass != nullptr;

		b1Log("[b1sdk] radar: AI=%p Player=%p Interactive=%p Drop=%p Collect=%p Meditation=%p\n",
					(void *)g_AICharClass, (void *)g_PlayerCharClass, (void *)g_InteractiveClass,
					(void *)g_DropItemClass, (void *)g_CollectionClass, (void *)g_MeditationClass);
		b1Log("[b1sdk] radar: IsUnitDead=%p IsEnemyTeam=%p IsUnitInBattle=%p -> chars %s, items %s\n",
					(void *)g_IsUnitDead, (void *)g_IsEnemyTeam, (void *)g_IsUnitInBattle,
					g_CharsUsable ? "usable" : "DISABLED", g_ItemsUsable ? "usable" : "DISABLED");
	}

	bool callOneActor(SDK::UObject *cdo, SDK::UFunction *func, SDK::AActor *actor)
	{
		Params_OneActor parms{};
		parms.Unit = actor;
		cdo->ProcessEvent(func, &parms);
		return (parms.ReturnValue & 1) != 0;
	}

	bool isEnemyTeam(SDK::AActor *self, SDK::AActor *other)
	{
		if (!g_IsEnemyTeam)
			return true; // 拿不到这个函数就一律当敌对，总比一个都不显示强
		Params_TwoActors parms{};
		parms.SelfUnit = self;
		parms.OtherUnit = other;
		g_FuncLibCDO->ProcessEvent(g_IsEnemyTeam, &parms);
		return (parms.ReturnValue & 1) != 0;
	}

	// 判断 actor 属于哪一类。不要的返回 false。
	bool classify(SDK::AActor *actor, uint32_t kindMask, SDK::AActor *player,
								uint8_t &kind, uint8_t &flags)
	{
		flags = 0;

		if (g_CharsUsable && actor->IsA(g_AICharClass))
		{
			if (actor->IsA(g_PlayerCharClass))
				return false;
			if (callOneActor(g_FuncLibCDO, g_IsUnitDead, actor))
				return false;

			kind = isEnemyTeam(player, actor) ? DOT_KIND_HOSTILE : DOT_KIND_NEUTRAL;
			if (((kindMask >> kind) & 1) == 0)
				return false;

			if (kind == DOT_KIND_HOSTILE && g_IsUnitInBattle &&
					callOneActor(g_TestLibCDO, g_IsUnitInBattle, actor))
				flags |= DOT_FLAG_IN_BATTLE;
			return true;
		}

		if (g_ItemsUsable && actor->IsA(g_InteractiveClass))
		{
			// 已经采过、捡过的东西通常被隐藏而不是销毁。
			if (actor->bHidden)
				return false;

			if (g_DropItemClass && actor->IsA(g_DropItemClass))
				kind = DOT_KIND_DROP;
			else if (g_CollectionClass && actor->IsA(g_CollectionClass))
				kind = DOT_KIND_COLLECT;
			else if (g_MeditationClass && actor->IsA(g_MeditationClass))
				kind = DOT_KIND_MEDITATION;
			else
				kind = DOT_KIND_INTERACT;

			return ((kindMask >> kind) & 1) != 0;
		}

		return false;
	}

	// 扫一个关卡，结果累加到 count 上。
	// 不要在这里放需要析构的 C++ 对象 —— 调用方用 SEH 包着。
	void scanLevel(SDK::ULevel *level, SDK::AActor *player, uint32_t kindMask,
								 double px, double py, double pz, double r2, double zLimit,
								 ActorDot *out, int32_t maxCount, int32_t &count)
	{
		if (!level)
			return;

		const int32_t total = level->Actors.Num();
		for (int32_t i = 0; i < total && count < maxCount; ++i)
		{
			SDK::AActor *actor = level->Actors[i];
			if (!actor || actor == player)
				continue;

			SDK::USceneComponent *root = actor->RootComponent;
			if (!root)
				continue;

			// 距离先筛，省下后面的 IsA 和 ProcessEvent。水平和垂直分开判：
			// 浮屠界那种竖直塔在俯视投影上是个细圆筒，只按水平距离筛的话
			// 每一层的 actor 都会落进范围，白白做几百次 ProcessEvent。
			const double dz = root->RelativeLocation.Z - pz;
			if (dz < -zLimit || dz > zLimit)
				continue;
			const double dx = root->RelativeLocation.X - px;
			const double dy = root->RelativeLocation.Y - py;
			if (dx * dx + dy * dy > r2)
				continue;

			uint8_t kind = 0;
			uint8_t flags = 0;
			if (!classify(actor, kindMask, player, kind, flags))
				continue;

			out[count].x = (float)root->RelativeLocation.X;
			out[count].y = (float)root->RelativeLocation.Y;
			out[count].z = (float)root->RelativeLocation.Z;
			out[count].kind = kind;
			out[count].flags = flags;
			++count;
		}
	}
}

extern "C" __declspec(dllexport) int32_t getNearbyActors(ActorDot *out, int32_t maxCount, float radius, float zLimit, uint32_t kindMask)
{
	if (!out || maxCount <= 0 || kindMask == 0)
		return 0;

	resolveRadarSymbols();
	if (!g_CharsUsable && !g_ItemsUsable)
		return 0;

	SDK::UWorld *World = SDK::UWorld::GetWorld();
	if (!World)
		return 0;

	SDK::ACharacter *player = SDK::UBGUFunctionLibrary::GetPlayerCharacter(World);
	if (!player || !player->RootComponent)
		return 0;

	const double px = player->RootComponent->RelativeLocation.X;
	const double py = player->RootComponent->RelativeLocation.Y;
	const double pz = player->RootComponent->RelativeLocation.Z;
	const double r2 = (double)radius * (double)radius;
	const double zCap = (double)zLimit;

	int32_t count = 0;
	__try
	{
		scanLevel(World->PersistentLevel, player, kindMask, px, py, pz, r2, zCap, out, maxCount, count);

		const int32_t streamCount = World->StreamingLevels.Num();
		for (int32_t i = 0; i < streamCount && count < maxCount; ++i)
		{
			SDK::ULevelStreaming *streaming = World->StreamingLevels[i];
			if (streaming)
				scanLevel(streaming->LoadedLevel, player, kindMask, px, py, pz, r2, zCap, out, maxCount,
									count);
		}
	}
	__except (EXCEPTION_EXECUTE_HANDLER)
	{
		// 关卡正在换、对象刚被 GC 掉之类。丢掉这一帧的结果就好。
		b1Log("[b1sdk] radar: exception while scanning actors, skipping this refresh\n");
		return 0;
	}

	return count;
}
