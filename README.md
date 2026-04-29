# nba3k

NBA 2K MyGM 风格的命令行总经理模拟器。Rust 实现，CLI + REPL + 只读 TUI 三种交互方式，单文件 SQLite 存档。

> 个人 / 非商业项目。NBA 数据来自 Basketball-Reference 公开页面，仅做引导式抓取并本地缓存，不做再分发。

## 快速开始

### 1. 环境要求

- macOS / Linux
- Rust ≥ 1.80（推荐通过 `rustup` 安装）
- Python 3 + `pip install nba_api`（仅在重建种子时需要，已自带的 `data/seed_2025_26.sqlite` 不需要）
- SQLite 已经被 `rusqlite = bundled` 静态链接，无需系统 SQLite

### 2. 编译

```bash
git clone https://github.com/jiaweizhang1995/nba3k.git
cd nba3k
cargo build --release
```

二进制产物在 `target/release/nba3k`。开发期可直接 `cargo run -- ...` 或 `cargo build` 后用 `target/debug/nba3k`。

### 3. 创建第一份存档并开局

**默认即今日实况起手**（M34）。`new` 不带任何起手 flag 就会从 ESPN 公开 JSON 接口拉今天的真实联盟状态：30 队当前 W-L、已打比赛比分、当前阵容（含赛季内的交易/签约）、当前伤病、本季 PPG/RPG 累计、剩余真实赛程。

```bash
# 默认：今日实况起手（NBA 2K MyNBA "Today's Game" 风格）
./target/release/nba3k --save my.db new --team BOS

# 离线 / 测试模式：跳过 ESPN，从 2025-10-21 开档（旧默认行为）
./target/release/nba3k --save my.db new --team BOS --offline

# 看一眼初始状态
./target/release/nba3k --save my.db status

# 进入 REPL 慢慢玩
./target/release/nba3k --save my.db
```

也可以直接进 TUI 仪表盘：

```bash
./target/release/nba3k --save my.db tui
```

**联网要求**：默认路径需要能访问 `https://site.api.espn.com/`。首跑约 30-60 秒，缓存命中后秒级；缓存写到 `data/cache/espn/`，TTL 6-12 小时。脱网会立刻报错并清掉半成品文件。CI / 离线场景请加 `--offline`。

**纯 Rust，不依赖 Python / nba_api**。

`--from-today` 旧 flag 已 deprecated（保留为 no-op 不破坏脚本）。

#### 已知短板（`phases/M33-tui-and-polish.md` 跟踪）

- **没有 Cup 回填**：选秀进档时杯赛阶段已结束，`cup` 命令为空。
- **News 只回填 30 天交易类**：完整赛季 news 不回灌（避免刷爆 ESPN）。
- **过去比赛只有最终比分**：没有 per-player 单场 box score；`recap`-类命令在历史比赛上为空。M31 的 `player_season_stats` 表填补了 PPG/RPG 排行榜需求。
- **球员名 exact 匹配**：少数后缀变体（Jr/Sr/III）做了 strip 重试；G-League / two-way 球员若 seed 没有则跳过 + 打 warn。
- **`records --scope season`** 当前还从 box-score 聚合，对今日实况存档为空。后续打补丁让它优先读 `player_season_stats`。
- **赛季已结束**：当 ESPN 显示常规赛全部完成（如 4 月底），`phase` 落到 `Playoffs`。季后赛对阵 bracket 不自动回填；`playoffs sim` 仍可手动跑出冠军。

### 4. 删除存档

存档就是单个 SQLite 文件（连同 `-shm` / `-wal` 副本）。两种删除方式：

**方式 A：用 CLI 自带命令（带二次确认）**

```bash
./target/release/nba3k saves list                 # 列出当前目录 + /tmp 下的存档
./target/release/nba3k saves show --path my.db    # 看存档元信息
./target/release/nba3k saves delete --path my.db --yes   # --yes 是必需的安全开关
```

**方式 B：直接删文件**

```bash
rm my.db my.db-shm my.db-wal      # 三个文件一起删，不留 WAL 残留
```

存档之间相互独立，删一份不会影响其他文件。`data/seed_2025_26.sqlite` 是只读的 league 种子，**不要删**——所有 `new` 都从它复制。

## 三种交互方式

### CLI 子命令（脚本友好）

