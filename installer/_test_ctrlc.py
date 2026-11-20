"""用 winpty 模拟真实终端，验证 Ctrl+C 只结束子进程而不杀死 Wren shell。"""
import subprocess, time, os, sys, threading, ctypes

EXE = r"D:\code\linuxTerminal\target\release\wren.exe"

# 用 winpty 的 --nocolor 启动一个带 PTY 的子进程
# winpty 会把主进程的 stdin/stdout 桥接到子进程的 PTY
p = subprocess.Popen(
    ["winpty", EXE],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
)

out_buf = bytearray()
def reader():
    while True:
        b = p.stdout.read(1)
        if not b:
            break
        out_buf.extend(b)

t = threading.Thread(target=reader, daemon=True)
t.start()

def wait_for(text, timeout=8):
    end = time.time() + timeout
    while time.time() < end:
        if text.encode() in bytes(out_buf):
            return True
        time.sleep(0.05)
    return False

def send(s):
    p.stdin.write(s.encode())
    p.stdin.flush()

# 等提示符出现
ok = wait_for("$", 10)
out_buf.clear()

# 输入一个长时间运行的前台命令（外部进程，非内置）
send("ping -n 60 127.0.0.1 > NUL\r\n")
time.sleep(2.0)
before = bytes(out_buf)
print("=== after starting long command (first 200 bytes) ===")
print(before[:200].decode("utf-8", "replace"))
out_buf.clear()

# 发送 Ctrl+C (0x03)
p.stdin.write(b"\x03")
p.stdin.flush()
time.sleep(1.5)
print("=== after Ctrl+C (first 300 bytes) ===")
print(bytes(out_buf)[:300].decode("utf-8", "replace"))
out_buf.clear()

# 关键验证：Ctrl+C 后 shell 是否还活着？输入 echo 看是否有回显
send("echo WREN_ALIVE_AFTER_CTRLC\r\n")
ok2 = wait_for("WREN_ALIVE_AFTER_CTRLC", 8)
print("=== shell alive check ===")
print("ALIVE" if ok2 else "DEAD (no echo response)")
print(bytes(out_buf)[:300].decode("utf-8", "replace"))

send("exit\r\n")
time.sleep(0.5)
try:
    p.wait(timeout=3)
except Exception:
    p.kill()
print("=== done ===")
