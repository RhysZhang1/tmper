#!/usr/bin/env python3
"""Exercise the built daemon, CLI, IPC, persistence and TUI in an isolated sandbox.

Run: dbus-run-session -- python3 scripts/smoke_test.py target/debug/tmper
Requires Linux and Python 3; supplies a private ALSA null device. No music plays
through the user's speakers and no real tmper configuration/state is touched.
"""
import base64
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import wave

BINARY = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/tmper").resolve()
REPO = Path(__file__).resolve().parents[1]


def wait_for(predicate, seconds=8):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        result = predicate()
        if result:
            return result
        time.sleep(0.02)
    raise AssertionError("timed out waiting for expected state")


class Peer:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(5)
        self.socket.connect(str(path))
        self.reader = self.socket.makefile("rb")
        self.send(t="hello", proto=2, version="smoke-test")
        assert self.read()["t"] == "welcome"
        self.initial = [self.read() for _ in range(4)]
        assert [e["t"] for e in self.initial] == ["snapshot", "queue", "library_paths", "playlists"]

    def send(self, **request):
        self.socket.sendall((json.dumps(request) + "\n").encode())

    def read(self):
        assembled = bytearray()
        while True:
            raw = self.reader.readline(1024 * 1024 + 2)
            assert raw and len(raw) <= 1024 * 1024 + 1, "EOF or invalid frame"
            event = json.loads(raw)
            if "_tmper_chunk" not in event:
                assert not assembled
                return event
            assembled.extend(base64.b64decode(event["_tmper_chunk"], validate=True))
            assert len(assembled) <= 64 * 1024 * 1024
            if not event["more"]:
                return json.loads(assembled)

    def until(self, predicate):
        observed = []
        end = time.monotonic() + 8
        while time.monotonic() < end:
            event = self.read()
            observed.append(event)
            if predicate(event):
                return event, observed
        raise AssertionError("expected event was not received")

    def sync(self, number):
        self.send(t="sync", id=number)
        return self.until(lambda e: e["t"] == "synced" and e["id"] == number)[1]

    def close(self):
        self.reader.close()
        self.socket.close()