每个读类命令都有 `--json` 开关，方便和 `jq` 串联。

```bash
nba3k --save my.db sim-day --days 5
nba3k --save my.db standings --json | jq '.east[0]'
nba3k --save my.db trade propose --from BOS --to LAL --send "Jaylen Brown" --recv "LeBron James"
```

### REPL（交互式）

```bash
nba3k --save my.db
> roster
> sim-week
> messages
> trade list
> quit
```

REPL 用 `rustyline` 提供历史 / 编辑功能。同一个 `Command` 枚举既解析 argv 也解析 REPL 行（通过 `shlex`），所以 CLI 能干的 REPL 都能干。

### 脚本模式

```bash
nba3k --save my.db --script my_season.txt
# 或者
echo "sim-to season-end\nstandings" | nba3k --save my.db
```

### TUI（M20 起：可玩，TV 模式）

```bash
nba3k tui                    # 没存档自动跳新游戏向导
nba3k --save my.db tui       # 进入主菜单
nba3k --save my.db tui --tv  # 高对比 TV 调色板 + 大间距，沙发上看清楚
nba3k --save my.db tui --legacy  # 回退到 M19 5-tab 只读旧版
```

基于 `ratatui 0.29` + `crossterm 0.28`，单二进制无外部依赖。

**主菜单 7 项**（`↑` `↓` 切换 / `1`-`7` 直跳 / `Enter` 进入）：

| # | 菜单 | 状态 | 内容 |
|---|------|------|------|
| 1 | Home | M20 完工 | 仪表盘：老板任务 + 今日比赛 + GM 收件箱（球员不满 / 报价 / 收藏）+ 最近联盟新闻 |
| 2 | Roster | M21 完工 | 阵容管理：My Roster / Free Agents 两 tab；排序 `o/p/a/s`（OVR/位置/年龄/薪资）；行动作 `t` 训练 / `e` 续约 / `x` 裁员 / `R` 改角色；`Enter` 看详情（数据/生涯/合同/化学反应 4 块）；FA tab `s` 签人 |
| 3 | Rotation | M21 完工 | 首发 5 人位置指派（Level A）：5 槽 PG/SG/SF/PF/C；`Enter` 选人（按相邻位置过滤，OVR 降序）；`c` 清单槽 / `C` 清空；自动覆盖 sim 引擎，bench + 分钟仍自动 |
| 4 | Trades | M22 完工 | 4 tab：Inbox / My Proposals / Builder / Rumors；`a/r/c` 接受 / 拒绝 / 反报价；Builder 支持 2 队球员交易提案 |
| 5 | Draft | M22 完工 | 选秀板 / 顺位两 tab；球探雾 `???`；`s` 球探 / `Enter` 选人 / `A` 自动选秀（仅选秀期可执行） |
| 6 | Finance | M22 完工 | 薪资、奢侈税、第一/第二 apron、最低工资线、合同表；排序 `t/y/n`；`e` 续约 |
| 7 | Calendar | M20 完工 | 7×6 月历 + 6 子页（赛程/排名/季后赛/奖项/全明星/Cup）|

**Calendar 子页按键**：

- `Space` 模拟 1 天
- `W` 模拟 1 周
- `M` 模拟 1 月
- `Enter` 在高亮事件日 → 模拟到那天（all-star / cup-final / trade-deadline / season-end）
- `A` 推进到下个赛季（带确认）
- `Tab` / `Shift+Tab` 切子页
- `[` `]` 换月（只看不动时间）

**全局按键**：

- `↑` `↓` 导航
- `Enter` 选择 / 进入
- `Esc` 返回上一层
- `Ctrl+S` 打开存档管理浮层（列表 / 新建 / 加载 / 删除 / 导出）
- `?` 打开当前页面按键帮助浮层
- `q` 退出（带确认）

**新游戏向导**（无存档启动时自动触发）：保存路径 → 球队 → 模式 → 赛季 → 种子 → 确认。

**约束**：

- 终端宽度 < 80 列时显示「请放大窗口」占位
- TUI 现已可玩。但范围只覆盖 7 个菜单项；菜单外的 CLI 功能（compare / records / hof / coach 等）仍要回 CLI 用。
- `--legacy` 旗保留 M19 旧 5-tab 只读版（v3.0 polish 才会移除）

