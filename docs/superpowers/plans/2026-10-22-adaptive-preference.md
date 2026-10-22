# 自适应偏好模型 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让输入法对「你选过什么」有带时间衰减的记忆，并让词排序、英文 / emoji 摆放、整句让位都读它。

**Architecture:** 学习器里把 `choices` 从「次数」扩成偏好条目（`count` 长期次数、`dee` 提交时钟衰减的短期权重、`tick`、`fresh` 自启用以来的次数），经 `Learner` trait 暴露两个只读查询（`choice_stats`、`input_evidence`）。排序把短期权重折进竞争圈用的次数；英文 / emoji 降级做成所有候选插入完之后的一次重排；整句让位复用已有的 `behind_spelled` 机制。总开关 `adaptive_learning` 关掉时三处全退回现状。

**Tech Stack:** Rust（edition 2024，toolchain 1.96.0，见 `rust-toolchain.toml`）；`qingjian-learning`、`qingjian-core`、`qingjian-platform`、`apps/cli`、`apps/settings`、`apps/macos`。

**Spec:** `docs/superpowers/specs/2026-10-22-adaptive-preference-design.md`

## Global Constraints

- 衰减时间常数 **200 次提交**：`dee = 1 + 旧dee × exp(−(clock − tick) / 200)`；读取时 `dee × exp(−(clock − tick) / 200)`。
- 降级：该输入串证据 `Σ fresh ≥ 5` 且候选 `fresh == 0`（英文词另要求全局 `weight(word) == 0`），移到前 **3 个**「汉字候选」（`Chinese`、`Sentence`、`Code`、`Cloud`）之后；不足 3 个放在最后一个汉字候选之后；排第 0 位的英文候选不降级。
- 整句只让给「这个输入串下 `count > 0`、且音节连写等于整段输入」的汉字词。
- 配置 `[general] adaptive_learning`，缺省 `true`；设置程序有开关；关掉时排序、降级、整句让位全部等于现状，记录照常进行。
- **升级当天行为等于现状**：老数据读入后 `dee = 0`、`fresh = 0`。
- `user-choices.tsv` 追加列后旧版本仍能读（旧解析只取前三列、跳过 `#` 行）；全局时钟存在文件头注释。
- 词频的 90 天衰减不动；`MAX_CHOICE_ENTRIES = 50_000` 沿用。
- 每键总耗时不超过 10 ms（长输入基线约 7 ms）。
- 仓库约定：代码标识符英文、注释与文档中文；新文件有 `//!` 文件头；单文件不超过 800 行；不用 `use super::*`（测试模块除外）；提交信息 `类型(范围): 中文说明`；不改 `CHANGELOG.md`；不写装饰性分隔注释。用户仓库**没有启用提交钩子**，每个任务的提交前手动跑 `cargo fmt --all` 与 `cargo clippy --workspace --all-targets -- -D warnings`。

## Review Focus

1. **损坏 / 异常的 `user-choices.tsv`**：列数不够、`dee` 不是数字、`tick` 大于文件头时钟、空文件。期望：坏行跳过并计数，`tick > clock` 按 0 差处理，不 panic。（Task 2）
2. **私密输入时的记录**：密码框、已判定为助记词的输入下，英文 / emoji 选择不被记录。（Task 3）
3. **撤销**：上屏英文词 / emoji 后立刻退格重选，它们的记录退回，`dee` 近似回退。（Task 3）
4. **候选很少 / 页容量很小**：汉字候选不足 3 个、`page_size = 1` 时降级不越界、不 panic。（Task 6）
5. **开关关闭的等价性**：`adaptive_learning = false` 时同一份数据的候选顺序与升级前逐项相同。（Task 5、6、7）

---

### Task 1: 偏好条目与衰减（学习器内存部分）

**Files:**
- Create: `crates/qingjian-learning/src/frequency_learner/preference.rs`
- Modify: `crates/qingjian-core/src/engine/learning/learner.rs`（trait 新增两个方法与 `ChoiceStats`）、`crates/qingjian-core/src/engine/learning/mod.rs`（导出 `ChoiceStats`）、`crates/qingjian-learning/src/frequency_learner/mod.rs`（`choices` 类型、`clock` 字段）、`.../tables.rs`、`.../learner_impl.rs`
- Test: `crates/qingjian-learning/src/frequency_learner/tests.rs`

