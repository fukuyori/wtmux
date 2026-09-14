import sys
sys.stdout.reconfigure(encoding="utf-8")
w = sys.stdout.write
w("\x1b[2J\x1b[H")
# row 1: sequential
w("\x1b[1;1H1seq: \U0001F1EF\U0001F1F5z|ｶﾞw|❤️x|end")
# row 2: positioned like wtmux's renderer (CHA to explicit columns)
w("\x1b[2;1H2pos: \x1b[7G\U0001F1EF\x1b[8G\U0001F1F5\x1b[9Gz|\x1b[11Gｶ\x1b[12Gﾞ\x1b[13Gw|\x1b[15G❤\x1b[16Gx|end")
# row 3: positioned but the pair written together
w("\x1b[3;1H3pair:\x1b[7G\U0001F1EF\U0001F1F5\x1b[9Gz|\x1b[11Gｶﾞ\x1b[13Gw|end")
w("\x1b[5;1H")
sys.stdout.flush()
