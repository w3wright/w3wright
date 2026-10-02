# w3wright

[English](README.md) | **中文**

Warcraft III 地图开发平台，Rust 实现。

本仓库是 `docs/` 里那套设计的实现。设计文档是权威，本 README 只说明代码在哪、
怎么跑、现在到哪一步。

> **注意**：`docs/` 并不在仓库里。它被称作权威设计，却从未提交；这是个该补的
> 缺口 —— 下面那些约定目前是其中一部分唯一的书面记录。

> 目标游戏版本：**1.27**
> CLI 二进制：**`war3`**，crate 前缀：**`war3-*`**

---

## 当前进度

Phase 1 覆盖 W3X 解析器：打开一张 `.w3x`，读出里面的元数据类、地形类与对象类文件。
真实地图与真实暴雪归档现在都能端到端读通。归档的**写侧也已可用**；
缺的是"源工程"那个方向。

| Crate | 状态 | 内容 |
| --- | --- | --- |
| `war3-core` | ✅ | `FourCC`、`Vec3`、错误与诊断机制、可注入资产源 |
| `war3-archive` | ✅ | 暴雪归档**读取与写入**：头扫描、表解密、哈希查找、枚举阶梯、扇区解密、自写 inflate、`ArchiveBuilder` |
| `war3-map` | ✅ | `.w3i`（版本 0–33）、`.wts`、`.imp`、`war3map.doo`、`war3mapUnits.doo`、Map 组合模型 |
| `war3-terrain` | ✅ | `.w3e` 的 v11 与 v12 两种布局，以及 SYLK 解析 |
| `war3-meta` | ✅ | 从游戏安装的 `*MetaData.slk` 读对象字段元数据、`TriggerData.txt` 触发定义、编辑器数据类型表 |
| `war3-object` | ✅ | 对象数据（`.w3u` 等）：继承、字段类型、`TRIGSTR_` 解析，**并能写回** —— 真实语料上 **7 种对象文件 137/137 个成员逐字节一致**（`cargo run --example objects_roundtrip -p war3-object -- <地图目录>`） |
| `war3-project` | ⚠️ 部分 | 源工程：`war3 map extract` / `war3 validate` / `war3 build`。`war3map.w3i`、**7 种对象文件与 `war3map.doo`** 已文本化（改单位字段、装饰物坐标或地图描述，游戏能看到）；其余成员是解码后二进制或原样块；**成员只有在文本形式被证明能逐字节还原之后才会文本化** |
| `war3-cli` | ✅ | `war3` 二进制 |
| `.w3x` 往返 | ✅ 成员内容 | `war3 map rebuild` 逐成员回写归档；`extract` / `build` 让每个成员的内容逐字节往返 |
| `war3-wtg` | ⬜ | 触发器数据（`.wtg` / `.wct`）：原型范围只做无损往返（ADR-0016） |

**255 个测试全绿（`cargo test --workspace`），`cargo clippy --all-targets` 无警告。**
除本工作区内的 crate 外没有任何依赖 —— 连 zlib 解压都是自己实现的，因此核心能编到
WASM，也能用纯 Rust 工具链构建。

### 现在能读出什么

| 目标 | 结果 |
| --- | --- |
| `D:\Warcraft3\war3.mpq` | 哈希表解出（32768 槽里 22084 个空槽）；块表里 **10684 个成员** |
| `D:\Warcraft3\War3xlocal.mpq` | 1133 个成员 |
| `(4)LostTemple.w3m` | 按名字列出 16 个成员；`.w3i` v18、`.w3e` v11（161x161 点位）、`war3map.j` 72697 字节 JASS、**5317 个装饰物**；它的 `war3mapUnits.doo` 是 **v7/sub9，仍未解出** —— 报出偏移而不是猜 |
| `ydwe-sample-1.19.w3x` | `.w3i` v25、`.w3e` v11、`.w3u`、**125 个已放置单位**，以及经字符串表解析出的非 ASCII 地图名（`YDWE的UI演示`） |
| `war3 map rebuild` | `(4)LostTemple.w3m` → **16/16 成员逐字节一致**（源 245.2 KB，产物 238.5 KB） |
| `.w3i` 重序列化 | **190 张图里 188 张 parse → 序列化逐字节一致**（`cargo run --example w3i_roundtrip -p war3-map -- <地图目录>`）；剩下的 2 张是 `war3map.w3i` 被 implode 压着 |
| 放置数据重序列化 | `war3map.doo` **188/188 逐字节一致**（735,618 个装饰物）；`war3mapUnits.doo` **175/175**（26,826 个单位）：`cargo run --release --example doodads_roundtrip -p war3-map -- <地图目录>` |
| 源工程往返 | 整条 `extract → build` 链跑遍本机所有地图，逐图打印成员去向与每一条拒绝理由：`cargo run --release --example project_regression -p war3-project -- <地图目录>` |
| `war3 meta check D:\Warcraft3` | 7 张字段元数据表（单位 267 字段、技能 747 字段…）与 164 种触发类型 / 1389 个动作 |

对 `war3map.w3i`、`war3map.w3e`、`war3map.j` 的导出与一份独立实现逐字节一致。

**尚未支持：PKWare implode**（`0x08` 扇区），地图的 `war3map.wts` 与暴雪自己的
`(listfile)` 都用它。Huffman、bzip2、ADPCM 同样会被如实报告为不支持。详见
[`examples/lost-temple/README.md`](examples/lost-temple/README.md)。
imploded 成员在重写时不会丢：它被原样搬运；依赖它的 `TRIGSTR_` 引用保持未解析
并报出诊断。本机 193 张图实测：**5 张因 `(listfile)` 被 implode 压着而完全枚举不出来**，
另有 13 张因 listfile 缺失或被削减（见 `docs/` 的 Q20）。