**Interfaces:**
- Produces（core，`qingjian_core`）：
  `pub struct ChoiceStats { pub count: u32, pub recent: f32, pub fresh: u32 }`（`#[derive(Debug, Clone, Copy, Default, PartialEq)]`）；
  `Learner::choice_stats(&self, input: &str, text: &str) -> ChoiceStats`（默认 `ChoiceStats::default()`）；
  `Learner::input_evidence(&self, input: &str) -> u32`（默认 0）。
- Produces（learning）：`pub(super) struct ChoiceEntry { pub count: u32, pub dee: f32, pub tick: u64, pub fresh: u32 }`；`pub(super) const TIME_CONSTANT: f64 = 200.0`；`ChoiceEntry::recent_at(&self, clock: u64) -> f32`；`ChoiceEntry::bump(&mut self, clock: u64, fresh: bool)`。
- 现有 `choice_weight(input, text) -> u32` 语义不变（返回 `count`）。

- [ ] **Step 1: 写失败测试** 在 `tests.rs` 加 `choice_recent_decays_by_the_commit_clock`：对同一输入串先选 `又` 8 次，`recent` 应接近 8 的指数累加值；再对别的词记 400 次后 `又` 的 `recent` 应小于 1（`exp(−400/200) ≈ 0.135` 乘以累加值）。再加 `choice_weight_still_counts_forever`：同样操作后 `choice_weight` 仍为 8。再加 `unrecord_rolls_back_count_and_fresh_and_approximates_recent`：记录一次后 `unrecord_choice`，`count` 与 `fresh` 各减 1、`recent` 变为 `max(0, 原值 − 1)`、时钟不回拨。再加 `input_evidence_ignores_raw_and_reading_keys`：`record_raw("hello")` 与 `record_choice("wod", "wode", "我的")` 之后，`input_evidence("hello") == 0`、`input_evidence("wod") == 1`、`input_evidence("wode") == 0`。
- [ ] **Step 2: 运行确认失败** `cargo test -p qingjian-learning choice_recent_decays` 预期编译失败（`choice_stats` 未定义）。
- [ ] **Step 3: 实现** 在 `preference.rs` 定义 `ChoiceEntry`、`TIME_CONSTANT`、`recent_at`（`dee × exp(−(clock.saturating_sub(tick)) / 200)`）、`bump`（`clock` 已加一后调用：`dee = 1 + recent_at(clock)`、`tick = clock`、`count += 1`，`fresh` 为真才 `fresh += 1`）；`choices` 改成 `HashMap<String, HashMap<String, ChoiceEntry>>`，`FrequencyLearner` 加 `clock: u64`；`record_choice` 里先 `clock += 1` 再对字面键 `bump(clock, true)`、读音键 `bump(clock, false)`；`<raw>` 条目不计 `fresh`；`choice_weight`、`unrecord_choice`（`count`、`fresh` 各减 1，`dee` 取 `max(0, dee − 1)`，`count` 减到 0 删条目，不回拨 `clock`）、`decay_choices`、`choice_count` 按新类型改写（`decay_choices` 仍把 `count` 减半、去掉为 0 的）；实现 `choice_stats` 与 `input_evidence`（`Σ fresh`，跳过 `RAW_MARK`）。
- [ ] **Step 4: 运行确认通过** `cargo test -p qingjian-learning` 全部通过（含原有测试）。
- [ ] **Step 5: 提交** `feat(learning): 选择记录加提交时钟衰减的短期权重与「自启用以来」次数`

### Task 2: 持久化与旧文件兼容

**Files:**
- Modify: `crates/qingjian-learning/src/frequency_learner/tables.rs`（`load_choices`、`save_choices_to`）、`.../mod.rs`（`parse_counted_pair` 旁加 `parse_choice_row`）
- Test: `crates/qingjian-learning/src/frequency_learner/tests.rs`

