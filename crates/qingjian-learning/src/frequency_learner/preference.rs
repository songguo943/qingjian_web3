//! 按输入串记的选择条目：长期次数、按提交时钟衰减的短期权重、自启用偏好模型以来的次数。
//!
//! 时钟不是日历时间，而是学习器上「又记了一次输入串下的选择」的计数（RIME 的做法）：
//! 连着选某个词它的短期权重会很快升高，之后每提交别的词就衰减一点，周末不打字也不会忘。

/// 短期权重的衰减时间常数，单位是提交次数：每过 200 次提交衰减到 1/e。
pub(super) const TIME_CONSTANT: f64 = 200.0;

/// 一个（输入串，候选文本）的选择记录。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct ChoiceEntry {
    /// 长期次数，不衰减（超过条目上限时整体减半，见 `decay_choices`）。
    pub count: u32,

    /// 最后一次更新时的短期权重（未按时钟衰减的原始值）。
    pub dee: f32,

    /// 最后一次更新时的时钟。
    pub tick: u64,

    /// 自启用偏好模型以来的次数；旧文件读进来是 0。
    pub fresh: u32,
}

impl ChoiceEntry {
    /// 时钟走到 `clock` 时的短期权重。`tick` 比时钟大（文件被手改过）按没有时间差处理。
    pub(super) fn recent_at(&self, clock: u64) -> f32 {
        let age = clock.saturating_sub(self.tick) as f64;
        (f64::from(self.dee) * (-age / TIME_CONSTANT).exp()) as f32
    }

    /// 记一次选择，`clock` 已经加过一。`fresh` 为假（读音键下泛化出来的那份、原样上屏）不计「自启用以来」。
    pub(super) fn bump(&mut self, clock: u64, fresh: bool) {
        self.dee = 1.0 + self.recent_at(clock);
        self.tick = clock;
        self.count = self.count.saturating_add(1);
        if fresh {
            self.fresh = self.fresh.saturating_add(1);
        }
    }

    /// 退回一次选择：次数各减一，短期权重近似回退一份，时钟不回拨。
    pub(super) fn unbump(&mut self) {
        self.count = self.count.saturating_sub(1);
        self.fresh = self.fresh.saturating_sub(1);
        self.dee = (self.dee - 1.0).max(0.0);
    }
}
