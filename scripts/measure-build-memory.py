"""Measure a Windows build process tree with the standard library only.

RSS sums include shared pages more than once; private bytes measure committed
private memory, not physical RAM. Sampling can miss children shorter than the
interval. This is a build measurement, not a service launcher.
"""
import argparse
import ctypes as C
from ctypes import wintypes as W
import json
import os
from pathlib import Path
import subprocess
import time


class ProcessEntry(C.Structure):
    _fields_ = [("size", W.DWORD), ("usage", W.DWORD), ("pid", W.DWORD),
                ("heap", C.c_size_t), ("module", W.DWORD), ("threads", W.DWORD),
                ("parent", W.DWORD), ("priority", W.LONG), ("flags", W.DWORD),
                ("exe", W.WCHAR * 260)]


class MemoryCounters(C.Structure):
    _fields_ = [("size", W.DWORD), ("faults", W.DWORD)] + [
        (name, C.c_size_t) for name in
        ("peak_rss", "rss", "peak_paged", "paged", "peak_nonpaged", "nonpaged",
         "pagefile", "peak_pagefile", "private")]


def measure(command, log, interval):
    kernel = C.WinDLL("kernel32", use_last_error=True)
    psapi = C.WinDLL("psapi", use_last_error=True)
    kernel.CreateToolhelp32Snapshot.argtypes = [W.DWORD, W.DWORD]
    kernel.CreateToolhelp32Snapshot.restype = W.HANDLE
    kernel.Process32FirstW.argtypes = [W.HANDLE, C.POINTER(ProcessEntry)]
    kernel.Process32NextW.argtypes = [W.HANDLE, C.POINTER(ProcessEntry)]
    kernel.OpenProcess.argtypes = [W.DWORD, W.BOOL, W.DWORD]
    kernel.OpenProcess.restype = W.HANDLE
    kernel.CloseHandle.argtypes = [W.HANDLE]
    kernel.GetExitCodeProcess.argtypes = [W.HANDLE, C.POINTER(W.DWORD)]
    kernel.GetProcessTimes.argtypes = [W.HANDLE] + [C.POINTER(W.FILETIME)] * 4
    psapi.GetProcessMemoryInfo.argtypes = [W.HANDLE, C.POINTER(MemoryCounters), W.DWORD]
    tracked, names = {}, set()
    peak_private = peak_rss = samples = peak_processes = failures = 0
    private_peak_components = []
    started = time.monotonic()
    def created_at(handle):
        values = [W.FILETIME() for _ in range(4)]
        if not kernel.GetProcessTimes(handle, *(C.byref(value) for value in values)):
            return None
        return (values[0].dwHighDateTime << 32) | values[0].dwLowDateTime

    with open(log, "wb") as output:
        root = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT,
                                creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            # Build drivers wait for their compiler children. End the sampling
            # window when the driver exits, not when an incidental console exits.
            while root.poll() is None:
                snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
                if snapshot == W.HANDLE(-1).value:
                    raise C.WinError(C.get_last_error())
                entries = []
                try:
                    entry = ProcessEntry()
                    entry.size = C.sizeof(entry)
                    more = kernel.Process32FirstW(snapshot, C.byref(entry))
                    while more:
                        entries.append((entry.pid, entry.parent, entry.exe))
                        more = kernel.Process32NextW(snapshot, C.byref(entry))
                finally:
                    kernel.CloseHandle(snapshot)
                # Keep handles, rather than repeatedly trusting reused process IDs.
                parents = set(tracked) | ({root.pid} if root.poll() is None else set())
                changed = True
                while changed:
                    changed = False
                    for pid, parent, name in entries:
                        if pid not in tracked and (pid == root.pid or parent in parents):
                            handle = kernel.OpenProcess(0x0400 | 0x0010, False, pid)
                            if handle:
                                creation = created_at(handle)
                                parent_creation = tracked.get(parent, (None, 0))[1]
                                # Toolhelp reports historical parent PIDs. Ignore
                                # older unrelated processes after PID reuse.
                                if creation is None or (pid != root.pid and (parent not in tracked or creation < parent_creation)):
                                    kernel.CloseHandle(handle)
                                    continue
                                tracked[pid] = (handle, creation)
                                parents.add(pid)
                                names.add(name)
                                changed = True
                total_private = total_rss = live = 0
                components = []
                for pid, (handle, _) in list(tracked.items()):
                    code = W.DWORD()
                    if not kernel.GetExitCodeProcess(handle, C.byref(code)) or code.value != 259:
                        kernel.CloseHandle(handle)
                        del tracked[pid]
                        continue
                    counters = MemoryCounters()
                    counters.size = C.sizeof(counters)
                    if psapi.GetProcessMemoryInfo(handle, C.byref(counters), counters.size):
                        total_private += counters.private
                        total_rss += counters.rss
                        live += 1
                        name = next((name for found, _, name in entries if found == pid), "exited")
                        components.append({"name": name, "private_bytes": counters.private, "rss_bytes": counters.rss})
                    else:
                        failures += 1
                if total_private > peak_private:
                    peak_private = total_private
                    private_peak_components = components
                peak_rss = max(peak_rss, total_rss)
                peak_processes = max(peak_processes, live)
                samples += 1
                time.sleep(interval)
        finally:
            for handle, _ in tracked.values():
                kernel.CloseHandle(handle)
        code = root.wait()
    return {"exit_code": code, "seconds": round(time.monotonic() - started, 3),
            "peak_private_bytes": peak_private, "peak_rss_bytes": peak_rss,
            "peak_processes": peak_processes, "sample_count": samples,
            "sample_interval_seconds": interval, "memory_read_failures": failures,
            "process_names": sorted(names), "private_peak_components": private_peak_components}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--label", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if os.name != "nt" or not command:
        parser.error("Windows and a build command are required")
    result = {"label": args.label, **measure(command, args.log, 0.05)}
    Path(args.output).write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps(result))
    raise SystemExit(result["exit_code"])


if __name__ == "__main__":
    main()
