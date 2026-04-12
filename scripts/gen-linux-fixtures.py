#!/usr/bin/env python3
"""Write the two SYNTHETIC Linux fixtures in the snapshot directory format.

    linux-x86-intel-hybrid  12th-gen Intel desktop layout: 8 P-cores with SMT (cpus 0-15) and
                            4 E-cores (16-19); L3 shared by every core; cpufreq present.
    linux-x86-amd-gce       AMD EPYC VM on Google Compute Engine: 8 cores x 2 threads with
                            non-adjacent siblings (i, i+8), 2 NUMA nodes each with its own L3,
                            hypervisor flag, no cpufreq.

Usage: scripts/gen-linux-fixtures.py tests/fixtures
"""
import pathlib
import sys

META = """snapshot_version = 1
cpu_version = "0.1.0"
os = "linux"
arch = "x86_64"
kernel = "6.8.0"
created_unix = 1790208000
note = "SYNTHETIC: {note}"
"""

X86_BASE_FLAGS = "fpu vme de pse tsc msr pae mce cx8 apic sep mtrr pge mca cmov pat pse36 clflush mmx fxsr sse sse2 ht syscall nx lm constant_tsc rep_good nopl xtopology cpuid pni pclmulqdq ssse3 fma cx16 sse4_1 sse4_2 movbe popcnt aes xsave avx f16c rdrand lahf_lm abm bmi1 avx2 smep bmi2 erms rdseed adx smap clflushopt sha_ni xsaveopt xsavec xgetbv1 xsaves"


def write(root, files):
    for path, text in files.items():
        target = root / "fs" / path.lstrip("/")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text if text.endswith("\n") else text + "\n")


def cpuinfo(blocks):
    return "".join("".join(f"{k}\t: {v}\n" for k, v in block) + "\n" for block in blocks)


def cpu_list(cpus):
    cpus = sorted(cpus)
    out, start, prev = [], cpus[0], cpus[0]
    for c in cpus[1:] + [None]:
        if c is not None and c == prev + 1:
            prev = c
            continue
        out.append(str(start) if start == prev else f"{start}-{prev}")
        if c is not None:
            start = prev = c
    return ",".join(out)


def cache(files, cpu, index, level, kind, size, shared):
    base = f"/sys/devices/system/cpu/cpu{cpu}/cache/index{index}"
    files[f"{base}/level"] = str(level)
    files[f"{base}/type"] = kind
    files[f"{base}/size"] = size
    files[f"{base}/shared_cpu_list"] = cpu_list(shared)


def intel_hybrid(root):
    files = {"/proc/sys/kernel/arch": "x86_64", "/proc/sys/kernel/osrelease": "6.8.0",
             "/sys/devices/system/cpu/online": "0-19", "/sys/devices/system/cpu/possible": "0-19",
             "/sys/devices/cpu_core/cpus": "0-15", "/sys/devices/cpu_atom/cpus": "16-19",
             "/sys/devices/system/node/node0/cpulist": "0-19"}
    blocks = []
    flags = X86_BASE_FLAGS + " vaes vpclmulqdq avx_vnni vmx ibt user_shstk"
    for cpu in range(20):
        p_core = cpu < 16
        siblings = [cpu - cpu % 2, cpu - cpu % 2 + 1] if p_core else [cpu]
        core_id = (cpu // 2) * 4 if p_core else 32 + (cpu - 16)
        topo = f"/sys/devices/system/cpu/cpu{cpu}/topology"
        files[f"{topo}/physical_package_id"] = "0"
        files[f"{topo}/core_id"] = str(core_id)
        files[f"{topo}/thread_siblings_list"] = cpu_list(siblings)
        freq = f"/sys/devices/system/cpu/cpu{cpu}/cpufreq"
        files[f"{freq}/cpuinfo_max_freq"] = "5000000" if p_core else "3800000"
        files[f"{freq}/base_frequency"] = "3600000" if p_core else "2700000"
        if p_core:
            cache(files, cpu, 0, 1, "Data", "48K", siblings)
            cache(files, cpu, 1, 1, "Instruction", "32K", siblings)
            cache(files, cpu, 2, 2, "Unified", "1280K", siblings)
        else:
            cache(files, cpu, 0, 1, "Data", "32K", [cpu])
            cache(files, cpu, 1, 1, "Instruction", "64K", [cpu])
            cache(files, cpu, 2, 2, "Unified", "2048K", range(16, 20))
        cache(files, cpu, 3, 3, "Unified", "25600K", range(20))
        blocks.append([("processor", cpu), ("vendor_id", "GenuineIntel"), ("cpu family", 6), ("model", 151),
                       ("model name", "12th Gen Intel(R) Core(TM) i7-12700K"), ("stepping", 2), ("microcode", "0x2f"),
                       ("cpu MHz", "3600.000"), ("physical id", 0), ("core id", core_id), ("flags", flags)])
    files["/proc/cpuinfo"] = cpuinfo(blocks)
    write(root / "linux-x86-intel-hybrid", files)
    (root / "linux-x86-intel-hybrid/meta.toml").write_text(
        META.format(note="12th-gen Intel desktop layout (8 P-cores with SMT, 4 E-cores), not captured from hardware"))


def amd_gce(root):
    node0 = [*range(0, 4), *range(8, 12)]
    node1 = [*range(4, 8), *range(12, 16)]
    files = {"/proc/sys/kernel/arch": "x86_64", "/proc/sys/kernel/osrelease": "6.8.0",
             "/sys/devices/system/cpu/online": "0-15", "/sys/devices/system/cpu/possible": "0-15",
             "/sys/class/dmi/id/sys_vendor": "Google", "/sys/class/dmi/id/product_name": "Google Compute Engine",
             "/sys/devices/system/node/node0/cpulist": cpu_list(node0),
             "/sys/devices/system/node/node1/cpulist": cpu_list(node1)}
    blocks = []
    flags = X86_BASE_FLAGS + " hypervisor vaes vpclmulqdq"
    for cpu in range(16):
        core = cpu % 8
        siblings = [core, core + 8]
        node = node0 if cpu in node0 else node1
        topo = f"/sys/devices/system/cpu/cpu{cpu}/topology"
        files[f"{topo}/physical_package_id"] = "0"
        files[f"{topo}/core_id"] = str(core)
        files[f"{topo}/thread_siblings_list"] = cpu_list(siblings)
        cache(files, cpu, 0, 1, "Data", "32K", siblings)
        cache(files, cpu, 1, 1, "Instruction", "32K", siblings)
        cache(files, cpu, 2, 2, "Unified", "512K", siblings)
        cache(files, cpu, 3, 3, "Unified", "16384K", node)
        blocks.append([("processor", cpu), ("vendor_id", "AuthenticAMD"), ("cpu family", 25), ("model", 1),
                       ("model name", "AMD EPYC 7B13"), ("stepping", 0), ("microcode", "0x1000065"),
                       ("cpu MHz", "2449.998"), ("physical id", 0), ("core id", core), ("flags", flags)])
    files["/proc/cpuinfo"] = cpuinfo(blocks)
    write(root / "linux-x86-amd-gce", files)
    (root / "linux-x86-amd-gce/meta.toml").write_text(
        META.format(note="AMD EPYC VM on Google Compute Engine (8 cores x 2 threads, 2 NUMA nodes), not captured from hardware"))


if __name__ == "__main__":
    root = pathlib.Path(sys.argv[1])
    intel_hybrid(root)
    amd_gce(root)
