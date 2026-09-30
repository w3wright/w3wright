# w3wright

[English](README.md) | **中文**

Warcraft III 地图开发平台，Rust 实现。

本仓库是 `docs/` 里那套设计的实现。设计文档是权威，本 README 只说明代码在哪、
怎么跑、现在到哪一步。

> 目标游戏版本：**1.27**
> CLI 二进制：**`war3`**，crate 前缀：**`war3-*`**

---

## 当前进度

Phase 1 覆盖 W3X 解析器：打开一张 `.w3x`，读出里面的元数据类与地形类文件。

| Crate | 状态 | 内容 |
| --- | --- | --- |
| `war3-core` | ✅ | `FourCC`、`Vec3`、错误与诊断机制、可注入资产源 |
| `war3-mpq` | ✅ | 头扫描、表解密、哈希查找、枚举阶梯、扇区解密、自写 inflate |
| `war3-map` | ✅ | `.w3i`（版本 0–33）、`.wts`、`.imp`、Map 组合模型 |
| `war3-terrain` | ✅ | `.w3e` 的 v11 与 v12 两种布局，以及 SYLK 解析 |
| `war3-meta` | ✅ | SYLK 元数据表与 `TriggerData.txt` 触发定义 |
| `war3-cli` | ✅ | `war3` 二进制 |
| `war3-object` | ⬜ | 对象数据（`.w3u` 等） |
| `.w3x` 写回 | ⬜ | build 方向 |

**192 个单测全绿，`cargo clippy --all-targets` 无警告。** 除本工作区内的 crate
外没有任何依赖 —— 连 zlib 解压都是自己实现的，因此核心能编到 WASM，也能用纯
Rust 工具链构建。

---

## 快速开始

```bash
cargo build --release

# 地图信息 —— Phase 1 的主要动作
./target/release/war3 map info "<地图>"

# 归档结构 —— 地图读不出来时先跑这个
./target/release/war3 map archive "<地图>"

# 地形统计
./target/release/war3 map terrain "<地图>"

# 成员清单，以及导出单个成员
./target/release/war3 map list "<地图>"
./target/release/war3 map file "<地图>" war3map.w3i > w3i.bin

# 检查本地安装里能否找到元数据与触发定义
./target/release/war3 meta check "D:\Warcraft3"
```

加 `--verbose` 会把 info 级诊断也打出来；默认只显示 warning 及以上。

**退出码**：`0` 成功，`1` 成功但有诊断，`2` 用法错误或读不了输入。

---

## 目录结构

```text
w3wright/
├── Cargo.toml                 # 虚拟清单，members = ["crates/*"]
├── crates/
│   ├── war3-core/             # FourCC / Vec3 / 错误 / 诊断 / AssetSource
│   ├── war3-mpq/              # MPQ 读取 + 自写 inflate
│   ├── war3-map/              # .w3i / .wts / .imp + Map 组合模型
│   ├── war3-terrain/          # .w3e v11 与 v12 + SYLK
│   ├── war3-meta/             # 对象字段元数据 + 触发定义
│   └── war3-cli/              # 伞包，提供 war3 二进制
├── examples/                  # 测试地图样本（二进制被 .gitignore 排除）
└── README.md, README_CN.md
```

### 依赖方向

```text
war3-cli          ← 伞包，唯一提供二进制的包
  ├── war3-map      → war3-terrain   （Map 模型持有地形）
  ├── war3-terrain
  ├── war3-meta     → war3-terrain   （复用 SYLK 解析器）
  ├── war3-mpq
  └── war3-core
```

`war3-cli` 依赖其他；其他包都不依赖 `war3-cli`。

---

## 几条代码约定

这些不是风格偏好，每条都对应一种**静默失败** —— 解析器报成功，但产出错误数据。

1. **绝不静默丢弃。** 读不懂的字段、与文件不符的版本号、缺失的元数据，一律
   产生诊断，而不是跳过。
2. **先验证几何，再信任版本字段。** `.w3e` 的记录长度由文件长度与点位数量推
   出来，再与版本字段比对。只信版本字段会损坏 v12 文件。
3. **先按无符号读，再掩码。** 水位字是承载 14 位水位的 `u16`，先读 `i16`
   再掩码会踩符号扩展。
4. **保留语义未知的字节。** 保留位在往返中原样写回，不做归一化。
5. **两套坐标系要分开。** MPQ 表偏移相对头起点，成员偏移相对文件起点。混用
   会得到整体偏移 512 字节的数据，而且不报错。
6. **没有理由就不加依赖。** core、terrain、meta 三个包零依赖；mpq 连 inflate
   都自己实现，以保证工作区是纯 Rust 且能到达 WASM。

---

## 测试数据

`examples/lost-temple/` 放地图样本，`examples/fresh/` 留给**确认未被改动过**的地图。
**地图二进制不进 Git**：官方地图属于暴雪，别的项目里附带的样本图带有各自的许可，
社区地图需要作者同意才能再分发。

有一份样本是本地生成的：
`cargo run --example make_synthetic -p war3-mpq` 会写出一份结构合法的 MPQ 归档 ——
加密的表、三种存储布局、且**故意不写 `(listfile)`**。它的作用是把"读取器写错了"
与"这份文件特殊"分开。详见
[`examples/lost-temple/README.md`](examples/lost-temple/README.md)。

---

## 已知问题：本机安装的表解密

用合成归档走完整条读取路径是可以的：头扫描、表解密、哈希查找、名字枚举、
扇区解密、zlib 解压，产出的内容与写进去的完全一致。crypt 表与文件名哈希也与
独立实现逐值吻合。

但本机的**真实文件** —— `D:\Warcraft3\` 下的地图，以及游戏自己的 `war3.mpq`、
`War3x.mpq`、`War3xLocal.mpq` —— 哈希表与块表解密出来是随机字节。决定性证据：
`war3.mpq` 的 32768 项哈希表里**空槽数为 0**，而合法归档里绝大多数应当是空槽
（合成归档是 61/64）。

也就是说，这些文件的表不是按格式文档描述的方式存放的。可能是地图保护工具、
repack 工具，或者文档未涵盖的密钥派生步骤。**需要一份干净的样本才能继续**，
[`examples/fresh/README.md`](examples/fresh/README.md) 说明了怎么造，以及该看什么。

这不阻塞 Phase 1 的其余部分：各格式解析器都有基于自造字节流的单测。缺的只是
"对着真实地图端到端跑一遍"。