## 命令清单（CLI / REPL 通用）

> 下面所有命令在 CLI（`nba3k --save x.db <cmd>`）和 REPL（`> <cmd>`）下都能用。

### 存档与流程

| 命令 | 作用 |
|------|------|
| `new --team BOS` | 从种子建新档，选定一支球队当玩家 GM |
| `load <path>` | 加载存档（`--save` 已经指定路径时是 no-op）|
| `status` | 当前赛季 / 比赛日 / 阶段 / 模式 |
| `save` | 显式 flush（SQLite 自动持久化，这里是占位）|
| `quit` / `exit` | 退出 REPL |
| `saves list/show/delete/export` | 存档管理 + JSON 导出 |

### 模拟时间

| 命令 | 作用 |
|------|------|
| `sim-day --days N` | 模拟 N 天 |
| `sim-week` / `sim-month` | 模拟 7 / 30 天，遇到交易报价或自队球员伤病自动暂停（`--no-pause` 关闭）|
| `sim-to <target>` | 模拟到目标。阶段：`regular` / `regular-end` / `playoffs` / `trade-deadline` / `offseason`；标记日：`all-star`（第 41 天）/ `cup-final`（第 55 天）/ `season-end`（自动跑季后赛 + 翻页 OffSeason）|
| `season-advance` | 推进到下个赛季（球员成长 + 自动选秀 + FA 桩）|

### 阵容与球员

| 命令 | 作用 |
|------|------|
| `roster [--team BOS]` | 看球队阵容（默认你的队）|
| `roster-set-role <player> <role>` | 标记角色：star / starter / sixth / role / bench / prospect |
| `player <name>` | 模糊匹配单个球员详情 |
| `chemistry --team BOS` | 球队化学反应分解 |
| `career <name>` | 球员历年生涯数据 |
| `training <name> --attr shoot` | 训练营加点：shoot / inside / def / reb / ath / handle（每赛季每人一次）|

### 交易

| 命令 | 作用 |
|------|------|
| `trade propose --from BOS --to LAL --send "A,B" --recv "C"` | 发起两队交易 |
| `trade propose3 --leg BOS:A --leg LAL:B --leg DAL:C` | 三方交易（M10）|
| `trade list` | 进行中的谈判 |
| `trade respond <id> <accept\|reject\|counter>` | 回应反报价 |
| `trade chain <id>` | 看完整谈判链 |
| `offers` | AI 主动发来的报价收件箱 |
| `rumors` | 全联盟交易传闻（AI 兴趣信号）|
| `messages` | GM 收件箱：球星不满 / 阵容警报 |

支持 2025-26 真实 CBA：薪资匹配、奢侈税 / 第一第二土豪线、交易加薪 (kicker) 不对称、不可交易条款 (NTC)。**God 模式**（`--god` 或建档时选）跳过 CBA 与拒绝逻辑。

### 球队管理

| 命令 | 作用 |
|------|------|
| `cap [--team BOS]` | 工资 / 奢侈税 / 各条线状态 |
| `extend <player> --salary 25 --years 4` | 自队球员续约谈判（士气影响接受率）|
| `fa list` | 自由球员市场 |
| `fa sign <name>` | 签自由球员 |
| `fa cut <name>` | 裁人。新赛季开赛前如果你的大名单超过 15 人，sim-day 会被拦下，必须先 `fa cut` 到 15 才能开赛 |
| `coach show/fire/pool` | 主教练查看 / 解雇 / 候选池 |

### 选秀

| 命令 | 作用 |
|------|------|
| `draft board` | 60 人新秀板凳 |
| `draft order` | 当年选秀顺位（含彩票排序）|
| `draft sim` | AI 一键完成整个选秀 |
| `draft pick <name>` | 你的顺位人工选人 |
| `picks [--team BOS] [--season 2027]` | 查看某队持有的未来选秀权 |
| `scout <name>` | 花一次球探名额揭示新秀真实评分（每赛季 5 次）|

### 选秀权交易

新档会写入未来 7 年、两轮、30 队的选秀权表。`new` 默认先写入 vanilla 自有签，再尝试从 Spotrac future draft picks 页面覆盖真实交易/互换信息；Spotrac 失败不会导致开档失败，会保留 vanilla 自有签并记录 warning。`new --offline` 只写 vanilla 自有签。

