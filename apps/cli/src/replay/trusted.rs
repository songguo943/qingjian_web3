//! 回放的「可信口径」：日志里当时的选择不全是用户想要的。
//!
//! 空格上屏的是当时排第一的候选，用户可能看都没看；当时的首选上屏后又被退格撤销 / 删掉重打的，
//! 更说明那次选择是错的。把这些当标准答案，「现在的首选和当时不同」就分不清是进步还是退步。
//! 这里按可信度把每条上屏分成三类，各自报现在的首选命中，而不是混成一个数。

use std::collections::HashSet;

use qingjian_core::{CommitEntry, InputLogEntry};

use super::line::Line;

/// 一条上屏在整份日志里的身份：上屏 id 每次会话从头数，要带上它在第几个会话里（遇到 `session` 行加一）。
pub type CommitKey = (usize, u64);

/// 日志里被撤销或删掉重打过的上屏：先扫一遍，回放到那条上屏时才知道它后来被推翻了。
pub fn undone_commits(text: &str) -> HashSet<CommitKey> {
    let mut session = 0;
    let mut undone = HashSet::new();
    for line in text
        .lines()
        .filter_map(|raw| serde_json::from_str::<Line>(raw).ok())
    {
        match line.entry {
            InputLogEntry::Session { .. } => session += 1,
            InputLogEntry::Retract { of, .. } | InputLogEntry::Retype { of, .. } => {
                undone.insert((session, of));
            }
            _ => {}
        }
    }
    undone
}

/// 一条上屏有多可信。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    /// 当时不是首选、用户翻 / 挑出来的，之后也没撤：最可信的标准答案。
    Picked,

    /// 当时就是首选、之后没撤：只能说明用户没反对，用来防退步。
    Accepted,

    /// 上屏后被撤销或删掉重打：那次的文本是错的，不能当答案。
    Undone,

    /// 旧格式日志没记当时选第几个：不进可信口径。
    Unknown,
}

/// 这条上屏属于哪一类。`session` 是它在第几个会话里，`undone` 见 [`undone_commits`]。
pub fn label(commit: &CommitEntry, session: usize, undone: &HashSet<CommitKey>) -> Label {
    if undone.contains(&(session, commit.id)) {
        return Label::Undone;
    }
    match commit.index {
        Some(0) => Label::Accepted,
        Some(_) => Label::Picked,
        None => Label::Unknown,
    }
}

/// 三类各自的（条数，现在首选就是日志里那个文本的条数）。
/// 对 `Undone` 来说后一个数是「现在仍把那个错的排第一」，越少越好。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Trusted {
    pub picked: (usize, usize),
    pub accepted: (usize, usize),
    pub undone: (usize, usize),
}

impl Trusted {
    pub fn note(&mut self, label: Label, now_top1: bool) {
        let slot = match label {
            Label::Picked => &mut self.picked,
            Label::Accepted => &mut self.accepted,
            Label::Undone => &mut self.undone,
            Label::Unknown => return,
        };
        slot.0 += 1;
        slot.1 += usize::from(now_top1);
    }

    pub fn is_empty(&self) -> bool {
        self.picked.0 + self.accepted.0 + self.undone.0 == 0
    }
}

/// 没命中例子里标可信度的短名。
pub fn tag(label: Label) -> &'static str {
    match label {
        Label::Picked => "挑选",
        Label::Accepted => "接受",
        Label::Undone => "撤销",
        Label::Unknown => "旧",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qingjian_core::InputSource;

    fn commit(id: u64, index: Option<usize>) -> CommitEntry {
        let json = format!(
            r#"{{"id":{id},"scope":"nihao","keys":"nihao","pinyin":"ni'hao","corrected":false,"text":"你好","source":"word","index":{},"top":["你好"],"scheme":"","english":false}}"#,
            index.map_or("null".to_owned(), |i| i.to_string())
        );
        let entry: CommitEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry.source, InputSource::Word);
        entry
    }

    #[test]
    fn undone_wins_over_the_position_it_was_chosen_at() {
        let undone = HashSet::from([(0, 7)]);
        assert_eq!(label(&commit(7, Some(0)), 0, &undone), Label::Undone);
        assert_eq!(label(&commit(7, Some(3)), 0, &undone), Label::Undone);
    }

    #[test]
    fn position_decides_between_picked_and_accepted() {
        let undone = HashSet::new();
        assert_eq!(label(&commit(1, Some(0)), 0, &undone), Label::Accepted);
        assert_eq!(label(&commit(1, Some(2)), 0, &undone), Label::Picked);
        assert_eq!(label(&commit(1, None), 0, &undone), Label::Unknown);
    }

    #[test]
    fn ids_restart_every_session_so_an_undo_only_marks_its_own_session() {
        let log = [
            r#"{"t":"","event":"session","v":1,"version":"0.2","platform":"macos","model":false,"scheme":""}"#,
            r#"{"t":"","event":"retract","of":3,"text":"你好","chosen":"拟好"}"#,
            r#"{"t":"","event":"session","v":1,"version":"0.2","platform":"macos","model":false,"scheme":""}"#,
        ]
        .join("\n");
        let undone = undone_commits(&log);
        assert_eq!(undone, HashSet::from([(1, 3)]));
        assert_eq!(label(&commit(3, Some(0)), 1, &undone), Label::Undone);
        assert_eq!(label(&commit(3, Some(0)), 2, &undone), Label::Accepted);
    }

    #[test]
    fn unknown_commits_stay_out_of_the_trusted_counts() {
        let mut trusted = Trusted::default();
        trusted.note(Label::Picked, true);
        trusted.note(Label::Picked, false);
        trusted.note(Label::Undone, true);
        trusted.note(Label::Unknown, true);
        assert_eq!(trusted.picked, (2, 1));
        assert_eq!(trusted.undone, (1, 1));
        assert_eq!(trusted.accepted, (0, 0));
    }
}
