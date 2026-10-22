//! 候选排序。
//!
//! 词级排序规则：
//! 1. 音节数与输入完全一致的词优先（`kaifa` → 开发 排在 开发者 前）
//! 2. 覆盖输入字母多者优先（`kaif` → 开发者 排在 开 前）
//! 3. 切分里非末尾的简拼音节少者优先（`kaifa` 按 `kai fa` 读的 开放 排在按 `kai f a` 读的 开放啊 前）
//! 4. 最后一个音节完整匹配优先（`kaifa` → 开发 排在 开放 前）
//! 5. 注音敲了声调时，声调代价小者优先（ㄇㄚ 一声 → 妈 排在 吗 前；表与规则见 `zhuyin::tone`）
//! 6. 同一输入串下用户选过的次数（Learner 的 `choice_weight`：`mgs` 选过 美国式，下次 `mgs` 它就是首选）；
//!    次数不少于最高者一半的词都在竞争圈内（`ba` 的 把 121 / 吧 84），圈内的交给下一条的上下文得分，所以「好」后面仍能出 吧；
//!    明显落后（不到一半）的排在圈外，见 [`contention_key`]
//! 7. 上下文得分：语言模型给的 `log P(词 | 上一个上屏的词)`（个人 bigram 插值，模型不认识的按词库词频兜底并扣分，
//!    见 `sentence::transition_log_prob`）加用户选择次数的加分（[`weight_bonus`]，对数且封顶），模糊音命中扣 ln 2、
//!    敲错变体命中扣那类敲错的代价（`correction::TypoKind::cost`，个人敲错表打折）。
//!    这样 `ba` 在「做了」后面出 吧、句首出 把；纯词频排序两处都只能出同一个
//! 8. 敲的原音节优先，词长短者优先，最后按字符串稳定排序保证结果可复现
//!
//! 词库静态词频只用于预选（命中太多时先按词频砍到够排的量）与兜底。

mod scored;

use std::collections::{HashMap, HashSet};

pub use scored::{PreselectKey, Scored, SortKey};

/// 选择次数在同一组里至少占最高者的 1/[`CONTENTION_RATIO`] 才算进入竞争圈。
pub const CONTENTION_RATIO: u32 = 2;

/// 同输入串下选择次数的竞争键，越大越靠前：圈内的（次数不少于最高者一半，`ba` 的 把 121 / 吧 84，`bi` 的 笔 16 / 比 10）
/// 键相同，交给下一条的上下文得分决定；圈外的（明显落后，不到一半）按次数排在圈子后面，所以新学的词（1 对 0、5 对 0）
/// 仍然一次就能上位。以组内最高者为基准而不是看绝对次数，没有「次数跨过某个整数就变成硬优先」的悬崖。
/// 圈内键取最大值，圈外键是次数本身，仍是全序，排序键可以事先算好。
pub fn contention_key(choice: u32, leader: u32) -> u32 {
    if choice.saturating_mul(CONTENTION_RATIO) >= leader {
        u32::MAX
    } else {
        choice
    }
}

/// 短期权重折算成「选择次数」的倍数：排序用的有效次数 = 长期次数 + 倍数 × 短期权重。
/// 起始值是猜测，由回放调参定（见设计稿「验收」）。短期权重为 0（老数据、没最近选过）时有效次数就是长期次数，行为等于升级前。
pub const RECENT_WEIGHT_BETA: f64 = 10.0;

/// 用户选择次数的加分系数：加分 = 系数 × ln(1 + min(次数, [`WEIGHT_CAP`]))。
/// 这个加分只负责「同音词里偏向用户常选的那个」，所以既取对数又封顶（最多约 1.5 分，e^1.5 ≈ 4.5 倍），
/// 让上下文（bigram）仍能压过它；不封顶时选过 173 次的 的 会带着 5 分加分把任何含 的 的拆分路径抬到整词之上
/// （`haode` 出 号的 而不是 好的）。用户用得多的词另有个人 bigram 里的一元项撑腰，不靠这里。
pub const WEIGHT_BONUS: f64 = 0.5;

/// 加分里计入的选择次数上限。
pub const WEIGHT_CAP: u32 = 20;

/// 模糊音命中扣的分（词频减半）。敲错变体的代价见 `correction::TypoKind`。
pub const FUZZY_PENALTY: f64 = std::f64::consts::LN_2;

/// 用户选择次数换算成得分加成，词级排序与整句路径共用，见 [`WEIGHT_BONUS`]。
pub fn weight_bonus(count: u32) -> f64 {
    WEIGHT_BONUS * (1.0 + f64::from(count.min(WEIGHT_CAP))).ln()
}