**Interfaces:**
- Consumes: Task 1 的 `ChoiceEntry`、`FrequencyLearner.clock`。
- Produces: 文件格式 `输入串\t词\t次数\tdee\ttick\tfresh`，另有一行注释 `# 偏好时钟\t<clock>`；`parse_choice_row(line: &str) -> Option<(&str, &str, ChoiceEntry)>`：只有前三列时 `dee = 0.0`、`tick = 0`、`fresh = 0`；`dee` 非数字、`tick` 非整数的行返回 `None`（计入跳过数）。

- [ ] **Step 1: 写失败测试** `old_three_column_file_loads_with_no_recency`（老格式读入后 `choice_stats` 为 `{count: N, recent: 0.0, fresh: 0}`）；`new_file_round_trips_and_old_parser_still_reads_it`（保存后用旧的 `parse_counted_pair` 逐行解析，前三列一致）；`bad_rows_are_skipped_not_fatal`（缺列、`dee = abc`、负数 `count`、空文件；加载返回跳过数，不 panic）；`tick_beyond_clock_is_treated_as_zero_age`（文件里 `tick` 比头部时钟大，`recent` 等于存储的 `dee`，不为 NaN / 无穷）。
- [ ] **Step 2: 运行确认失败** `cargo test -p qingjian-learning old_three_column` 预期失败。
- [ ] **Step 3: 实现** `save_choices_to` 写六列并在头部追加时钟注释；`load_choices` 先从注释行取时钟（没有则 0），逐行 `parse_choice_row`；读入的 `tick` 超过时钟时把时钟抬到它（保证 `recent_at` 的 `saturating_sub` 不吞掉信息）。
- [ ] **Step 4: 运行确认通过** `cargo test -p qingjian-learning`。
- [ ] **Step 5: 提交** `feat(learning): user-choices.tsv 追加偏好列，旧版本仍可读`

### Task 3: 记录点、撤销与静音

**Files:**
- Modify: `crates/qingjian-core/src/engine/commit/mod.rs`（`CandidateKind::English` 分支、emoji / 颜文字 / 符号分支、`learned` 与 `LastCommit.choice` 的条件，约 330–345 行）、`crates/qingjian-core/src/engine/composing.rs`（`take_raw` 里 `english_word` 分支，约 372 行）、`crates/qingjian-core/src/engine/learning/muted.rs`（转发 `choice_stats`、`input_evidence`）
- Test: `crates/qingjian-core/src/engine/tests/learning.rs`

**Interfaces:**
- Consumes: Task 1 的 trait 方法；既有 `Learner::record_choice(input, reading, text)`、`unrecord_choice`、`Choice { reading, text }`。
- Produces: 英文候选、emoji、颜文字、符号上屏时 `record_choice(&input, "", &candidate.text)`，其中 `input` 是该分支里已经算出的那个 `input`（与 `LastCommit.input` 同值）；原样上屏英文词时 `record_choice(&scope, "", &raw)`（`scope` 是 `take_raw` 里已有的 `self.composition.scope()`）；`LastCommit.choice` 对这些种类也填 `Some(Choice { reading: String::new(), text })`。

- [ ] **Step 1: 写失败测试**（桩学习器记录 `record_choice` 调用）`english_candidate_commit_records_a_choice_under_the_whole_input`、`emoji_commit_records_a_choice`、`raw_english_commit_counts_as_choosing_that_english_word`（输入 `hello` 回车后 `choice_weight("hello", "hello") == 1`）、`undoing_an_emoji_commit_unrecords_it`（上屏 emoji 后退格重选别的候选，桩里该记录被退回）、`private_input_records_no_extra_choices`（`set_private(true)` 后上屏英文 / emoji，桩学习器没有任何 `record_choice`）。
- [ ] **Step 2: 运行确认失败** `cargo test -p qingjian-core english_candidate_commit_records` 预期失败。
- [ ] **Step 3: 实现** 在上述三处补 `record_choice`，并把这几种候选纳入 `learned` 分支与 `choice` 的 `matches!`；`MutedLearner` 的两个新方法直接转发（读取不受静音影响，写入已经在 `record_choice` 里挡了）。
- [ ] **Step 4: 运行确认通过** `cargo test -p qingjian-core`。
- [ ] **Step 5: 提交** `feat(core): 英文词、emoji、颜文字、符号的选择也按输入串记录，撤销与静音一并覆盖`

