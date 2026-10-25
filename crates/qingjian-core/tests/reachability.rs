//! 可达性不变量：词库里的词按完整拼音敲出来，不管个人数据怎么变，都必须在候选里。
//!
//! 个人数据（词的使用次数、同输入串下的选择次数、近期权重）只能改变顺序，不能让词读不出来。
//! 这里用一份小词库（同音字很多、有一个音节之差的近音音节）和按种子随机的个人数据去撞这条不变量。
//!
//! 只守「词库里的词」。`shangougou` 的 山沟沟、`zhegene` 的 这个呢 是整句路径上的问题（词库里没有这个词，
//! 要靠整句拼出来）：整句只给一两条，同音的别条（山沟够）合法地赢，没法用一条通用的不变量守住，
//! 它们各有自己的回归测试（`sentence::viterbi` 与 `engine::tests::english`）。

use qingjian_core::{Candidate, ChoiceStats, Engine, Learner};
use qingjian_dictionary::Dictionary;

/// 按种子给每个词 / 每对（输入串，词）随机一个次数的桩：有的词用了成千上万次，有的从没用过。
struct RandomHabits {
    seed: u64,
}

impl RandomHabits {
    /// 简单的哈希：同样的（种子，键）总是同样的数。
    fn roll(&self, key: &str) -> u64 {
        let mut hash = self.seed ^ 0x9E37_79B9_7F4A_7C15;
        for byte in key.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01B3);
            hash ^= hash >> 29;
        }
        hash
    }

    /// 长尾分布：多数是 0，少数很大。
    fn heavy(&self, key: &str) -> u32 {
        let roll = self.roll(key);
        match roll % 8 {
            0..=3 => 0,
            4 | 5 => (roll >> 8) as u32 % 20,
            6 => (roll >> 8) as u32 % 400,
            _ => (roll >> 8) as u32 % 5000,
        }
    }
}

impl Learner for RandomHabits {
    fn record(&mut self, _candidate: &Candidate) {}

    fn weight(&self, text: &str) -> u32 {
        self.heavy(&format!("w:{text}"))
    }

    fn choice_weight(&self, input: &str, text: &str) -> u32 {
        self.heavy(&format!("c:{input}:{text}"))
    }

    fn choice_stats(&self, input: &str, text: &str) -> ChoiceStats {
        let count = self.choice_weight(input, text);
        ChoiceStats {
            count,
            recent: (self.roll(&format!("r:{input}:{text}")) % 400) as f32 / 10.0
                * f32::from(count > 0),
            fresh: count.min(30),
        }
    }
}

/// 一份小词库：声母韵母排列出的单字（含一个音节之差的近音字）与它们拼出的双字词。
fn dictionary() -> (Dictionary, Vec<(String, String)>) {
    // 每个音节配足够多的同音字（超过整句词图每格留的前几名），再配几个一个音节之差、字很常用的近音音节
    // （gou 的 you / hou / guo）：个人次数大的近音词最容易把按原样读的字挤出词图
    let syllables = [
        ("shan", "山杉衫闪陕善扇单"),
        ("gou", "沟狗够构购勾钩垢"),
        ("you", "有又友优游油右由"),
        ("hou", "后厚候猴侯吼喉"),
        ("guo", "过国果锅郭裹"),
        ("zhe", "这者着遮折哲浙"),
        ("ge", "个歌哥各格割隔"),
        ("ne", "呢讷"),
        ("le", "了乐勒"),
        ("de", "的得地德"),
        ("chu", "出初处楚础除储"),
        ("shuo", "说硕烁"),
        ("wo", "我握卧窝"),
        ("men", "们门闷"),
    ];
    let mut source = String::new();
    let mut phrases = Vec::new();
    let mut freq = 900_000u32;
    for (pinyin, chars) in syllables {
        for c in chars.chars() {
            source.push_str(&format!("{c}\t{pinyin}\t{freq}\n"));
            freq = freq.saturating_sub(37_000).max(500);
        }
    }
    // 双字词：相邻两个音节的第一个字拼起来（词频中等）
    for pair in syllables.windows(2) {
        let (a, ac) = pair[0];
        let (b, bc) = pair[1];
        let word: String = [ac.chars().next().unwrap(), bc.chars().next().unwrap()]
            .iter()
            .collect();
        source.push_str(&format!("{word}\t{a} {b}\t30000\n"));
        phrases.push((word, format!("{a}{b}")));
    }
    (Dictionary::parse(&source).unwrap(), phrases)
}

#[test]
fn dictionary_words_stay_reachable_under_any_personal_habits() {
    let (_, phrases) = dictionary();
    let mut unreachable = Vec::new();
    for seed in 0..40u64 {
        let (dictionary, _) = dictionary();
        let mut engine = Engine::new(dictionary).with_learner(Box::new(RandomHabits { seed }));
        for (word, input) in &phrases {
            engine.set_input(input);
            let query = engine.query().unwrap();
            if !query.candidates.items.iter().any(|c| &c.text == word) {
                unreachable.push(format!("seed {seed}: {input} → {word}"));
            }
        }
    }
    assert!(
        unreachable.is_empty(),
        "词库里的词被个人数据挤出了候选（{} 处）：{:#?}",
        unreachable.len(),
        &unreachable[..unreachable.len().min(15)]
    );
}