可用 `data/pick_swaps_overrides.toml` 手工覆盖 `(year, original_team, round)` 行。交易命令支持 `--send-picks 2027-R1-BOS` / `--receive-picks 2028-R2-LAL`，并会执行七年规则与 Stepien 规则。

**TUI 表现**：

- **Trades / Builder**：左右两栏球员列表下方各有 Picks 区，光标在球员区走完后会进入 Picks 区，`Space` 选中。每个 pick 旁边有 1-5 星评级（`★★★★☆`），算法看 round + 距今年数 + protection 等级 + Spotrac prose 关键字。Spotrac 标记为 "Not Tradable" / "FROZEN PICK" 的 pick 显示 `🔒 frozen`，**God 模式下** 这个锁失效，所有 pick 都按正常星级显示并可交易（God 模式本来就跳过 CBA 校验）。
- **Roster** 屏：`1` 我方阵容 / `2` Picks 子页；按 year/round/origin 排序，列出 own 还是 via X，protection 列原样保留 prose 文本。
- **Draft Order** 屏：交易过的顺位会在 VIA 列显示 `via NYK`，OWNER 列指向当前持有方而不是原球队。

### 季后赛与赛季奖项

| 命令 | 作用 |
|------|------|
| `playoffs bracket` | 首轮对阵图 |
| `playoffs sim` | 一键模拟整个季后赛 |
| `season-summary` | 总冠军 + 总决赛 MVP + 全奖项打包 |
| `awards [--season YYYY]` | 赛季末 MVP / DPOY / ROY / 6MOY / MIP / All-NBA |
| `awards-race` | 赛季中段奖项榜（前 5 + 票数）|
| `all-star [--season YYYY]` | 全明星阵容 + 比赛结果 |
| `cup [--season YYYY]` | 季中 NBA Cup 杯赛对阵 + 结果 |

### 联盟历史与排行

| 命令 | 作用 |
|------|------|
| `standings [--season YYYY]` | 东西部排名（支持回看历年）|
| `news [--limit N]` | 联盟动态：交易 / 签约 / 退役 / 伤病 / 奖项 |
| `recap --days N` | 最近 N 天比赛回顾（含每场最佳得分手）|
| `compare BOS LAL` | 两队并排对比：薪资 / 前 8 / 化学反应 |
| `records --scope season\|career --stat ppg` | 排行榜：ppg / rpg / apg / spg / bpg / three_made / fg_pct |
| `hof` | 名人堂（退役球员按生涯产出排）|
| `retire <name>` | 强制退役 |
| `mandate` | 老板下达的赛季目标 + 评分 |

### 笔记

| 命令 | 作用 |
|------|------|
| `notes add <player> --comment "..."` | 收藏球员 + 备注 |
| `notes remove <player>` | 取消收藏 |
| `notes list` | 看所有收藏（也会出现在 `messages` 里）|

### 开发 / 调试

| 命令 | 作用 |
|------|------|
| `dev calibrate-trade` | 跨 GM 配对随机交易评估，校准用 |
| `dev team-strength <abbrev>` | 球队 9 维实力向量 + 派生 ORtg / DRtg |

## 游戏模式

建档时通过 `--mode` 指定，或运行时 `--god` 强制覆盖：

- **standard**：完整 2025-26 CBA。薪资必须匹配、不可交易条款生效、AI 会拒绝亏本交易。默认。
- **god**：跳过 CBA 校验、AI 强制接受、可手动改评分 / 合同 / 抽彩票。
- **hardcore**：限制更严的 standard（保留位）。
- **sandbox**：跳过校验但保留 AI 拒绝（保留位）。

## 大名单规则

NBA 2025-26 CBA 把大名单分成两个窗口；`nba3k` 按当前赛季阶段分别生效：

| 阶段 | 大名单上限（每队） | 备注 |
|------|--------------------|------|
| OffSeason / FreeAgency / Draft / PreSeason | **21** | 训练营窗口，模拟 NBA 训练营+常规合同+双向合同+Exhibit 10 名额（本档暂不区分双向合同，统一计 21）|
| Regular / TradeDeadlinePassed / Playoffs | **18** | 15 标准 + 3 双向；交易后名单必须落在 13-18 |