### Task 4: 引擎开关、配置与命令行

**Files:**
- Modify: `crates/qingjian-core/src/engine/mod.rs`（字段 `adaptive_learning: bool`，缺省 `true`；常量 `RECENT_WEIGHT_BETA`、`DEMOTE_AFTER_WORDS`、`DEMOTE_EVIDENCE`）、`.../setup.rs`、`crates/qingjian-platform/src/config/general.rs`、`.../config/mod.rs`（模板注释）、`.../config/tests.rs`、`apps/cli/src/args.rs`、`apps/cli/src/main.rs`
- Test: `crates/qingjian-platform/src/config/tests.rs`、`crates/qingjian-core/src/engine/tests/lookup.rs`

**Interfaces:**
- Produces: `Engine::set_adaptive_learning(&mut self, on: bool)`、`Engine::adaptive_learning(&self) -> bool`；`GeneralConfig.adaptive_learning: bool`（缺省 `true`）；`pub const RECENT_WEIGHT_BETA: f64 = 10.0`（`ranking`，起始值，Task 9 调）；`const DEMOTE_AFTER_WORDS: usize = 3`、`const DEMOTE_EVIDENCE: u32 = 5`（`engine/mod.rs`）；命令行 `--no-adaptive`（`args.no_adaptive: bool`）。照 `strict_match` 的写法，位置见 `grep -rn strict_match crates apps`。

- [ ] **Step 1: 写失败测试** 配置：`adaptive_learning_defaults_to_true_and_parses`（空配置为 `true`，写 `false` 解析为 `false`）；引擎：`adaptive_learning_defaults_on`、`set_adaptive_learning_round_trips`。
- [ ] **Step 2: 运行确认失败**。
- [ ] **Step 3: 实现** 配置字段、模板注释（中文，说明缺省 `true` 与关掉的效果）、引擎字段与 setter、命令行参数（`main.rs` 里 `engine.set_adaptive_learning(!args.no_adaptive)`）。
- [ ] **Step 4: 运行确认通过** `cargo test -p qingjian-platform -p qingjian-core` 并 `cargo check -p qingjian-cli`。
- [ ] **Step 5: 提交** `feat(core): 加 adaptive_learning 总开关与命令行 --no-adaptive`

### Task 5: 排序读有效偏好

**Files:**
- Modify: `crates/qingjian-core/src/engine/query/mod.rs`（新增 `Engine::preference_choice`）、`.../query/phonetic.rs`（约 159–167 行的闭包）、`.../query/code.rs`（约 46–49 行）
- Test: `crates/qingjian-core/src/engine/tests/learning.rs`

**Interfaces:**
- Consumes: Task 1 的 `choice_stats`、Task 4 的 `adaptive_learning()` 与 `RECENT_WEIGHT_BETA`。
- Produces: `fn preference_choice(&self, input: &str, text: &str) -> u32`（`pub(super)`）：开关关或 `recent == 0` 时返回 `choice_weight(input, text)`；否则返回 `(choice_weight as f64 + RECENT_WEIGHT_BETA × recent as f64).round() as u32`。`phonetic.rs` 与 `code.rs` 取选择次数处改调它。

- [ ] **Step 1: 写失败测试** `recent_selections_lift_a_word_in_ranking`（桩学习器：`choice_weight` 两词为 150 / 39，`recent` 为 1.2 / 8.0 时，`又` 的有效偏好进入竞争圈、排序结果随之变化）；`stats_without_recency_rank_exactly_as_before`（`recent = 0` 时与升级前排序逐项相同）；`adaptive_off_ignores_recency`（同样的桩，开关关闭时顺序等于只用 `choice_weight`）。
- [ ] **Step 2: 运行确认失败**。
- [ ] **Step 3: 实现** 如上；`ranking::contention_key` 与 `rank()` 不改。
- [ ] **Step 4: 运行确认通过** `cargo test -p qingjian-core`，并确认 `ranking::tests` 与既有性质测试（`ba`、`bi`）全部仍通过。
- [ ] **Step 5: 提交** `feat(core): 词排序把短期权重折进选择次数`

### Task 6: 英文词与 emoji 降级