---

## 快速开始

```bash
cargo build --release

# 地图信息 —— Phase 1 的主要动作
./target/release/war3 map info "<地图>"

# 归档结构 —— 地图读不出来时先跑这个
./target/release/war3 map archive "<地图>"

# 地形统计、已放置的装饰物、已放置的单位
./target/release/war3 map terrain "<地图>"
./target/release/war3 map doodads "<地图>"
./target/release/war3 map units "<地图>"

# 对象数据。给了游戏目录，字段 ID 会变成可读名；不给就打成 FourCC。
./target/release/war3 map objects "<地图>" --game-dir "D:\Warcraft3"

# 成员清单，以及导出单个成员
./target/release/war3 map list "<地图>"
./target/release/war3 map file "<地图>" war3map.w3i > w3i.bin

# 用本工作区的写入器重写归档，并校验产物
./target/release/war3 map rebuild "<地图>" out.w3x

# 源工程：地图变目录，再变回地图
./target/release/war3 map extract "<地图>" my-project/
./target/release/war3 validate my-project/          # 只读：一次报出所有问题，有问题退出码 2
./target/release/war3 build my-project/ --out rebuilt.w3x

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
│   ├── war3-archive/          # 暴雪归档读写 + 自写 inflate
│   ├── war3-map/              # .w3i / .wts / .imp / .doo / war3mapUnits.doo + Map 组合模型
│   ├── war3-terrain/          # .w3e v11 与 v12 + SYLK
│   ├── war3-meta/             # 对象字段元数据 + 触发定义
│   ├── war3-object/           # 对象数据（.w3u 等）
│   ├── war3-project/          # 源工程：extract / build
│   └── war3-cli/              # 伞包，提供 war3 二进制
├── examples/                  # 测试地图样本（二进制被 .gitignore 排除）
└── README.md, README_CN.md
```

### 依赖方向

```text
war3-cli  ← 伞包，唯一提供二进制的包
  ├── war3-map     → war3-archive, war3-terrain, war3-core
  ├── war3-project → war3-archive, war3-core
  ├── war3-object  → war3-meta, war3-core
  ├── war3-meta    → war3-terrain, war3-core
  ├── war3-archive → war3-core
  └── war3-terrain → war3-core
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
5. **格式里每个偏移都相对归档头。** 这既包括头里的表偏移，也包括块表项里的
   成员偏移；而 `HM3W` 前缀让头通常落在 512 而不是 0。读成员时不加上头偏移，
   会拿到整体错位 512 字节的数据，而且不报错。
6. **绝不要用加解密互证。** 两者是严格互逆的一对，写入方与读取方犯同一个错误时
   往返完全正确。要把密文固定值对着真实归档钉死 —— 这就是 `war3-archive` 的测试
   里带固定向量的原因。
7. **没有理由就不加依赖。** core、terrain、meta 三个包零依赖；mpq 连 inflate
   都自己实现，以保证工作区是纯 Rust 且能到达 WASM。

---

## 测试数据

`examples/lost-temple/` 放地图样本，`examples/fresh/` 留给**确认未被改动过**的地图。
**地图二进制不进 Git**：官方地图属于暴雪，别的项目里附带的样本图带有各自的许可，
社区地图需要作者同意才能再分发。

有一份样本是本地生成的：
`cargo run --example make_synthetic -p war3-archive` 会写出一份结构合法的 MPQ 归档 ——
加密的表、四种存储布局、真正被 deflate 压缩的扇区与按原样存储的扇区，且**故意不写
`(listfile)`**。详见
[`examples/lost-temple/README.md`](examples/lost-temple/README.md)。

---

## 关于本 README 曾经的误判

本文件与 `examples/lost-temple/README.md` 的早期版本断言：本机所有真实文件都被
保护或重打包过，锅在地图保护工具或某个未记载的密钥派生步骤，并且**"需要一份干净
样本才能继续"**。这是错的。文件本身完全正常，是读取器有四个缺陷，而且每一个都
是静默失败：

1. 两张表用了 ASCII 常量（`HASH`、`BLK#`）当密钥，而格式要求的是
   `HashString("(hash table)", MPQ_HASH_FILE_KEY)`，块表同理用 `(block table)`；
2. 加解密的状态递推把明文卷进了 key，而 key 应当走自己独立的序列；
3. 成员偏移被当成相对文件，实际是相对归档头；
4. 把扇区首字节 `0xFF` 当作"已压缩"标记；格式里没有这个字节，判定方式是拿扇区的
   存储长度与它必须承载的数据长度比较。

当时"这些文件不正常"的证据是：哈希表解出来 32768 个槽里 0 个空槽，而合法表应当
绝大多数是空槽。**这个测量是对的，推论是错的** —— 那正是密钥错误会产生的现象。
决定性的对照样本其实就在仓库里却帮不上忙：生成器与读取器共用同一套加密和同样的
密钥，而旧 README 甚至已经警告过这个风险，只是没意识到它恰好适用于自己正在用的
那个检查。唯一能定论的是真实归档，它们现在就是参照物：实测证据见
[`examples/lost-temple/README.md`](examples/lost-temple/README.md)，
防止复发的固定向量在 `war3_archive::crypto` 的测试里。
