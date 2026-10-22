//! BIP-39 助记词防泄露：连续命中转私密，命中的词先攒着不学。见 `crate::engine::mnemonic`。

use super::{Candidate, Dictionary, Engine, Learner, SAMPLE, WordList, engine};

/// 跟 `english.rs` 里 `raw_committed_english_words_are_learned_and_come_back_as_candidates`
/// 用的是同一种桩：真记 `learn_english` 被叫过哪些词，比只看候选排序更直接。
#[derive(Default)]
struct EnglishLearner {
    words: Vec<String>,
    list: Option<WordList>,
}

impl Learner for EnglishLearner {
    fn record(&mut self, _candidate: &Candidate) {}
    fn weight(&self, _text: &str) -> u32 {
        0
    }
    fn learn_english(&mut self, word: &str) {
        self.words.push(word.to_owned());
        let tsv: String = self
            .words
            .iter()
            .map(|w| format!("{w}\t{w}\t1\n"))
            .collect();
        self.list = WordList::parse(&tsv).ok();
    }
    fn user_english(&self) -> Option<&WordList> {
        self.list.as_ref()
    }
}

/// 原样上屏一个字母串（回车），跟 `english.rs` 里的用法一样：先 query 一次再 take_raw。
fn commit_raw(engine: &mut Engine, word: &str) {
    engine.set_input(word);
    engine.query().unwrap();
    assert_eq!(engine.take_raw(), word);
}

/// 个人英文词表里学到了哪些词（桩学习器记下的 `learn_english` 调用）。
fn learned(engine: &Engine) -> Vec<String> {
    [
        "abstract", "about", "gist", "across", "actress", "address", "afraid", "angry",
    ]
    .into_iter()
    .filter(|w| {
        engine
            .learner()
            .user_english()
            .and_then(|list| list.get(w))
            .is_some()
    })
    .map(str::to_owned)
    .collect()
}

#[test]
fn bip39_words_are_held_back_and_learned_normally_once_the_streak_breaks() {
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap())
        .with_learner(Box::new(EnglishLearner::default()));

    // "abstract" 在 BIP-39 表里：连击还没成，先攒着，不落盘
    commit_raw(&mut engine, "abstract");
    commit_raw(&mut engine, "about");
    assert!(
        learned(&engine).is_empty(),
        "连击进行中，命中表的词不该落盘"
    );

    // "gist" 不在表里：连击断了，攒着的词照常学进去，gist 本身也照常学
    commit_raw(&mut engine, "gist");
    assert_eq!(learned(&engine), ["abstract", "about", "gist"]);
}

#[test]
fn a_word_in_the_table_is_still_learned_like_any_other_english_word() {
    // 表里全是 about / time / work 这类最常用的词：单独出现、紧接着敲别的，它们必须能被学到
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap())
        .with_learner(Box::new(EnglishLearner::default()));
    commit_raw(&mut engine, "about");
    commit_raw(&mut engine, "gist");
    assert!(learned(&engine).contains(&"about".to_owned()));
}

#[test]
fn a_completed_streak_drops_the_held_back_words_so_none_reach_disk() {
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap())
        .with_learner(Box::new(EnglishLearner::default()));
    // 连敲 6 个：前 5 个攒着，第 6 个触发私密，攒着的直接丢掉
    for word in STREAK_WORDS {
        commit_raw(&mut engine, word);
    }
    assert!(engine.is_private());
    // 连击断了：丢掉的不会再冒出来，只有断了之后的普通词照常学
    commit_raw(&mut engine, "gist");
    assert_eq!(learned(&engine), ["gist"], "整串助记词一个都不该落盘");
}

#[test]
fn five_in_a_row_is_not_a_streak_so_all_five_are_learned_when_it_breaks() {
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap())
        .with_learner(Box::new(EnglishLearner::default()));
    for word in &STREAK_WORDS[..5] {
        commit_raw(&mut engine, word);
    }
    assert!(!engine.is_private());
    commit_raw(&mut engine, "gist");
    assert_eq!(
        learned(&engine).len(),
        6,
        "差一个到阈值：5 个 + gist 都照常学"
    );
}

#[test]
fn words_held_back_before_a_private_field_are_dropped_not_learned_later() {
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap())
        .with_learner(Box::new(EnglishLearner::default()));
    commit_raw(&mut engine, "abstract");
    // 焦点进了密码框：攒着的词丢掉，期间敲的词也不攒
    engine.set_private(true);
    commit_raw(&mut engine, "about");
    engine.set_private(false);
    commit_raw(&mut engine, "gist");
    assert_eq!(
        learned(&engine),
        ["gist"],
        "密码框前后攒着的词都不该在之后被冲进词库"
    );
}

#[test]
fn bip39_matching_ignores_case() {
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap())
        .with_learner(Box::new(EnglishLearner::default()));
    // 英文模式下敲的大写同样算命中：六个大写的助记词照样触发私密
    engine.set_english_mode(true);
    for word in STREAK_WORDS {
        commit_raw(&mut engine, &word.to_ascii_uppercase());
    }
    assert!(engine.is_private(), "大小写不该绕过连续命中");
}

/// 连续 6 个「学英文词」事件都命中 BIP-39 表，够 `BIP39_STREAK_THRESHOLD`，
/// 即使没有任何密码框声明，也该临时转成私密输入。
const STREAK_WORDS: [&str; 6] = [
    "abstract", "across", "actress", "address", "afraid", "angry",
];

