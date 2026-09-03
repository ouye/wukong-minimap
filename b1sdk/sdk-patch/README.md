# SDK 补丁覆盖层

`b1sdk/src/SDK` 是 `5.0.0-0+++UE5+Release-5.0-b1/CppSDK` 的副本，由 `build.ps1`
在构建前自动生成，**不提交进仓库**（1561 个文件、约 70MB，与仓库里已有的那份重复）。

本目录存放需要覆盖上去的少量文件：

- `SDK/Basic.hpp` — 偏移改回 `inline` 以便运行时特征码扫描；`GWorld` 置 0；
  断言可通过 `DUMPER7_NO_ASSERTS` 停用
- `AssertionsStub.inl` — 25843 个空的 `DUMPER7_ASSERTS_*` 宏。Dumper-7 原本的
  `Assertions.inl` 有 21MB 且会被 include 进每一个翻译单元，编译极慢；
  这些断言只校验 SDK 自身的一致性，与运行中的游戏无关

重新 dump SDK 之后，把新的 `CppSDK` 放到仓库根目录，删掉 `b1sdk/src/SDK`，
重新跑 `build.ps1` 即可——它会重新复制并覆盖这两个文件。
