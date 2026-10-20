#!/usr/bin/env python3
"""Measure an editor's startup (or idle) cost, for the README's numbers.

    tools/bench.py [--runs N] [--wait SECONDS] EDITOR [ARGS...] FILE

Runs the editor in an 80x24 pseudo-terminal, waits, types `<Esc>:q!<Enter>`
and reports the median CPU time (user + system, including every process it
started: Neovim's server, git) and the peak memory of the largest process.
One extra warm-up run comes first and isn't counted.

    tools/bench.py fred src/main.rs
    tools/bench.py nvim --clean src/main.rs
    tools/bench.py --wait 6 fred src/main.rs     # include 6 s of idling
"""
import argparse
import fcntl
import os
import pty
import resource
import select
import statistics
import struct
import sys
import termios
import time


def run_once(cmd, wait):
    """CPU seconds one run used."""
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    pid, fd = pty.fork()
    if pid == 0:
        os.environ.setdefault("TERM", "xterm-256color")
        os.execvp(cmd[0], cmd)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
    end = time.time() + wait
    while time.time() < end:  # keep reading so the editor never blocks
        if select.select([fd], [], [], 0.01)[0]:
            try:
                os.read(fd, 65536)
            except OSError:
                break
    # Keys typed before an editor sets up its terminal can be dropped: resend
    # until it quits.
    while True:
        try:
            os.write(fd, b"\x1b:q!\r")
        except OSError:
            break
        try:
            while select.select([fd], [], [], 1)[0] and os.read(fd, 65536):
                pass
        except OSError:
            break
    os.waitpid(pid, 0)
    os.close(fd)
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    return (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--runs", type=int, default=15)
    p.add_argument("--wait", type=float, default=0.3, help="seconds before quitting")
    p.add_argument("cmd", nargs=argparse.REMAINDER)
    a = p.parse_args()
    if not a.cmd:
        p.error("give the editor command and a file")
    run_once(a.cmd, a.wait)
    cpu = [run_once(a.cmd, a.wait) * 1000 for _ in range(a.runs)]
    rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    rss_mb = rss / 1e6 if sys.platform == "darwin" else rss / 1e3  # bytes vs KB
    print(
        f"cpu median {statistics.median(cpu):.1f} ms "
        f"(min {min(cpu):.1f}, max {max(cpu):.1f}), peak memory {rss_mb:.1f} MB"
    )


if __name__ == "__main__":
    main()