#[test]
fn a_streak_of_bip39_words_triggers_private_input_without_any_password_field() {
    let mut engine = engine();
    assert!(!engine.is_private());

    for (index, word) in STREAK_WORDS.iter().enumerate() {
        commit_raw(&mut engine, word);
        let expect_private = index + 1 >= STREAK_WORDS.len();
        assert_eq!(
            engine.is_private(),
            expect_private,
            "敲到第 {} 个 BIP-39 词时私密状态应为 {expect_private}",
            index + 1
        );
    }

    // 接一个不在表里的英文词：连续命中打断，启发式私密退出
    commit_raw(&mut engine, "gist");
    assert!(!engine.is_private(), "命中中断后启发式私密应该自动退出");
}

#[test]
fn committing_chinese_between_bip39_words_breaks_the_streak() {
    let mut engine = engine();
    // 敲 5 个（差一个到阈值），中间插一次中文上屏，连续计数应该被打断
    for word in &STREAK_WORDS[..5] {
        commit_raw(&mut engine, word);
    }
    engine.set_input("kaifa");
    let candidate = engine
        .query()
        .unwrap()
        .candidates
        .items
        .into_iter()
        .find(|c| c.text == "开发")
        .unwrap();
    engine.commit(&candidate);
    assert!(!engine.is_private(), "中文上屏打断了连续命中，不该转私密");

    // 打断之后重新数，5 个还不够
    for word in &STREAK_WORDS[..5] {
        commit_raw(&mut engine, word);
    }
    assert!(!engine.is_private());
}

#[test]
fn heuristic_private_never_overrides_a_password_field_declared_by_the_shell() {
    let mut engine = engine();
    // 壳先声明这是密码框
    engine.set_private(true);
    assert!(engine.is_private());

    // 期间敲的词打断了启发式那一路（其实从没启动过），但壳声明的私密不能被这一步误关掉
    commit_raw(&mut engine, "gist");
    assert!(
        engine.is_private(),
        "启发式那一路的『撤销』不该动壳声明的私密状态"
    );

    // 只有壳自己撤，才真正退出私密
    engine.set_private(false);
    assert!(!engine.is_private());
}

/// `ChoiceSpy` 记下的 `record_choice` 调用：（输入串键，词）。
type ChoiceLog = std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>;

/// 记下 `record_choice` 调用的桩：整串助记词不该有一个词落进选择表。
#[derive(Default)]
struct ChoiceSpy {
    choices: ChoiceLog,
}

impl Learner for ChoiceSpy {
    fn record(&mut self, _candidate: &Candidate) {}
    fn weight(&self, _text: &str) -> u32 {
        0
    }
    fn record_choice(&mut self, input: &str, _reading: &str, text: &str) {
        self.choices
            .lock()
            .unwrap()
            .push((input.to_owned(), text.to_owned()));
    }
}

fn spy_engine() -> (Engine, ChoiceLog) {
    let spy = ChoiceSpy::default();
    let log = spy.choices.clone();
    let engine = Engine::new(Dictionary::parse(SAMPLE).unwrap()).with_learner(Box::new(spy));
    (engine, log)
}

#[test]
fn a_completed_streak_leaves_no_bip39_word_in_the_choice_table() {
    let (mut engine, log) = spy_engine();
    for word in STREAK_WORDS {
        commit_raw(&mut engine, word);
    }
    assert!(engine.is_private());
    commit_raw(&mut engine, "gist");
    let recorded: Vec<String> = log.lock().unwrap().iter().map(|(_, t)| t.clone()).collect();
    assert!(
        recorded.iter().all(|t| !STREAK_WORDS.contains(&t.as_str())),
        "助记词不该写进选择表：{recorded:?}"
    );
}

#[test]
fn held_back_bip39_choices_are_recorded_when_the_streak_breaks() {
    let (mut engine, log) = spy_engine();
    for word in &STREAK_WORDS[..5] {
        commit_raw(&mut engine, word);
    }
    assert!(log.lock().unwrap().is_empty(), "连击没断前先攒着，不记");
    commit_raw(&mut engine, "gist");
    let recorded: Vec<String> = log.lock().unwrap().iter().map(|(_, t)| t.clone()).collect();
    assert_eq!(
        recorded.len(),
        6,
        "断了之后 5 个攒着的 + gist 照常记：{recorded:?}"
    );
}

#[test]
fn a_retracted_held_back_choice_is_not_recorded_when_the_streak_breaks() {
    // smoke / smooth 都在 BIP-39 表里：选了 smoke 又退格改选 smooth，smoke 的选择还攒着、该一并撤掉
    let words = WordList::parse("smoke\nsmooth\n").unwrap();
    let spy = ChoiceSpy::default();
    let log = spy.choices.clone();
    let mut engine = Engine::new(Dictionary::parse(SAMPLE).unwrap())
        .with_english(words)
        .with_learner(Box::new(spy));
    let pick = |engine: &mut Engine, text: &str| {
        engine.set_input("smo");
        let candidate = engine
            .query()
            .unwrap()
            .candidates
            .items
            .into_iter()
            .find(|c| c.text == text)
            .unwrap_or_else(|| panic!("候选里没有 {text}"));
        engine.commit(&candidate);
    };
    pick(&mut engine, "smoke");
    for _ in 0.."smoke".len() {
        engine.note_backspace();
    }
    pick(&mut engine, "smooth");
    commit_raw(&mut engine, "gist");
    let recorded: Vec<String> = log.lock().unwrap().iter().map(|(_, t)| t.clone()).collect();
    assert!(
        !recorded.contains(&"smoke".to_owned()),
        "撤销过的选择不该落盘：{recorded:?}"
    );
    assert!(recorded.contains(&"smooth".to_owned()), "{recorded:?}");
}