/// 排序并按词文本去重（同一个词可能被多种切分命中，保留得分最高的一条），最多留 `limit` 条。
/// `context` 给每条命中算（同输入串下的选择次数, 上下文 log 概率），只对预选后剩下的那些调用。
///
/// 排序键先算好再排：单字母简拼能命中两万条，比较器里每次数字符数会让排序占掉几十毫秒；去重也只做到够数为止。
pub fn rank(
    items: &mut Vec<Scored<'_>>,
    limit: usize,
    context: impl Fn(&Scored<'_>) -> (u32, f64),
) {
    // 远超上限时先按结构键 + 词频线性选出前面一段：同一个词会被多种切分命中，多选一倍留给去重（结果仍可能略少于上限，无妨）
    let preselect = limit.saturating_mul(2);
    if items.len() > preselect.saturating_mul(2) {
        let mut keyed: Vec<(PreselectKey, Scored<'_>)> = items
            .drain(..)
            .map(|item| (item.preselect_key(), item))
            .collect();
        keyed.select_nth_unstable_by(preselect, |a, b| b.0.cmp(&a.0));
        keyed.truncate(preselect);
        items.extend(keyed.into_iter().map(|(_, item)| item));
    }
    let scored: Vec<(Scored<'_>, u32, f64)> = items
        .drain(..)
        .map(|item| {
            let (choice, log_prob) = context(&item);
            (item, choice, log_prob)
        })
        .collect();
    // 每组（见 [`Scored::group`]）里选择次数最高的：竞争圈以它为基准，次数不少于它一半的都在圈内
    let mut leader: HashMap<scored::Group, u32> = HashMap::new();
    for (item, choice, _) in &scored {
        let top = leader.entry(item.group()).or_insert(0);
        *top = (*top).max(*choice);
    }
    let mut keyed: Vec<(SortKey<'_>, Scored<'_>)> = scored
        .into_iter()
        .map(|(item, choice, log_prob)| {
            let score = log_prob + weight_bonus(item.weight) - item.penalty;
            let key = contention_key(choice, leader[&item.group()]);
            (item.key(key, score), item)
        })
        .collect();
    keyed.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut seen: HashSet<&str> = HashSet::with_capacity(limit.min(keyed.len()));
    items.extend(
        keyed
            .into_iter()
            .map(|(_, item)| item)
            .filter(|item| seen.insert(item.hit.text))
            .take(limit),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use qingjian_dictionary::Match;

    fn hit<'a>(text: &'a str, pinyin: &'a str, frequency: u32, exact: bool) -> Match<'a> {
        Match {
            text,
            pinyin,
            frequency,
            reading_total: 0,
            tones: Default::default(),
            secondary: false,
            exact,
        }
    }

    #[test]
    fn exact_beats_frequency_and_weight_beats_static_frequency() {
        let mut items = vec![
            Scored {
                hit: hit("开发者", "kai fa zhe", 99999, false),
                full_last: true,
                coverage: 5,
                abbreviated: 0,
                weight: 0,
                penalty: 0.0,
                tone_cost: 0,
            },
            Scored {
                hit: hit("开放", "kai fang", 20000, true),
                full_last: false,
                coverage: 5,
                abbreviated: 0,
                weight: 5,
                penalty: 0.0,
                tone_cost: 0,
            },
            Scored {
                hit: hit("开发", "kai fa", 9000, true),
                full_last: true,
                coverage: 5,
                abbreviated: 0,
                weight: 0,
                penalty: 0.0,
                tone_cost: 0,
            },
        ];
        rank(&mut items, usize::MAX, |_| (0, 0.0));
        let texts: Vec<&str> = items.iter().map(|s| s.hit.text).collect();
        assert_eq!(texts, ["开发", "开放", "开发者"]);
    }

    #[test]
    fn context_score_orders_within_the_same_structure() {
        let mut items = vec![
            Scored {
                hit: hit("把", "ba", 3_000_000, true),
                full_last: true,
                coverage: 2,
                abbreviated: 0,
                weight: 0,
                penalty: 0.0,
                tone_cost: 0,
            },
            Scored {
                hit: hit("吧", "ba", 2_000_000, true),
                full_last: true,
                coverage: 2,
                abbreviated: 0,
                weight: 0,
                penalty: 0.0,
                tone_cost: 0,
            },
        ];
        // 上下文说 吧 更像：词频高的 把 让位
        let by_context = |s: &Scored<'_>| (0, if s.hit.text == "吧" { -1.0 } else { -6.0 });
        rank(&mut items, usize::MAX, by_context);
        let texts: Vec<&str> = items.iter().map(|s| s.hit.text).collect();
        assert_eq!(texts, ["吧", "把"]);
        // 同一输入串下选过的压过上下文
        rank(&mut items, usize::MAX, |s| {
            (
                u32::from(s.hit.text == "把"),
                if s.hit.text == "吧" { -1.0 } else { -6.0 },
            )
        });
        assert_eq!(items[0].hit.text, "把");
        // 同分时用户选过的、非模糊音的靠前
        for item in &mut items {
            item.weight = u32::from(item.hit.text == "把") * 3;
        }
        rank(&mut items, usize::MAX, |_| (0, -2.0));
        assert_eq!(items[0].hit.text, "把");
        for item in &mut items {
            item.weight = 3;
            item.penalty = if item.hit.text == "把" {
                FUZZY_PENALTY
            } else {
                0.0
            };
        }
        rank(&mut items, usize::MAX, |_| (0, -2.0));
        assert_eq!(items[0].hit.text, "吧");
    }

    /// 选择次数不少于最高者一半的词都进入竞争，由上下文决定；只有明显落后（不到一半）的才排在圈外。
    fn rank_homophones(counts: (u32, u32), context_prefers: &str) -> String {
        let make = |text, frequency| Scored {
            hit: hit(text, "ba", frequency, true),
            full_last: true,
            coverage: 2,
            abbreviated: 0,
            weight: 0,
            penalty: 0.0,
            tone_cost: 0,
        };
        let mut items = vec![make("把", 3_000_000), make("吧", 2_000_000)];
        rank(&mut items, usize::MAX, |s| {
            (
                if s.hit.text == "把" {
                    counts.0
                } else {
                    counts.1
                },
                if s.hit.text == context_prefers {
                    -1.0
                } else {
                    -6.0
                },
            )
        });
        items[0].hit.text.to_owned()
    }

    #[test]
    fn comparable_choice_counts_let_context_decide() {
        // ba：把 121 次 / 吧 84 次，差不多，上下文决定（「好」后面出 吧、「帮我」后面出 把）
        assert_eq!(rank_homophones((121, 84), "吧"), "吧");
        assert_eq!(rank_homophones((121, 84), "把"), "把");
        // 比 10 次对 笔 16 次也算差不多
        assert_eq!(rank_homophones((16, 10), "吧"), "吧");
    }

    #[test]
    fn contention_has_no_count_cliff() {
        // 不管绝对次数落在哪：127 对 94（比例 1.35）与 15 对 11 一样交给上下文，不会因为跨过 2 的幂次就变成硬优先
        for counts in [
            (127, 94),
            (128, 94),
            (255, 200),
            (15, 11),
            (16, 11),
            (63, 40),
            (64, 40),
        ] {
            assert_eq!(rank_homophones(counts, "吧"), "吧", "{counts:?}");
        }
    }

    #[test]
    fn clearly_lagging_candidates_stay_behind() {
        // 次数悬殊（5 对 0、10 对 4）：新学的词仍然一次就能上位，上下文翻不了盘
        assert_eq!(rank_homophones((5, 0), "吧"), "把");
        assert_eq!(rank_homophones((10, 4), "吧"), "把");
        // 恰好一半（10 对 5）仍在圈内
        assert_eq!(rank_homophones((10, 5), "吧"), "吧");
    }

    #[test]
    fn one_selection_beats_never_chosen() {
        assert_eq!(rank_homophones((0, 1), "把"), "吧");
        // 一次对两次也在圈内（1 × 2 >= 2），交给上下文
        assert_eq!(rank_homophones((2, 1), "吧"), "吧");
    }

    #[test]
    fn fewer_abbreviated_syllables_win_at_equal_coverage() {
        let mut items = vec![
            Scored {
                hit: hit("开放啊", "kai fang a", 132, true),
                full_last: true,
                coverage: 5,
                abbreviated: 1,
                weight: 0,
                penalty: 0.0,
                tone_cost: 0,
            },
            Scored {
                hit: hit("开放", "kai fang", 500_000, true),
                full_last: false,
                coverage: 5,
                abbreviated: 0,
                weight: 0,
                penalty: 0.0,
                tone_cost: 0,
            },
        ];
        rank(&mut items, usize::MAX, |_| (0, 0.0));
        assert_eq!(items[0].hit.text, "开放");
    }

    #[test]
    fn weight_bonus_is_logarithmic_and_capped() {
        assert_eq!(weight_bonus(0), 0.0);
        assert!(weight_bonus(1) < weight_bonus(10));
        assert_eq!(weight_bonus(WEIGHT_CAP), weight_bonus(WEIGHT_CAP * 50));
        assert!(weight_bonus(u32::MAX) < 1.6);
    }

    #[test]
    fn deduplicates_by_text_keeping_best() {
        let mut items = vec![
            Scored {
                hit: hit("西安", "xi an", 4000, false),
                full_last: true,
                coverage: 4,
                abbreviated: 0,
                weight: 0,
                penalty: 0.0,
                tone_cost: 0,
            },
            Scored {
                hit: hit("西安", "xi an", 4000, true),
                full_last: true,
                coverage: 4,
                abbreviated: 0,
                weight: 0,
                penalty: 0.0,
                tone_cost: 0,
            },
        ];
        rank(&mut items, usize::MAX, |_| (0, 0.0));
        assert_eq!(items.len(), 1);
        assert!(items[0].hit.exact);
    }
}