def exercise(root):
    env = os.environ.copy()
    for kind in ["CONFIG", "DATA", "STATE", "RUNTIME"]:
        env[f"TMPER_{kind}_DIR"] = str(root / kind.lower())
    env["XDG_CACHE_HOME"] = str(root / "cache")
    env["TERM"] = "xterm-256color"
    # Reuse the existing multiplexer fallback to keep the pty probe silent.
    env["TMUX"] = "tmper-smoke-test"
    alsa = root / "alsa.conf"
    alsa.write_text("pcm.!default { type null }\nctl.!default { type null }\n")
    env["ALSA_CONFIG_PATH"] = str(alsa)
    music = root / "music"
    music.mkdir()
    song = music / "long.wav"
    with wave.open(str(song), "wb") as output:
        output.setparams((2, 2, 44100, 0, "NONE", "not compressed"))
        output.writeframes(b"\0" * (60 * 44100 * 4))
    song.with_suffix(".lrc").write_text("[offset:250]\n[00:00.00]<00:00.00>Hello <00:01.00>world\n")
    state_file = root / "state/state.json"
    sock = root / "runtime/socket"
    processes = []
    peers = []

    def start():
        process = subprocess.Popen([str(BINARY), "daemon"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        processes.append(process)
        def ready():
            if process.poll() is not None:
                raise AssertionError((root / "state/tmper-daemon.log").read_text())
            try:
                with socket.socket(socket.AF_UNIX) as probe:
                    probe.connect(str(sock))
                return True
            except OSError:
                return False
        wait_for(ready)
        return process

    def cli(*args):
        return subprocess.run([str(BINARY), *args], env=env, text=True, capture_output=True, timeout=8, check=True).stdout

    try:
        process = start()
        a, b = Peer(sock), Peer(sock)
        peers += [a, b]
        a.send(t="playlist_create", name="Private cursor")
        a.until(lambda e: e["t"] == "playlist_added")
        events = b.sync(1)
        assert any(e["t"] == "playlists" for e in events)
        assert not any(e["t"] == "playlist_added" for e in events)
        a.send(t="search_library", query="private search")
        a.until(lambda e: e["t"] == "search_results")
        assert not any(e["t"] == "search_results" for e in b.sync(2))
        print("PASS: two clients share state without sharing operation replies")

        cli("volume", "37")
        assert "volume    37%" in cli("status")
        if env.get("DBUS_SESSION_BUS_ADDRESS") and shutil.which("busctl"):
            bus = ["busctl", "--user"]
            address = ["org.mpris.MediaPlayer2.tmper", "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player", "Volume"]
            def desktop_volume():
                result = subprocess.run(bus + ["get-property"] + address, env=env, text=True, capture_output=True, timeout=5)
                return result.returncode == 0 and abs(float(result.stdout.split()[1]) - 0.37) < 0.001
            wait_for(desktop_volume)
            subprocess.run(bus + ["set-property"] + address + ["d", "0.42"], env=env, capture_output=True, timeout=5, check=True)
            wait_for(lambda: "volume    42%" in cli("status"))
            cli("volume", "37")
            print("PASS: real MPRIS property reads/setters agree with CLI state")
        else:
            print("SKIP: MPRIS property checks require a session bus and busctl")
        a.send(t="play", path=str(song))
        a.send(t="pause")
        a.until(lambda e: e["t"] == "snapshot" and e["status"] == "Paused")
        a.send(t="seek_relative", secs=1.234)
        a.sync(3)
        state, _ = a.until(lambda e: e["t"] == "snapshot" and e["status"] == "Paused")
        assert 1.20 <= state["position_secs"] <= 1.27, state
        a.send(t="set_lyrics_offset", ms=500)
        a.sync(4)
        wait_for(lambda: state_file.exists() and json.loads(state_file.read_text()).get("lyrics_offset_ms") == 500)
        saved = json.loads(state_file.read_text())
        assert saved["queue"] and saved["position_secs"] >= 1.2
        assert not list(state_file.parent.glob("*.tmp"))
        print("PASS: CLI acknowledgements, paused seek and atomic live checkpoint")

        a.send(t="add_library_path", path=str(REPO / "tests/fixtures/formats"))
        report, _ = a.until(lambda e: e["t"] == "scan_finished")
        assert report["failed"] == 0 and report["scanned"] == 8, report
        print("PASS: all eight format fixtures enter the real SQLite library")

        # The UI receives a real player's state under a pseudo-terminal.
        master, slave = pty.openpty()
        fcntl.ioctl(slave, 0x5414, struct.pack("HHHH", 35, 120, 0, 0))
        tui = subprocess.Popen([str(BINARY)], env=env, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        processes.append(tui)
        os.close(slave)
        output = bytearray()
        for key in [b"", b"2", b"3", b"4", b"5", b"6", b"7", b"8", b"8", b"1"]:
            if key:
                os.write(master, key)
            until = time.monotonic() + 0.25
            while time.monotonic() < until:
                if select.select([master], [], [], 0.05)[0]:
                    output.extend(os.read(master, 65536))
        os.write(master, b"q")
        tui.wait(timeout=5)
        os.close(master)
        assert tui.returncode == 0 and b"Hello" in output, bytes(output[-2000:])
        (root / "tui-capture.ansi").write_bytes(output)
        assert process.poll() is None
        print("PASS: seven TUI views/help render, q leaves the daemon alive")

        a.close(); b.close(); peers.clear()
        process.kill()
        process.wait(timeout=5)
        process = start()
        restored = Peer(sock)
        peers.append(restored)
        state = restored.initial[0]
        assert state["volume"] == 0.37 and state["lyrics_offset_ms"] == 500, state
        assert state["position_secs"] >= 1.2 and restored.initial[1]["tracks"], state
        print("PASS: SIGKILL/restart restores the live checkpoint and stale socket")
        restored.close(); peers.clear()
        cli("quit")
        process.wait(timeout=5)
        assert process.returncode == 0 and not sock.exists()
        process = start()
        process.send_signal(signal.SIGTERM)
        process.wait(timeout=5)
        assert process.returncode == 0 and not sock.exists()
        print("PASS: CLI quit and SIGTERM flush state and clean up the socket")
    finally:
        for peer in peers:
            peer.close()
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)


if __name__ == "__main__":
    assert BINARY.is_file(), f"build the binary first: {BINARY}"
    with tempfile.TemporaryDirectory(prefix="tmper-smoke-") as directory:
        exercise(Path(directory))
    print("All isolated smoke checks passed")