**Files:**
- Create: `crates/qingjian-core/src/engine/query/demote.rs`
- Modify: `crates/qingjian-core/src/engine/query/mod.rs`（`mod demote;`）、`.../query/phonetic.rs`（在 `insert_emoji` 之后调用）
- Test: `crates/qingjian-core/src/engine/tests/demote.rs`（新建，并在 `tests/mod.rs` 加 `mod demote;`）

**Interfaces:**
- Consumes: Task 1 的 `choice_stats`、`input_evidence`；Task 4 的常量；`Engine::whole_scope()` 与 `Engine::consumed_by(&Candidate)`（取与记录时同一个键：候选音节为空用整段作用域，否则用 `consumed_by`）。
- Produces: `pub(super) fn demote_unchosen_extras(&self, items: &mut Vec<Candidate>)`，只在 `adaptive_learning()` 为真时调用。

- [ ] **Step 1: 写失败测试**（桩学习器返回指定的 `ChoiceStats` / `input_evidence`）：`unchosen_english_moves_behind_the_third_chinese_word`（证据 5、`fresh = 0`、`weight(word) = 0`：英文词与它的 emoji 移到第 3 个汉字之后，相对顺序不变）；`four_commits_is_not_enough_evidence`（证据 4 不动）；`one_selection_restores_the_original_slot`（`fresh = 1` 不动）；`english_with_a_global_weight_is_not_demoted`（`weight(word) > 0` 不动）；`english_at_index_zero_is_never_demoted`；`fewer_than_three_chinese_words_puts_extras_after_the_last_one`；`page_size_one_does_not_panic`；`adaptive_off_leaves_order_untouched`；`emoji_attached_to_a_chinese_word_is_judged_by_its_own_count`。
- [ ] **Step 2: 运行确认失败**。
- [ ] **Step 3: 实现** `demote_unchosen_extras`：先取 `input_evidence(整段键)`，不足 `DEMOTE_EVIDENCE` 直接返回；遍历列表找出满足条件的 `English`、`Emoji`、`Kaomoji`、`Symbol` 候选（第 0 位的英文例外），摘出来后按原有相对顺序插回前 `DEMOTE_AFTER_WORDS` 个汉字候选（`Chinese`、`Sentence`、`Code`、`Cloud`）之后，汉字不足时放在最后一个汉字候选之后。
- [ ] **Step 4: 运行确认通过** `cargo test -p qingjian-core`。
- [ ] **Step 5: 提交** `feat(core): 没选过的英文词与 emoji 排到前 3 个汉字之后`

### Task 7: 整句让位

**Files:**
- Modify: `crates/qingjian-core/src/engine/query/converting.rs`（`plain_sentence` 里 `behind_spelled` 的触发条件）
- Test: `crates/qingjian-core/src/engine/tests/lookup.rs`

**Interfaces:**
- Consumes: `Learner::choice_weight(input, text)`、Task 4 的 `adaptive_learning()`、既有 `behind_spelled: Option<String>` 与它在函数末尾的插入逻辑。
- Produces: 开关开、且 `items` 里有汉字词 `c` 满足「`c.syllables.concat()` 等于整段输入字母」且 `choice_weight(整段键, &c.text) > 0` 时，把 `behind_spelled = Some(letters)`，整句插到它后面；其余情况不变。

- [ ] **Step 1: 写失败测试** `sentence_yields_to_a_word_the_user_has_chosen`（`youshuo`：词库有 `又说` 与可拼成 `有说` 的两个词；桩学习器 `choice_weight("youshuo", "又说") = 4` 时首位是 `又说`、`有说` 整句在其后）；`sentence_stays_first_when_the_word_was_never_chosen`；`sentence_yields_only_when_adaptive_is_on`。
- [ ] **Step 2: 运行确认失败**。
- [ ] **Step 3: 实现** 如上，注意不要影响已有的模糊音降级与 `alternates` 逻辑。
- [ ] **Step 4: 运行确认通过** `cargo test -p qingjian-core`，已有的整句测试全部通过。
- [ ] **Step 5: 提交** `feat(core): 整句不再压住用户选过的词`

### Task 8: 平台接线与设置界面

