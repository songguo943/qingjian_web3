#!/usr/bin/env python3
"""本地回归：用户报过的例子逐个检查（只在本地分支用，读的是个人数据的临时副本）。

用法（仓库根目录）：
    scripts/local/regression.py target/release/qingjian-cli

会把 ~/Library/Application Support/Qingjian/user*.tsv 复制到临时目录再用（CLI 带 --user-dict 会写回，
不能直接指向真实数据），跑完删掉。全部通过退出码 0，否则 1。
"""
import glob, os, re, shutil, subprocess, sys, tempfile

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DATA = os.path.expanduser("~/Library/Application Support/Qingjian")


class Session:
    """一个交互式 CLI 会话：敲拼音看候选、敲序号上屏。"""

    def __init__(self, cli, user_dict):
        cmd = [cli, "--limit", "40"] + (["--user-dict", user_dict] if user_dict else [])
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, bufsize=0, cwd=REPO)
        self.p.stdout.read(2)  # 开头的提示符 "> "

    def send(self, line):
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()
        out = ""
        while not out.endswith("> "):
            c = self.p.stdout.read(1)
            if not c:
                break
            out += c
        return out

    def cands(self, pinyin):
        return {int(m.group(1)): m.group(2)
                for m in re.finditer(r"^\s+(\d+)\. (\S+)", self.send(pinyin), re.M)}

    def pick(self, pinyin, text):
        for index, candidate in self.cands(pinyin).items():
            if candidate == text:
                self.send(str(index))
                return
        raise RuntimeError(f"{pinyin} 的候选里没有 {text}")

    def close(self):
        try:
            self.p.stdin.write(":q\n")
            self.p.stdin.close()
        except OSError:
            pass
        self.p.wait()


def top(items, n=3):
    return " / ".join(items[i] for i in sorted(items)[:n])


def main():
    cli = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "target/release/qingjian-cli"))
    scratch = tempfile.mkdtemp(prefix="qj-regression-")
    for path in glob.glob(os.path.join(DATA, "user*.tsv")):
        shutil.copy2(path, scratch)
    user = os.path.join(scratch, "user.tsv")
    failures = 0

    def check(label, ok):
        nonlocal failures
        failures += not ok
        print(("  OK  " if ok else "  XX  ") + label)

    def run(user_dict, steps, query):
        s = Session(cli, user_dict)
        try:
            for pinyin, text in steps:
                s.pick(pinyin, text)
            return s.cands(query)
        finally:
            s.close()

    try:
        print("== 带个人数据（临时副本）")
        for ctx, py, want in [("好", "hao", "吧"), ("走", "zou", "吧"), ("没有", "meiyou", "把"), ("帮我", "bangwo", "把")]:
            d = run(user, [(py, ctx)], "ba")
            check(f"{ctx}+ba → {top(d)}（期望 {want}）", d.get(1) == want)
        d = run(user, [("hao", "好"), ("ba", "吧")] * 2 + [("bangwo", "帮我")], "ba")
        check(f"连选两次 吧 后 帮我+ba → {top(d)}（期望 把）", d.get(1) == "把")
        for py, want in [("bi", "比"), ("shangougou", "山沟沟"), ("zhegene", "这个呢"), ("lianxiang", "联想")]:
            d = run(user, [], py)
            check(f"{py} → {top(d)}（期望 {want}）", d.get(1) == want)
        print("== 冷启动")
        for index in (3, 7, 9):
            s = Session(cli, None)
            try:
                text = s.cands("shi")[index]
                s.send(str(index))
                after = s.cands("shi")
            finally:
                s.close()
            check(f"shi 选第 {index} 位 {text} 后 → {top(after)}", after.get(1) == text)
        d = run(None, [("youshuo", "又说")] * 4, "youshuo")
        check(f"youshuo 选 4 次 又说 后 → {top(d)}", d.get(1) == "又说")
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    print(f"\n{'全部通过' if failures == 0 else f'{failures} 项不符合'}")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
