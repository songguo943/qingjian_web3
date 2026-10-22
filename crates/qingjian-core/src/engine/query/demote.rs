//! 没选过的英文词与 emoji 往后挪：同一个输入串自启用以来上屏够多次，这个候选却从没被选过，
//! 就排到前几个汉字候选之后，不再占着固定的第 2、3 位。

use super::*;

impl Engine {
    /// 证据（该输入串自启用以来上屏 ≥ [`DEMOTE_EVIDENCE`] 次）够了，就把「从没选过」的英文词 / emoji / 颜文字 / 符号
    /// 挪到前 [`DEMOTE_AFTER_WORDS`] 个汉字候选之后（汉字不足就挪到最后一个汉字之后），相对顺序不变。
    /// 排第一位的英文词是「拼音不像话」的判断，不动；已经在那之后的不动；一个汉字都没有就不动。
    pub(super) fn demote_unchosen_extras(&self, items: &mut Vec<Candidate>) {
        if !self.adaptive_learning {
            return;
        }
        let whole = self.whole_scope().1;
        if self.learner.input_evidence(&whole) < DEMOTE_EVIDENCE {
            return;
        }
        let is_word = |c: &Candidate| {
            matches!(
                c.kind,
                CandidateKind::Chinese
                    | CandidateKind::Sentence
                    | CandidateKind::Code
                    | CandidateKind::Cloud
            )
        };
        // 前 N 个汉字之后的位置（不足 N 个就是最后一个汉字之后）
        let mut seen = 0;
        let mut end = None;
        for (index, candidate) in items.iter().enumerate() {
            if is_word(candidate) {
                seen += 1;
                end = Some(index + 1);
                if seen == DEMOTE_AFTER_WORDS {
                    break;
                }
            }
        }
        let Some(end) = end else { return };
        let movable: Vec<bool> = items[..end]
            .iter()
            .enumerate()
            .map(|(index, candidate)| self.is_unchosen_extra(candidate, index, &whole))
            .collect();
        if !movable.contains(&true) {
            return;
        }
        let tail = items.split_off(end);
        let mut kept = Vec::with_capacity(end);
        let mut moved = Vec::new();
        for (candidate, movable) in items.drain(..).zip(movable) {
            if movable {
                moved.push(candidate);
            } else {
                kept.push(candidate);
            }
        }
        kept.extend(moved);
        kept.extend(tail);
        *items = kept;
    }

    /// 英文词 / emoji / 颜文字 / 符号里，自启用以来没为这个输入串选过的。
    /// 查的键必须与上屏时记的一致：没有音节的候选（英文词）按整段作用域，其余按它覆盖的那段输入。
    fn is_unchosen_extra(&self, candidate: &Candidate, index: usize, whole: &str) -> bool {
        let key = |this: &Self| {
            if candidate.syllables.is_empty() {
                whole.to_owned()
            } else {
                this.consumed_by(candidate).1
            }
        };
        match candidate.kind {
            CandidateKind::English => {
                index != 0
                    && self.learner.weight(&candidate.text) == 0
                    && self
                        .learner
                        .choice_stats(&key(self), &candidate.text.to_ascii_lowercase())
                        .fresh
                        == 0
            }
            CandidateKind::Emoji | CandidateKind::Kaomoji | CandidateKind::Symbol => {
                self.learner.choice_stats(&key(self), &candidate.text).fresh == 0
            }
            _ => false,
        }
    }
}
