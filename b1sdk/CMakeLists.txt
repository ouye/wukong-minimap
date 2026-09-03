# Hand-written replacement for the cmkr-generated CMakeLists.txt.
# (The original bootstrapped cmkr from the network on every configure; this
#  one is self-contained and lets you skip the parts that make MSVC crawl.)

cmake_minimum_required(VERSION 3.20)
project(b1sdk VERSION 1.0.0 LANGUAGES CXX)

if(NOT MSVC)
    message(FATAL_ERROR "b1sdk must be built with MSVC (the Dumper-7 SDK uses __declspec / MSVC intrinsics).")
endif()

# ---------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------
# The generated SDK has 317 .cpp files. We only need the packages whose
# functions b1sdk.cpp actually calls. Flip this ON if the Rust link step
# reports unresolved externals.
option(B1SDK_FULL "Compile every generated SDK translation unit (slow: ~70 MB of source)" OFF)

# Dumper-7's Assertions.inl is 21 MB and is pulled into every translation
# unit through Basic.hpp. It only checks the SDK against itself, never
# against the running game, so it is stubbed out by default.
option(B1SDK_ASSERTS "Compile Dumper-7's full Assertions.inl (very slow)" OFF)

set(SDK_DIR "${CMAKE_CURRENT_SOURCE_DIR}/src/SDK/SDK")

if(B1SDK_FULL)
    file(GLOB SDK_SOURCES "${SDK_DIR}/*.cpp")
    message(STATUS "b1sdk: FULL build - every generated translation unit (this will take a while)")
else()
    # Basic.cpp                -> GObjects / FName / class lookup plumbing
    # CoreUObject_functions    -> UObject::ProcessEvent and friends
    # Engine_functions         -> UWorld::GetWorld, UEngine::GetEngine,
    #                             UGameplayStatics::*, AActor::K2_GetActor*
    # b1_functions             -> UBGUFunctionLibrary::GetPlayerCharacter
    # UnrealExtent_functions   -> UGSE_EngineFuncLib::SetInputMode*
    set(SDK_SOURCES
        "${SDK_DIR}/Basic.cpp"
        "${SDK_DIR}/CoreUObject_functions.cpp"
        "${SDK_DIR}/Engine_functions.cpp"
        "${SDK_DIR}/b1_functions.cpp"
        "${SDK_DIR}/UnrealExtent_functions.cpp"
    )
    foreach(f ${SDK_SOURCES})
        if(NOT EXISTS "${f}")
            message(FATAL_ERROR "Missing SDK source: ${f}\nDid the package names change in a newer dump? Configure with -DB1SDK_FULL=ON to compile everything.")
        endif()
    endforeach()
    message(STATUS "b1sdk: minimal build (5 SDK translation units). Use -DB1SDK_FULL=ON if you hit unresolved externals.")
endif()

add_library(b1sdk STATIC
    "${CMAKE_CURRENT_SOURCE_DIR}/src/b1sdk.cpp"
    ${SDK_SOURCES}
)

target_include_directories(b1sdk PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/src")

target_compile_features(b1sdk PUBLIC cxx_std_23)

target_compile_options(b1sdk PUBLIC
    /GS-        # no buffer security checks - we run inside someone else's process
    /bigobj     # the SDK blows past the default section limit
    /EHa
    /MP         # parallel compilation

    # CRITICAL: b1sdk.cpp / b1sdk.h carry Chinese comments and are UTF-8
    # without a BOM. On a Chinese-locale Windows, MSVC decodes the source as
    # cp936 (GBK); a UTF-8 Chinese comment then leaves a dangling trail byte
    # that pairs with the newline and swallows the FOLLOWING line of code.
    # That is not a warning you can ignore -- it silently deletes statements
    # (it ate `SDK::FVector Location = ...`, giving C2065 three lines later).
    /utf-8

    /wd4369     # Dumper-7 emits enum values that overflow their underlying type
    /wd4005     # WIN32_LEAN_AND_MEAN redefined between stdafx.h and Basic.hpp
)

# NOTE: WIN32_LEAN_AND_MEAN is deliberately NOT defined here -- both stdafx.h
# and the generated Basic.hpp define it themselves, and defining it on the
# command line as well produces C4005 for every translation unit.
target_compile_definitions(b1sdk PUBLIC
    NOMINMAX
    _CRT_SECURE_NO_WARNINGS
)

if(NOT B1SDK_ASSERTS)
    target_compile_definitions(b1sdk PUBLIC DUMPER7_NO_ASSERTS)
endif()

# The Rust side links against the MSVC toolchain's default CRT (/MD).
set_property(TARGET b1sdk PROPERTY MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")