**新赛季开赛门**：从 PreSeason 翻 Regular 时，如果你的大名单超过 **15 人**，`sim-day` 会被拦下：

```
regular season start blocked: BOS has 16 players (limit 15).
Cut a player with `fa cut <name>` until you are at 15. AI teams are not checked.
```

用 `fa cut <name>` 减到 15 即可开赛。AI 队不在这个门里——他们可以带着 16 人进入常规赛，本档不做 AI 自动裁员（`--god` / `sandbox` 跳过校验）。

## 数据来源

- 球员名单 + 历史数据：Basketball-Reference 公开页面（限速 1 req / 3s，仅引导抓取，本地缓存到 `data/cache/`）
- 2026 选秀新秀：公开 mock draft（Cooper Flagg 等）
- CBA 数字：写死 2025-26 工资帽 / 奢侈税 / 第一土豪线 / 第二土豪线 / 中产 / BAE
- 合同：scraper 落库后用合成的按 OVR 分级合约回填（HoopsHype 改成 React 渲染了）

种子产物 `data/seed_2025_26.sqlite` 已被 `.gitignore` 排除。要重新生成种子：

```bash
cargo run -p nba3k-scrape --release -- --out data/seed_2025_26.sqlite
```

## 项目结构

```
crates/
  nba3k-core/        # 公共类型：Player / Team / LeagueYear / TradeOffer / LeagueSnapshot
  nba3k-models/      # 7 个可解释的评分模型（球员价值 / 合同 / 球队语境 / 球星保护 / 阵容契合 / 交易接受 / 数据投射）
  nba3k-sim/         # 比赛模拟引擎（统计分布；逐回合留作 v2）+ 9 维球队实力向量
  nba3k-trade/       # 交易引擎（评估 + CBA 校验 + 性格 + 多轮谈判）
  nba3k-season/      # 赛程生成 / 季后赛 / 奖项 / 名人堂
  nba3k-store/       # SQLite 持久层 + refinery 迁移
  nba3k-scrape/      # 引导抓取 + 评分校准
  nba3k-cli/         # CLI 解析 + REPL + TUI + 命令实现
data/
  archetype_profiles.toml   # 10 种球员原型
  personalities.toml        # 30 队 GM 性格
  realism_weights.toml      # 模型权重
  star_roster.toml          # 24 队 / 28 个球星标签
  rating_overrides.toml     # 手维护的合同特殊条款
  sim_params.toml           # 模拟引擎参数
phases/                     # M1-M19 的开发日志
```

## 测试

```bash
cargo test --workspace
```

当前 275 个单测 + 1 个集成测试通过。

## 已完成里程碑

M1 骨架 → M2 数据 + 模拟 → M3 交易引擎 → M4 评分模型 → M5 21 维属性 + 化学反应 + 季后赛 → M6 选秀 + 休赛期 → M7 整赛季 e2e → M8 真实年龄曲线 → M9 交易评估器 → M10 三方交易 + 生涯 + FA + 训练 → M11 合同 + 退役 → M12 联盟经济 → M13 联盟生命力（伤病 + 新闻 + 奖项榜）→ M14 元游戏（教练 + 球探 + 排行榜）→ M15 全明星 + 历史回看 + 存档管理 → M16 NBA Cup + 传闻 + 队伍对比 → M17 GM 工具（报价 + 续约 + 笔记）→ M18 老板任务 + 复盘 + 导出 → M19 TUI 仪表盘。

## 已知短板

- 没有「看比赛」体验：当前是统计分布出最终比分，没有逐回合 / 逐场流播放。
- 部分球队战绩失真：年初阵容快照导致 KD-on-PHO 等不合时宜的强阵；CLE 因主控位置识别不准受影响。
- 没有受限自由球员（RFA）/ 资格报价 / Bird 权 / 签换 / 合同回购 / 交易特例的细节流程。
- 教练只有 overall 与解雇阈值，没有体系树 / 助教 / 训练加成。
- 老板只下任务，不会因为成绩差炒掉 GM。

下一阶段方向：M20 看比赛逐节 box score / M21 RFA 全套 / M22 逐回合模拟试点。

## 许可

MIT。仅个人非商业用途。NBA、所有球队、球员名称归各自版权方所有，本项目不附属于、不被 NBA / 2K Games 背书。
