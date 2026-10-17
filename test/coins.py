#!/usr/bin/env python3
"""Coin counter. Type + to add one, - to take one away, q to quit."""

import sys
import termios
import tty


def main():
    coins = 0

    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    try:
        tty.setraw(fd)
        sys.stdout.write("coins: 0   (+ adds, - takes one away, q quits)\r\n")
        sys.stdout.flush()

        while True:
            ch = sys.stdin.read(1)
            if ch == "+":
                coins += 1
            elif ch == "-":
                coins -= 1
            elif ch in ("q", "\x03", "\x04"):
                break
            else:
                continue

            sys.stdout.write("\rcoins: %d   " % coins)
            sys.stdout.flush()
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)
        sys.stdout.write("\r\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()