**Files:**
- Modify: `apps/macos/src/host/config/mod.rs`、`apps/linux/server/src/main.rs`、`apps/linux/server/src/dispatch/reload/mod.rs`、`apps/windows/server/src/main.rs`、`apps/windows/server/src/dispatch/reload/mod.rs`、`apps/settings/src/view/pages/candidates.rs`
- Test: `apps/settings` 的现有测试（含 `config_keys.rs` 写入键核对）。

**Interfaces:**
- Consumes: `Engine::set_adaptive_learning`、`GeneralConfig.adaptive_learning`。
- Produces: 五处 `engine.set_adaptive_learning(config.general.adaptive_learning)`（启动与热重载）；设置程序「候选」页新增一行开关，写 `("general", "adaptive_learning")`，说明文案「按最近的选择调整排序，并把你从没选过的英文词和 emoji 往后排」。照 `strict_match` 的接法。

- [ ] **Step 1: 基线** 加界面行之前先运行 `cargo test -p qingjian-settings`，确认现有测试（含 `config_keys.rs` 的写入键核对）全部通过。这一任务没有可以先失败的单元测试：配置键在 Task 4 已存在，界面行只是接线，所以用核对测试加截图验证。
- [ ] **Step 2: 实现** 五处接线与设置界面一行。
- [ ] **Step 3: 验证** `cargo test -p qingjian-settings`（`config_keys.rs` 要覆盖到新键）；`cargo check -p qingjian-macos -p qingjian-linux-server -p qingjian-windows-server`；用设置程序的 `--png <新建空目录>` 截图确认开关渲染（需要临时让「候选」页多一步滚到底，看完撤销该临时改动，并删除含配置副本的截图目录）。
- [ ] **Step 4: 提交** `feat(settings): 候选页加「自适应学习」开关，三个平台服务端都接上`

### Task 9: 回放验收、调参与安装

**Files:**
- Modify: `docs/notes/crate-notes.md`（学习器、排序、extras 的实现要点）、`docs/user/input/` 下对应页（用户能感知的候选顺序变化，措辞面向用户）
- 产出（不入库）：回放对比表。

**Interfaces:**
- Consumes: 全部前序任务；`apps/cli` 的 `--replay`、`--no-adaptive`、`--misses`。

- [ ] **Step 1: 冷启动回放对比** 构建 release 版 CLI，对 `~/Library/Application Support/Qingjian/input-log.jsonl` 各跑一次开 / 关（`--no-adaptive`），**不带 `--user-dict`**。验收：词首选命中**不低于**关闭时的基线（约 89.5%）；`you→又` 首选没命中从 40 降到约 28 以下。
- [ ] **Step 2: 调 `RECENT_WEIGHT_BETA`** 在 {3, 10, 30, 100} 上各跑一次冷启动回放，取词首选命中最高且 `you→又` 改善最大的值写回常量；变化超出 ±0.3 个百分点或出现候选掉出列表就回头查原因，不硬调。
- [ ] **Step 3: 性质回归** 用带 `--user-dict` 的真实数据逐项核对：`ba` 在好 / 走 / 行 后出「吧」、没有 / 我们 / 帮我 后出「把」；`bi`（笔 16 / 币 13 / 比 10）「比」第一；冷引擎 `shi` 选第 3 / 7 / 9 位的词一次后升到第 1；`youshuo` 在「又说」选过 4 次后首位是它。
- [ ] **Step 4: 性能** 长输入（如 `jintiantianqihenhao`）每键总耗时不超过 10 ms（命令行输出的 `total`）。
- [ ] **Step 5: 全量检查** `cargo fmt --all --check`；`cargo clippy --workspace --all-targets -- -D warnings`；`cargo test --workspace --no-fail-fast`（预期只剩 `apps/linux/server` 的 `selection` 三个在这台 Mac 上本来就失败的用例）。
- [ ] **Step 6: 文档** 更新 `crate-notes.md` 与用户文档。
- [ ] **Step 7: 提交** `docs: 自适应偏好模型的实现说明与用户文档`
- [ ] **Step 8: 本机安装** 先备份当前已装的 `~/Library/Input Methods/Qingjian.app` 到 `~/Downloads/qingjian-build/`，再 `apps/macos/scripts/bundle.sh --install`，核对版本、签名与新进程启动时间；请用户试用后确认。
