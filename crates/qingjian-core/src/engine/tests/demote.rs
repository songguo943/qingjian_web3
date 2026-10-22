//! 没选过的英文词 / emoji 往后挪：证据够了（该输入串自启用以来上屏 ≥ 5 次）且这个候选从没选过，
//! 就排到前 3 个汉字候选之后。

use super::*;

/// 证据、「选过」「有全局次数」都由测试给定的桩。
struct ExtrasLearner {
    evidence: u32,
    chosen: Vec<&'static str>,
    weighted: Vec<&'static str>,
}

impl ExtrasLearner {
    fn new(evidence: u32) -> Self {
        Self {
            evidence,
            chosen: Vec::new(),
            weighted: Vec::new(),
        }
    }
}

impl Learner for ExtrasLearner {
    fn record(&mut self, _candidate: &Candidate) {}

    fn weight(&self, text: &str) -> u32 {
        if self.weighted.contains(&text) { 3 } else { 0 }
    }

    fn choice_stats(&self, _input: &str, text: &str) -> ChoiceStats {
        ChoiceStats {
            fresh: u32::from(self.chosen.contains(&text)),
            ..ChoiceStats::default()
        }
    }

    fn input_evidence(&self, _input: &str) -> u32 {
        self.evidence
    }
}

const FIVE_YOU: &str =
    "有\tyou\t100000\n又\tyou\t50000\n由\tyou\t30000\n友\tyou\t20000\n优\tyou\t10000\n";

/// `you` 下：汉字 有 又 由 友 优，英文词 you，有 配 🈶（词库里 you 开头的更长的词排在这些之后）。
fn you_engine(words: &str, learner: ExtrasLearner) -> Engine {
    let mut engine = Engine::new(Dictionary::parse(words).unwrap())
        .with_english(WordList::parse("you\n").unwrap())
        .with_emoji(EmojiTable::parse("有\t🈶\n").unwrap())
        .with_learner(Box::new(learner));
    engine.set_emoji_limit(3);
    engine
}

fn order(engine: &mut Engine, input: &str) -> Vec<String> {
    engine.set_input(input);
    engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .map(|c| c.text)
        .collect()
}

#[test]
fn unchosen_english_moves_behind_the_third_chinese_word() {
    // 前提：关掉自适应时是 有 you 🈶 又 …（用户看到的现状）
    let mut baseline = you_engine(FIVE_YOU, ExtrasLearner::new(5));
    baseline.set_adaptive_learning(false);
    assert_eq!(
        order(&mut baseline, "you")[..5],
        ["有", "you", "🈶", "又", "由"]
    );
    // 证据 5、都没选过：英文词与 emoji 一起挪到前 3 个汉字（有 又 由）之后，相对顺序不变
    let mut engine = you_engine(FIVE_YOU, ExtrasLearner::new(5));
    assert_eq!(
        order(&mut engine, "you")[..7],
        ["有", "又", "由", "you", "🈶", "友", "优"]
    );
}

#[test]
fn four_commits_is_not_enough_evidence() {
    let mut engine = you_engine(FIVE_YOU, ExtrasLearner::new(4));
    assert_eq!(order(&mut engine, "you")[..4], ["有", "you", "🈶", "又"]);
}

#[test]
fn one_selection_restores_the_original_slot() {
    let mut learner = ExtrasLearner::new(9);
    learner.chosen = vec!["you"];
    let mut engine = you_engine(FIVE_YOU, learner);
    // 英文词选过一次留在原位；没选过的 🈶 单独挪到前 3 个汉字之后
    assert_eq!(
        order(&mut engine, "you")[..5],
        ["有", "you", "又", "由", "🈶"]
    );
}

#[test]
fn english_with_a_global_weight_is_not_demoted() {
    let mut learner = ExtrasLearner::new(9);
    learner.weighted = vec!["you"];
    let mut engine = you_engine(FIVE_YOU, learner);
    let items = order(&mut engine, "you");
    assert_eq!(items[1], "you", "有全局次数的英文词不降级：{items:?}");
    // 没选过的 emoji 仍然降级
    assert_eq!(items[..4], ["有", "you", "又", "由"]);
}

#[test]
fn english_at_index_zero_is_never_demoted() {
    // 拼音不像话时英文词排第一（既有规则）：不降级
    let words = "和\the\t100000\n喝\the\t50000\n河\the\t30000\n";
    let mut engine = Engine::new(Dictionary::parse(words).unwrap())
        .with_english(WordList::parse("hello\n").unwrap())
        .with_learner(Box::new(ExtrasLearner::new(9)));
    let items = order(&mut engine, "hello");
    assert_eq!(items[0], "hello", "{items:?}");
}

#[test]
fn fewer_than_three_chinese_words_puts_extras_after_the_last_one() {
    let mut engine = Engine::new(Dictionary::parse("我\two\t100000\n").unwrap())
        .with_english(WordList::parse("wo\n").unwrap())
        .with_emoji(EmojiTable::parse("我\t🙋\n").unwrap())
        .with_learner(Box::new(ExtrasLearner::new(5)));
    engine.set_emoji_limit(3);
    engine.set_input("wo");
    let items = engine.query().unwrap().candidates.items;
    let is_extra = |c: &Candidate| matches!(c.kind, CandidateKind::English | CandidateKind::Emoji);
    let chinese = items.iter().filter(|c| !is_extra(c)).count();
    assert!((1..3).contains(&chinese), "前提：汉字不足 3 个：{items:?}");
    assert!(
        items.iter().any(is_extra),
        "前提：有英文词或 emoji：{items:?}"
    );
    let last_chinese = items.iter().rposition(|c| !is_extra(c)).unwrap();
    let first_extra = items.iter().position(is_extra).unwrap();
    assert!(first_extra > last_chinese, "{items:?}");
}

#[test]
fn no_chinese_candidates_leaves_the_order_untouched() {
    let mut engine = Engine::new(Dictionary::parse("有\tyou\t100000\n").unwrap())
        .with_english(WordList::parse("smile\n").unwrap())
        .with_emoji(EmojiTable::parse("smile\t😀 😄\n").unwrap())
        .with_learner(Box::new(ExtrasLearner::new(9)));
    engine.set_emoji_limit(3);
    let items = order(&mut engine, "smile");
    assert_eq!(items[0], "smile", "没有汉字可挪到后面时原样保留：{items:?}");
}

#[test]
fn adaptive_off_leaves_order_untouched() {
    let mut engine = you_engine(FIVE_YOU, ExtrasLearner::new(9));
    engine.set_adaptive_learning(false);
    assert_eq!(order(&mut engine, "you")[..4], ["有", "you", "🈶", "又"]);
}

#[test]
fn emoji_attached_to_a_chinese_word_is_judged_by_its_own_count() {
    // 🈶 配在 有 上、选过一次：留在原位；英文词没选过：挪走
    let mut learner = ExtrasLearner::new(9);
    learner.chosen = vec!["🈶"];
    let mut engine = you_engine(FIVE_YOU, learner);
    assert_eq!(
        order(&mut engine, "you")[..5],
        ["有", "🈶", "又", "由", "you"]
    );
}
