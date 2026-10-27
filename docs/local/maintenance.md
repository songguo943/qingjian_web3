# 本地分支维护（只在本机用，不提交上游）

本机装的青简来自 `local/main`：上游 `origin/main` + 我们自己的改动。目前只在自己的分支上维护，不给上游提 PR。

## 分支

- `local/main`：主干，装到本机的就是它。上游更新、我们的修改都进这里。
- `pr/*`、`local/strict-match-v2`、`local/integration`：2026-10-26 同步上游时按改动拆开的分支，留作记录，之后不再单独维护。
- `main`：老的本地 main，不再用；`fork` 远端是自己的 GitHub 仓库，`origin` 是上游。

我们在上游之上的改动：可信口径回放、格子保底留按原样读的词（山沟沟）、混输比分按最好的切分（这个呢、#766）、
模糊音整句排在敲对的词后面、选择次数竞争圈、BIP-39 助记词先攒着不学、严格匹配、自适应偏好（近期权重、英文 / emoji 降级、整句让位）。

## 与远端保持一致（自动）

远端是自己的仓库 `fork`（songguo943/qingjian_web3）；上游 `origin` 只拉不推，不提 PR。

- `.git/hooks/` 里的 `post-commit` / `post-merge` / `post-rewrite` 调 `auto-push-fork`：当前分支跟踪 `fork` 时，
  提交、合并、amend / rebase 之后自动 `git push --force-with-lease fork <分支>`；失败只记 `.git/auto-push.log`，不挡提交。
  这些钩子在 `.git/` 里，不进代码库，换机器要重装。
- 推送用 songguo943 身份：仓库级 `credential.https://github.com/songguo943/qingjian_web3.git.helper` 现场调
  `gh auth token -u songguo943` 取令牌（不切换 gh 的全局活动账号，令牌不落进配置）。
- 新建分支要自动推，先 `git push -u fork <分支>` 设好跟踪。
- PR #290 的源分支 `fix/bip39-mnemonic-protection` 不推：推了会改动上游的那个 PR。

## 同步上游

```bash
git fetch origin
git checkout local/main
git merge origin/main          # 有冲突就逐个解决：两边都加的配置 / 接线通常两边都留
```

解决冲突时留意：上游改过的函数，我们旧提交里的那份可能是按老代码写的（比如进私密时要清的状态），以上游的新逻辑为准再补我们的改动。

## 合并后必须跑的检查

```bash
export PATH="$HOME/.cargo/bin:$PATH" RUSTUP_TOOLCHAIN=1.96.0
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release -p qingjian-cli
scripts/local/regression.py target/release/qingjian-cli          # 报过的例子，全部 OK 才装
target/release/qingjian-cli --replay "$HOME/Library/Application Support/Qingjian/input-log.jsonl" --misses 0
```

回放看「可信口径」三行，和上一次装的版本比（2026-10-26 的数：挑选 18.2%、接受 97.6%、撤销 73.0%）：

- 挑选（当时不是首选、挑出来后没撤）：越高越好，主要看这个。
- 接受（当时就是首选、没撤）：不能明显下降，防退步。
- 撤销（当时的首选被撤销或删掉重打，现在仍把它排第一）：越低越好。

旧的「词 / 整句 首选」那两行被当时的首选污染，只作参考。`--no-adaptive` 可以对比关掉自适应的口径。

## 安装

```bash
cp -pR "$HOME/Library/Input Methods/Qingjian.app" ~/Downloads/qingjian-build/Qingjian-backup-<旧版本>.app
apps/macos/scripts/bundle.sh --install
```

装完核对版本号（`defaults read "$HOME/Library/Input Methods/Qingjian.app/Contents/Info" CFBundleShortVersionString`）与签名（`codesign -v`），
切换一次输入法或注销再登录让新进程起来。

## 排查新问题的顺序

1. 先用个人数据的**临时副本**复现（`--user-dict <副本>/user.tsv`，CLI 带它会写回，不能指向真实数据）。
2. 逐个只带一个 `user*.tsv` 二分，看是哪类个人数据触发的。
3. 上游最新版也跑一遍，确认是不是上游已经修了。
4. 修复先写会失败的测试；改完跑上面全部检查，再把例子加进 `scripts/local/regression.py`。
